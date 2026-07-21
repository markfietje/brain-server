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

/// Parse a base such as `http://127.0.0.1:8765` into `(host, port, root_path)`.
fn parse_base(base: &str) -> Result<(String, u16, String), String> {
    let s = base.trim();
    let s = s
        .strip_prefix("http://")
        .or_else(|| s.strip_prefix("https://"))
        .unwrap_or(s);
    let (authority, path) = match s.find('/') {
        Some(i) => (s[..i].to_string(), s[i..].to_string()),
        None => (s.to_string(), "/".to_string()),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (
            h.to_string(),
            p.parse::<u16>()
                .map_err(|e| format!("invalid port in {base}: {e}"))?,
        ),
        None => (authority, 80),
    };
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
/// byte that is not an unreserved ASCII char (RFC 3986 §2.3).
fn url_encode(input: &str) -> String {
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

/// Perform a single GET request. `bearer`, when `Some`, sends
/// `Authorization: Bearer <token>`; required for non-public routes when the
/// server has auth enabled. Passing `None` is fine for public routes
/// (`/health`, `/health/db`, `/ready`, `/version`).
///
/// The `mcp` binary and `client_example` include this file via `#[path]` but
/// don't issue GETs against authed routes — `allow(dead_code)` keeps the public
/// API symmetrical without forcing every consumer to call it.
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

    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|e| format!("response read failed: {e}"))?;
    let text = String::from_utf8_lossy(&raw).into_owned();
    parse_response(&text)
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
