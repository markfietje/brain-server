//! Minimal, dependency-free blocking HTTP/1.1 client over `std::net::TcpStream`.
//!
//! Shared by the `brain` and `mcp` binaries via `#[path]` inclusion so we avoid
//! pulling in `ureq`/`reqwest` (neither is a normal dependency of the server).
//!
//! Scope is deliberately tiny: one request per connection, no keep-alive,
//! Content-Length or chunked bodies. That covers every endpoint the
//! brain-server exposes (small JSON payloads).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

pub struct Url {
    pub host: String,
    pub port: u16,
    pub path_query: String,
}

/// The fixed refusal for a base that names `https://`. This client is
/// dependency-free and speaks no TLS, so the old behavior — strip the scheme,
/// connect to port 80 — turned an operator's explicit request for transport
/// security into a silent cleartext bearer. A fixed string, because a caller
/// may match on it and because a rendered URL in an error is a URL to log.
pub(crate) const HTTPS_BASE_REFUSED: &str = "BRAIN_URL names https:// but this client speaks plain HTTP only — it has no TLS \
     stack. Point BRAIN_URL at a loopback http:// base, or use a client that can do \
     TLS (the steward harness) instead of downgrading a secure request to cleartext.";

/// The fixed refusal for a non-loopback plain-HTTP base. These client binaries
/// are loopback-only by contract — the steward harness already refuses
/// non-loopback plain HTTP — so a remote authority is a cleartext bearer by
/// construction, whether the scheme was written or defaulted.
pub(crate) const REMOTE_BASE_REFUSED: &str = "BRAIN_URL names a non-loopback host over plain HTTP — the bearer would cross the \
     network in cleartext. Point BRAIN_URL at a loopback http:// base, or use a client \
     that can do TLS (the steward harness) instead.";

/// Whether an authority host is loopback. Accepts the IPv4 forms, `localhost`,
/// and the bracketed IPv6 loopback; anything else is remote by construction
/// (a name is never assumed to be loopback — resolution is not this layer's
/// job, and a name that resolves to 127.0.0.1 must be pinned by the operator,
/// not guessed here).
fn is_loopback_host(host: &str) -> bool {
    let h = host.trim().trim_start_matches('[').trim_end_matches(']');
    h.eq_ignore_ascii_case("localhost")
        || h.parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Parse a base such as `http://127.0.0.1:8765` into `(host, port, root_path)`.
///
/// Fail-closed on both ways a bearer can leave in cleartext: an explicit
/// `https://` (which this client cannot honour, and used to silently downgrade)
/// and a non-loopback plain-HTTP authority (a cleartext bearer by
/// construction). The refusal is a FIXED string in both arms.
fn parse_base(base: &str) -> Result<(String, u16, String), String> {
    let s = base.trim();
    if s.starts_with("https://") {
        return Err(HTTPS_BASE_REFUSED.to_string());
    }
    let s = s.strip_prefix("http://").unwrap_or(s);
    // No slash at all is the base itself, and the root path stays "/" — the
    // default this client has always used.
    let split = s.find('/').unwrap_or(s.len());
    let authority = s[..split].to_string();
    let path = if split == s.len() {
        "/".to_string()
    } else {
        s[split..].to_string()
    };
    // No port in the authority means 80 — the default this client has always
    // used, kept here so the loopback contract above stays the only refusal.
    let (host, port) = authority
        .rsplit_once(':')
        .map_or((authority.clone(), Ok(80)), |(h, p)| {
            (
                h.to_string(),
                p.parse::<u16>()
                    .map_err(|e| format!("invalid port in {base}: {e}")),
            )
        });
    let port = port?;
    if !is_loopback_host(&host) {
        return Err(REMOTE_BASE_REFUSED.to_string());
    }
    Ok((host, port, path))
}

/// Build a request `Url` from a base, a path, and ordered query pairs.
pub fn build_url(base: &str, path: &str, query: &[(String, String)]) -> Result<Url, String> {
    let (host, port, root) = parse_base(base)?;
    let mut pq = root.trim_end_matches('/').to_string();
    if !path.starts_with('/') {
        pq.push('/');
    }
    pq.push_str(path);
    if !query.is_empty() {
        pq.push('?');
        for (i, (k, v)) in query.iter().enumerate() {
            if i > 0 {
                pq.push('&');
            }
            pq.push_str(&url_encode(k));
            pq.push('=');
            pq.push_str(&url_encode(v));
        }
    }
    Ok(Url {
        host,
        port,
        path_query: pq,
    })
}

/// Percent-encode a string for use in a URL (query component). Encodes every
/// byte that is not an unreserved ASCII char (RFC 3986 §2.3). Also the path-
/// segment encoder: a positional like `/`, `?`, or `#` must never rewrite the
/// request-target.
pub(crate) fn url_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => {
                out.push('%');
                out.push_str(&format!("{b:02X}"));
            }
        }
    }
    out
}

pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

/// Normalize any configured bearer-token source into exactly ONE token.
///
/// Token files written by `scripts/install-service.sh` may carry multiple
/// newline/whitespace-separated rotation slots — the SERVER accepts every
/// slot ([`crate::config::auth_tokens`] splits on whitespace), but a client
/// that pastes the whole file into one `Authorization` header corrupts the
/// request (the embedded newline yields an empty-body 400 before auth even
/// runs). Every `#[path]` consumer of this module resolves tokens through
/// this helper; do not re-derive it per binary.
#[allow(dead_code)]
pub(crate) fn first_token(raw: &str) -> Option<String> {
    raw.split_whitespace().next().map(str::to_string)
}

/// Perform a single GET request. `bearer`, when `Some`, sends
/// `Authorization: Bearer <token>`; required for non-public routes when the
/// server has auth enabled. Passing `None` is fine for public routes
/// (`/health`, `/health/db`, `/ready`, `/version`).
///
/// The `mcp` binary uses `get` for the `ump.*` GET tools; `bench` and
/// `client_example` include this file via `#[path]` but issue no GETs —
/// `allow(dead_code)` keeps the public API symmetrical without forcing every
/// consumer to call it.
#[allow(dead_code)]
pub fn get(
    base: &str,
    path: &str,
    query: &[(String, String)],
    bearer: Option<&str>,
) -> Result<HttpResponse, String> {
    let url = build_url(base, path, query)?;
    request("GET", &url, "", None, bearer)
}

/// Perform a single POST with a `Content-Type` header and a body.
pub fn post(
    base: &str,
    path: &str,
    query: &[(String, String)],
    content_type: &str,
    body: &str,
    bearer: Option<&str>,
) -> Result<HttpResponse, String> {
    let url = build_url(base, path, query)?;
    request("POST", &url, content_type, Some(body), bearer)
}

/// Perform a single DELETE. Body-less by HTTP convention; `bearer` is sent on
/// the same footing as `get`/`post`. Used by `DELETE /sources/{id}` via the
/// `brain source-delete` CLI command. The `mcp` and `bench` binaries include
/// this file via `#[path]` but don't issue DELETEs yet — `allow(dead_code)` keeps
/// the public API symmetrical without forcing those binaries to consume it.
#[allow(dead_code)]
pub fn delete(
    base: &str,
    path: &str,
    query: &[(String, String)],
    bearer: Option<&str>,
) -> Result<HttpResponse, String> {
    let url = build_url(base, path, query)?;
    request("DELETE", &url, "", None, bearer)
}

/// Perform a single PUT with a `Content-Type` header and a body (the
/// `brain valet consent` verb). The `mcp`/`bench` binaries include this file
/// via `#[path]` but don't issue PUTs — same `allow(dead_code)` posture as
/// `delete`.
#[allow(dead_code)]
pub fn put(
    base: &str,
    path: &str,
    query: &[(String, String)],
    content_type: &str,
    body: &str,
    bearer: Option<&str>,
) -> Result<HttpResponse, String> {
    let url = build_url(base, path, query)?;
    request("PUT", &url, content_type, Some(body), bearer)
}

fn request(
    method: &str,
    url: &Url,
    content_type: &str,
    body: Option<&str>,
    bearer: Option<&str>,
) -> Result<HttpResponse, String> {
    let addr = format!("{}:{}", url.host, url.port);
    let mut stream =
        TcpStream::connect(&addr).map_err(|e| format!("cannot connect to {addr}: {e}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(15))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(15))).ok();

    let body = body.unwrap_or("");
    let mut head = format!(
        "{method} {pq} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nAccept: application/json\r\n",
        pq = url.path_query,
        host = url.host,
    );
    if let Some(t) = bearer {
        head.push_str(&format!("Authorization: Bearer {t}\r\n"));
    }
    if !body.is_empty() {
        head.push_str(&format!("Content-Type: {content_type}\r\n"));
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    let mut wire = head.into_bytes();
    wire.extend_from_slice(b"\r\n");
    wire.extend_from_slice(body.as_bytes());

    stream
        .write_all(&wire)
        .map_err(|e| format!("request write failed: {e}"))?;

    let raw = read_response_bounded(&mut stream, MAX_RESPONSE_BYTES)?;
    let text = String::from_utf8_lossy(&raw).into_owned();
    parse_response(&text)
}

/// Ceiling on ONE response body read from the kernel: this client is
/// loopback-only, but a confused or compromised server must not be able to
/// stream the CLI into unbounded memory. Generous by design — `brain ump
/// export` pulls the full-record projection of a large deployment through
/// this path, so the bound sits far above any legitimate export, and an
/// exactly-at-the-bound body is refused too (fail-closed: the reader cannot
/// distinguish "exactly N" from "N and more behind it").
pub(crate) const MAX_RESPONSE_BYTES: u64 = 256 * 1024 * 1024;

/// Read one response body, refusing at `cap` bytes. The cap is a parameter
/// so the bound itself is testable at a small scale; production always
/// passes [`MAX_RESPONSE_BYTES`].
fn read_response_bounded(stream: &mut TcpStream, cap: u64) -> Result<Vec<u8>, String> {
    let mut raw = Vec::new();
    let n = (&mut *stream)
        .take(cap)
        .read_to_end(&mut raw)
        .map_err(|e| format!("response read failed: {e}"))?;
    if n as u64 >= cap {
        return Err(format!(
            "response exceeds the {cap}-byte read bound; refusing to buffer it"
        ));
    }
    Ok(raw)
}

fn parse_response(text: &str) -> Result<HttpResponse, String> {
    let (header_part, body) = match text.find("\r\n\r\n") {
        Some(i) => (&text[..i], &text[i + 4..]),
        None => return Err("malformed HTTP response: missing header/body separator".into()),
    };

    let mut lines = header_part.split("\r\n");
    let status_line = lines.next().ok_or("empty status line")?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse::<u16>().ok())
        .ok_or("cannot parse status code")?;

    let mut transfer_chunked = false;
    let mut content_length: Option<usize> = None;
    for line in lines {
        let (k, v) = match line.split_once(':') {
            Some(kv) => kv,
            None => continue,
        };
        match k.trim().to_ascii_lowercase().as_str() {
            "transfer-encoding" if v.to_ascii_lowercase().contains("chunked") => {
                transfer_chunked = true
            }
            "content-length" => {
                content_length = v.trim().parse::<usize>().ok();
            }
            _ => {}
        }
    }

    let body = if transfer_chunked {
        decode_chunked(body)
    } else if let Some(len) = content_length {
        body.get(..len).unwrap_or(body).to_string()
    } else {
        body.to_string()
    };

    Ok(HttpResponse { status, body })
}

/// Decode an RFC 9112 chunked-transfer body. Falls back to the raw body if the
/// framing is malformed (better a slightly wrong body than a hard error here).
fn decode_chunked(body: &str) -> String {
    let mut out = String::new();
    let mut rest = body;
    while let Some(line_end) = rest.find("\r\n") {
        let size_hex = &rest[..line_end];
        let size = match usize::from_str_radix(size_hex.trim(), 16) {
            Ok(s) => s,
            Err(_) => break,
        };
        if size == 0 {
            break;
        }
        let data_start = line_end + 2;
        if rest.len() < data_start + size {
            break;
        }
        out.push_str(&rest[data_start..data_start + size]);
        rest = &rest[data_start + size + 2..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        HTTPS_BASE_REFUSED, MAX_RESPONSE_BYTES, REMOTE_BASE_REFUSED, first_token, parse_base,
        read_response_bounded,
    };

    /// Token files may carry multiple newline-separated rotation slots (the
    /// server accepts every slot); a client must send exactly ONE — the old
    /// trim-only normalization kept the embedded newline and corrupted the
    /// Authorization header into an empty-body 400.
    #[test]
    fn first_token_takes_one_slot_and_drops_rotation_remainder() {
        assert_eq!(
            first_token("aaaabbbbccccdddd\n1111222233334444\n"),
            Some("aaaabbbbccccdddd".to_string())
        );
        assert_eq!(
            first_token("  solo-token  "),
            Some("solo-token".to_string())
        );
        assert_eq!(first_token("   \n\t "), None);
        assert_eq!(first_token(""), None);
    }

    /// A base written `https://` used to be silently STRIPPED to a plain
    /// `http://` connection on port 80 — the operator asked for transport
    /// security and got the bearer in cleartext with no signal. This client is
    /// dependency-free (no TLS), so the honest answer is a refusal, not a
    /// downgrade.
    #[test]
    fn an_https_base_refuses_rather_than_downgrading_to_cleartext() {
        let err = parse_base("https://brain.example.com")
            .expect_err("an https:// base must refuse — this client speaks no TLS");
        assert_eq!(
            err, HTTPS_BASE_REFUSED,
            "the refusal string is a fixed contract, not a rendered URL"
        );
        // …and the loopback defaults the whole toolchain ships are untouched.
        for base in [
            "http://127.0.0.1:8765",
            "127.0.0.1:8765",
            "http://localhost:8765",
        ] {
            let (host, port, path) = parse_base(base).expect(base);
            assert_eq!(host, host.trim(), "{base}");
            assert!(port > 0, "{base}");
            assert!(path.starts_with('/'), "{base}");
        }
        assert_eq!(
            parse_base("http://127.0.0.1:8765").expect("loopback"),
            ("127.0.0.1".to_string(), 8765, "/".to_string())
        );
    }

    /// The same rule for a bare-HOST default port: with no scheme the client
    /// assumes 80, so a non-loopback authority is a cleartext bearer by
    /// construction. These binaries are loopback-only by contract (the
    /// steward harness already refuses non-loopback plain HTTP); the refusal
    /// makes the contract enforceable rather than aspirational.
    #[test]
    fn a_remote_plain_http_base_refuses_rather_than_leaking_the_bearer() {
        for base in [
            "http://brain.example.com",
            "brain.example.com:8765",
            "10.0.0.5:8765",
        ] {
            let err = parse_base(base).expect_err("a non-loopback base must refuse");
            assert_eq!(err, REMOTE_BASE_REFUSED, "{base}");
        }
        for base in [
            "http://127.0.0.1:8765",
            "http://localhost:8765",
            "http://[::1]:8765",
            "http://127.0.0.1",
        ] {
            assert!(
                parse_base(base).is_ok(),
                "{base} is loopback and must keep working"
            );
        }
    }

    /// The read bound fires: a server that streams past the cap gets a
    /// REFUSAL, not an unbounded buffer. The cap is exercised at a small
    /// scale (the parameter exists for exactly this); the production value
    /// is pinned below.
    #[test]
    fn a_response_past_the_bound_is_refused_not_buffered() {
        use std::io::Write as _;
        use std::net::TcpListener;
        let body = "x".repeat(4096);
        let server = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = server.local_addr().expect("addr");
        let payload = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let payload2 = payload.clone(); // clone BEFORE the first thread takes ownership
        let writer = std::thread::spawn(move || {
            if let Ok((mut sock, _)) = server.accept() {
                let _ = sock.write_all(payload.as_bytes());
            }
        });
        let mut stream = std::net::TcpStream::connect(addr).expect("connect");
        let over =
            read_response_bounded(&mut stream, 2048).expect_err("a body past the cap must refuse");
        assert!(
            over.contains("read bound"),
            "the refusal names the bound, got: {over}"
        );
        writer.join().expect("server thread");

        // The same body under a larger cap reads whole.
        let server = TcpListener::bind("127.0.0.1:0").expect("bind loopback 2");
        let addr = server.local_addr().expect("addr 2");
        let writer = std::thread::spawn(move || {
            if let Ok((mut sock, _)) = server.accept() {
                let _ = sock.write_all(payload2.as_bytes());
            }
        });
        let mut stream = std::net::TcpStream::connect(addr).expect("connect 2");
        let raw = read_response_bounded(&mut stream, 64 * 1024).expect("under the cap reads");
        assert!(
            raw.len() > 4096,
            "headers plus body arrived, got {}",
            raw.len()
        );
        writer.join().expect("server thread 2");
    }

    /// The production ceiling is a DECISION, not an accident: 256 MiB sits
    /// far above any legitimate `ump export` of the full-record projection
    /// while still bounding what a compromised server can make the CLI
    /// buffer. Moving it is a product call, recorded here.
    #[test]
    fn the_production_read_bound_is_256mib() {
        assert_eq!(MAX_RESPONSE_BYTES, 256 * 1024 * 1024);
    }
}
