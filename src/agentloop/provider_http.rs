//! The production provider adapter: the loop's ONE real egress, over the
//! locked `reqwest` dependency. The seam stays dependency-free and typed
//! ([`LlmProvider`]); this module maps the wire onto that vocabulary — it
//! never invents one.
//!
//! Trust posture: provider output is UNTRUSTED INPUT. It enters the loop
//! only as typed streamed deltas; context shaping, payload caps, and the
//! hook boundaries apply downstream of this seam. Key material rides the
//! `secret_file` path — never source, never logs, never the loop. The
//! endpoint is screened for SSRF at construction (the webhook sink-screen
//! precedent: every resolved address must be globally routable; the client
//! is DNS-pinned to the validated set, closing rebinding). No retries in
//! the seam (a failed stream is the caller's typed error), no silent
//! fallback (a missing/unreachable provider is a named
//! [`ProviderError::Unavailable`]), no env knobs (constructor config only).
//!
//! Wire dialect (bounded, own contract): POST the canonical
//! `ProviderRequest` serde shape plus `model`; read `text/event-stream`
//! frames whose `data:` lines carry exactly one JSON object:
//! `{"type":"message_start"}`, `{"type":"text_delta","text":…}`,
//! `{"type":"tool_call_delta","id":…,"name":…,"arguments_delta":…}`,
//! `{"type":"message_end","stop_reason":"end_turn"|"tool_use"|"max_tokens",
//! "usage":{"input_tokens":N,"output_tokens":N}}`, or
//! `{"type":"error","message":…}`. Anything else is a named refusal.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio_stream::StreamExt;

use super::provider::{
    LlmProvider, ProviderError, ProviderRequest, STREAM_CHANNEL_CAP, StopReason, StreamEvent, Usage,
};
use crate::webhook;

/// Default whole-response byte bound (4 MiB) — tied to the session
/// payload-cap scale: the assembled text of a response is bounded by the
/// loop's session caps; the raw SSE envelope around it stays an order of
/// magnitude under this ceiling.
pub(crate) const DEFAULT_MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

/// A single SSE line above this size is refused BEFORE accumulation —
/// the bounded parser's per-frame bound (a model delta never needs a
/// megabyte line; a wall of text that size is hostile or broken).
pub(crate) const MAX_SSE_LINE_BYTES: usize = 64 * 1024;

/// Externally configured constructor input for the adapter. Resolved at
/// the authenticated boundary (the case handler); nothing here is read
/// from the environment inside the loop.
#[derive(Debug, Clone)]
pub(crate) struct HttpProviderConfig {
    /// The provider endpoint (base URL, screened at construction).
    pub base_url: String,
    /// Model identifier sent with every request.
    pub model: String,
    /// The Authorization header value (from the secret-file seam).
    pub auth_header: String,
    /// Connect-phase timeout.
    pub connect_timeout: Duration,
    /// Per-read idle timeout (first byte and every subsequent read —
    /// a stalled stream refuses instead of hanging).
    pub first_byte_timeout: Duration,
    /// Whole-response byte ceiling (SSE bytes, headers excluded).
    pub max_response_bytes: usize,
}

/// The screened, DNS-pinned, bounded HTTP transport implementing
/// [`LlmProvider`]. Kernel-side by design: the SDK gains nothing.
pub(crate) struct HttpProvider {
    label: String,
    url: String,
    model: String,
    auth_header: String,
    client: reqwest::Client,
    max_response_bytes: usize,
}

impl HttpProvider {
    /// Production constructor: screen `base_url` (private/loopback/
    /// metadata refused, every resolved address validated), pin the
    /// client to the validated address set, apply the configured bounds.
    pub(crate) fn new(cfg: HttpProviderConfig) -> Result<Arc<Self>, ProviderError> {
        let (host, port) = split_host_port(&cfg.base_url)?;
        let addrs = webhook::resolve_and_validate_sink(&host, port, false).map_err(|e| {
            ProviderError::Refused(format!("endpoint refused by egress screen: {e}"))
        })?;
        let client = Self::build_client(&cfg, Some((&host, &addrs)));
        Ok(Arc::new(Self {
            label: "provider_http".to_string(),
            url: cfg.base_url,
            model: cfg.model,
            auth_header: cfg.auth_header,
            client,
            max_response_bytes: cfg.max_response_bytes,
        }))
    }

    /// Test-only constructor: pins the adapter to an exact socket (the
    /// in-process SSE server on 127.0.0.1:0) WITHOUT the egress screen —
    /// the screen's production posture is unchanged; tests must reach a
    /// loopback server, production must never.
    #[cfg(test)]
    pub(crate) fn new_unscreened(cfg: HttpProviderConfig, addr: std::net::SocketAddr) -> Arc<Self> {
        let host = addr.ip().to_string();
        let client = Self::build_client(&cfg, Some((&host, &[addr])));
        Arc::new(Self {
            label: "provider_http_test".to_string(),
            url: cfg.base_url,
            model: cfg.model,
            auth_header: cfg.auth_header,
            client,
            max_response_bytes: cfg.max_response_bytes,
        })
    }

    fn build_client(
        cfg: &HttpProviderConfig,
        pin: Option<(&str, &[std::net::SocketAddr])>,
    ) -> reqwest::Client {
        let mut builder = reqwest::Client::builder()
            // Redirects are never followed: the screen validated ONE
            // endpoint; a redirect is a different endpoint (named refusal).
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(cfg.connect_timeout)
            .read_timeout(cfg.first_byte_timeout);
        if let Some((host, addrs)) = pin {
            builder = builder.resolve_to_addrs(host, addrs);
        }
        builder
            .build()
            .expect("provider http client has no invalid defaults")
    }
}

impl LlmProvider for HttpProvider {
    fn name(&self) -> &str {
        &self.label
    }

    fn stream(
        &self,
        req: ProviderRequest,
    ) -> Result<mpsc::Receiver<Result<StreamEvent, ProviderError>>, ProviderError> {
        let (tx, rx) = mpsc::channel(STREAM_CHANNEL_CAP);
        let url = self.url.clone();
        let model = self.model.clone();
        let auth = self.auth_header.clone();
        let client = self.client.clone();
        let max_bytes = self.max_response_bytes;
        let body = serde_json::json!({
            "model": model,
            "system_prompt": req.system_prompt,
            "messages": req.messages,
            "tools": req.tools,
        });
        tokio::spawn(async move {
            let response = match client
                .post(&url)
                .header("authorization", &auth)
                .header("accept", "text/event-stream")
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx
                        .send(Err(ProviderError::Unavailable(format!(
                            "provider request failed: {e}"
                        ))))
                        .await;
                    return;
                }
            };
            let status = response.status();
            if status.is_client_error() {
                let _ = tx
                    .send(Err(ProviderError::Refused(format!(
                        "provider refused the request (HTTP {status})"
                    ))))
                    .await;
                return;
            }
            if !(status.is_success()) {
                let _ = tx
                    .send(Err(ProviderError::Unavailable(format!(
                        "provider unavailable (HTTP {status})"
                    ))))
                    .await;
                return;
            }
            let mut stream = response.bytes_stream();
            let mut buf: Vec<u8> = Vec::new();
            let mut ended = false;
            // The annotation is load-bearing: the numeric literal's type is
            // only fixed by the saturating_add against a usize below, and
            // inference cannot see through the method receiver.
            let mut total: usize = 0;
            while let Some(item) = stream.next().await {
                let chunk = match item {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = tx
                            .send(Err(ProviderError::Unavailable(format!(
                                "provider stream failed: {e}"
                            ))))
                            .await;
                        return;
                    }
                };
                total = total.saturating_add(chunk.len());
                if total > max_bytes {
                    let _ = tx
                        .send(Err(ProviderError::Refused(format!(
                            "response exceeded max_response_bytes ({max_bytes} bytes): \
                             refusing, never truncating"
                        ))))
                        .await;
                    return;
                }
                buf.extend_from_slice(&chunk);
                while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                    let line: Vec<u8> = buf.drain(..=pos).collect();
                    if line.len() > MAX_SSE_LINE_BYTES {
                        let _ = tx
                            .send(Err(ProviderError::Refused(format!(
                                "oversized SSE frame: {} bytes exceeds the \
                                 {MAX_SSE_LINE_BYTES}-byte line bound",
                                line.len()
                            ))))
                            .await;
                        return;
                    }
                    match parse_line(&line) {
                        Parsed::Ignore => {}
                        Parsed::Event(event) => {
                            if matches!(event, StreamEvent::MessageEnd { .. }) {
                                ended = true;
                            }
                            if tx.send(Ok(event)).await.is_err() {
                                // Consumer dropped the stream — the
                                // drop-cancel contract; the producer exits.
                                return;
                            }
                        }
                        Parsed::Refuse(e) => {
                            let _ = tx.send(Err(e)).await;
                            return;
                        }
                    }
                }
                if ended {
                    // The exchange is over; trailing body bytes are not read.
                    return;
                }
            }
            // Stream ended without message_end: the server closed
            // mid-stream — the drop-cancel-compatible acknowledgement.
            if !ended {
                let _ = tx.send(Err(ProviderError::Cancelled)).await;
            }
        });
        Ok(rx)
    }
}

fn split_host_port(url: &str) -> Result<(String, u16), ProviderError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|e| ProviderError::Refused(format!("endpoint unparseable: {e}")))?;
    if parsed.scheme() != "https" && parsed.scheme() != "http" {
        return Err(ProviderError::Refused(format!(
            "endpoint scheme must be http(s), got {:?}",
            parsed.scheme()
        )));
    }
    let port = parsed.port_or_known_default().unwrap_or(80);
    let raw = parsed
        .host_str()
        .ok_or_else(|| ProviderError::Refused("endpoint has no host".to_string()))?;
    let host = raw
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(raw)
        .to_ascii_lowercase();
    Ok((host, port))
}

/// One parsed SSE line: a loop event, an ignorable framing line, or a
/// named refusal (malformed/oversized/hostile payloads NEVER pass
/// silently — the RED run against the naive parser demonstrated the
/// silent-drop class this parser refuses).
enum Parsed {
    Event(StreamEvent),
    Ignore,
    Refuse(ProviderError),
}

/// The bounded per-line parse: extract `data:` lines and map them onto
/// the dialect. Any deviation — non-UTF8 bytes, invalid JSON, unknown
/// event type, missing/unknown stop_reason, a provider `error` frame —
/// is a named [`ProviderError::Refused`] carrying the reason.
fn parse_line(line: &[u8]) -> Parsed {
    let s = match std::str::from_utf8(line) {
        Ok(s) => s.trim_end_matches(['\n', '\r']),
        Err(_) => {
            return Parsed::Refuse(ProviderError::Refused(
                "malformed SSE payload: line is not UTF-8".to_string(),
            ));
        }
    };
    let Some(data) = s.strip_prefix("data:") else {
        // Non-data lines (SSE comments, `event:`/`id:`/`retry:` framing)
        // are ignorable under this dialect: each `data:` line is
        // self-contained.
        return Parsed::Ignore;
    };
    let payload = data.trim_start();
    if payload.is_empty() {
        return Parsed::Ignore;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(payload) else {
        return Parsed::Refuse(ProviderError::Refused(format!(
            "malformed SSE payload: not valid JSON: {payload}"
        )));
    };
    let Some(kind) = v.get("type").and_then(|t| t.as_str()) else {
        return Parsed::Refuse(ProviderError::Refused(
            "malformed SSE payload: event has no \"type\"".to_string(),
        ));
    };
    match kind {
        "message_start" => Parsed::Event(StreamEvent::MessageStart),
        "text_delta" => v.get("text").and_then(|t| t.as_str()).map_or_else(
            || {
                Parsed::Refuse(ProviderError::Refused(
                    "malformed SSE payload: text_delta without \"text\"".to_string(),
                ))
            },
            |text| Parsed::Event(StreamEvent::TextDelta(text.to_string())),
        ),
        "tool_call_delta" => {
            let field = |name: &str| v.get(name).and_then(|x| x.as_str()).map(str::to_string);
            match (field("id"), field("name"), field("arguments_delta")) {
                (Some(id), Some(name), Some(arguments_delta)) => {
                    Parsed::Event(StreamEvent::ToolCallDelta {
                        id,
                        name,
                        arguments_delta,
                    })
                }
                _ => Parsed::Refuse(ProviderError::Refused(
                    "malformed SSE payload: tool_call_delta needs \
                     \"id\", \"name\" and \"arguments_delta\""
                        .to_string(),
                )),
            }
        }
        "message_end" => {
            let stop_reason = match v.get("stop_reason").and_then(|s| s.as_str()) {
                Some("end_turn") => StopReason::EndTurn,
                Some("tool_use") => StopReason::ToolUse,
                Some("max_tokens") => StopReason::MaxTokens,
                _ => {
                    return Parsed::Refuse(ProviderError::Refused(
                        "malformed SSE payload: message_end without a known \
                         \"stop_reason\" (end_turn|tool_use|max_tokens)"
                            .to_string(),
                    ));
                }
            };
            let usage = match v
                .get("usage")
                .map(|u| serde_json::from_value::<Usage>(u.clone()))
            {
                Some(Ok(u)) => u,
                Some(Err(e)) => {
                    return Parsed::Refuse(ProviderError::Refused(format!(
                        "malformed SSE payload: usage: {e}"
                    )));
                }
                None => Usage::default(),
            };
            Parsed::Event(StreamEvent::MessageEnd { stop_reason, usage })
        }
        "error" => Parsed::Refuse(ProviderError::Refused(
            v.get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("provider returned an error frame without a message")
                .to_string(),
        )),
        other => Parsed::Refuse(ProviderError::Refused(format!(
            "malformed SSE payload: unknown event type {other:?}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    fn cfg(url: String) -> HttpProviderConfig {
        HttpProviderConfig {
            base_url: url,
            model: "pilot-model".to_string(),
            auth_header: "Bearer test-key".to_string(),
            connect_timeout: Duration::from_secs(2),
            first_byte_timeout: Duration::from_secs(5),
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
        }
    }

    /// What the scripted endpoint does on POST.
    enum Behavior {
        /// 200 + a chunked SSE body carrying exactly these lines, then a
        /// clean close (a clean close WITHOUT a `message_end` line is the
        /// mid-stream-close case).
        Sse(Vec<String>),
        /// A bare HTTP status, empty body.
        Status(u16),
        /// Record the raw request bytes, then serve start+end SSE frames.
        RecordThenSse,
    }

    static RECORDED_REQUEST: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

    /// Read one HTTP/1.1 request off the socket: headers, then the
    /// content-length body. Bounded by the loopback test frame.
    async fn read_request(sock: &mut tokio::net::TcpStream) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
            let n = sock.read(&mut chunk).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        let head_end = buf
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|p| p + 4)
            .unwrap_or(buf.len());
        let head = String::from_utf8_lossy(&buf[..head_end]).to_ascii_lowercase();
        let len: usize = head
            .lines()
            .find_map(|l| l.strip_prefix("content-length:"))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0);
        while buf.len() < head_end + len {
            let n = sock.read(&mut chunk).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        buf
    }

    /// Bind 127.0.0.1:0, serve ONE scripted response (the adapter opens
    /// exactly one connection per stream()), return the bound address.
    async fn spawn_server(behavior: Behavior) -> (SocketAddr, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let request = read_request(&mut sock).await;
            if matches!(behavior, Behavior::RecordThenSse) {
                RECORDED_REQUEST
                    .lock()
                    .unwrap()
                    .replace(String::from_utf8_lossy(&request).to_string());
            }
            let lines = match behavior {
                Behavior::Status(code) => {
                    let head = format!(
                        "HTTP/1.1 {code} NOPE\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                    );
                    let _ = sock.write_all(head.as_bytes()).await;
                    let _ = sock.shutdown().await;
                    return;
                }
                Behavior::RecordThenSse => vec![
                    format!("data: {}\n\n", sse_event("message_start")),
                    format!("data: {}\n\n", sse_event("message_end")),
                ],
                Behavior::Sse(lines) => lines,
            };
            let mut body = String::from(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
                 transfer-encoding: chunked\r\nconnection: close\r\n\r\n",
            );
            for line in &lines {
                body.push_str(&format!("{:x}\r\n{}\r\n", line.len(), line));
            }
            body.push_str("0\r\n\r\n");
            let _ = sock.write_all(body.as_bytes()).await;
            let _ = sock.shutdown().await;
        });
        (addr, handle)
    }

    fn sse_event(kind: &str) -> String {
        match kind {
            "message_start" => r#"{"type":"message_start"}"#.to_string(),
            "message_end" => {
                r#"{"type":"message_end","stop_reason":"end_turn","usage":{"input_tokens":11,"output_tokens":7}}"#
                    .to_string()
            }
            other => format!(r#"{{"type":"{other}"}}"#),
        }
    }

    async fn spawn_sse(lines: Vec<String>) -> (SocketAddr, tokio::task::JoinHandle<()>) {
        spawn_server(Behavior::Sse(lines)).await
    }

    fn adapter_for(addr: SocketAddr) -> Arc<HttpProvider> {
        HttpProvider::new_unscreened(cfg(format!("http://{addr}/v1/stream")), addr)
    }

    fn request() -> ProviderRequest {
        ProviderRequest {
            system_prompt: "sys".to_string(),
            messages: vec![],
            tools: vec![],
        }
    }

    async fn drain(
        mut rx: mpsc::Receiver<Result<StreamEvent, ProviderError>>,
    ) -> Vec<Result<StreamEvent, ProviderError>> {
        let mut out = Vec::new();
        while let Some(ev) = rx.recv().await {
            out.push(ev);
            if out.len() > 64 {
                break;
            }
        }
        out
    }

    #[tokio::test]
    async fn sse_text_deltas_assemble_in_order() {
        let (addr, server) = spawn_sse(vec![
            format!("data: {}\n\n", sse_event("message_start")),
            "data: {\"type\":\"text_delta\",\"text\":\"hel\"}\n\n".to_string(),
            "data: {\"type\":\"text_delta\",\"text\":\"lo\"}\n\n".to_string(),
            format!("data: {}\n\n", sse_event("message_end")),
        ])
        .await;
        let provider = adapter_for(addr);
        let events = drain(provider.stream(request()).unwrap()).await;
        assert_eq!(
            events,
            vec![
                Ok(StreamEvent::MessageStart),
                Ok(StreamEvent::TextDelta("hel".to_string())),
                Ok(StreamEvent::TextDelta("lo".to_string())),
                Ok(StreamEvent::MessageEnd {
                    stop_reason: StopReason::EndTurn,
                    usage: Usage {
                        input_tokens: 11,
                        output_tokens: 7
                    }
                }),
            ],
            "deltas assemble in arrival order with start/end framing"
        );
        server.abort();
    }

    #[tokio::test]
    async fn sse_tool_call_deltas_assemble() {
        let (addr, server) = spawn_sse(vec![
            format!("data: {}\n\n", sse_event("message_start")),
            "data: {\"type\":\"tool_call_delta\",\"id\":\"c1\",\"name\":\"read\",\"arguments_delta\":\"{\\\"p\\\"\"}\n\n".to_string(),
            "data: {\"type\":\"tool_call_delta\",\"id\":\"c1\",\"name\":\"read\",\"arguments_delta\":\":1}\"}\n\n".to_string(),
            "data: {\"type\":\"message_end\",\"stop_reason\":\"tool_use\",\"usage\":{\"input_tokens\":3,\"output_tokens\":2}}\n\n".to_string(),
        ])
        .await;
        let provider = adapter_for(addr);
        let events = drain(provider.stream(request()).unwrap()).await;
        assert_eq!(events.len(), 4, "start + 2 deltas + end");
        assert_eq!(
            events[1],
            Ok(StreamEvent::ToolCallDelta {
                id: "c1".to_string(),
                name: "read".to_string(),
                arguments_delta: "{\"p\"".to_string(),
            })
        );
        assert_eq!(
            events[2],
            Ok(StreamEvent::ToolCallDelta {
                id: "c1".to_string(),
                name: "read".to_string(),
                arguments_delta: ":1}".to_string(),
            })
        );
        assert!(
            matches!(
                events[3],
                Ok(StreamEvent::MessageEnd {
                    stop_reason: StopReason::ToolUse,
                    ..
                })
            ),
            "tool_use stop maps onto the existing StopReason"
        );
        server.abort();
    }

    #[tokio::test]
    async fn sse_server_close_mid_stream_maps_to_cancelled() {
        // The body ends cleanly after a start + one delta, with no
        // message_end: the server closed mid-stream.
        let (addr, server) = spawn_sse(vec![
            format!("data: {}\n\n", sse_event("message_start")),
            "data: {\"type\":\"text_delta\",\"text\":\"partial\"}\n\n".to_string(),
        ])
        .await;
        let provider = adapter_for(addr);
        let events = drain(provider.stream(request()).unwrap()).await;
        assert_eq!(events.len(), 3, "start + delta + the close acknowledgement");
        assert_eq!(events[2], Err(ProviderError::Cancelled));
        server.abort();
    }

    #[tokio::test]
    async fn connect_refused_maps_to_unavailable() {
        // Bind a socket, learn its port, drop it: a guaranteed-closed
        // loopback port (no server, no DNS, no external network).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let provider = adapter_for(addr);
        let events = drain(provider.stream(request()).unwrap()).await;
        assert!(
            matches!(events.as_slice(), [Err(ProviderError::Unavailable(_))]),
            "connect-refused must name Unavailable, got {events:?}"
        );
    }

    #[tokio::test]
    async fn http_5xx_maps_to_unavailable() {
        let (addr, server) = spawn_server(Behavior::Status(500)).await;
        let provider = adapter_for(addr);
        let events = drain(provider.stream(request()).unwrap()).await;
        assert!(
            matches!(events.as_slice(), [Err(ProviderError::Unavailable(_))]),
            "5xx must name Unavailable, got {events:?}"
        );
        server.abort();
    }

    #[tokio::test]
    async fn http_4xx_maps_to_refused() {
        let (addr, server) = spawn_server(Behavior::Status(401)).await;
        let provider = adapter_for(addr);
        let events = drain(provider.stream(request()).unwrap()).await;
        assert!(
            matches!(events.as_slice(), [Err(ProviderError::Refused(_))]),
            "4xx must name Refused, got {events:?}"
        );
        server.abort();
    }

    #[tokio::test]
    async fn oversized_line_is_refused_not_truncated() {
        let big = "x".repeat(MAX_SSE_LINE_BYTES + 1);
        let (addr, server) = spawn_sse(vec![format!(
            "data: {{\"type\":\"text_delta\",\"text\":\"{big}\"}}\n\n"
        )])
        .await;
        let provider = adapter_for(addr);
        let events = drain(provider.stream(request()).unwrap()).await;
        assert!(
            matches!(
                events.as_slice(),
                [.., Err(ProviderError::Refused(m))] if m.contains("oversized"),
            ),
            "an oversized frame must be a NAMED refusal, got {:?}",
            events.last()
        );
        server.abort();
    }

    #[tokio::test]
    async fn response_beyond_max_bytes_is_refused() {
        // Two lines: each under the per-line cap, together far beyond a
        // tiny configured whole-response cap.
        let half = "y".repeat(4096);
        let (addr, server) = spawn_sse(vec![
            format!("data: {{\"type\":\"text_delta\",\"text\":\"{half}\"}}\n\n"),
            format!("data: {{\"type\":\"text_delta\",\"text\":\"{half}\"}}\n\n"),
            format!("data: {}\n\n", sse_event("message_end")),
        ])
        .await;
        let mut c = cfg(format!("http://{addr}/v1/stream"));
        c.max_response_bytes = 4096;
        let provider = HttpProvider::new_unscreened(c, addr);
        let events = drain(provider.stream(request()).unwrap()).await;
        assert!(
            matches!(
                events.as_slice(),
                [.., Err(ProviderError::Refused(m))] if m.contains("max_response_bytes"),
            ),
            "a response beyond the cap must be a NAMED refusal, got {:?}",
            events.last()
        );
        server.abort();
    }

    #[tokio::test]
    async fn malformed_sse_is_named_refusal() {
        let (addr, server) = spawn_sse(vec![
            "data: {not json}\n\n".to_string(),
            "data: {\"type\":\"text_delta\",\"text\":\"after\"}\n\n".to_string(),
        ])
        .await;
        let provider = adapter_for(addr);
        let events = drain(provider.stream(request()).unwrap()).await;
        assert!(
            matches!(
                events.as_slice(),
                [Err(ProviderError::Refused(m)), ..] if m.contains("malformed"),
            ),
            "malformed SSE must be a NAMED refusal, got {events:?}"
        );
        server.abort();
    }

    #[tokio::test]
    async fn request_carries_model_and_canonical_shape() {
        RECORDED_REQUEST.lock().unwrap().take();
        let (addr, server) = spawn_server(Behavior::RecordThenSse).await;
        let provider = adapter_for(addr);
        let req = ProviderRequest {
            system_prompt: "the system".to_string(),
            messages: vec![crate::agentloop::provider::ChatMessage::User {
                text: "hi".to_string(),
            }],
            tools: vec![],
        };
        let events = drain(provider.stream(req).unwrap()).await;
        assert!(
            events.iter().all(|e| e.is_ok()),
            "serves cleanly: {events:?}"
        );
        let raw = RECORDED_REQUEST.lock().unwrap().clone().unwrap();
        let body = raw
            .split_once("\r\n\r\n")
            .map(|(_, b)| b.to_string())
            .unwrap_or(raw.clone());
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["model"], "pilot-model");
        assert_eq!(v["system_prompt"], "the system");
        assert_eq!(v["messages"][0]["type"], "user");
        assert_eq!(
            v["messages"][0]["text"], "hi",
            "the canonical ProviderRequest serde shape rides the wire"
        );
        server.abort();
    }

    #[test]
    fn screen_refuses_loopback_endpoint() {
        // The production screen refuses a loopback endpoint BEFORE any
        // request exists — the SSRF posture the tests bypass explicitly.
        let err = match HttpProvider::new(cfg("http://127.0.0.1:9/v1/stream".to_string())) {
            Err(e) => e,
            Ok(_) => panic!("loopback endpoint must be refused at construction"),
        };
        assert!(
            matches!(err, ProviderError::Refused(ref m) if m.contains("egress screen")),
            "loopback endpoint refused at construction: {err}"
        );
    }

    #[test]
    fn screen_refuses_private_and_metadata_endpoints() {
        for url in [
            "http://10.0.0.1:9/v1/stream",
            "http://192.168.1.1:9/v1/stream",
            "http://metadata.amazonaws.com/v1/stream",
        ] {
            let err = match HttpProvider::new(cfg(url.to_string())) {
                Err(e) => e,
                Ok(_) => panic!("{url} must be refused at construction"),
            };
            assert!(
                matches!(err, ProviderError::Refused(ref m) if m.contains("egress screen")),
                "{url} refused at construction: {err}"
            );
        }
    }
}

/// The scripted multi-turn SSE endpoint for composed-app tests: REAL
/// HTTP/1.1 over a bound socket, one connection per `stream()` call, each
/// connection served the next scripted turn. Lives here so the adapter
/// tests and the route-level tests drive the SAME wire shape.
#[cfg(test)]
pub(crate) mod scripted_server {
    use std::net::SocketAddr;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// One scripted exchange: start + a single text delta (the artifact
    /// verbatim) + end_turn with zero usage — the same event shape the
    /// loopback fixture's `scripted_text` produces.
    pub(crate) fn text_turn(artifact: &str) -> Vec<String> {
        vec![
            "data: {\"type\":\"message_start\"}\n\n".to_string(),
            format!(
                "data: {{\"type\":\"text_delta\",\"text\":{}}}\n\n",
                serde_json::to_string(artifact).unwrap()
            ),
            "data: {\"type\":\"message_end\",\"stop_reason\":\"end_turn\",\"usage\":{\"input_tokens\":0,\"output_tokens\":0}}\n\n"
                .to_string(),
        ]
    }

    /// One scripted exchange that asks for a tool (the fail-closed
    /// unknown-tool path).
    pub(crate) fn tool_use_turn(id: &str, name: &str) -> Vec<String> {
        vec![
            "data: {\"type\":\"message_start\"}\n\n".to_string(),
            format!(
                "data: {{\"type\":\"tool_call_delta\",\"id\":\"{id}\",\"name\":\"{name}\",\"arguments_delta\":\"{{}}\"}}\n\n"
            ),
            "data: {\"type\":\"message_end\",\"stop_reason\":\"tool_use\",\"usage\":{\"input_tokens\":0,\"output_tokens\":0}}\n\n"
                .to_string(),
        ]
    }

    async fn read_request(sock: &mut tokio::net::TcpStream) {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
            let n = sock.read(&mut chunk).await.unwrap_or(0);
            if n == 0 {
                return;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        let head_end = buf
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|p| p + 4)
            .unwrap_or(buf.len());
        let head = String::from_utf8_lossy(&buf[..head_end]).to_ascii_lowercase();
        let len: usize = head
            .lines()
            .find_map(|l| l.strip_prefix("content-length:"))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0);
        while buf.len() < head_end + len {
            let n = sock.read(&mut chunk).await.unwrap_or(0);
            if n == 0 {
                return;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
    }

    /// Serve the scripted turns sequentially: one HTTP request per turn,
    /// `connection: close` between turns (the client cannot pool).
    pub(crate) async fn spawn_turns(turns: Vec<Vec<String>>) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let mut queue = std::collections::VecDeque::from(turns);
            while let Some(lines) = queue.pop_front() {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                read_request(&mut sock).await;
                let mut body = String::from(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
                     transfer-encoding: chunked\r\nconnection: close\r\n\r\n",
                );
                for line in &lines {
                    body.push_str(&format!("{:x}\r\n{}\r\n", line.len(), line));
                }
                body.push_str("0\r\n\r\n");
                let _ = sock.write_all(body.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });
        addr
    }
}
