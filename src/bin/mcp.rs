//! `mcp` — a minimal MCP (Model Context Protocol) server for brain-server.
//!
//! Speaks JSON-RPC 2.0 over newline-delimited stdio (no external MCP crate).
//! Implements `initialize`, `tools/list`, and `tools/call`, translating tool
//! calls into HTTP requests against a running brain-server using the shared
//! dependency-free client in `bin_common/http.rs`.

#[path = "../bin_common/http.rs"]
mod http;

use http::post;
use std::io::{BufRead, Write};

const PROTOCOL_VERSION: &str = "2024-11-05";
const SERVER_NAME: &str = "brain-server-mcp";
/// Drive version from Cargo.toml so the MCP binary and the server never drift.
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_URL: &str = "http://127.0.0.1:8765";

fn base_url() -> String {
    std::env::var("BRAIN_URL").unwrap_or_else(|_| DEFAULT_URL.to_string())
}

/// Resolve the bearer token for authenticated routes, mirroring the server's
/// `AUTH_TOKEN_FILE` → `AUTH_TOKEN` ladder (see `src/config.rs`).
///
/// 1. `BRAIN_TOKEN_FILE` — explicit path to a `0600`-mode secret file.
/// 2. `BRAIN_TOKEN` — raw env var (dev convenience).
/// 3. `~/.config/brain-server/auth-token` — default install path written by
///    `scripts/install-service.sh`. Zero-config for the common case.
fn auth_token() -> Option<String> {
    if let Ok(path) = std::env::var("BRAIN_TOKEN_FILE") {
        let p = path.trim();
        if let Ok(s) = std::fs::read_to_string(p) {
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
    let default_path = dirs_home().join(".config/brain-server/auth-token");
    if let Ok(s) = std::fs::read_to_string(&default_path) {
        let s = s.trim();
        if !s.is_empty() {
            return Some(s.to_string());
        }
    }
    None
}

/// Minimal HOME discovery (no `dirs` dependency in the bin crates).
fn dirs_home() -> std::path::PathBuf {
    if let Ok(h) = std::env::var("HOME") {
        return std::path::PathBuf::from(h);
    }
    std::path::PathBuf::from(".")
}

fn main() {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut buf = String::new();

    loop {
        buf.clear();
        let n = match stdin.lock().read_line(&mut buf) {
            Ok(0) => break, // EOF
            Ok(n) => n,
            Err(e) => {
                eprintln!("mcp: stdin read error: {e}");
                break;
            }
        };
        let line = buf[..n].trim();
        if line.is_empty() {
            continue;
        }
        match handle_line(line) {
            Ok(Some(response)) => {
                let _ = stdout.write_all(response.as_bytes());
                let _ = stdout.write_all(b"\n");
                let _ = stdout.flush();
            }
            Ok(None) => {}
            Err(e) => {
                eprintln!("mcp: error handling request: {e}");
            }
        }
    }
}

fn handle_line(line: &str) -> Result<Option<String>, String> {
    let req: serde_json::Value =
        serde_json::from_str(line).map_err(|e| format!("invalid JSON-RPC request: {e}"))?;

    // Notifications (no id) need no response per JSON-RPC 2.0 §4. A missing id
    // on a request that would otherwise produce an error response is also
    // treated as a notification — silently dropping is safer than panicking.
    let id = req.get("id").cloned();
    let method = req
        .get("method")
        .and_then(|m| m.as_str())
        .ok_or("missing 'method'")?
        .to_string();

    // If there's no id, this is a notification — acknowledge and return no reply
    // regardless of whether the method succeeded (JSON-RPC 2.0 §4.1).
    let id = match id {
        Some(v) => v,
        None => return Ok(None),
    };
    let params = req
        .get("params")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    let result = match method.as_str() {
        "initialize" => method_initialize(),
        "tools/list" => method_tools_list(),
        "tools/call" => method_tools_call(&params),
        "ping" => Ok(serde_json::json!({})),
        other => {
            return Ok(Some(error_response(
                &id,
                -32601,
                &format!("method not found: {other}"),
            )));
        }
    };

    match result {
        Ok(r) => Ok(Some(success_response(&id, r))),
        Err(e) => Ok(Some(error_response(&id, -32603, &e))),
    }
}

fn success_response(id: &serde_json::Value, result: serde_json::Value) -> String {
    let resp = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    });
    resp.to_string()
}

fn error_response(id: &serde_json::Value, code: i64, message: &str) -> String {
    let resp = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    });
    resp.to_string()
}

fn method_initialize() -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": SERVER_NAME, "version": SERVER_VERSION },
    }))
}

fn method_tools_list() -> Result<serde_json::Value, String> {
    let tools = serde_json::json!([
        {
            "name": "brain_search",
            "description": "Hybrid semantic + lexical search over the brain memory store (v0.9.5 structured query).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "The search query." },
                    "limit": { "type": "integer", "description": "Max hits (default 5)." },
                    "phrases": { "type": "array", "items": { "type": "string" }, "description": "Quoted phrase matches." },
                    "exclude": { "type": "array", "items": { "type": "string" }, "description": "Terms to exclude (FTS5 NOT)." },
                    "code": { "type": "array", "items": { "type": "string" }, "description": "Exact identifier / code path." },
                    "sources": { "type": "array", "items": { "type": "string" }, "description": "Multi-source OR scope." },
                    "source": { "type": "string", "description": "Single-source equality (legacy)." },
                    "since": { "type": "string", "description": "Only results newer than an ISO timestamp." },
                    "intent": { "type": "string", "description": "Intent label, recorded for provenance only." },
                    "provenance": { "type": "boolean", "description": "Include per-retriever provenance + telemetry." }
                },
                "required": ["query"]
            }
        },
        {
            "name": "brain_recall",
            "description": "Deterministic end-to-end recall (embed -> hybrid search). Alias of brain_search on POST /recall.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "The recall query." },
                    "limit": { "type": "integer", "description": "Max hits (1..100)." },
                    "phrases": { "type": "array", "items": { "type": "string" }, "description": "Quoted phrase matches." },
                    "exclude": { "type": "array", "items": { "type": "string" }, "description": "Terms to exclude (FTS5 NOT)." },
                    "code": { "type": "array", "items": { "type": "string" }, "description": "Exact identifier / code path." },
                    "sources": { "type": "array", "items": { "type": "string" }, "description": "Multi-source OR scope." },
                    "domain": { "type": "string", "description": "Optional domain label." },
                    "source": { "type": "string", "description": "Filter by source." },
                    "since": { "type": "string", "description": "Only results newer than an ISO timestamp." },
                    "intent": { "type": "string", "description": "Intent label, recorded for provenance only." },
                    "provenance": { "type": "boolean", "description": "Include per-retriever provenance + telemetry." }
                },
                "required": ["query"]
            }
        },
        {
            "name": "brain_ingest",
            "description": "Ingest a memory with optional structured entities/relations into the brain's knowledge graph (v1.0 primary path). Calls POST /ingest; the agent does entity extraction client-side and passes the graph data here.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "content": { "type": "string", "description": "The text to ingest (full prose)." },
                    "title": { "type": "string", "description": "Title for the memory." },
                    "domain": { "type": "string", "description": "Target domain (defaults to 'global'). Must match ^[a-z0-9][a-z0-9_-]{0,62}$." },
                    "entities": {
                        "type": "array",
                        "description": "Entities mentioned in the content. The server trusts these (no regex extraction).",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": { "type": "string", "description": "Entity name (allows spaces, e.g. 'vitamin d3')." },
                                "type": { "type": "string", "description": "Optional entity type/kind (≤64 chars)." }
                            },
                            "required": ["name"]
                        }
                    },
                    "relations": {
                        "type": "array",
                        "description": "Relations between entities, anchored to this memory.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "from": { "type": "string" },
                                "to": { "type": "string" },
                                "type": { "type": "string", "description": "snake_case relation type, e.g. 'helps' / 'relates_to'." }
                            },
                            "required": ["from", "to", "type"]
                        }
                    },
                    "source": { "type": "string", "description": "Source label (legacy memory-ingest mode only; ignored when entities/relations are present)." }
                },
                "required": ["content"]
            }
        }
    ]);
    Ok(serde_json::json!({ "tools": tools }))
}

fn method_tools_call(params: &serde_json::Value) -> Result<serde_json::Value, String> {
    let name = params
        .get("name")
        .and_then(|n| n.as_str())
        .ok_or("tools/call missing 'name'")?
        .to_string();
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    let payload: String = match name.as_str() {
        "brain_search" => tool_brain_search(&args)?,
        "brain_recall" => tool_brain_recall(&args)?,
        "brain_ingest" => tool_brain_ingest(&args)?,
        other => {
            return Ok(serde_json::json!({
                "content": [ { "type": "text", "text": format!("unknown tool: {other}") } ],
                "isError": true,
            }));
        }
    };

    Ok(serde_json::json!({
        "content": [ { "type": "text", "text": payload } ],
        "isError": false,
    }))
}

fn arg_str(args: &serde_json::Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn arg_u64(args: &serde_json::Value, key: &str) -> Option<u64> {
    args.get(key).and_then(|v| v.as_u64())
}

fn arg_bool(args: &serde_json::Value, key: &str) -> Option<bool> {
    args.get(key).and_then(|v| v.as_bool())
}

fn tool_brain_search(args: &serde_json::Value) -> Result<String, String> {
    let q = arg_str(args, "query").ok_or("brain_search requires 'query'")?;
    let body = recall_body(args, &q)?;
    let resp = post(
        &base_url(),
        "/recall",
        &[],
        "application/json",
        &body,
        auth_token().as_deref(),
    )?;
    Ok(format_response(resp.status, &resp.body))
}

fn tool_brain_recall(args: &serde_json::Value) -> Result<String, String> {
    let q = arg_str(args, "query").ok_or("brain_recall requires 'query'")?;
    let body = recall_body(args, &q)?;
    let resp = post(
        &base_url(),
        "/recall",
        &[],
        "application/json",
        &body,
        auth_token().as_deref(),
    )?;
    Ok(format_response(resp.status, &resp.body))
}

/// Lower an MCP tool's arguments into the v0.9.5 structured `QueryDoc` body for
/// `POST /recall`. Both `brain_search` and `brain_recall` share this so the MCP
/// surface mirrors the HTTP contract exactly.
fn recall_body(args: &serde_json::Value, query: &str) -> Result<String, String> {
    let mut body = serde_json::json!({ "query": query });
    if let Some(l) = arg_u64(args, "limit").or_else(|| arg_u64(args, "k")) {
        body["limit"] = serde_json::json!(l);
    }
    if let Some(d) = arg_str(args, "domain") {
        body["domain"] = serde_json::json!(d);
    }
    if let Some(s) = arg_str(args, "source") {
        body["source"] = serde_json::json!(s);
    }
    if let Some(s) = arg_str(args, "since") {
        body["since"] = serde_json::json!(s);
    }
    if let Some(s) = arg_str(args, "intent") {
        body["intent"] = serde_json::json!(s);
    }
    if let Some(p) = arg_bool(args, "provenance") {
        body["provenance"] = serde_json::json!(p);
    }
    if let Some(sources) = args.get("sources").and_then(|v| v.as_array()) {
        body["sources"] = serde_json::json!(sources);
    }
    let lex = build_lex(args);
    if lex.as_object().map(|o| !o.is_empty()).unwrap_or(false) {
        body["lex"] = lex;
    }
    Ok(body.to_string())
}

/// Build a `LexSpec` from the MCP tool's flat lexical fields.
fn build_lex(args: &serde_json::Value) -> serde_json::Value {
    let mut lex = serde_json::json!({});
    for key in ["phrases", "exclude", "code"] {
        if let Some(v) = args.get(key).and_then(|x| x.as_array()) {
            lex[key] = serde_json::json!(v);
        }
    }
    lex
}

fn tool_brain_ingest(args: &serde_json::Value) -> Result<String, String> {
    let content = arg_str(args, "content").ok_or("brain_ingest requires 'content'")?;
    let title = arg_str(args, "title");
    let domain = arg_str(args, "domain");
    let entities = args.get("entities").cloned();
    let relations = args.get("relations").cloned();
    let has_structured = entities.as_ref().is_some_and(|v| v.is_array())
        || relations.as_ref().is_some_and(|v| v.is_array());

    // v1.0 primary path: POST /ingest with structured fields. Triggered when
    // the caller supplies entities/relations/domain — the agent did extraction
    // client-side, per the plan M4.
    if has_structured || domain.is_some() {
        let mut body = serde_json::json!({ "content": content });
        if let Some(t) = title {
            body["title"] = serde_json::json!(t);
        } else {
            // POST /ingest requires a non-empty title; default to first line.
            let default_title = content
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(100)
                .collect::<String>();
            body["title"] = serde_json::json!(default_title);
        }
        if let Some(d) = domain {
            body["domain"] = serde_json::json!(d);
        }
        if let Some(e) = entities {
            body["entities"] = e;
        }
        if let Some(r) = relations {
            body["relations"] = r;
        }
        let resp = post(
            &base_url(),
            "/ingest",
            &[],
            "application/json",
            &body.to_string(),
            auth_token().as_deref(),
        )?;
        return Ok(format_response(resp.status, &resp.body));
    }

    // Legacy paths: keep back-compat for memory-style ingests with no graph data.
    let source = arg_str(args, "source");
    let resp = if let Some(title) = title {
        let body = serde_json::json!({ "content": content, "title": title }).to_string();
        post(
            &base_url(),
            "/ingest/markdown",
            &[],
            "application/json",
            &body,
            auth_token().as_deref(),
        )?
    } else {
        let q = source
            .as_ref()
            .map(|s| vec![("source".to_string(), s.clone())])
            .unwrap_or_default();
        post(
            &base_url(),
            "/ingest/memory",
            &q,
            "text/plain",
            &content,
            auth_token().as_deref(),
        )?
    };
    Ok(format_response(resp.status, &resp.body))
}

fn format_response(status: u16, body: &str) -> String {
    if status == 200 {
        body.to_string()
    } else {
        format!("HTTP {status}: {body}")
    }
}
