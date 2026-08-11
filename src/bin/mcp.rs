//! `mcp` — a minimal MCP (Model Context Protocol) server for brain-server.
//!
//! Speaks JSON-RPC 2.0 over newline-delimited stdio (no external MCP crate),
//! translating tool calls into HTTP requests against a running brain-server
//! using the shared dependency-free client in `bin_common/http.rs`.
//!
//! Protocol surface (2026-07-28 spec): stateless — no `initialize` handshake.
//! Every modern request carries `_meta` (`io.modelcontextprotocol/
//! protocolVersion` + `io.modelcontextprotocol/clientCapabilities`),
//! discovery happens via `server/discover`, and every result carries
//! `resultType` + `_meta.serverInfo`. `tools/list` and `server/discover`
//! advertise `ttlMs`/`cacheScope` caching hints (SEP-2549). For legacy
//! (2025-11-25) clients, an `initialize` request selects legacy semantics
//! scoped to this stdio process: bare requests without `_meta` dispatch
//! directly and responses keep the legacy shape (no `resultType` envelope).

#[path = "../bin_common/http.rs"]
mod http;

use http::{get, post};
use std::io::{BufRead, Write};

/// Modern protocol version (final 2026-07-28 spec): stateless, per-request
/// `_meta`, `server/discover` instead of `initialize`.
const MODERN_VERSION: &str = "2026-07-28";
/// Legacy protocol version, still served when a client selects it via the
/// `initialize` handshake (dual-era per the 2026-07-28 versioning spec).
const LEGACY_VERSION: &str = "2025-11-25";
const SUPPORTED_VERSIONS: [&str; 2] = [MODERN_VERSION, LEGACY_VERSION];
/// `server/discover` cache TTL (SEP-2549): capabilities are static — 1 hour.
const DISCOVER_TTL_MS: u64 = 3_600_000;
/// `tools/list` cache TTL (SEP-2549): the tool table is a compile-time constant.
const TOOLS_TTL_MS: u64 = 300_000;
/// Both discovery documents are identical for every caller (compile-time static).
const CACHE_SCOPE: &str = "public";
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
    // Legacy (2025-11-25) semantics are selected by a legacy client's
    // `initialize` request and stay active for this stdio process.
    let mut legacy = false;

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
        if let Some(response) = handle_line(line, &mut legacy) {
            let _ = stdout.write_all(response.as_bytes());
            let _ = stdout.write_all(b"\n");
            let _ = stdout.flush();
        }
    }
}

/// Handle one JSON-RPC line. Returns the response line to write, or `None`
/// for notifications (which get no reply). `legacy` flips to `true` the first
/// time a legacy client sends `initialize` and selects 2025-11-25 semantics
/// for the rest of this stdio process.
fn handle_line(line: &str, legacy: &mut bool) -> Option<String> {
    // Parse error → -32700 with null id (JSON-RPC 2.0 §5.1).
    let req: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => {
            return Some(error_response(
                &serde_json::Value::Null,
                -32700,
                "parse error",
                None,
            ));
        }
    };

    let method = match req.get("method").and_then(|m| m.as_str()) {
        Some(m) => m,
        None => {
            return Some(error_response(
                &req.get("id").cloned().unwrap_or(serde_json::Value::Null),
                -32600,
                "invalid request: missing 'method'",
                None,
            ));
        }
    };
    let id = match req.get("id") {
        // MCP ids are string|number; an explicit null id is an invalid request.
        Some(v) if v.is_null() => {
            return Some(error_response(
                &serde_json::Value::Null,
                -32600,
                "invalid request: null id",
                None,
            ));
        }
        Some(v) => v.clone(),
        // No id → notification; never replied to (JSON-RPC 2.0 §4.1). The
        // spec's `notifications/cancelled` lands here as a no-op. `ponytail:`
        // this is a synchronous stdio server — no in-flight requests to cancel.
        None => return None,
    };

    let params = req
        .get("params")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    let (result, modern): (Result<serde_json::Value, (i64, String)>, bool) =
        if params.get("_meta").is_some() {
            // Modern protocol: validate the mandatory per-request `_meta` fields,
            // then dispatch on the 2026-07-28 surface.
            match check_meta(&params) {
                Ok(()) => (dispatch(method, &params), true),
                Err(e) => return Some(error_response(&id, e.code, &e.message, e.data)),
            }
        } else if method == "initialize" {
            // Legacy handshake: select 2025-11-25 semantics for this process.
            *legacy = true;
            (method_initialize().map_err(|e| (-32603, e)), false)
        } else if *legacy {
            // Legacy mode: bare requests dispatch without `_meta` or `resultType`.
            (dispatch(method, &params), false)
        } else {
            return Some(error_response(
                &id,
                -32602,
                "invalid params: missing required '_meta' field \
             (io.modelcontextprotocol/protocolVersion, \
             io.modelcontextprotocol/clientCapabilities)",
                None,
            ));
        };

    match result {
        Ok(r) => Some(success_response(&id, r, modern)),
        Err((code, message)) => Some(error_response(&id, code, &message, None)),
    }
}

/// Success envelope. Modern responses carry `resultType: "complete"` plus
/// `_meta.io.modelcontextprotocol/serverInfo` (2026-07-28 §result); legacy
/// responses keep the plain JSON-RPC result shape.
fn success_response(id: &serde_json::Value, result: serde_json::Value, modern: bool) -> String {
    let mut resp = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    });
    if modern {
        if let Some(r) = resp["result"].as_object_mut() {
            r.insert("resultType".into(), serde_json::json!("complete"));
            r.insert(
                "_meta".into(),
                serde_json::json!({
                    "io.modelcontextprotocol/serverInfo": {
                        "name": SERVER_NAME,
                        "version": SERVER_VERSION,
                    },
                }),
            );
        }
    }
    resp.to_string()
}

fn error_response(
    id: &serde_json::Value,
    code: i64,
    message: &str,
    data: Option<serde_json::Value>,
) -> String {
    let mut error = serde_json::json!({ "code": code, "message": message });
    if let Some(d) = data {
        error["data"] = d;
    }
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": error,
    })
    .to_string()
}

/// Validation error with a JSON-RPC code + optional `data` payload (used for
/// the `-32022` UnsupportedProtocolVersion details).
struct MetaError {
    code: i64,
    message: String,
    data: Option<serde_json::Value>,
}

/// Validate the per-request `_meta` object (2026-07-28 §requests): the client
/// MUST declare `io.modelcontextprotocol/protocolVersion` (a supported
/// version) and `io.modelcontextprotocol/clientCapabilities` (an object);
/// `io.modelcontextprotocol/clientInfo` is optional.
fn check_meta(params: &serde_json::Value) -> Result<(), MetaError> {
    let meta = match params.get("_meta").and_then(|m| m.as_object()) {
        Some(m) => m,
        None => {
            return Err(MetaError {
                code: -32602,
                message: "invalid params: missing required '_meta' field".into(),
                data: None,
            });
        }
    };
    let version = match meta
        .get("io.modelcontextprotocol/protocolVersion")
        .and_then(|v| v.as_str())
    {
        Some(v) => v,
        None => {
            return Err(MetaError {
                code: -32602,
                message: "invalid params: '_meta' is missing \
                          'io.modelcontextprotocol/protocolVersion'"
                    .into(),
                data: None,
            });
        }
    };
    if !matches!(version, MODERN_VERSION | LEGACY_VERSION) {
        return Err(MetaError {
            code: -32022,
            message: format!("unsupported protocol version: {version}"),
            data: Some(serde_json::json!({
                "supported": SUPPORTED_VERSIONS,
                "requested": version,
            })),
        });
    }
    if !meta
        .get("io.modelcontextprotocol/clientCapabilities")
        .is_some_and(|c| c.is_object())
    {
        return Err(MetaError {
            code: -32602,
            message: "invalid params: '_meta' is missing \
                      'io.modelcontextprotocol/clientCapabilities'"
                .into(),
            data: None,
        });
    }
    Ok(())
}

/// Dispatch one method to its handler. Errors return a JSON-RPC error code
/// that the caller maps onto the reply. `server/discover` + `tools/list` are
/// static (no external calls) → -32603; `tools/call` failures are client
/// errors → -32602 (transport failures like "cannot connect to …" land here
/// too — `ponytail:` a distinguishable code would need a richer error type
/// than a String).
fn dispatch(method: &str, params: &serde_json::Value) -> Result<serde_json::Value, (i64, String)> {
    match method {
        "server/discover" => method_discover().map_err(|e| (-32603, e)),
        "tools/list" => method_tools_list().map_err(|e| (-32603, e)),
        "tools/call" => method_tools_call(params).map_err(|e| (-32602, e)),
        // `ping` was removed from the 2026-07-28 schema; kept as a harmless
        // no-op for legacy tooling that still probes with it.
        "ping" => Ok(serde_json::json!({})),
        other => Err((-32601, format!("method not found: {other}"))),
    }
}

/// Legacy (2025-11-25) handshake, served only to legacy clients. The response
/// deliberately keeps the legacy shape — no `resultType`/`_meta` envelope.
fn method_initialize() -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "protocolVersion": LEGACY_VERSION,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": SERVER_NAME, "version": SERVER_VERSION },
    }))
}

/// 2026-07-28 `server/discover`: the modern, stateless replacement for
/// `initialize` — no handshake, no negotiation, cacheable for an hour.
fn method_discover() -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "supportedVersions": SUPPORTED_VERSIONS,
        "capabilities": { "tools": {} },
        "instructions": "brain-server MCP: tools map 1:1 onto the brain-server HTTP API (POST /recall, /ingest, /ump/*).",
        "ttlMs": DISCOVER_TTL_MS,
        "cacheScope": CACHE_SCOPE,
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
                    "sources": { "type": "array", "items": { "type": "string" }, "description": "OR filter over ingest kind: memory | markdown | structured | manual | vault." },
                    "source": { "type": "string", "description": "Ingest kind (memory|markdown|structured|manual|vault), retrieval leg (vector|fts|graph), or both. Unknown values are rejected." },
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
                    "sources": { "type": "array", "items": { "type": "string" }, "description": "OR filter over ingest kind: memory | markdown | structured | manual | vault." },
                    "domain": { "type": "string", "description": "Optional domain label." },
                    "source": { "type": "string", "description": "Ingest kind (memory|markdown|structured|manual|vault), retrieval leg (vector|fts|graph), or both. Unknown values are rejected." },
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
        },
        {
            "name": "ump.capabilities",
            "description": "UMP 1.0 negotiation handshake (v1.17.3): conformance level (L3/L2), kinds, bindings, retrieval signals, max_recall, writable, audit. GET /ump/capabilities.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "ump.remember",
            "description": "Store a memory record (UMP §3.3). Accepts a partial record `{record: {...}}`; lowered through the structured-ingest path. Consent: a declared `scope.owner` must match the authenticated principal. L3: signed records verify against the operator key. Returns `{id, result: created|merged|rejected}`.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "record": {
                        "type": "object",
                        "description": "The UMP record (body, scope, time, lifecycle, links, integrity, ...)."
                    }
                },
                "required": ["record"]
            }
        },
        {
            "name": "ump.get",
            "description": "Read one record by id (UMP §5.3): integrity re-verified on read; other owners' rows are §2.7-redacted. GET /ump/memory/{id}.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "integer", "description": "Chunk / record id." }
                },
                "required": ["id"]
            }
        },
        {
            "name": "ump.recall",
            "description": "Ranked recall with per-result signals (UMP §3.2): `{results: [{record, signals, score}]}`. Runs the shared deterministic recall core; `filter.kind` maps UMP kinds to brain memory_kinds; `filter.valid_at` maps to the bi-temporal filter. `ranking_hints` accepted but ignored.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "The recall query." },
                    "limit": { "type": "integer", "description": "Max results (clamped to max_recall)." },
                    "scope": { "type": "string", "description": "Project scope hint." },
                    "filter": {
                        "type": "object",
                        "description": "Kind + bi-temporal filter.",
                        "properties": {
                            "kind": { "type": "string", "description": "UMP kind (memory|procedure|decision|goal|concept|...)." },
                            "valid_at": { "type": "string", "description": "ISO-8601: only records valid at this time." }
                        }
                    },
                    "ranking_hints": { "type": "object", "description": "Accepted but ignored (no rank steering)." }
                },
                "required": ["query"]
            }
        },
        {
            "name": "ump.revise",
            "description": "Patch a record (UMP §3.5): `{id, patch}` — the patch is deep-merged over the stored record (`id`/`integrity` are server-authoritative and never patched), stored as a new revision, and the old chunk is expired via supersession (default recall returns the new revision).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "integer", "description": "Id of the record to revise." },
                    "patch": { "type": "object", "description": "Partial record; deep-merged over the stored one." }
                },
                "required": ["id", "patch"]
            }
        },
        {
            "name": "ump.forget",
            "description": "Delete or soft-delete a record (UMP §3.4): `{id, reason?, hard?}`. `hard:false` (default) flags the row + tombstone + audit; `hard:true` runs the v1.14 erase path. Returns `{result: tombstoned}`.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "integer", "description": "Id of the record to forget." },
                    "reason": { "type": "string", "description": "Optional forget reason (audited)." },
                    "hard": { "type": "boolean", "description": "Hard erase (default false)." }
                },
                "required": ["id"]
            }
        },
        {
            "name": "ump.feedback",
            "description": "Record outcome feedback (UMP §3.6): `{id, outcome, reason?}` with outcome in followed|overridden|ignored|contradicted. Mapped to the suggest-feedback last-wins upsert (followed → accept, rest → dismiss).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "integer", "description": "Record id." },
                    "outcome": { "type": "string", "enum": ["followed", "overridden", "ignored", "contradicted"], "description": "How the record's guidance was used." },
                    "reason": { "type": "string", "description": "Optional free-text reason (hashed at rest)." }
                },
                "required": ["id", "outcome"]
            }
        },
        {
            "name": "ump.audit",
            "description": "Reference audit facility (UMP §9): recent hash-chained audit rows `{kind?, limit?, offset?}`. Admin + tenant-scoped.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "description": "Audit kind filter (e.g. auth)." },
                    "limit": { "type": "integer", "description": "Max rows (default 100)." },
                    "offset": { "type": "integer", "description": "Pagination offset." }
                }
            }
        },
        {
            "name": "ump.audit.verify",
            "description": "Fresh full audit-chain verification (UMP §9). Admin. Returns `{ok: bool}`.",
            "inputSchema": { "type": "object", "properties": {} }
        }
    ]);
    Ok(serde_json::json!({
        "tools": tools,
        "ttlMs": TOOLS_TTL_MS,
        "cacheScope": CACHE_SCOPE,
    }))
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
        // v1.17.3 "UMP" M3: the §4.1 PRIMARY binding. Dispatch is derived from
        // the `ump_route` table below — one source of truth for method/path.
        name if ump_route(name).is_some() => tool_ump_call(name, &args)?,
        "brain_search" => tool_brain_search(&args)?,
        "brain_recall" => tool_brain_recall(&args)?,
        "brain_ingest" => tool_brain_ingest(&args)?,
        other => return Err(format!("unknown tool: {other}")),
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

/// Route table for the v1.17.3 UMP §4.1 PRIMARY binding: tool name → HTTP
/// method + path template. `{id}` is substituted by the caller. Pure — the
/// tool-list entries and the dispatch match both mirror this table, and the
/// unit tests pin all three against each other.
fn ump_route(name: &str) -> Option<(&'static str, &'static str)> {
    match name {
        "ump.capabilities" => Some(("GET", "/ump/capabilities")),
        "ump.remember" => Some(("POST", "/ump/remember")),
        "ump.get" => Some(("GET", "/ump/memory/{id}")),
        "ump.recall" => Some(("POST", "/ump/recall")),
        "ump.revise" => Some(("POST", "/ump/revise")),
        "ump.forget" => Some(("POST", "/ump/forget")),
        "ump.feedback" => Some(("POST", "/ump/feedback")),
        "ump.audit" => Some(("POST", "/ump/audit")),
        "ump.audit.verify" => Some(("GET", "/ump/audit/verify")),
        _ => None,
    }
}

/// Resolve a tool name + args to a concrete URL path (pure; `{id}` templates
/// are substituted from the integer `id` argument).
fn ump_path(name: &str, args: &serde_json::Value) -> Result<String, String> {
    let (_, template) = ump_route(name).ok_or_else(|| format!("unknown ump tool: {name}"))?;
    if template.contains("{id}") {
        let id = args
            .get("id")
            .and_then(|v| v.as_u64())
            .ok_or("tool requires integer 'id'")?;
        Ok(template.replace("{id}", &id.to_string()))
    } else {
        Ok(template.to_string())
    }
}

/// Execute an `ump.*` tool: thin HTTP proxy mirroring the §3 wire shapes.
/// GET tools pass args nowhere (query-less); POST tools send the args object
/// verbatim as the JSON body — the §3 shapes are the handler's request types.
fn tool_ump_call(name: &str, args: &serde_json::Value) -> Result<String, String> {
    let (method, _) = ump_route(name).ok_or_else(|| format!("unknown ump tool: {name}"))?;
    let path = ump_path(name, args)?;
    let resp = match method {
        "GET" => get(&base_url(), &path, &[], auth_token().as_deref())?,
        _ => post(
            &base_url(),
            &path,
            &[],
            "application/json",
            &args.to_string(),
            auth_token().as_deref(),
        )?,
    };
    Ok(format_response(resp.status, &resp.body))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The nine v1.17.3 `ump.*` tools (§4.1 PRIMARY binding).
    const UMP_TOOLS: [&str; 9] = [
        "ump.capabilities",
        "ump.remember",
        "ump.get",
        "ump.recall",
        "ump.revise",
        "ump.forget",
        "ump.feedback",
        "ump.audit",
        "ump.audit.verify",
    ];

    /// Expected method/path per tool — the pinned wire contract.
    fn expected_routes() -> Vec<(&'static str, &'static str, &'static str)> {
        vec![
            ("ump.capabilities", "GET", "/ump/capabilities"),
            ("ump.remember", "POST", "/ump/remember"),
            ("ump.get", "GET", "/ump/memory/{id}"),
            ("ump.recall", "POST", "/ump/recall"),
            ("ump.revise", "POST", "/ump/revise"),
            ("ump.forget", "POST", "/ump/forget"),
            ("ump.feedback", "POST", "/ump/feedback"),
            ("ump.audit", "POST", "/ump/audit"),
            ("ump.audit.verify", "GET", "/ump/audit/verify"),
        ]
    }

    #[test]
    fn tool_list_contains_all_nine_ump_tools() {
        let list = method_tools_list().expect("tools/list");
        let tools = list["tools"].as_array().expect("tools array");
        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        for tool in UMP_TOOLS {
            assert!(
                names.contains(&tool),
                "missing ump tool in tools/list: {tool}"
            );
        }
        assert_eq!(tools.len(), 12, "3 brain_* + 9 ump.*");
    }

    #[test]
    fn ump_route_table_pins_methods_and_paths() {
        for (name, method, path) in expected_routes() {
            assert_eq!(ump_route(name), Some((method, path)), "route for {name}");
        }
        assert_eq!(
            ump_route("brain_recall"),
            None,
            "non-ump names stay unlisted"
        );
    }

    #[test]
    fn ump_path_substitutes_id_from_args() {
        let args = serde_json::json!({ "id": 42 });
        assert_eq!(ump_path("ump.get", &args).expect("path"), "/ump/memory/42");
        assert_eq!(
            ump_path("ump.recall", &serde_json::json!({ "query": "x" })).expect("path"),
            "/ump/recall"
        );
        let err = ump_path("ump.get", &serde_json::json!({})).expect_err("missing id");
        assert!(err.contains("'id'"), "error: {err}");
    }

    #[test]
    fn discover_returns_modern_protocol_surface() {
        let mut legacy = false;
        let out = handle_line(
            r#"{"jsonrpc":"2.0","id":1,"method":"server/discover","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}}}"#,
            &mut legacy,
        )
        .expect("reply");
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        let result = &v["result"];
        assert_eq!(result["resultType"], "complete");
        assert_eq!(
            result["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
            SERVER_NAME
        );
        assert_eq!(result["supportedVersions"][0], MODERN_VERSION);
        assert_eq!(result["supportedVersions"][1], LEGACY_VERSION);
        assert_eq!(result["capabilities"]["tools"], serde_json::json!({}));
        assert_eq!(result["ttlMs"], DISCOVER_TTL_MS);
        assert_eq!(result["cacheScope"], CACHE_SCOPE);
        assert_eq!(v["id"], 1);
        assert!(!legacy, "discover must not flip legacy mode");
    }

    #[test]
    fn tools_list_modern_is_complete_and_cacheable() {
        let mut legacy = false;
        let out = handle_line(
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}}}"#,
            &mut legacy,
        )
        .expect("reply");
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        let result = &v["result"];
        assert_eq!(result["resultType"], "complete");
        assert_eq!(result["tools"].as_array().expect("tools").len(), 12);
        assert_eq!(result["ttlMs"], TOOLS_TTL_MS);
        assert_eq!(result["cacheScope"], CACHE_SCOPE);
    }

    #[test]
    fn bare_request_without_meta_is_rejected() {
        let mut legacy = false;
        let out = handle_line(
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/list","params":{}}"#,
            &mut legacy,
        )
        .expect("reply");
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["error"]["code"], -32602);
        assert_eq!(v["id"], 3);
    }

    #[test]
    fn missing_meta_fields_are_rejected() {
        let mut legacy = false;
        // Version present, capabilities missing.
        let out = handle_line(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}}}"#,
            &mut legacy,
        )
        .expect("reply");
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["error"]["code"], -32602);

        // Capabilities present, version missing.
        let out = handle_line(
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{"_meta":{"io.modelcontextprotocol/clientCapabilities":{}}}}"#,
            &mut legacy,
        )
        .expect("reply");
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["error"]["code"], -32602);
    }

    #[test]
    fn unsupported_protocol_version_returns_32022() {
        let mut legacy = false;
        let out = handle_line(
            r#"{"jsonrpc":"2.0","id":1,"method":"server/discover","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"1900-01-01","io.modelcontextprotocol/clientCapabilities":{}}}}"#,
            &mut legacy,
        )
        .expect("reply");
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["error"]["code"], -32022);
        assert_eq!(v["error"]["data"]["supported"][0], MODERN_VERSION);
        assert_eq!(v["error"]["data"]["requested"], "1900-01-01");
    }

    #[test]
    fn initialize_selects_legacy_mode() {
        let mut legacy = false;
        let out = handle_line(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"legacy","version":"1.0"}}}"#,
            &mut legacy,
        )
        .expect("reply");
        assert!(legacy, "initialize must select legacy mode");
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        let result = &v["result"];
        assert!(result.get("resultType").is_none(), "legacy envelope only");
        assert_eq!(result["protocolVersion"], LEGACY_VERSION);
        assert_eq!(result["serverInfo"]["name"], SERVER_NAME);

        // After initialize, bare legacy requests dispatch without `_meta`.
        let out = handle_line(
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
            &mut legacy,
        )
        .expect("reply");
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert!(v.get("error").is_none(), "legacy tools/list succeeds");
        assert!(v["result"].get("resultType").is_none());
        assert_eq!(v["result"]["tools"].as_array().expect("tools").len(), 12);
    }

    #[test]
    fn unknown_tool_is_a_protocol_error() {
        let mut legacy = false;
        let out = handle_line(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}},"name":"nope","arguments":{}}}"#,
            &mut legacy,
        )
        .expect("reply");
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["error"]["code"], -32602);
        assert!(v["error"]["message"]
            .as_str()
            .expect("msg")
            .contains("nope"));
        assert!(v.get("result").is_none(), "no result for an unknown tool");
    }

    #[test]
    fn parse_error_returns_32700_with_null_id() {
        let mut legacy = false;
        let out = handle_line("{not json", &mut legacy).expect("reply");
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(v["error"]["code"], -32700);
        assert_eq!(v["id"], serde_json::Value::Null);
    }
}
