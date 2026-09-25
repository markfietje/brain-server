//! The production provider adapter: the loop's ONE real egress, over the
//! locked `reqwest` dependency. The seam stays dependency-free and typed
//! ([`LlmProvider`]); this module maps the wire onto that vocabulary — it
//! never invents one.
//!
//! Trust posture: provider output is UNTRUSTED INPUT. It enters the loop
//! only as typed streamed deltas; context shaping, payload caps, and the
//! hook boundaries apply downstream of this seam. Key material rides the
//! server-owned, root-confined secret path — never source, never logs, never
//! the loop. The
//! endpoint is screened for SSRF at construction (the webhook sink-screen
//! precedent: every resolved address must be globally routable; the client
//! is DNS-pinned to the validated set, closing rebinding). No retries in
//! the seam (a failed stream is the caller's typed error), no silent
//! fallback (a missing/unreachable provider is a named
//! [`ProviderError::Unavailable`]), no request-selected configuration
//! (constructor config only).
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
/// Provider endpoints are operator configuration, not arbitrary request
/// strings. Keep URL-shape validation bounded before DNS or secret work.
const MAX_ENDPOINT_URL_BYTES: usize = 2048;
const MAX_PROVIDER_MODEL_BYTES: usize = 256;
const MAX_PROVIDER_SECRET_HEADER_BYTES: usize = 16 * 1024 + 16;

/// Externally configured constructor input for the adapter. Resolved at
/// the authenticated boundary (the case handler); nothing here is read
/// from the environment inside the loop.
#[derive(Clone)]
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
    /// a stalled read refuses instead of hanging).
    pub first_byte_timeout: Duration,
    /// Total wall-clock deadline from request start through response-body end.
    pub total_timeout: Duration,
    /// Whole-response byte ceiling (SSE bytes, headers excluded).
    pub max_response_bytes: usize,
}

impl std::fmt::Debug for HttpProviderConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HttpProviderConfig")
            .field("base_url", &"[REDACTED]")
            .field("model", &self.model)
            .field("auth_header", &"[REDACTED]")
            .field("connect_timeout", &self.connect_timeout)
            .field("first_byte_timeout", &self.first_byte_timeout)
            .field("total_timeout", &self.total_timeout)
            .field("max_response_bytes", &self.max_response_bytes)
            .finish()
    }
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
    /// Production constructor: validate endpoint shape, require HTTPS,
    /// screen `base_url` (private/loopback/metadata refused, every resolved
    /// address validated), pin the client to the validated address set, and
    /// apply the configured bounds.
    pub(crate) fn new(cfg: HttpProviderConfig) -> Result<Arc<Self>, ProviderError> {
        Self::validate_config(&cfg)?;
        let (host, port) = split_host_port(&cfg.base_url)?;
        let addrs = webhook::resolve_and_validate_sink(&host, port, false)
            .map_err(|_| ProviderError::Refused)?;
        let client = Self::build_client(&cfg, Some((&host, &addrs)), true);
        Ok(Arc::new(Self {
            label: "provider_http".to_string(),
            url: cfg.base_url,
            model: cfg.model,
            auth_header: cfg.auth_header,
            client,
            max_response_bytes: cfg.max_response_bytes,
        }))
    }

    /// Validate only the endpoint shape. The GDL boundary calls this before
    /// reading a secret; DNS/address screening remains in `new`.
    pub(crate) fn validate_endpoint(url: &str) -> Result<(), ProviderError> {
        split_host_port(url).map(|_| ())
    }

    fn validate_config(cfg: &HttpProviderConfig) -> Result<(), ProviderError> {
        Self::validate_endpoint(&cfg.base_url)?;
        if cfg.model.trim().is_empty()
            || cfg.model.len() > MAX_PROVIDER_MODEL_BYTES
            || cfg.model.chars().any(char::is_control)
        {
            return Err(ProviderError::Refused);
        }
        if cfg.auth_header.is_empty()
            || cfg.auth_header.len() > MAX_PROVIDER_SECRET_HEADER_BYTES
            || cfg
                .auth_header
                .chars()
                .any(|character| character == '\r' || character == '\n' || character.is_control())
        {
            return Err(ProviderError::Refused);
        }
        if cfg.max_response_bytes == 0 || cfg.total_timeout.is_zero() {
            return Err(ProviderError::Refused);
        }
        Ok(())
    }

    /// Test-only constructor: pins the adapter to an exact socket (the
    /// in-process SSE server on 127.0.0.1:0) WITHOUT the egress screen —
    /// the screen's production posture is unchanged; tests must reach a
    /// loopback server, production must never.
    #[cfg(test)]
    pub(crate) fn new_unscreened(cfg: HttpProviderConfig, addr: std::net::SocketAddr) -> Arc<Self> {
        let host = addr.ip().to_string();
        let client = Self::build_client(&cfg, Some((&host, &[addr])), false);
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
        require_https: bool,
    ) -> reqwest::Client {
        let mut builder = reqwest::Client::builder()
            // Redirects are never followed: the screen validated ONE
            // endpoint; a redirect is a different endpoint (named refusal).
            .redirect(reqwest::redirect::Policy::none())
            // The validated address set is meaningful only for a direct
            // connection; never let ambient proxy variables bypass the pin.
            .no_proxy()
            .connect_timeout(cfg.connect_timeout)
            .read_timeout(cfg.first_byte_timeout)
            .timeout(cfg.total_timeout);
        if require_https {
            builder = builder.https_only(true);
        }
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
            let response = tokio::select! {
                biased;
                _ = tx.closed() => return,
                result = client
                    .post(&url)
                    .header("authorization", &auth)
                    .header("accept", "text/event-stream")
                    .json(&body)
                    .send() => result,
            };
            let response = match response {
                Ok(r) => r,
                Err(error) => {
                    let failure = if error.is_timeout() {
                        ProviderError::Timeout
                    } else {
                        ProviderError::Unavailable
                    };
                    let _ = tx.send(Err(failure)).await;
                    return;
                }
            };
            let status = response.status();
            if status.is_client_error() {
                let _ = tx.send(Err(ProviderError::Refused)).await;
                return;
            }
            if !status.is_success() {
                let _ = tx.send(Err(ProviderError::Unavailable)).await;
                return;
            }
            let mut stream = response.bytes_stream();
            let mut buf: Vec<u8> = Vec::new();
            let mut ended = false;
            // The literal suffix is load-bearing: the accumulator's type is
            // only fixed by the saturating_add against a usize below, and
            // inference cannot see through the method receiver.
            let mut total = 0usize;
            loop {
                let item = tokio::select! {
                    biased;
                    _ = tx.closed() => return,
                    item = stream.next() => item,
                };
                let Some(item) = item else {
                    if !ended {
                        let _ = tx.send(Err(ProviderError::Malformed)).await;
                    }
                    return;
                };
                let chunk = match item {
                    Ok(c) => c,
                    Err(error) => {
                        let failure = if error.is_timeout() {
                            ProviderError::Timeout
                        } else {
                            ProviderError::Unavailable
                        };
                        let _ = tx.send(Err(failure)).await;
                        return;
                    }
                };
                total = total.saturating_add(chunk.len());
                if total > max_bytes {
                    let _ = tx.send(Err(ProviderError::Malformed)).await;
                    return;
                }
                buf.extend_from_slice(&chunk);
                while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                    let line: Vec<u8> = buf.drain(..=pos).collect();
                    if line.len() > MAX_SSE_LINE_BYTES {
                        let _ = tx.send(Err(ProviderError::Malformed)).await;
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
            // A stream that ends without message_end is a provider protocol
            // failure, not consumer cancellation.
        });
        Ok(rx)
    }
}

fn split_host_port(url: &str) -> Result<(String, u16), ProviderError> {
    if url.len() > MAX_ENDPOINT_URL_BYTES || url.bytes().any(|byte| byte <= b' ' || byte == 0x7f) {
        return Err(ProviderError::Refused);
    }
    let Some(authority) = url.strip_prefix("https://") else {
        return Err(ProviderError::Refused);
    };
    if authority.is_empty() || authority.starts_with('/') {
        return Err(ProviderError::Refused);
    }
    let parsed = reqwest::Url::parse(url).map_err(|_| ProviderError::Refused)?;
    if parsed.scheme() != "https" {
        return Err(ProviderError::Refused);
    }
    let authority_head = authority.split('/').next().unwrap_or_default();
    if authority_head.contains('@')
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
        || parsed.query().is_some()
    {
        return Err(ProviderError::Refused);
    }
    let port = parsed.port_or_known_default().unwrap_or(443);
    let raw = parsed.host_str().ok_or(ProviderError::Refused)?;
    if raw.is_empty() || raw.chars().any(char::is_whitespace) {
        return Err(ProviderError::Refused);
    }
    let host = raw.to_ascii_lowercase();
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
        Err(_) => return Parsed::Refuse(ProviderError::Malformed),
    };
    let Some(data) = s.strip_prefix("data:") else {
        return Parsed::Ignore;
    };
    let payload = data.trim_start();
    if payload.is_empty() {
        return Parsed::Ignore;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(payload) else {
        return Parsed::Refuse(ProviderError::Malformed);
    };
    let Some(kind) = v.get("type").and_then(|t| t.as_str()) else {
        return Parsed::Refuse(ProviderError::Malformed);
    };
    match kind {
        "message_start" => Parsed::Event(StreamEvent::MessageStart),
        "text_delta" => v.get("text").and_then(|t| t.as_str()).map_or_else(
            || Parsed::Refuse(ProviderError::Malformed),
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
                _ => Parsed::Refuse(ProviderError::Malformed),
            }
        }
        "message_end" => {
            let stop_reason = match v.get("stop_reason").and_then(|s| s.as_str()) {
                Some("end_turn") => StopReason::EndTurn,
                Some("tool_use") => StopReason::ToolUse,
                Some("max_tokens") => StopReason::MaxTokens,
                _ => return Parsed::Refuse(ProviderError::Malformed),
            };
            let usage = match v
                .get("usage")
                .map(|u| serde_json::from_value::<Usage>(u.clone()))
            {
                Some(Ok(u)) => u,
                Some(Err(_)) => return Parsed::Refuse(ProviderError::Malformed),
                None => Usage::default(),
            };
            Parsed::Event(StreamEvent::MessageEnd { stop_reason, usage })
        }
        "error" => Parsed::Refuse(ProviderError::Refused),
        _ => Parsed::Refuse(ProviderError::Malformed),
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
            total_timeout: Duration::from_secs(10),
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
        /// Send headers and one start frame, then hold the body open until
        /// the client closes it. Used to prove the body future is dropped.
        HoldAfterStart,
        /// Send a valid partial SSE line repeatedly before the total deadline.
        SlowDrip,
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
            if matches!(&behavior, Behavior::RecordThenSse) {
                RECORDED_REQUEST
                    .lock()
                    .unwrap()
                    .replace(String::from_utf8_lossy(&request).to_string());
            }
            let lines = match &behavior {
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
                Behavior::Sse(lines) => lines.clone(),
                Behavior::HoldAfterStart | Behavior::SlowDrip => Vec::new(),
            };
            let mut body = String::from(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
                 transfer-encoding: chunked\r\nconnection: close\r\n\r\n",
            );
            if matches!(&behavior, Behavior::HoldAfterStart) {
                let start = format!("data: {}\n\n", sse_event("message_start"));
                let _ = sock
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n{:x}\r\n{}\r\n",
                            start.len(),
                            start
                        )
                        .as_bytes(),
                    )
                    .await;
                let mut byte = [0_u8; 1];
                let _ = sock.read(&mut byte).await;
                return;
            }
            if matches!(&behavior, Behavior::SlowDrip) {
                let _ = sock
                    .write_all(
                        b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n",
                    )
                    .await;
                let line = "data: {\"type\":\"text_delta\",\"text\":\"x\"}\n\n";
                let chunk = format!("{:x}\r\n{}\r\n", line.len(), line);
                let mut ticker = tokio::time::interval(Duration::from_millis(10));
                ticker.tick().await;
                loop {
                    if sock.write_all(chunk.as_bytes()).await.is_err() {
                        return;
                    }
                    ticker.tick().await;
                }
            }
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
    async fn gdl_provider_timeout_cancels_in_flight_http_body() {
        let (addr, server) = spawn_server(Behavior::HoldAfterStart).await;
        let mut config = cfg(format!("http://{addr}/v1/stream"));
        config.total_timeout = Duration::from_millis(120);
        config.first_byte_timeout = Duration::from_secs(2);
        let provider = HttpProvider::new_unscreened(config, addr);
        let events = tokio::time::timeout(
            Duration::from_secs(2),
            drain(provider.stream(request()).unwrap()),
        )
        .await
        .expect("provider timeout must be bounded");
        assert!(
            matches!(events.last(), Some(Err(ProviderError::Timeout))),
            "held body must end at the total deadline: {events:?}"
        );
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("provider task must observe the dropped body")
            .expect("test server task");
    }

    #[tokio::test]
    async fn gdl_provider_slow_drip_obeys_total_deadline() {
        let (addr, server) = spawn_server(Behavior::SlowDrip).await;
        let mut config = cfg(format!("http://{addr}/v1/stream"));
        config.total_timeout = Duration::from_millis(120);
        config.first_byte_timeout = Duration::from_secs(2);
        let provider = HttpProvider::new_unscreened(config, addr);
        let started = std::time::Instant::now();
        let events = tokio::time::timeout(
            Duration::from_secs(2),
            drain(provider.stream(request()).unwrap()),
        )
        .await
        .expect("slow-drip response must obey the total deadline");
        assert!(
            matches!(events.last(), Some(Err(ProviderError::Timeout))),
            "slow drip must not reset the total deadline: {events:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "slow-drip response exceeded the deterministic bound"
        );
        server.abort();
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
        assert_eq!(events[2], Err(ProviderError::Malformed));
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
            matches!(events.as_slice(), [Err(ProviderError::Unavailable)]),
            "connect-refused must name Unavailable, got {events:?}"
        );
    }

    #[tokio::test]
    async fn http_5xx_maps_to_unavailable() {
        let (addr, server) = spawn_server(Behavior::Status(500)).await;
        let provider = adapter_for(addr);
        let events = drain(provider.stream(request()).unwrap()).await;
        assert!(
            matches!(events.as_slice(), [Err(ProviderError::Unavailable)]),
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
            matches!(events.as_slice(), [Err(ProviderError::Refused)]),
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
            matches!(events.as_slice(), [.., Err(ProviderError::Malformed)],),
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
            matches!(events.as_slice(), [.., Err(ProviderError::Malformed)],),
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
            matches!(events.as_slice(), [Err(ProviderError::Malformed), ..],),
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
        let err = match HttpProvider::new(cfg("https://127.0.0.1:9/v1/stream".to_string())) {
            Err(e) => e,
            Ok(_) => panic!("loopback endpoint must be refused at construction"),
        };
        assert!(
            matches!(err, ProviderError::Refused),
            "loopback endpoint refused at construction: {err}"
        );
    }

    #[test]
    fn screen_refuses_private_and_metadata_endpoints() {
        for url in [
            "https://10.0.0.1:9/v1/stream",
            "https://192.168.1.1:9/v1/stream",
            "https://metadata.amazonaws.com/v1/stream",
        ] {
            let err = match HttpProvider::new(cfg(url.to_string())) {
                Err(e) => e,
                Ok(_) => panic!("{url} must be refused at construction"),
            };
            assert!(
                matches!(err, ProviderError::Refused),
                "{url} refused at construction: {err}"
            );
        }
    }

    #[test]
    fn gdl_provider_rejects_public_http() {
        let err = split_host_port("http://provider.example/v1/stream")
            .expect_err("production provider endpoints must require HTTPS");
        assert!(
            matches!(err, ProviderError::Refused),
            "HTTP refusal must stay typed: {err}"
        );
    }

    #[tokio::test]
    async fn gdl_provider_keeps_existing_dns_pinning_and_redirect_refusal() {
        let screened = match HttpProvider::new(cfg("https://127.0.0.1:9/v1/stream".to_string())) {
            Ok(_) => panic!("private DNS target must be screened"),
            Err(error) => error,
        };
        assert!(
            matches!(screened, ProviderError::Refused),
            "address screening remains in the production constructor: {screened}"
        );

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let redirect_hit = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let hit = redirect_hit.clone();
        let server = tokio::spawn(async move {
            let (mut first, _) = listener.accept().await.unwrap();
            read_request(&mut first).await;
            let location = format!("http://{addr}/redirected");
            let head = format!(
                "HTTP/1.1 307 Temporary Redirect\r\nlocation: {location}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
            );
            first.write_all(head.as_bytes()).await.unwrap();
            if let Ok(Ok((mut second, _))) =
                tokio::time::timeout(Duration::from_millis(500), listener.accept()).await
            {
                hit.store(true, std::sync::atomic::Ordering::SeqCst);
                let body = "data: {\"type\":\"message_start\"}\n\ndata: {\"type\":\"message_end\",\"stop_reason\":\"end_turn\"}\n\n";
                let head = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                second.write_all(head.as_bytes()).await.unwrap();
                second.write_all(body.as_bytes()).await.unwrap();
            }
        });
        let provider = HttpProvider::new_unscreened(cfg(format!("http://{addr}/v1/stream")), addr);
        let events = drain(provider.stream(request()).unwrap()).await;
        assert!(
            matches!(events.as_slice(), [Err(ProviderError::Unavailable)]),
            "redirect responses are not followed: {events:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !redirect_hit.load(std::sync::atomic::Ordering::SeqCst),
            "the redirect target must never receive a second request"
        );
        server.abort();
    }

    #[test]
    fn gdl_provider_rejects_url_userinfo_and_fragment() {
        for url in [
            "https://user:password@provider.example/v1/stream",
            "https://provider.example/v1/stream#fragment",
            "https://provider.example/v1/stream?token=not-allowed",
        ] {
            assert!(
                split_host_port(url).is_err(),
                "unsafe provider URL shape must be refused: {url}"
            );
        }
    }

    #[test]
    fn gdl_provider_rejects_unsupported_or_malformed_endpoint() {
        for url in [
            "file:///etc/passwd",
            "ftp://provider.example/v1/stream",
            "https:///missing-host",
            "https://provider.example/v1/stream?query=not-allowed",
        ] {
            assert!(
                split_host_port(url).is_err(),
                "unsupported or malformed provider endpoint must be refused: {url}"
            );
        }
    }

    #[test]
    fn provider_error_frame_is_not_reflected() {
        let raw = "provider-secret-and-upstream-text";
        let line = format!("data: {{\"type\":\"error\",\"message\":\"{raw}\"}}\n");
        let Parsed::Refuse(error) = parse_line(line.as_bytes()) else {
            panic!("provider error frame must be refused");
        };
        let rendered = error.to_string();
        assert!(
            !rendered.contains(raw),
            "provider error text must not cross the typed error boundary: {rendered}"
        );
    }

    #[test]
    fn malformed_sse_does_not_echo_raw_payload() {
        let raw = "Bearer should-never-echo";
        let line = format!("data: {{not-json:{raw}}}\n");
        let Parsed::Refuse(error) = parse_line(line.as_bytes()) else {
            panic!("malformed SSE must be refused");
        };
        let rendered = error.to_string();
        assert!(
            !rendered.contains(raw),
            "malformed SSE must not echo its raw payload: {rendered}"
        );
    }

    #[test]
    fn provider_error_logs_contain_no_secret_or_raw_body() {
        let raw = "upstream-body-secret";
        let line = format!("data: {{\"type\":\"error\",\"message\":\"{raw}\"}}\n");
        let Parsed::Refuse(error) = parse_line(line.as_bytes()) else {
            panic!("provider error frame must be refused");
        };
        assert!(
            !error.to_string().contains(raw),
            "provider error rendering is log-safe"
        );
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
