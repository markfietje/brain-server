//! `brain-connector-stub` — reference connector binary.
//!
//! Proves the connector contract end-to-end without any
//! real external source. Spawns, reads its argv (`--config`, `--checkpoint`),
//! emits the JSON-lines event stream the supervisor expects, ingests one doc
//! via the server's existing `/ingest/markdown` route, then exits 0.
//!
//! Used by:
//! - `connector::supervisor::tests::test_spawn_once_runs_stub_binary_and_returns_zero`
//!   (verifies spawn + clean exit)
//! - the future `brain connect` smoke test (verifies end-to-end ingest)
//!
//! This is a *binary*, not a library module. It pulls in the shared
//! dependency-free HTTP client via `#[path]` to avoid a `reqwest` dep on the
//! stub (which would defeat the point of the contract — real connectors like
//! `brain-connector-gh` are free to depend on `reqwest`).

#[path = "../bin_common/http.rs"]
mod http;

use http::post;

const DEFAULT_URL: &str = "http://127.0.0.1:8765";

fn main() {
    // Parse argv per the connector-binary contract. The stub ignores the
    // values; a real connector would read its config JSON and open the
    // checkpoint DB at these paths.
    let mut config_path: Option<String> = None;
    let mut checkpoint_path: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--config" => config_path = args.next(),
            "--checkpoint" => checkpoint_path = args.next(),
            other => {
                emit_log("warn", &format!("ignoring unknown argv: {other}"));
            }
        }
    }
    if config_path.is_none() || checkpoint_path.is_none() {
        emit_error("missing --config and/or --checkpoint argv", false);
        std::process::exit(2);
    }

    emit_log("info", "stub connector starting");

    let base = std::env::var("BRAIN_URL").unwrap_or_else(|_| DEFAULT_URL.to_string());
    let token = auth_token();
    let body = serde_json::json!({
        "content": "# stub connector doc\n\nThis is a single test document ingested by brain-connector-stub to prove the connector contract end-to-end.",
        "title": "stub connector test doc",
        "source_path": "stub://default/test-doc",
    })
    .to_string();

    match post(
        &base,
        "/ingest/markdown",
        &[],
        "application/json",
        &body,
        token.as_deref(),
    ) {
        Ok(resp) => {
            if resp.status == 200 || resp.status == 201 {
                emit_progress("default", 1);
                emit_done();
                std::process::exit(0);
            } else {
                emit_error(
                    &format!("server returned status {}: {}", resp.status, resp.body),
                    true,
                );
                std::process::exit(1);
            }
        }
        Err(e) => {
            // The supervisor will restart us on non-zero exit. The `retry`
            // flag in the error event is informational — the supervisor
            // doesn't parse it yet.
            emit_error(&format!("HTTP request failed: {e}"), true);
            std::process::exit(1);
        }
    }
}

/// Resolve the bearer token, mirroring `src/bin/brain.rs::auth_token`.
/// Duplicated rather than shared because the stub is its own binary and we
/// keep the bin_common surface tiny (HTTP only).
fn auth_token() -> Option<String> {
    if let Ok(path) = std::env::var("BRAIN_TOKEN_FILE") {
        if let Ok(s) = std::fs::read_to_string(path.trim()) {
            let s = s.trim();
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    if let Ok(t) = std::env::var("BRAIN_TOKEN") {
        let t = t.trim();
        if !t.is_empty() {
            return Some(t.to_string());
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let default_path = std::path::Path::new(&home).join(".config/brain-server/auth-token");
    std::fs::read_to_string(&default_path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Emit one JSON-lines event to stdout (the connector → supervisor protocol).
/// Uses serde_json so we never have to hand-escape strings — the supervisor's
/// parser will be serde-based too.
fn emit_log(level: &str, msg: &str) {
    let _ = serde_json::to_writer(
        std::io::stdout(),
        &serde_json::json!({
            "type": "log",
            "level": level,
            "msg": msg,
        }),
    );
    println!();
}
fn emit_progress(cursor: &str, count: usize) {
    let _ = serde_json::to_writer(
        std::io::stdout(),
        &serde_json::json!({
            "type": "progress",
            "cursor": cursor,
            "count": count,
        }),
    );
    println!();
}
fn emit_done() {
    let _ = serde_json::to_writer(
        std::io::stdout(),
        &serde_json::json!({
            "type": "done",
            "report": {},
        }),
    );
    println!();
}
fn emit_error(msg: &str, retry: bool) {
    let _ = serde_json::to_writer(
        std::io::stdout(),
        &serde_json::json!({
            "type": "error",
            "msg": msg,
            "retry": retry,
        }),
    );
    println!();
}
