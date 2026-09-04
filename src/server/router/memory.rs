//! The memory family: the ingestion/recall CRUD surface, embeddings,
//! quarantine, the graph reads, the domain/profile/role registries, the
//! legacy /ingest + /memory/{id} plugin contract, sources/connectors,
//! verify, suggest, procedure, consolidation, and the HITL proposal
//! queue + GDPR lifecycle (decayed/export/purge). Handlers moved
//! verbatim from main.rs. The 1 GiB import dial's sub-router lives here
//! too — `import_router()` — and MUST be merged after the shared 1 MiB
//! body limit (see mod.rs, the F-49a comment).

use anyhow::Context;
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Path, Query, State},
    response::{IntoResponse, Json, Response},
    routing::{delete, get, post},
};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use xxhash_rust::xxh3::{xxh3_64, xxh3_64_with_seed};
use zerocopy::IntoBytes;

use crate::config::{MAX_EXPLAIN_BYTES, MAX_MULTI_GET, MAX_QUERY_LENGTH, MODEL_ID};
use crate::handlers;
use crate::http_limit::process_rss_mib;
use crate::search as search_mod;
use crate::search::{perform_search_with_prf, query::QueryDoc};

use crate::audit;
use crate::server::bootstrap::AppState;
use crate::{
    chunker, config, domain_router, graph_read, hygiene, linker, screen, sources, trace, vault,
};
use crate::{
    config::{DEFAULT_K, MAX_K, MAX_REQUEST_SIZE},
    http_limit::TrackerEntry,
};
use std::time::Duration as StdDuration;
use tokio::{task, time::timeout};

/// The three legacy surfaces (`/add`, `/ingest/memory`, `/search`) that
/// sit BEFORE the `Deprecation` header route_layer in `mod.rs` — the
/// header's application set is order-frozen (see mod.rs).
pub fn legacy_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/add", post(add_chunk))
        .route("/ingest/memory", post(ingest_memory))
        .route("/search", get(search))
    // Legacy contract markers: `/add` and GET `/search` are superseded by
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/embeddings", post(embeddings))
        .route("/ingest/markdown", post(ingest_markdown))
        .route("/reindex", post(reindex))
        .route("/get/{id}", get(get_chunk))
        .route("/multi-get", post(multi_get))
        // quarantine operator surface. `GET /quarantine` lists
        // flagged chunks; release clears the flag; delete purges the chunk.
        .route("/quarantine", get(list_quarantined))
        .route("/quarantine/{id}/release", post(release_quarantine))
        .route("/quarantine/{id}/delete", post(delete_quarantine))
        .route("/graph/entity/{name}", get(get_entity))
        .route("/graph/relations", get(get_relations))
        .route("/graph/traverse", get(traverse_graph))
        .route("/graph/relationships/{id}/history", get(get_edge_history))
        // Plugin API (contract: API_CONTRACT.md). Wire is locked.
        .route("/recall", post(handlers::recall::recall))
        .route("/ingest", post(handlers::ingest::ingest))
        // the UMP 1.0 HTTP ops binding. Capabilities +
        // `/.well-known/ump.json` are PUBLIC (negotiation handshake); the
        // rest are authz-gated per the §3.3 matrix.
        .route("/memory/{id}", delete(handlers::forget::forget))
        .route(
            "/domains",
            get(handlers::domains::domains).post(handlers::domains::create_domain),
        )
        .route("/domains/{name}", delete(handlers::domains::delete_domain))
        // bulk relabel of chunks across domains (the non-re-ingest
        // fix for the 99%-in-global corpus). A POST on a distinct path, so it
        // cannot collide with the `/domains/{name}` DELETE above.
        .route("/domains/move", post(handlers::domains::move_domains))
        // one-shot recompute sweep over every domain's centroid.
        .route(
            "/domains/recompute",
            post(handlers::domains::recompute_domains),
        )
        // per-domain lifecycle. Vacuum reclaims free pages; export
        // streams a consistent snapshot; import restores a snapshot into a new
        // domain name. `name` is validated inside each handler.
        .route(
            "/domains/{name}/vacuum",
            post(handlers::domains::vacuum_domain),
        )
        .route(
            "/domains/{name}/export",
            get(handlers::domains::export_domain),
        )
        // the preset API. Reads are Read-gated; writes
        // (profile upsert + domain binding) are Admin + audited. Dual-method
        // paths register GET first then POST (the /retention precedent) so the
        // authz source-scan lands on the Admin POST as the conservative check.
        .route("/profiles", get(handlers::profiles::list_profiles))
        .route("/profiles/{name}", get(handlers::profiles::get_profile))
        .route("/profiles/{name}", post(handlers::profiles::upsert_profile))
        .route(
            "/domains/{name}/profile",
            get(handlers::profiles::domain_profile_get),
        )
        .route(
            "/domains/{name}/profile",
            post(handlers::profiles::domain_profile_bind),
        )
        // the role API. Reads are Read-gated; writes
        // (role upsert) are Admin + audited. Dual-method on {name}: GET then
        // POST, so the authz source-scan lands on the Admin POST as the
        // conservative check (the /retention + /profiles precedent).
        .route("/roles", get(handlers::roles::list_roles))
        .route("/roles/{name}", get(handlers::roles::get_role))
        .route("/roles/{name}", post(handlers::roles::upsert_role))
        // legal hold — place/release/list holds that
        // freeze ids against erasure (decay, /purge, DSAR).
        .route("/legal-hold", post(handlers::holds::post_legal_hold))
        .route(
            "/legal-hold/{id}/release",
            post(handlers::holds::release_legal_hold),
        )
        .route("/legal-holds", get(handlers::holds::list_legal_holds))
    // the breach-notification workflow. Human-
    // opened by the DPO role; every event is hash-chained into the audit.
}

#[derive(Deserialize)]
pub struct SearchParams {
    q: String,
    #[serde(default)]
    k: Option<usize>,
    #[serde(default)]
    source: Option<String>,
    /// Comma-separated multi-source OR scope (GET-friendly). Empty = none.
    #[serde(default)]
    sources: Option<String>,
    #[serde(default)]
    since: Option<String>,
    /// Structured query: lexical/keyword terms (exact, code, phrases, `-excl`).
    #[serde(default)]
    lex: Option<String>,
    /// Structured query: semantic intent (used for the dense embedding).
    #[serde(default)]
    vec: Option<String>,
    /// Structured query: caller-supplied hypothetical answer (takes priority
    /// over `vec` for the dense embedding when present).
    #[serde(default)]
    hyde: Option<String>,
    /// Structured query: free-form intent label (recorded for provenance).
    #[serde(default)]
    intent: Option<String>,
    /// Retrieval profile hint (passthrough).
    #[serde(default)]
    profile: Option<String>,
    /// When set, include per-stage telemetry + the query plan in the response.
    #[serde(default)]
    explain: bool,
    /// include quarantined (`flagged`) chunks in results.
    #[serde(default)]
    include_flagged: bool,
    /// point-in-time recall. RFC3339 instant; returns the
    /// revision current at that time (historical mode).
    #[serde(default)]
    as_of: Option<String>,
    /// include structured `Evidence` (time + lifecycle +
    /// links) on every hit.
    #[serde(default)]
    evidence: bool,
    /// target domain. When set, search is scoped to this
    /// domain's pool (multi-db mode) or filtered by the `domain` column (shim
    /// mode). Falls back to "global" when absent.
    #[serde(default)]
    domain: Option<String>,
    /// enable the graph-PPR retriever as a third RRF leg.
    /// Default `true` (the connected multi-hop recall). Callers may pass
    /// `graph=false` per-request; the kill switch is
    /// `BRAIN_RECALL_GRAPH_ENABLED=false`.
    #[serde(default = "crate::config::brain_recall_graph_enabled")]
    graph: bool,
}

#[derive(Deserialize)]
pub struct AddRequest {
    pub text: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default = "default_source")]
    pub source: String,
}

/// the closed `/add` `source` vocabulary for
/// JWT (agent) principals. Values match what `knowledge.source` stores — the
/// search vocabulary (memory|markdown|structured|vault, `search_mod::query::
/// INGEST_KINDS`) plus the connector family kinds mapped from the shipped
/// `CONNECTOR_KINDS`. `manual` is DELIBERATELY EXCLUDED: it is the interactive
/// loopback-only value that derives `origin:human` (`gate::origin_for_source`),
/// so a token-authenticated agent cannot forge human authorship.
const ADD_SOURCES_FOR_JWT: &[&str] = &[
    "memory",
    "markdown",
    "structured",
    "vault",
    // Connector family kinds (see src/connector/kind.rs::CONNECTOR_KINDS).
    "github",
    "crm",
    "slack",
    "email",
    "jira",
    "linear",
    "notion",
    "hris",
    "ehr",
];

#[derive(Serialize, Default)]
struct AddResponse {
    success: bool,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    chunk_id: Option<i64>,
    /// every real inserted rowid from this request
    /// (empty for the single-chunk `/add` path and for no-op/duplicate runs).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    chunk_ids: Vec<i64>,
    /// count of chunks actually inserted.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    entries_added: Option<i64>,
    /// count of dedup-skipped entries.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    duplicates_skipped: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl AddResponse {
    /// Legacy error envelope: `{ success: false, status: "error", error: msg }`.
    /// The shape `/add` and `/ingest/memory` have always returned on failure.
    fn error(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            status: "error".to_string(),
            error: Some(msg.into()),
            ..Default::default()
        }
    }
}

fn default_source() -> String {
    "manual".to_string()
}

#[derive(Deserialize)]
#[serde(untagged)]
enum EmbeddingsInput {
    Single(String),
    Batch(Vec<String>),
}

/// cap on `/v1/embeddings` batch size. Bounds the response
/// amplification (each input → 256 floats × 4 bytes in the buffered JSON).
/// 64 is the OpenAI default; matches the upstream contract.
const MAX_EMBEDDING_BATCH: usize = 64;

#[derive(Deserialize)]
pub(crate) struct EmbeddingsRequest {
    #[serde(deserialize_with = "deserialize_input")]
    input: EmbeddingsInput,
    #[serde(default = "default_model")]
    model: String,
}

fn deserialize_input<'de, D>(deserializer: D) -> Result<EmbeddingsInput, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum RawInput {
        Single(String),
        Batch(Vec<String>),
    }

    let raw = RawInput::deserialize(deserializer)?;
    Ok(match raw {
        RawInput::Single(s) => EmbeddingsInput::Single(s),
        RawInput::Batch(v) => EmbeddingsInput::Batch(v),
    })
}

fn default_model() -> String {
    MODEL_ID.to_string()
}

#[derive(Deserialize)]
pub(crate) struct MarkdownPayload {
    content: String,
    title: Option<String>,
    /// absolute file path for vault ingest provenance. When set, the
    /// server treats this as a vault file: dedup + replace are scoped to this
    /// path, and frontmatter/wikilinks are parsed into the knowledge graph.
    #[serde(default)]
    source_path: Option<String>,
    /// Target domain for the ingested content. When set, overrides any
    /// `domain:` key in YAML frontmatter. Falls back to `"global"` when
    /// neither is present.
    #[serde(default)]
    domain: Option<String>,
    /// When true and source_path is set, re-process the file even if content
    /// is unchanged — sweeps existing chunks before re-inserting. Used after
    /// linker upgrades to regenerate the knowledge graph for all docs.
    #[serde(default)]
    replace: bool,
}

#[derive(Deserialize)]
pub(crate) struct RelationsQuery {
    from: Option<String>,
    to: Option<String>,
}

/// graph endpoints read a `?limit=` that is clamped to
/// `MAX_GRAPH_EDGES` (bounded output on the operator Graph surface).
#[derive(Deserialize)]
pub(crate) struct GraphLimit {
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Deserialize)]
pub(crate) struct TraverseQuery {
    /// Start entity. `name` and `entity` accepted as aliases (the response
    /// field is `entity`, so callers may mirror it back). Docs canonical: `start`.
    #[serde(alias = "name", alias = "entity")]
    start: Option<String>,
    max_depth: Option<u8>,
    /// when true, walk edges across every known domain pool
    /// (labelled per hop). When false (default) the resolved domain only.
    #[serde(default)]
    cross_domain: bool,
    /// bi-temporal point-in-time traversal. RFC3339 or
    /// `YYYY-MM-DD`; edges whose valid-interval (valid_at, invalid_at) does
    /// NOT contain this instant are skipped (Graphiti semantics).
    #[serde(default)]
    at: Option<String>,
    /// restrict the walk to edges whose `relation_type`
    /// matches this value (exact match) or prefix (if it ends with `:`,
    /// e.g. `causes:` for the causal subgraph). Empty/absent = walk all
    /// edge types. Opt-in filter — does not claim causality.
    #[serde(default)]
    kind: Option<String>,
    /// when true, the response includes a `paths` array
    /// with structured per-hop explanations (from_entity, relation, to_entity,
    /// valid_at, invalid_at). The flat `traversal` array stays for back-compat.
    #[serde(default)]
    explain: bool,
}

#[derive(Debug)]
pub enum AppError {
    BadRequest(&'static str),
    NotFound(&'static str),
    /// HTTP 507 — over capacity envelope.
    InsufficientStorage(String),
    /// HTTP 403 — AuthZ gate. The legacy
    /// main.rs write handlers use `AppError`, so the JWT AuthZ gate needs a
    /// 403 channel here (the modern `HandlerError` paths already have one).
    Forbidden(String),
    /// HTTP 409 — an erasure refused by an active legal
    /// hold (the quarantine delete path runs on the legacy `AppError` type).
    Conflict(String),
    /// HTTP 202 — Seatbelt review posture: the write became a pending
    /// proposal instead of inserting. A success-shaped body on a non-error
    /// status (rendered verbatim, no `error` envelope).
    Accepted(serde_json::Value),
    Internal(#[allow(dead_code)] String),
}

impl axum::response::IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        use axum::http::StatusCode;
        // Coerce every arm to `(StatusCode, String)` so the `InsufficientStorage`
        // variant (which carries a dynamic message) type-checks alongside the
        // `&'static str` arms.
        let (status, msg): (StatusCode, String) = match self {
            AppError::BadRequest(s) => (StatusCode::BAD_REQUEST, s.to_string()),
            AppError::NotFound(s) => (StatusCode::NOT_FOUND, s.to_string()),
            AppError::InsufficientStorage(s) => (StatusCode::INSUFFICIENT_STORAGE, s),
            AppError::Forbidden(s) => (StatusCode::FORBIDDEN, s),
            AppError::Conflict(s) => (StatusCode::CONFLICT, s),
            AppError::Accepted(v) => return (StatusCode::ACCEPTED, Json(v)).into_response(),
            AppError::Internal(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal error".to_string(),
            ),
        };
        (status, Json(serde_json::json!({ "error": msg }))).into_response()
    }
}

/// capacity guard for the main.rs ingest handlers that use
/// `AppError` (the legacy `/add` + `/ingest/memory`). Returns
/// `AppError::InsufficientStorage` when the envelope is exceeded. Best-effort:
/// fails open if the pool or measurement errors. Mirrors
/// `handlers::guard_capacity` (which uses `HandlerError` for the `/ingest` +
/// `/ingest/markdown` paths).
pub(crate) fn guard_capacity(state: &AppState) -> Result<(), AppError> {
    use crate::capacity::{CapacityEnvelope, capacity_target, classify};
    let Some(conn) = state.pool.get().ok() else {
        return Ok(());
    };
    let docs: usize = conn
        .query_row("SELECT COUNT(*) FROM knowledge", [], |r| r.get::<_, i64>(0))
        .unwrap_or(0)
        .max(0) as usize;
    let db_mib: u64 = std::fs::metadata(&state.db_path)
        .map(|m| m.len() / 1_000_000)
        .unwrap_or(0);
    // CRITICAL: process RSS, not system-wide (see measure_capacity).
    let rss_mib = process_rss_mib();
    let env = CapacityEnvelope::for_target(capacity_target());
    let status = classify(docs, db_mib, rss_mib, &env);
    if status.blocks_writes() {
        return Err(AppError::InsufficientStorage(format!(
            "capacity_exceeded: docs={docs}/{} db_mib={db_mib}/{} rss_mib={rss_mib}/{}",
            env.max_docs, env.max_db_mib, env.max_rss_mib
        )));
    }
    Ok(())
}

#[inline(always)]
pub async fn add_chunk(
    State(s): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    Json(req): Json<AddRequest>,
) -> Response {
    // AuthZ write gate. `/add` is the legacy
    // path — we return its existing `{ success: false, error }` shape rather
    // than a real 403 so the response stays shape-compatible (mirrors the
    // capacity-guard choice below). `None` principal (no JWT) = superuser.
    if let Err(e) =
        crate::handlers::authorize(&principal.0, crate::auth::Action::Write, "", "global")
    {
        return Json(AddResponse::error(e.inner.message)).into_response();
    }
    // capacity guard. `/add` is the legacy path; we return its existing
    // `{ success: false, error }` shape rather than an HTTP 507 so the
    // response stays shape-compatible. The primary paths (`/ingest`,
    // `/ingest/markdown`) return a proper 507 via HandlerError.
    if let Err(AppError::InsufficientStorage(msg)) = guard_capacity(&s) {
        return Json(AddResponse::error(msg)).into_response();
    }

    // strip reasoning/trace blocks from the raw text before
    // it is embedded/stored (manual `/add` is single explicit text, so the
    // skip-pattern drop is not applied here — that's for batch `/ingest/memory`).
    let text = hygiene::strip_reasoning_blocks(req.text.trim());
    if text.trim().is_empty() {
        return Json(AddResponse::error("text cannot be empty")).into_response();
    }
    // `source` is a trust label, not a
    // free-form field. For a JWT (agent) principal the vocabulary is closed —
    // and deliberately EXCLUDES `manual` (the `origin:human` marker,
    // `gate::origin_for_source`): an agent cannot forge human authorship.
    // `manual` stays the loopback/operator default (the interactive path is
    // the one place human provenance is real).
    let source = req.source.trim().to_string();
    if principal.0.is_some() && !ADD_SOURCES_FOR_JWT.contains(&source.as_str()) {
        return Json(AddResponse::error(format!(
            "invalid source: '{source}' is not allowed for token-authenticated \
             principals; allowed: {}",
            ADD_SOURCES_FOR_JWT.join("|")
        )))
        .into_response();
    }
    // enforce MAX_CONTENT on the legacy /add path too (its siblings
    // /ingest + /ingest/memory + /ingest/markdown all do). Previously /add
    // relied only on the global MAX_REQUEST_SIZE body limit, which is slightly
    // larger — inconsistent + wrong if the body is split across fields.
    if text.len() > crate::handlers::MAX_CONTENT {
        return Json(AddResponse::error(format!(
            "text exceeds {} bytes",
            crate::handlers::MAX_CONTENT
        )))
        .into_response();
    }

    // Seatbelt (Seatbelt): under BRAIN_WRITE_POSTURE=review the agent-facing
    // write surface proposes instead of inserting — no `knowledge` row until
    // an operator approves.
    if crate::config::write_posture() == "review" {
        let proposal = crate::handlers::gate::create_proposal(
            s.clone(),
            principal.0.clone(),
            crate::handlers::gate::ProposalRequest {
                content: text.clone(),
                kind: "fact".to_string(),
                source: Some(source),
                authority: None,
                observed_at: None,
                domain: Some("global".to_string()),
                title: None,
                source_prompt: None,
            },
        )
        .await;
        return proposal
            .map(|p| {
                (
                    axum::http::StatusCode::ACCEPTED,
                    Json(serde_json::json!({
                        "success": true,
                        "status": "proposal_pending",
                        "proposal_id": p.id
                    })),
                )
                    .into_response()
            })
            .unwrap_or_else(axum::response::IntoResponse::into_response);
    }

    // injection screen. Now the full two-layer
    // screen ([`screen::screen`] = blocklist + optional classifier). `Reject`
    // keeps the old HTTP-400 shape; `Quarantine` ingests then flags post-insert;
    // `Allow` disables the screen. The screen runs inside the blocking closure
    // so the (opt-in) classifier never blocks the async runtime.
    let model = Arc::clone(&s.model);
    let pool = s.pool.clone();
    let title = req.title.filter(|t| !t.is_empty());
    // record the creating principal (JWT `sub`) so `/dsar` + `/purge`
    // can locate by subject. `None` (loopback/opaque) keeps the legacy NULL.
    let owner = crate::handlers::gate::principal_to_owner(&principal.0);

    let add_future = task::spawn_blocking(move || {
        let screen_result = screen::screen(&text, title.as_deref().unwrap_or(""));
        let quarantine = match screen_result {
            screen::ScreenResult::Reject => {
                return AddResponse::error("Input contains suspicious patterns");
            }
            screen::ScreenResult::Quarantine => true,
            screen::ScreenResult::Clean => false,
        };

        let embedding = model.encode_one(&text);
        if embedding.is_empty() {
            return AddResponse::error("Embedding generation failed");
        }

        let content_hash = format!("{:016x}", xxh3_64(text.as_bytes()));
        let mut conn = match pool.get() {
            Ok(c) => c,
            Err(e) => {
                return AddResponse::error(format!("DB connection failed: {}", e));
            }
        };

        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM knowledge WHERE content_hash=? LIMIT 1",
                [&content_hash],
                |r| r.get::<_, i32>(0),
            )
            .unwrap_or(0)
            == 1;

        if exists {
            return AddResponse {
                success: true,
                status: "duplicate".to_string(),
                chunk_id: Some(0),
                ..Default::default()
            };
        }

        let tx = match conn.transaction() {
            Ok(t) => t,
            Err(e) => {
                return AddResponse::error(format!("Transaction failed: {}", e));
            }
        };

        if let Err(e) = tx.execute(
            "INSERT INTO knowledge(content, title, source, content_hash, owner, origin)
             VALUES(?, ?, ?, ?, ?, ?)",
            params![
                text,
                title,
                source,
                content_hash,
                &owner,
                crate::gate::origin_for_source(Some(&source)),
            ],
        ) {
            return AddResponse::error(format!("Insert failed: {}", e));
        }

        let chunk_id = tx.last_insert_rowid();
        if chunk_id > 0 {
            // ── store quantized vectors in vec0 (int8 + binary) ────
            if let Err(e) = tx.execute(
                "INSERT INTO vec_knowledge(knowledge_id, embedding_int8, embedding_bit, source, created_at)
                 VALUES (?1, vec_quantize_int8(?2, 'unit'), vec_quantize_binary(?2), ?3, datetime('now'))",
                params![chunk_id, embedding.as_bytes(), &source],
            ) {
                return AddResponse::error(format!("vec0 insert failed: {}", e));
            }

            // raw f32 vectors are no longer written to the legacy
            // `embeddings` JSON column. vec0 (int8 + binary) is the sole write
            // target. The `embeddings` table is retained read-only for one-time
            // backfill of DBs created before the vec0 store existed (see run_migration).

            // under Quarantine policy, flag the row IN-TX, BEFORE the
            // commit: the flag write is part of the
            // ingest, so a failure rolls the whole chunk back — the
            // `/ingest/memory` posture. Previously the flag ran post-commit:
            // a failed flag write left the injection chunk durably stored
            // `flagged = 0` and retrievable while the caller was told it
            // failed. `flag_if_quarantined`'s "never stored clean" doc is
            // now true on this path too.
            if let Err(e) = screen::flag_if_quarantined(&tx, chunk_id, quarantine) {
                return AddResponse::error(format!("quarantine flag failed: {e}"));
            }

            if let Err(e) = tx.commit() {
                return AddResponse::error(format!("Commit failed: {}", e));
            }

            // audit successful ingest (hash only, never raw text).
            audit::record(
                &conn,
                audit::AuditKind::Ingest,
                "api",
                &content_hash,
                audit::AuditStatus::Ok,
                &source,
            );

            AddResponse {
                success: true,
                status: "created".to_string(),
                chunk_id: Some(chunk_id),
                ..Default::default()
            }
        } else {
            AddResponse::error("Failed to get chunk_id")
        }
    });

    match timeout(StdDuration::from_secs(30), add_future).await {
        Ok(Ok(resp)) => Json(resp).into_response(),
        Ok(Err(_)) => Json(AddResponse::error("Task join error")).into_response(),
        Err(_) => Json(AddResponse::error("Request timed out")).into_response(),
    }
}

pub async fn search(
    State(s): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    Query(p): Query<SearchParams>,
) -> (axum::http::StatusCode, Json<serde_json::Value>) {
    // AuthZ read gate. Legacy shape — see `/add`.
    // the gate must match the pool the query
    // actually runs against (`p.domain`, defaulting to the caller's tenant
    // domain) — authorizing `global` while querying a foreign pool let a
    // tenant-scoped principal read by name. `None` principal (loopback/
    // opaque) stays superuser; loopback behavior unchanged.
    let target_domain = p
        .domain
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("global");
    if let Err(e) =
        crate::handlers::authorize(&principal.0, crate::auth::Action::Read, "", target_domain)
    {
        return (
            axum::http::StatusCode::OK,
            Json(serde_json::json!({ "success": false, "error": e.inner.message })),
        );
    }
    let q = p.q.trim().to_string();
    if q.len() > MAX_QUERY_LENGTH {
        return (
            axum::http::StatusCode::OK,
            Json(serde_json::json!({ "success": false, "error": "Query too long" })),
        );
    }
    if screen::contains_suspicious_pattern(&q) {
        return (
            axum::http::StatusCode::OK,
            Json(serde_json::json!({
                "success": false,
                "error": "Input contains suspicious patterns"
            })),
        );
    }
    // parse `source` once. Ingest-kind → SQL equality;
    // retrieval-leg → post-fusion filter; "both"/omitted → unrestricted. Unknown
    // values get HTTP 422 before any DB/embed work (the one place this legacy
    // 200-envelope endpoint fails loud, matching `POST /recall`).
    let source_filter = p
        .source
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(search_mod::query::parse_source_filter)
        .transpose();
    let (source_kind, source_leg) = match source_filter {
        Ok(f) => search_mod::query::split_source_filter(f.as_ref()),
        Err(e) => {
            return (
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({ "success": false, "error": e.to_string() })),
            );
        }
    };

    // Lower the legacy GET params into the structured QueryDoc. The old
    // raw `lex` string maps to LexSpec.terms (now FTS5-quoted, strictly safer).
    // The domain label the authorize already validated must also SCOPE the
    // query: in shim mode the pool is shared by every tenant, so without the
    // SQL predicate a `read:<t>/global` principal would rank other tenants'
    // rows. Multi-db mirrors /recall: the pool IS the domain, drop the label.
    let doc_domain = if s.registry.is_multi_db() {
        None
    } else {
        Some(p.domain.clone().unwrap_or_else(|| "global".to_string()))
    };
    let mut doc = QueryDoc {
        q: Some(q.clone()),
        domain: doc_domain,
        k: p.k.map(|k| k as u32),
        source: source_kind,
        sources: p
            .sources
            .iter()
            .flat_map(|s| s.split(','))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        since: p.since.filter(|s| !s.trim().is_empty()),
        vec: p.vec.as_ref().map(|s| s.trim().to_string()),
        hyde: p.hyde.as_ref().map(|s| s.trim().to_string()),
        intent: p.intent.filter(|s| !s.trim().is_empty()),
        profile: p.profile.filter(|s| !s.trim().is_empty()),
        explain: p.explain,
        // Quarantine review is operator posture — non-operators are clamped.
        include_flagged: handlers::review_flags_allowed(&principal.0) && p.include_flagged,
        as_of: p.as_of.filter(|s| !s.trim().is_empty()),
        evidence: p.evidence,
        graph: p.graph,
        ..Default::default()
    };
    if let Some(lex) = p.lex.filter(|s| !s.trim().is_empty()) {
        doc.lex.terms.push(lex);
    }

    let k = doc
        .k
        .map(|k| (k as usize).clamp(1, MAX_K))
        .unwrap_or(DEFAULT_K);
    let (qtext, mut filters) = match doc.into_filters() {
        Ok(pair) => pair,
        Err(e) => {
            return (
                axum::http::StatusCode::OK,
                Json(serde_json::json!({ "success": false, "error": e.to_string() })),
            );
        }
    };
    filters.source_leg = source_leg;

    let model = Arc::clone(&s.model);
    // resolve pool from domain param (defaults to global).
    let pool = match handlers::resolve_domain_pool(&s.registry, p.domain.as_deref()) {
        Ok(p) => p,
        Err(e) => {
            return (
                axum::http::StatusCode::OK,
                Json(serde_json::json!({ "success": false, "error": e.inner.message })),
            );
        }
    };

    let task_filters = filters.clone();
    let task_q = qtext.clone();
    let search_future = task::spawn_blocking(move || {
        perform_search_with_prf(&pool, &*model, task_q.clone(), k, &task_filters).map(
            |(mut results, tel)| {
                // Attach faithful, bounded snippets derived from the query.
                let snippet_q = task_filters
                    .lex
                    .clone()
                    .or_else(|| task_filters.embedding_query.clone())
                    .unwrap_or_else(|| task_q.clone());
                for r in &mut results {
                    r.with_snippet(&snippet_q);
                }
                (results, tel, snippet_q)
            },
        )
    });

    (
        axum::http::StatusCode::OK,
        match timeout(StdDuration::from_secs(8), search_future).await {
            Ok(Ok(Ok((mut results, tel, snippet_q)))) => {
                // Enrich each hit with span + source link + highlights via one
                // batched join (best-effort; enrichment failure must not fail search).
                if let Ok(conn) = s.pool.get() {
                    let _ = crate::search::SearchResult::enrich_evidence(
                        &conn,
                        &mut results,
                        &snippet_q,
                        filters.as_of.is_some(),
                    );
                }

                // strip snippet/evidence for flagged hits (after
                // enrichment, which would otherwise re-populate evidence) unless the
                // request opted into flagged rows (operator review path).
                for r in &mut results {
                    screen::suppress_flagged_evidence(r, filters.include_flagged);
                }

                // PII read-projection uniformity — the same
                // `redact_content` gate /recall applies, now on the legacy
                // search surface (loopback/opaque principals stay unmasked).
                // upgrade the search surface from
                // content-only redaction to the full read seam over every stored
                // text field (content, title, snippet, evidence.text/heading) —
                // the same bidi/ZW/markdown-ref boundary `/recall` uses.
                for r in &mut results {
                    r.content = crate::gate::sanitize_read_cow(&r.content, r.pii, &principal.0)
                        .into_owned();
                    r.title = crate::gate::sanitize_read_opt(r.title.take(), r.pii, &principal.0);
                    r.snippet =
                        crate::gate::sanitize_read_opt(r.snippet.take(), r.pii, &principal.0);
                    if let Some(mut ev) = r.evidence.take() {
                        ev.text = crate::gate::sanitize_read_cow(&ev.text, r.pii, &principal.0)
                            .into_owned();
                        if let Some(h) = ev.heading_path {
                            ev.heading_path =
                                crate::gate::sanitize_read_opt(Some(h), r.pii, &principal.0);
                        }
                        r.evidence = Some(ev);
                    }
                }

                // read-event audit for search reads
                // (best-effort, never fails the search the caller asked for).
                if crate::config::audit_read_events(principal.0.is_some())
                    && let Ok(conn) = s.pool.get()
                {
                    let actor = handlers::recall::principal_label(&principal.0);
                    let tenant = handlers::recall::principal_tenant(&principal.0);
                    crate::audit::record_read_event(
                        &conn,
                        crate::audit::AuditKind::Search,
                        &actor,
                        &q,
                        None,
                        &tenant,
                    );
                }

                if p.explain {
                    // Redaction: explain never serializes full `content` beyond
                    // the bounded `evidence.text`/`snippet` windows, so the payload
                    // cannot leak unrelated source text. Drop `content` per result.
                    let mut redacted: Vec<serde_json::Value> = results
                        .iter()
                        .map(serde_json::to_value)
                        .collect::<Result<_, _>>()
                        .unwrap_or_default();
                    for r in redacted.iter_mut() {
                        if let Some(obj) = r.as_object_mut() {
                            obj.remove("content");
                        }
                    }
                    let payload = serde_json::json!({
                        "results": redacted,
                        "telemetry": tel,
                        "query_plan": {
                            "k": k,
                            "lex": filters.lex,
                            "vec": p.vec,
                            "hyde": p.hyde,
                            "intent": filters.intent,
                            "sources": filters.sources.as_ref(),
                            "source": filters.source,
                            "domain": filters.domain,
                            "since": filters.since,
                            "profile": filters.profile,
                            "embedding_query": tel.embedding_query,
                        }
                    });
                    // Hard cap: if the explain payload still exceeds the redaction
                    // budget (e.g. very many hits), return the summary only.
                    if serde_json::to_vec(&payload).map(|b| b.len()).unwrap_or(0)
                        > MAX_EXPLAIN_BYTES
                    {
                        Json(serde_json::json!({
                            "telemetry": tel,
                            "query_plan": {
                                "lex": filters.lex,
                                "vec": p.vec,
                                "hyde": p.hyde,
                                "intent": filters.intent,
                                "sources": filters.sources.as_ref(),
                                "embedding_query": tel.embedding_query,
                                "note": "results omitted: explain payload exceeded size cap",
                            }
                        }))
                    } else {
                        Json(payload)
                    }
                } else {
                    Json(serde_json::json!({ "results": results }))
                }
            }
            Ok(Ok(Err(e))) => Json(serde_json::json!({ "error": e.to_string() })),
            Ok(Err(_)) => Json(serde_json::json!({ "error": "Search task failed" })),
            Err(_) => Json(serde_json::json!({ "error": "Search timed out" })),
        },
    )
}

pub async fn ingest_memory(
    State(s): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    body: Body,
) -> Response {
    // the legacy always-200 JSON-error shell gains
    // two real 4xx rejections for entries that would previously be silently
    // stored or mis-reported; every existing wire shape is unchanged.
    fn error_json(status: &str, message: &str) -> Response {
        (
            axum::http::StatusCode::OK,
            Json(serde_json::json!({ "success": false, "status": status, "message": message })),
        )
            .into_response()
    }
    // AuthZ write gate. Legacy shape — see
    // `/add`. `None` principal (no JWT) = superuser.
    if let Err(e) =
        crate::handlers::authorize(&principal.0, crate::auth::Action::Write, "", "global")
    {
        return error_json("error", &e.inner.message);
    }
    let content = match to_bytes(body, MAX_REQUEST_SIZE).await {
        Ok(b) => match String::from_utf8(b.to_vec()) {
            // Invalid UTF-8 no longer collapses to "Empty content" —
            // it is rejected up front.
            Ok(utf8) => utf8,
            Err(_) => {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "success": false,
                        "status": "error",
                        "code": "invalid_utf8",
                        "message": "request body is not valid UTF-8"
                    })),
                )
                    .into_response();
            }
        },
        // An over-cap body (the request alone exceeds MAX_REQUEST_SIZE)
        // is a size rejection, not "empty content".
        Err(_) => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "success": false,
                    "status": "error",
                    "code": "entry_too_large",
                    "message": format!(
                        "request body too large (limit {} bytes)",
                        MAX_REQUEST_SIZE
                    )
                })),
            )
                .into_response();
        }
    };

    let content = content.trim().to_string();
    if content.is_empty() {
        return error_json("error", "Empty content");
    }

    // capacity guard. `/ingest/memory` returns the legacy JSON shape;
    // the primary `/ingest` path returns a proper 507.
    if let Err(AppError::InsufficientStorage(msg)) = guard_capacity(&s) {
        return error_json("error", &msg);
    }

    // Seatbelt (Seatbelt): under BRAIN_WRITE_POSTURE=review the capture
    // proposes instead of inserting — one proposal per request (the raw
    // content), no `knowledge` row until an operator approves.
    if crate::config::write_posture() == "review" {
        let proposal = crate::handlers::gate::create_proposal(
            s.clone(),
            principal.0.clone(),
            crate::handlers::gate::ProposalRequest {
                content: content.clone(),
                kind: "fact".to_string(),
                source: Some("memory".to_string()),
                authority: None,
                observed_at: None,
                domain: Some("global".to_string()),
                title: None,
                source_prompt: None,
            },
        )
        .await;
        return proposal
            .map(|p| {
                (
                    axum::http::StatusCode::ACCEPTED,
                    Json(serde_json::json!({
                        "success": true,
                        "status": "proposal_pending",
                        "proposal_id": p.id
                    })),
                )
                    .into_response()
            })
            .unwrap_or_else(axum::response::IntoResponse::into_response);
    }

    let model = Arc::clone(&s.model);
    let pool = s.pool.clone();
    let tracker = std::sync::Arc::clone(&s.connection_tracker);
    // record the creating principal (see add_chunk).
    let owner = crate::handlers::gate::principal_to_owner(&principal.0);

    // the two rejections the closure can raise
    // before any write happens. Everything else keeps the legacy wire shape.
    #[derive(Debug)]
    enum MemoryReject {
        EntryTooLarge { len: usize },
        TooManyEntries { count: usize },
    }

    let ingest_future = task::spawn_blocking(move || -> Result<AddResponse, MemoryReject> {
        // RAII — the slot releases on EVERY exit (return, panic, or the
        // 60 s timeout dropping this task), not just the explicit paths.
        let _tracker_entry = TrackerEntry::new(tracker, "ingest_memory");
        let entries = parse_memory_content(&content);

        // Explicit entry-count bound (the 1 MiB body cap alone would still
        // admit thousands of micro-entries in one linear tx).
        if entries.len() > crate::handlers::MAX_INGEST_ENTRIES {
            return Err(MemoryReject::TooManyEntries {
                count: entries.len(),
            });
        }

        // Per-entry content cap, the same MAX_CONTENT bound `/ingest`
        // and `/add` enforce. All-or-nothing: one oversized entry rejects the
        // whole request with a 400 before any write.
        if let Some(len) = entries
            .iter()
            .find_map(|(t, _)| (t.len() > crate::handlers::MAX_CONTENT).then_some(t.len()))
        {
            return Err(MemoryReject::EntryTooLarge { len });
        }

        if entries.is_empty() {
            return Ok(AddResponse::error("No valid entries found"));
        }

        let mut conn = match pool.get() {
            Ok(c) => c,
            Err(e) => return Ok(AddResponse::error(format!("DB connection failed: {}", e))),
        };

        let mut added = 0;
        let mut duplicates = 0;
        // capture the real inserted rowids so the
        // response can name what it just wrote (the old `entry_id` was the
        // COUNT of added rows — useless for delete/verify round-trips).
        let mut chunk_ids: Vec<i64> = Vec::new();

        for (text, title) in entries {
            let content_hash = format!("{:016x}", xxh3_64(text.as_bytes()));
            let exists: bool = conn
                .query_row(
                    "SELECT 1 FROM knowledge WHERE content_hash=? LIMIT 1",
                    [&content_hash],
                    |r| r.get::<_, i32>(0),
                )
                .unwrap_or(0)
                == 1;

            if exists {
                duplicates += 1;
                continue;
            }

            let embedding = model.encode_one(&text);
            if embedding.is_empty() {
                continue;
            }

            // prep source/revision identity for this entry before the
            // transaction opens. URI is `manual://{content_hash}` so each
            // distinct memory is its own source (no PII in the URI; stable
            // across re-ingests of the same content). Kind = 'manual' keeps
            // these immune to vault reconcile (which is kind-scoped).
            let source_uri = format!("manual://{content_hash}");
            let revision = sources::compute_revision(&text);
            let title_for_source = title.clone();
            let text_len = text.len();
            // screen each memory entry through the full two-layer
            // screen. Memory keeps its "trusted local write surface" contract —
            // never dropped, but injection-y content is flagged out of
            // retrieval. A `Quarantine` verdict flags the row; a `Reject`
            // verdict — stricter, still never dropped on this surface — also
            // flags: an injection hit the classifier is *confident*
            // about must at minimum be excluded from retrieval under the
            // default Quarantine operational posture, not stored clean.
            // (Explicit `Reject` policy still hard-rejects at the other
            // ingest surfaces via the pre-insert branch — unchanged.)
            let quarantine = matches!(
                screen::screen(&text, title.as_deref().unwrap_or("")),
                screen::ScreenResult::Quarantine | screen::ScreenResult::Reject
            );

            let tx = match conn.transaction() {
                Ok(t) => t,
                Err(_) => continue,
            };

            if tx
                .execute(
                    "INSERT INTO knowledge(content, title, source, content_hash, owner, origin)
                     VALUES(?, ?, ?, ?, ?, ?6)",
                    params![
                        text,
                        title,
                        "memory",
                        content_hash,
                        &owner,
                        // derive, never hardcode: the origin is a function of
                        // the source kind (the Seatbelt posture label truth).
                        crate::gate::origin_for_source(Some("memory"))
                    ],
                )
                .is_err()
            {
                continue;
            }

            let chunk_id = tx.last_insert_rowid();
            if chunk_id > 0 {
                // write to vec0 (int8 + binary quantized). No raw
                // f32 JSON is written to the legacy `embeddings` column.
                // propagate — a chunk stored without
                // its vector is silently degraded (FTS-only retrieval with no
                // embedding to fuse). The entry is skipped, never half-stored.
                if tx
                    .execute(
                        "INSERT INTO vec_knowledge(knowledge_id, embedding_int8, embedding_bit, source, created_at)
                         VALUES (?1, vec_quantize_int8(?2, 'unit'), vec_quantize_binary(?2), 'memory', datetime('now'))",
                        params![chunk_id, embedding.as_bytes()],
                    )
                    .is_err()
                {
                    continue;
                }

                // link this memory to its source + revision. Best-effort
                // inside the tx — a failure here rolls back the whole entry (the
                // chunk INSERT + vec0 INSERT), preserving the invariant that a
                // visible memory always has source linkage. Matches the existing
                // fail-soft style: a failed entry is skipped, not fatal.
                if let Ok(source_id) = sources::upsert_source(
                    &tx,
                    &source_uri,
                    sources::KIND_MANUAL,
                    title_for_source.as_deref(),
                ) && let Ok(outcome) = sources::upsert_revision(
                    &tx,
                    source_id,
                    &revision,
                    Some(&content_hash),
                    1,
                    text_len as u64,
                ) {
                    let revision_id = match outcome {
                        sources::RevisionOutcome::Unchanged(id)
                        | sources::RevisionOutcome::Created { id, .. } => id,
                    };
                    // was `let _ =` + a comment
                    // claiming failure "rolls back the whole entry" — it did
                    // not (the tx committed regardless), so a failed link
                    // stored a visible memory with no source linkage. Now a
                    // linkage/evidence failure skips the entry for real (tx
                    // drops uncommitted), matching the fail-soft style.
                    if sources::link_chunks(
                        &tx,
                        source_id,
                        revision_id,
                        std::slice::from_ref(&chunk_id),
                    )
                    .is_err()
                    {
                        eprintln!("⚠️ source link failed — rolling back chunk {chunk_id}");
                        continue;
                    }
                    // manual memories are observed == valid_from
                    // == now and remain current (valid_to NULL); highest
                    // authority (trusted local write surface).
                    if sources::stamp_evidence(
                        &tx,
                        chunk_id,
                        &chrono::Utc::now().to_rfc3339(),
                        None,
                        None,
                        sources::AUTHORITY_MANUAL,
                    )
                    .is_err()
                    {
                        eprintln!("⚠️ evidence stamp failed — rolling back chunk {chunk_id}");
                        continue;
                    }
                }

                // quarantine path only (no Reject branch here —
                // memory is a trusted local write surface; flagging keeps
                // injection-y content out of retrieval without dropping it).
                // Runs inside the tx (Transaction derefs to Connection) so it
                // commits atomically with the chunk.
                // fail closed — if the flag write
                // fails, log and let the tx drop uncommitted (rollback), so an
                // injection hit that MUST be flagged is never stored clean.
                if screen::flag_if_quarantined(&tx, chunk_id, quarantine).is_err() {
                    eprintln!("⚠️ quarantine flag failed — rolling back chunk {chunk_id}");
                    continue;
                }
                if tx.commit().is_ok() {
                    added += 1;
                    chunk_ids.push(chunk_id);
                    // audit successful ingest (hash only).
                    audit::record(
                        &conn,
                        audit::AuditKind::Ingest,
                        "api",
                        &content_hash,
                        audit::AuditStatus::Ok,
                        &source_uri,
                    );
                }
            }
        }

        // No explicit release — `_tracker_entry` drops on every exit.
        Ok(AddResponse {
            success: true,
            status: "completed".to_string(),
            chunk_id: chunk_ids.first().copied(),
            chunk_ids,
            entries_added: Some(added as i64),
            duplicates_skipped: Some(duplicates as i64),
            error: if duplicates > 0 {
                Some(format!("{} duplicates skipped", duplicates))
            } else {
                None
            },
        })
    });

    match timeout(StdDuration::from_secs(60), ingest_future).await {
        Ok(Ok(Ok(resp))) => {
            let added = resp.entries_added.unwrap_or(0);
            let status = if added == 0 { "unchanged" } else { "success" };
            (
                axum::http::StatusCode::OK,
                Json(serde_json::json!({
                    "status": status,
                    // real first inserted rowid (null when
                    // nothing was added). `entry_id` is the deprecated alias.
                    "chunk_id": resp.chunk_id,
                    "chunk_ids": resp.chunk_ids,
                    "entries_added": added,
                    "duplicates_skipped": resp.duplicates_skipped.unwrap_or(0),
                    "entry_id": resp.chunk_id,
                    "similarity_score": 1.0
                })),
            )
                .into_response()
        }
        Ok(Ok(Err(MemoryReject::EntryTooLarge { len }))) => (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "success": false,
                "status": "error",
                "code": "entry_too_large",
                "message": format!(
                    "memory entry too large ({} chars; limit {})",
                    len,
                    crate::handlers::MAX_CONTENT
                )
            })),
        )
            .into_response(),
        Ok(Ok(Err(MemoryReject::TooManyEntries { count }))) => (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "success": false,
                "status": "error",
                "code": "too_many_entries",
                "message": format!(
                    "too many memory entries ({}; limit {})",
                    count,
                    crate::handlers::MAX_INGEST_ENTRIES
                )
            })),
        )
            .into_response(),
        Ok(Err(_)) => (
            axum::http::StatusCode::OK,
            Json(serde_json::json!({ "status": "error", "error": "internal error" })),
        )
            .into_response(),
        Err(_) => {
            // The timed-out clone is dropped here — `_tracker_entry`'s
            // Drop has ALREADY released the slot (the closure itself exits
            // when the task is dropped), so no leak remains to report.
            // spawn_blocking tasks cannot be cancelled mid-flight (honest
            // ceiling: the task keeps running to completion) — but the guard
            // slot it holds is freed at its exit, closing the tracker leak.
            eprintln!("⚠️ ingest_memory timed out after 60s - task dropped");
            (
                axum::http::StatusCode::OK,
                Json(serde_json::json!({ "status": "error", "error": "Ingest timed out" })),
            )
                .into_response()
        }
    }
}

pub(crate) fn parse_memory_content(text: &str) -> Vec<(String, Option<String>)> {
    let mut entries = Vec::new();
    let mut current = String::new();
    let mut title = None;

    for line in text.lines() {
        if line.starts_with("## [") || line.starts_with("##[") {
            if !current.trim().is_empty() {
                entries.push((current.trim().to_string(), title));
            }
            current.clear();
            title = Some(line.trim_start_matches('#').trim().to_string());
        } else {
            current.push_str(line);
            current.push('\n');
        }
    }

    if !current.trim().is_empty() {
        entries.push((current.trim().to_string(), title));
    }

    // strip reasoning/trace blocks; drop entries matching a
    // BRAIN_INGEST_SKIP_PATTERNS prefix (autoCapture dream prompts). Stops the
    // bleeding at the ingest door; historical cleanup is a separate sweep.
    let patterns = hygiene::skip_patterns();
    entries
        .into_iter()
        .filter_map(|(t, title)| hygiene::clean(&t, &patterns).map(|c| (c, title)))
        .collect()
}

/// measure the current capacity utilization and classify it
/// against the active target's envelope. Shared by `/health` (reports it) and
/// the ingest handlers (reject with 507 when `Exceeded`). Returns the measured
/// counts + the classification so callers don't re-query.
///
/// All three inputs are read in the caller's `spawn_blocking` context; this fn
/// is pure and side-effect-free so it composes cleanly with the existing
/// health/stats query patterns.
pub fn measure_capacity(conn: &Connection, db_path: &std::path::Path) -> serde_json::Value {
    let target = crate::capacity::capacity_target();
    let envelope = crate::capacity::CapacityEnvelope::for_target(target);
    let docs: usize = conn
        .query_row("SELECT COUNT(*) FROM knowledge", [], |r| r.get::<_, i64>(0))
        .unwrap_or(0)
        .max(0) as usize;
    let db_mib: u64 = std::fs::metadata(db_path)
        .map(|m| m.len() / 1_000_000)
        .unwrap_or(0);
    // CRITICAL: measure the *process's own* RSS, not system-wide memory.
    // The envelope's max_rss_mib (320 MB) is a per-process ceiling; using
    // System::used_memory() (system-wide) would always exceed it on any
    // machine with real workload, blocking every write with a spurious 507.
    let rss_mib = process_rss_mib();
    let status = crate::capacity::classify(docs, db_mib, rss_mib, &envelope);
    serde_json::json!({
        "target": match target {
            crate::capacity::CapacityTarget::Desktop => "desktop",
            crate::capacity::CapacityTarget::Jetson => "jetson",
        },
        "docs": docs,
        "max_docs": envelope.max_docs,
        "db_mib": db_mib,
        "max_db_mib": envelope.max_db_mib,
        "rss_mib": rss_mib,
        "max_rss_mib": envelope.max_rss_mib,
        "status": status.as_str(),
    })
}

pub(crate) async fn embeddings(
    State(s): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    Json(req): Json<EmbeddingsRequest>,
) -> Json<serde_json::Value> {
    // AuthZ write gate. Legacy OpenAI-style shape.
    if let Err(e) =
        crate::handlers::authorize(&principal.0, crate::auth::Action::Write, "", "global")
    {
        return Json(serde_json::json!({
            "error": { "message": e.inner.message, "type": "forbidden" }
        }));
    }
    let inputs = match &req.input {
        EmbeddingsInput::Single(s) if s.trim().is_empty() => {
            return Json(serde_json::json!({
                "error": { "message": "input is required", "type": "invalid_request_error" }
            }));
        }
        EmbeddingsInput::Single(s) => vec![s.trim().to_string()],
        EmbeddingsInput::Batch(v) if v.is_empty() => {
            return Json(serde_json::json!({
                "error": { "message": "input is required", "type": "invalid_request_error" }
            }));
        }
        EmbeddingsInput::Batch(v) if v.len() > MAX_EMBEDDING_BATCH => {
            // bound the batch to prevent memory amplification. A
            // 1 MiB body of ~50k short strings produces ~50 MB of buffered
            // JSON response; concurrent calls OOM the server.
            return Json(serde_json::json!({
                "error": {
                    "message": format!("batch exceeds {MAX_EMBEDDING_BATCH} items"),
                    "type": "invalid_request_error"
                }
            }));
        }
        EmbeddingsInput::Batch(v) => v
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
    };

    if inputs.is_empty() {
        return Json(serde_json::json!({
            "error": { "message": "input is required", "type": "invalid_request_error" }
        }));
    }

    let model = Arc::clone(&s.model);
    let model_name = req.model;

    let encode_future = task::spawn_blocking(move || {
        // The Embedder trait takes &[&str]; build the refs from the owned inputs.
        let refs: Vec<&str> = inputs.iter().map(String::as_str).collect();
        model.encode(&refs)
    });

    match timeout(StdDuration::from_secs(30), encode_future).await {
        Ok(Ok(embeddings)) => {
            let total_tokens: usize = embeddings.iter().map(|e| e.len()).sum();
            let data: Vec<_> = embeddings
                .into_iter()
                .enumerate()
                .map(|(i, emb)| serde_json::json!({ "object": "embedding", "embedding": emb, "index": i }))
                .collect();

            Json(serde_json::json!({
                "object": "list",
                "data": data,
                "model": model_name,
                "usage": { "prompt_tokens": total_tokens, "total_tokens": total_tokens }
            }))
        }
        _ => Json(serde_json::json!({
            "error": { "message": "Failed to generate embedding", "type": "server_error" }
        })),
    }
}

// === KNOWLEDGE GRAPH FUNCTIONS ===

pub fn parse_annotations(content: &str) -> Vec<(String, String)> {
    let mut results = Vec::new();
    let bytes = content.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if i + 4 <= len && bytes[i] == b'[' && bytes[i + 1] == b'[' {
            let start = i + 2;
            let mut mid = start;
            let mut found = false;

            while mid < len && mid - start < 50 {
                if bytes[mid] == b':' && mid + 1 < len && bytes[mid + 1] == b':' {
                    found = true;
                    break;
                }
                if bytes[mid] == b']' {
                    break;
                }
                mid += 1;
            }

            if found {
                let mut end = mid + 2;
                while end < len && end - start < 100 {
                    if bytes[end] == b']' && end + 1 < len && bytes[end + 1] == b']' {
                        break;
                    }
                    end += 1;
                }

                if end + 1 < len {
                    let relation = String::from_utf8_lossy(&bytes[start..mid])
                        .trim()
                        .to_string();
                    let entity = String::from_utf8_lossy(&bytes[mid + 2..end])
                        .trim()
                        .to_string();

                    if !relation.is_empty()
                        && !entity.is_empty()
                        && relation
                            .chars()
                            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
                        && entity
                            .chars()
                            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
                    {
                        results.push((relation, entity));
                    }
                    i = end + 2;
                    continue;
                }
            }
        }
        i += 1;
    }

    results
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

pub(crate) async fn ingest_markdown(
    State(state): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    Json(payload): Json<MarkdownPayload>,
) -> Result<Json<serde_json::Value>, AppError> {
    // AuthZ write gate. This is the primary
    // vault ingest path, so it gets a proper HTTP 403 via AppError::Forbidden.
    // `None` principal (no JWT) = superuser.
    crate::handlers::authorize(&principal.0, crate::auth::Action::Write, "", "global")
        .map_err(|e| AppError::Forbidden(e.inner.message))?;
    // capacity guard — the primary vault ingest path returns a proper
    // HTTP 507 when the envelope is exceeded.
    guard_capacity(&state)?;

    // vault semantics. Frontmatter is stripped before chunking (never
    // useful prose to embed); wikilinks and tags/aliases become KG edges.
    let (yaml, body) = vault::split_frontmatter(&payload.content);
    let fm = vault::parse_frontmatter(&yaml);
    let content = if yaml.is_empty() {
        payload.content.clone()
    } else {
        body
    };

    let source_path = payload
        .source_path
        .clone()
        .filter(|s| !s.trim().is_empty() && search_mod::is_client_safe_uri(s));

    // Title precedence depends on the caller:
    //   - vault ingest (source_path set): frontmatter title > filename fallback.
    //   - interactive add (no source_path): explicit payload title > frontmatter.
    let title = if source_path.is_some() {
        fm.title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .or_else(|| payload.title.clone().filter(|t| !t.trim().is_empty()))
            .unwrap_or_default()
    } else {
        payload
            .title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .or_else(|| fm.title.clone())
            .unwrap_or_default()
    };

    if content.len() > 1_000_000 {
        return Err(AppError::BadRequest("Content too large (max 1MB)"));
    }
    if title.len() > 500 {
        return Err(AppError::BadRequest("Title too long (max 500 chars)"));
    }
    if title.is_empty() {
        return Err(AppError::BadRequest("Title is required"));
    }
    // injection screen. Now the full two-layer
    // screen ([`screen::screen`] = blocklist + optional classifier). `Reject`
    // keeps HTTP-400; `Quarantine` (default) proceeds to ingest and flags every
    // inserted chunk; `Allow` disables the screen.
    let screen_result = screen::screen(&content, &title);
    if screen_result == screen::ScreenResult::Reject {
        return Err(AppError::BadRequest("Input contains suspicious patterns"));
    }
    let quarantine_flagged = screen_result == screen::ScreenResult::Quarantine;

    let escaped_title = html_escape(&title);

    // KG edges, document-level (attached to the first chunk):
    //   - inline [[relation::entity]] annotations (legacy)
    //   - wikilink targets (vault): source note `references` target note
    //   - frontmatter tags: note `tagged_with` tag
    //   - frontmatter aliases: alias `alias_of` note
    //   - deterministic linker: auto-discovers entities + typed relationships
    //     from section headings, bold terms, and sentence patterns
    let mut kg_edges: Vec<(String, String, String)> = parse_annotations(&content)
        .into_iter()
        .map(|(rel, ent)| (rel, escaped_title.to_lowercase(), ent.to_lowercase()))
        .collect();
    if !content.is_empty() {
        let from = escaped_title.to_lowercase();

        // deterministic entity linker — Aho-Corasick backed, zero LLM.
        // Builds vocabulary from document structure, merges in existing entities
        // from the database (cross-document linking), then finds mentions and
        // typed relationship patterns. Code blocks and tables are excluded.
        let code_ranges = linker::find_code_ranges(&content);
        let table_ranges = linker::find_table_ranges(&content);
        let list_bold_ranges = linker::find_list_item_bold_ranges(&content);
        let mut excluded_ranges: Vec<(usize, usize)> = code_ranges;
        excluded_ranges.extend(table_ranges);
        excluded_ranges.extend(list_bold_ranges);
        let mut vocab = linker::extract_vocabulary(&content, &excluded_ranges);
        // Merge existing entities from DB for cross-document recognition.
        if let Ok(conn) = state.pool.get()
            && let Ok(mut stmt) = conn.prepare("SELECT DISTINCT name FROM entities")
            && let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0))
        {
            for row in rows.flatten() {
                vocab.insert(&row);
            }
        }
        // Save entity set before consuming vocab.
        let entity_set: std::collections::HashSet<String> =
            std::collections::HashSet::from_iter(vocab.entities.iter().cloned());
        vocab.finalize();
        let matcher: linker::EntityMatcher = vocab.into();
        let doc_lower = title.trim().to_lowercase();

        for mention in matcher.find_mentions(&content, &excluded_ranges) {
            // Skip self-references (document title matching an entity)
            if mention == doc_lower {
                continue;
            }
            kg_edges.push(("references".to_string(), from.clone(), mention.to_string()));
        }
        // Heading hierarchy → part_of relationships.
        // A heading at level N+1 under a heading at level N creates a
        // `part_of` edge if both are known entities.
        for edge in linker::extract_heading_relationships(&content, &entity_set, &excluded_ranges) {
            if edge.from != doc_lower && edge.to != doc_lower {
                kg_edges.push((edge.relation, edge.from, edge.to));
            }
        }
        // Discover domain-specific verb patterns from this document.
        // min_freq=3 filters out words that happen to appear between entities
        // by accident and keeps genuine relationship verbs. The built-in
        // RELATION_PATTERNS handle the common infrastructure verbs as a fallback.
        let discovered = matcher.discover_verb_patterns(&content, 3, &excluded_ranges);
        let discovered_refs: Vec<(&str, &str)> = discovered
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        for edge in matcher.find_relationships(&content, &excluded_ranges, &discovered_refs) {
            let from_lower = edge.from;
            let to_lower = edge.to;
            // Skip self-references and empty sides
            if from_lower == to_lower || from_lower.is_empty() || to_lower.is_empty() {
                continue;
            }
            kg_edges.push((edge.relation, from_lower, to_lower));
        }

        for target in vault::parse_wikilinks(&content) {
            kg_edges.push((
                "references".to_string(),
                from.clone(),
                target.to_lowercase(),
            ));
        }
        for tag in &fm.tags {
            kg_edges.push(("tagged_with".to_string(), from.clone(), tag.to_lowercase()));
        }
        for alias in &fm.aliases {
            // alias -> note (so a query for the alias resolves to the note).
            kg_edges.push(("alias_of".to_string(), alias.to_lowercase(), from.clone()));
        }
    }
    // Distinct (rel, from, to) — dedup edges from repeated links.
    kg_edges.sort();
    kg_edges.dedup();

    let chunks = chunker::chunk_markdown(&content);
    let document_id = format!("{:016x}", xxh3_64(title.trim().as_bytes()));

    // Resolve domain: explicit payload field > YAML frontmatter > "global".
    let domain = payload
        .domain
        .clone()
        .filter(|d| !d.trim().is_empty())
        .or_else(|| fm.domain.clone().filter(|d| !d.trim().is_empty()))
        .unwrap_or_else(|| "global".to_string());

    // Seatbelt (Seatbelt): under BRAIN_WRITE_POSTURE=review the vault ingest
    // proposes instead of inserting — one proposal per chunk (capped), the
    // connector fetch loop inherits this automatically. No `knowledge` row
    // until an operator approves.
    if crate::config::write_posture() == "review" {
        const MAX_REVIEW_CHUNKS: usize = 50;
        if chunks.len() > MAX_REVIEW_CHUNKS {
            return Err(AppError::BadRequest(
                "too_many_chunks_for_review (max 50 per document under review posture)",
            ));
        }
        let mut proposal_ids = Vec::with_capacity(chunks.len());
        for chunk in &chunks {
            let p = crate::handlers::gate::create_proposal(
                state.clone(),
                principal.0.clone(),
                crate::handlers::gate::ProposalRequest {
                    content: chunk.text.clone(),
                    kind: "fact".to_string(),
                    source: Some("markdown".to_string()),
                    authority: None,
                    observed_at: None,
                    domain: Some(domain.clone()),
                    title: None,
                    source_prompt: None,
                },
            )
            .await
            .map_err(|e| AppError::Internal(e.inner.message))?;
            proposal_ids.push(p.id);
        }
        return Err(AppError::Accepted(serde_json::json!({
            "success": true,
            "status": "proposal_pending",
            "proposal_ids": proposal_ids
        })));
    }

    let pool = state.pool.clone();
    let model = Arc::clone(&state.model);

    let chunk_texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
    let total_chunks = chunks.len();
    let embeddings = task::spawn_blocking(move || {
        // The Embedder trait takes &[&str]; build the refs from the owned chunk texts.
        let refs: Vec<&str> = chunk_texts.iter().map(String::as_str).collect();
        model.encode(&refs)
    })
    .await
    .map_err(|_| AppError::Internal("Embedding task failed".into()))?;
    if embeddings.len() != chunks.len() {
        return Err(AppError::Internal("Embedding count mismatch".into()));
    }

    let doc_title = escaped_title.clone();
    let doc_id = document_id.clone();
    let edges = kg_edges.clone();
    let raw_content_for_source = payload.content.clone();
    let replace = payload.replace;
    // record the creating principal (see add_chunk).
    let owner = crate::handlers::gate::principal_to_owner(&principal.0);
    let result = task::spawn_blocking(move || -> Result<(i64, usize, usize), AppError> {
        let mut conn = pool.get().map_err(|e| AppError::Internal(e.to_string()))?;
        let tx = conn
            .transaction()
            .map_err(|e| AppError::Internal(e.to_string()))?;
        if replace && let Some(sp) = source_path.as_deref() {
            // Collect stale knowledge IDs for this source_path,
            // then sweep vec_knowledge + relationships + knowledge.
            let stale_ids: Vec<i64> = {
                let mut stmt = tx
                    .prepare("SELECT id FROM knowledge WHERE source_path = ?1")
                    .map_err(|e| AppError::Internal(e.to_string()))?;
                let rows = stmt
                    .query_map(params![sp], |r| r.get::<_, i64>(0))
                    .map_err(|e| AppError::Internal(e.to_string()))?;
                rows.filter_map(|r| r.ok()).collect()
            };
            for id in &stale_ids {
                // was `let _ =` — a stale vec0 row
                // would surface the old chunk in retrieval after the replace.
                tx.execute(
                    "DELETE FROM vec_knowledge WHERE knowledge_id = ?1",
                    params![id],
                )
                .map_err(|e| AppError::Internal(e.to_string()))?;
            }
            // Sweep relationships: both those still linked (previously
            // stale_ids) AND orphans already NULLed by prior re-ingests.
            // The replace sweep is an erasure path: a held chunk refuses
            // the whole re-ingest (same 409 fence as /purge) — held
            // evidence must not be destroyed by routine supersession.
            crate::legal_hold::refuse_if_held(&tx, &stale_ids).map_err(|e| {
                AppError::Conflict(format!("{}: {}", e.inner.code, e.inner.message))
            })?;
            tx.execute("DELETE FROM relationships WHERE knowledge_id IS NULL", [])
                .map_err(|e| AppError::Internal(e.to_string()))?;
            for id in &stale_ids {
                tx.execute(
                    "DELETE FROM relationships WHERE knowledge_id = ?1",
                    params![id],
                )
                .map_err(|e| AppError::Internal(e.to_string()))?;
            }
            tx.execute("DELETE FROM knowledge WHERE source_path = ?1", params![sp])
                .map_err(|e| AppError::Internal(e.to_string()))?;
        }
        let r = write_markdown_ingest(
            &tx,
            &chunks,
            &embeddings,
            &doc_title,
            &doc_id,
            &source_path,
            &edges,
            &raw_content_for_source,
            quarantine_flagged,
            &owner,
        )?;
        tx.commit().map_err(|e| AppError::Internal(e.to_string()))?;
        // audit successful markdown ingest (identifier only).
        audit::record(
            &conn,
            audit::AuditKind::Ingest,
            "api",
            &doc_id,
            audit::AuditStatus::Ok,
            &source_path
                .clone()
                .unwrap_or_else(|| "markdown".to_string()),
        );
        Ok(r)
    })
    .await
    .map_err(|_| AppError::Internal("Task join error".into()))??;

    let (first_id, inserted, duplicates) = result;

    // Apply domain from frontmatter or payload field. When the domain differs
    // from the default ("global"), update every chunk for this document_id.
    // Using an UPDATE keeps write_markdown_ingest's signature unchanged and
    // avoids touching 14 test call sites.
    if domain != "global" {
        let pool = state.pool.clone();
        let d = domain.clone();
        let did = document_id.clone();
        // was `let _ = ... .await;` — a failed post-commit
        // domain move left chunks in "global" with no signal. Post-commit, so a
        // rejection is dishonest; log instead.
        match task::spawn_blocking(move || -> Result<(), AppError> {
            let conn = pool.get().map_err(|e| AppError::Internal(e.to_string()))?;
            conn.execute(
                "UPDATE knowledge SET domain = ?1 WHERE document_id = ?2 AND domain = 'global'",
                params![d, did],
            )
            .map_err(|e| AppError::Internal(e.to_string()))?;
            Ok(())
        })
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(e)) => eprintln!("⚠️ domain move failed for {document_id}: {e:?}"),
            Err(join) => eprintln!("⚠️ domain move task failed for {document_id}: {join}"),
        }
    }

    // Refresh the domain centroid so routing stays current.
    // was `let _ =` — a failed refresh silently left
    // stale routing. Post-commit; log.
    if let Err(e) = domain_router::recompute_centroid(&state.pool, &domain, &state.pool) {
        eprintln!("⚠️ centroid refresh failed for domain {domain}: {e}");
    }

    Ok(Json(serde_json::json!({
        "success": true,
        "id": first_id,
        "document_id": document_id,
        "chunks_inserted": inserted,
        "chunks_duplicate": duplicates,
        "total_chunks": total_chunks
    })))
}

/// pure DB-write for a markdown ingest. Extracted from `ingest_markdown`
/// so the vault dedup/replace + KG-edge logic is unit-testable without the
/// embedding model. Caller provides pre-computed embeddings (one per chunk).
///
/// Vault semantics (when `source_path` is `Some`):
///   - unchanged file (existing chunk-hash set == new set) → true no-op;
///   - changed file → old chunks + vec0 rows for that path swept, then re-inserted;
///   - content_hash namespaced with source_path so vault chunks never collide
///     with memories or other files under the global unique index.
///
/// `edges` are document-level KG relations `(rel, from, to)` attached to the
/// first inserted chunk.
///
/// `raw_content` is the original payload (frontmatter + body). It feeds
/// `sources::compute_revision` so any change anywhere in the file — including
/// frontmatter that never reaches the chunks — yields a new revision. Vault
/// ingests (source_path set) are linked to a `sources`/`source_revisions` row;
/// interactive adds (no source_path) stay unlinked, matching the legacy
/// behavior (from before source linkage existed).
//
// ponytail: 8 positional args is past clippy's 7-arg default. Bundling them into
// a `MarkdownIngest` struct is pure ceremony here — this is a private fn with one
// production caller (ingest_markdown) plus unit tests, all of which would have
// to spell every field by name anyway. The signature stays verbose-but-honest.
#[allow(clippy::too_many_arguments)]
pub fn write_markdown_ingest(
    tx: &rusqlite::Transaction<'_>,
    chunks: &[chunker::Chunk],
    embeddings: &[Vec<f32>],
    doc_title: &str,
    doc_id: &str,
    source_path: &Option<String>,
    edges: &[(String, String, String)],
    raw_content: &str,
    quarantine_flagged: bool,
    owner: &Option<String>,
) -> Result<(i64, usize, usize), AppError> {
    let mut first_id = 0;
    let mut inserted = 0usize;
    let mut duplicates = 0usize;
    // collect inserted chunk ids so we can link them to source+revision
    // after the per-chunk INSERT loop. The vec is left empty for unchanged files.
    let mut inserted_ids: Vec<i64> = Vec::with_capacity(chunks.len());

    if let Some(sp) = source_path.as_deref() {
        let seed = xxh3_64(sp.as_bytes());
        let new_hashes: Vec<String> = chunks
            .iter()
            .map(|c| format!("{:016x}", xxh3_64_with_seed(c.text.as_bytes(), seed)))
            .collect();
        let mut existing: Vec<String> = {
            let mut stmt = tx
                .prepare("SELECT content_hash FROM knowledge WHERE source_path = ?1")
                .map_err(|e| AppError::Internal(e.to_string()))?;
            let rows = stmt
                .query_map(params![sp], |r| r.get::<_, String>(0))
                .map_err(|e| AppError::Internal(e.to_string()))?;
            rows.filter_map(|r| r.ok()).collect()
        };
        existing.sort();
        if existing == new_hashes {
            // Unchanged file: true no-op on chunks, but still refresh source /
            // revision observed_at and link any legacy rows that have NULL
            // source_id (the first source-linked ingest of a file ingested before
            // linkage existed).
            let existing_ids: Vec<i64> = {
                let mut stmt = tx
                    .prepare("SELECT id FROM knowledge WHERE source_path = ?1 ORDER BY id")
                    .map_err(|e| AppError::Internal(e.to_string()))?;
                let rows = stmt
                    .query_map(params![sp], |r| r.get::<_, i64>(0))
                    .map_err(|e| AppError::Internal(e.to_string()))?;
                rows.filter_map(|r| r.ok()).collect()
            };
            // Best-effort: source linkage on the no-op path must not retroactively
            // break a previously-working ingest. The chunks themselves are unchanged.
            let _ = link_vault_source(tx, sp, doc_title, raw_content, &existing_ids);
            let first = existing_ids.first().copied().unwrap_or(0);
            return Ok((first, 0, chunks.len()));
        }
        // Changed file: sweep old chunks (+ their vec0 rows) for this path.
        let stale_ids: Vec<i64> = {
            let mut stmt = tx
                .prepare("SELECT id FROM knowledge WHERE source_path = ?1")
                .map_err(|e| AppError::Internal(e.to_string()))?;
            let rows = stmt
                .query_map(params![sp], |r| r.get::<_, i64>(0))
                .map_err(|e| AppError::Internal(e.to_string()))?;
            rows.filter_map(|r| r.ok()).collect()
        };
        // The changed-file sweep is an erasure path: a held chunk refuses the
        // re-ingest (same 409 fence as /purge) — litigation-frozen evidence
        // must not vanish because its source file changed on disk.
        crate::legal_hold::refuse_if_held(tx, &stale_ids)
            .map_err(|e| AppError::Conflict(format!("{}: {}", e.inner.code, e.inner.message)))?;
        for id in &stale_ids {
            // was `let _ =` — a stale vec0 row would
            // surface the old chunk in retrieval after the file changed.
            tx.execute(
                "DELETE FROM vec_knowledge WHERE knowledge_id = ?1",
                params![id],
            )
            .map_err(|e| AppError::Internal(e.to_string()))?;
        }
        // FTS trigger + relationships FK SET NULL clean up the rest.
        tx.execute("DELETE FROM knowledge WHERE source_path = ?1", params![sp])
            .map_err(|e| AppError::Internal(e.to_string()))?;
    }

    for (idx, chunk) in chunks.iter().enumerate() {
        let content_hash = match source_path.as_deref() {
            Some(sp) => format!(
                "{:016x}",
                xxh3_64_with_seed(chunk.text.as_bytes(), xxh3_64(sp.as_bytes()))
            ),
            None => format!("{:016x}", xxh3_64(chunk.text.as_bytes())),
        };
        // Idempotent: skip chunks already present (stable per content hash).
        let exists: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM knowledge WHERE content_hash = ?1",
                params![&content_hash],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if exists > 0 {
            duplicates += 1;
            continue;
        }

        tx.execute(
            "INSERT INTO knowledge
               (title, content, source, content_hash, document_id, chunk_index,
                heading_path, line_start, line_end, source_path, owner)
             VALUES (?1, ?2, 'markdown', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                doc_title,
                &chunk.text,
                &content_hash,
                doc_id,
                idx as i64,
                &chunk.heading_path,
                chunk.line_start as i64,
                chunk.line_end as i64,
                source_path,
                owner,
            ],
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;

        let k_id = tx.last_insert_rowid();
        if first_id == 0 {
            first_id = k_id;
        }
        inserted_ids.push(k_id);

        let emb = &embeddings[idx];
        // was `let _ =` — a chunk stored without its
        // vector is silently degraded. Fail the batch; no half-stored chunks.
        tx.execute(
            "INSERT INTO vec_knowledge(knowledge_id, embedding_int8, embedding_bit, source, created_at)
             VALUES (?1, vec_quantize_int8(?2, 'unit'), vec_quantize_binary(?2), 'markdown', datetime('now'))",
            params![k_id, emb.as_bytes()],
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;
        inserted += 1;
    }

    // under Quarantine policy the ingested content tripped the
    // injection screen — flag every inserted chunk so retrieval excludes it.
    if quarantine_flagged && !inserted_ids.is_empty() {
        let ph = std::iter::repeat_n("?", inserted_ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let params_ref: Vec<&dyn rusqlite::ToSql> = inserted_ids
            .iter()
            .map(|id| id as &dyn rusqlite::ToSql)
            .collect();
        tx.execute(
            &format!("UPDATE knowledge SET flagged = 1 WHERE id IN ({ph})"),
            params_ref.as_slice(),
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;
    }

    // link the freshly-inserted chunks to their canonical source +
    // revision. Fail-loud here (unlike the unchanged path above): an orphan
    // chunk with no source linkage is a real bug we want to surface, not a
    // degraded ingest. Vault ingests only — interactive adds stay unlinked.
    if let Some(sp) = source_path.as_deref()
        && !inserted_ids.is_empty()
    {
        link_vault_source(tx, sp, doc_title, raw_content, &inserted_ids)?;
    }

    // Document-level knowledge graph: attach relations to the first chunk.
    // Targets that don't exist yet are still created as placeholder entities
    // so the graph is complete when their file is later ingested.
    //
    // quarantined evidence must NOT become durable graph
    // structure — skip edge creation when this ingest was flagged.
    if !quarantine_flagged && first_id != 0 && !edges.is_empty() {
        for (rel, from, to) in edges {
            tx.execute(
                "INSERT OR IGNORE INTO entities (name) VALUES (?1)",
                params![from],
            )
            .map_err(|e| AppError::Internal(e.to_string()))?;
            tx.execute(
                "INSERT OR IGNORE INTO entities (name) VALUES (?1)",
                params![to],
            )
            .map_err(|e| AppError::Internal(e.to_string()))?;
            let from_id: i64 = tx
                .query_row(
                    "SELECT id FROM entities WHERE name = ?1",
                    params![from],
                    |r| r.get(0),
                )
                .map_err(|e| AppError::Internal(e.to_string()))?;
            let to_id: i64 = tx
                .query_row(
                    "SELECT id FROM entities WHERE name = ?1",
                    params![to],
                    |r| r.get(0),
                )
                .map_err(|e| AppError::Internal(e.to_string()))?;
            tx.execute(
                "INSERT OR IGNORE INTO relationships
                   (from_entity_id, to_entity_id, relation_type, knowledge_id)
                 VALUES (?1, ?2, ?3, ?4)",
                params![from_id, to_id, rel, first_id],
            )
            .map_err(|e| AppError::Internal(e.to_string()))?;
        }
    }

    Ok((first_id, inserted, duplicates))
}

/// compose the source/revision/link calls for one vault file into a
/// single helper so the unchanged and changed paths in `write_markdown_ingest`
/// share one implementation. Idempotent at every layer:
///   - `upsert_source` reuses the row (or reactivates a deleted one);
///   - `upsert_revision` is a no-op if the revision hash already exists for
///     this source (just bumps `observed_at`);
///   - `link_chunks` re-runs UPDATEs that are no-ops on already-linked rows.
///
/// `raw_content` is the file's full original payload; `chunk_ids` are the
/// knowledge rows that should point at this source + revision.
pub(crate) fn link_vault_source(
    tx: &rusqlite::Transaction<'_>,
    source_path: &str,
    title: &str,
    raw_content: &str,
    chunk_ids: &[i64],
) -> Result<(), AppError> {
    let source_id = sources::upsert_source(tx, source_path, sources::KIND_VAULT, Some(title))
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let revision = sources::compute_revision(raw_content);
    let outcome = sources::upsert_revision(
        tx,
        source_id,
        &revision,
        None,
        chunk_ids.len(),
        raw_content.len() as u64,
    )
    .map_err(|e| AppError::Internal(e.to_string()))?;
    let revision_id = match outcome {
        sources::RevisionOutcome::Unchanged(id) | sources::RevisionOutcome::Created { id, .. } => {
            id
        }
    };
    if !chunk_ids.is_empty() {
        sources::link_chunks(tx, source_id, revision_id, chunk_ids)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        // stamp temporal evidence on the freshly-linked vault chunks.
        // valid_from defaults to observed_at (no world-time beyond ingest time is
        // known for a vault file); authority is the vault kind's constant.
        let observed = chrono::Utc::now().to_rfc3339();
        // was `let _ =` — a silently-missed evidence
        // stamp left vault chunks with no temporal provenance. Fail the ingest;
        // the caller reports it (matching link_chunks above).
        for cid in chunk_ids {
            sources::stamp_evidence(tx, *cid, &observed, None, None, sources::AUTHORITY_VAULT)
                .map_err(|e| AppError::Internal(e.to_string()))?;
        }
    }
    Ok(())
}

pub(crate) async fn reindex(
    State(s): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
) -> Json<serde_json::Value> {
    // AuthZ admin gate (v1.2 matrix: reindex is an operator
    // surface). Legacy shape — see `/add`. `None` principal (no JWT) = superuser.
    if let Err(e) =
        crate::handlers::authorize(&principal.0, crate::auth::Action::Admin, "", "global")
    {
        return Json(serde_json::json!({ "error": e.inner.message }));
    }
    let pool = s.pool.clone();
    let model = Arc::clone(&s.model);
    let res = task::spawn_blocking(move || -> Result<(usize, usize), anyhow::Error> {
        let conn = pool.get().context("DB connection failed")?;
        let ids: Vec<(i64, String)> = conn
            .prepare("SELECT id, content FROM knowledge ORDER BY id")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .filter_map(|r| r.ok())
            .collect();
        let mut reembedded = 0usize;
        let mut skipped = 0usize;
        for (id, content) in &ids {
            let v = model.encode_one(content);
            if v.is_empty() {
                skipped += 1;
                continue;
            };
            let tx = conn.unchecked_transaction()?;
            // Replace the vec0 row (delete + re-insert, since vec0 has no UPSERT
            // for changed vectors). vec0 is the sole vector store;
            // the legacy JSON `embeddings` column is no longer written.
// was `let _ =`; propagate — a silently
            // lost vector is a silent retrieval regression.
            tx.execute(
                "DELETE FROM vec_knowledge WHERE knowledge_id = ?1",
                params![id],
            )?;
            tx.execute(
                "INSERT INTO vec_knowledge(knowledge_id, embedding_int8, embedding_bit, source, created_at)
                 SELECT ?1, vec_quantize_int8(?2, 'unit'), vec_quantize_binary(?2),
                        COALESCE((SELECT source FROM knowledge WHERE id = ?1), 'manual'),
                        datetime('now')",
                params![id, v.as_bytes()],
            )?;
            tx.commit()?;
            reembedded += 1;
        }
        Ok((reembedded, skipped))
    })
    .await;

    match res {
        Ok(Ok((reembedded, skipped))) => Json(serde_json::json!({
            "status": "completed",
            "reembedded": reembedded,
            "skipped": skipped
        })),
        Ok(Err(e)) => Json(serde_json::json!({ "error": e.to_string() })),
        Err(e) => Json(serde_json::json!({ "error": format!("task join error: {e}") })),
    }
}

pub async fn get_chunk(
    State(state): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    headers: axum::http::HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, AppError> {
    // AuthZ read gate, scoped to the requested domain.
    let domain = handlers::domain_from_headers(&headers);
    crate::handlers::authorize(
        &principal.0,
        crate::auth::Action::Read,
        "",
        domain.as_deref().unwrap_or("global"),
    )
    .map_err(|e| AppError::Forbidden(e.inner.message))?;
    // resolve pool from X-Brain-Domain header.
    let pool = handlers::resolve_domain_pool(&state.registry, domain.as_deref())
        .unwrap_or(state.pool.clone());
    // the read scope is the header-resolved
    // domain label. The SQL predicate below binds the same label so an id can
    // never cross domains in shim mode (multi-db pools are territory-scoped
    // already); the composite record gate (v1.14 scope / v1.23 role) resolves
    // once here, before the blocking closure may touch the pool.
    let label = domain
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("global")
        .to_string();
    let record_gate = crate::handlers::gate::record_read_gate(&principal.0, &state.pool);
    // mask PII for non-admin principals like /recall does —
    // the pii-flagged row's content never leaves unmasked through the legacy
    // read path (loopback/opaque stays unmasked by design). The row LOAD
    // lives in the lifecycle fetch core (domain-scoped, stored forms); the
    // read seam + the row's own re-authz stay HERE at the emission boundary.
    let pii_principal = principal.0.clone();
    let row = task::spawn_blocking(move || -> Result<Option<serde_json::Value>, AppError> {
        let conn = pool.get().map_err(|e| AppError::Internal(e.to_string()))?;
        let rec = crate::service::lifecycle::fetch::chunk_in_domain(&conn, id, &label)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let Some(rec) = rec else {
            return Ok(None);
        };
        let pii_flag = rec.pii;
        // title + heading_path ride the same read seam as
        // content (PII redaction + invisible-Unicode strip).
        let title = crate::gate::sanitize_read_opt(rec.title, pii_flag, &pii_principal);
        let heading_path =
            crate::gate::sanitize_read_opt(rec.heading_path, pii_flag, &pii_principal);
        let value = serde_json::json!({
            "id": rec.id,
            "title": title,
            "content": crate::gate::sanitize_read(&rec.content, pii_flag, &pii_principal),
            "source": rec.source,
            "document_id": rec.document_id,
            "chunk_index": rec.chunk_index,
            "heading_path": heading_path,
            "line_start": rec.line_start,
            "line_end": rec.line_end,
            "created_at": rec.created_at,
            "source_uri": rec.source_uri,
            "revision_id": rec.revision_id,
        });
        // belt-and-braces — re-authorize against the
        // row's OWN domain, and run the record gate (the recall
        // parity), so any future predicate loosening cannot leak the
        // row to a principal that could not read it via recall.
        if !handlers::can_read_domain(&pii_principal, &rec.domain) {
            return Ok(None);
        }
        if !record_gate.admits(&rec.owner, &rec.access_scope) {
            return Ok(None);
        }
        Ok(Some(value))
    })
    .await
    .map_err(|_| AppError::Internal("Task join error".into()))??;

    match row {
        Some(v) => {
            // read-event audit for direct chunk reads
            // (best-effort). Target is the chunk id — no content leaves the row.
            if crate::config::audit_read_events(principal.0.is_some())
                && let Ok(conn) = state.pool.get()
            {
                let actor = handlers::recall::principal_label(&principal.0);
                let tenant = handlers::recall::principal_tenant(&principal.0);
                crate::audit::record_read_event(
                    &conn,
                    crate::audit::AuditKind::Get,
                    &actor,
                    &format!("chunk:{id}"),
                    None,
                    &tenant,
                );
            }
            Ok(Json(v))
        }
        None => Err(AppError::NotFound("chunk not found")),
    }
}

#[derive(Deserialize)]
pub struct MultiGetRequest {
    pub ids: Vec<i64>,
}

pub async fn multi_get(
    State(state): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    headers: axum::http::HeaderMap,
    Json(req): Json<MultiGetRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // AuthZ read gate FIRST (then size check), scoped to the
    // requested domain. Reorder — auth before size so an unauth'd
    // caller learns nothing about the request shape.
    let domain = handlers::domain_from_headers(&headers);
    crate::handlers::authorize(
        &principal.0,
        crate::auth::Action::Read,
        "",
        domain.as_deref().unwrap_or("global"),
    )
    .map_err(|e| AppError::Forbidden(e.inner.message))?;
    if req.ids.len() > MAX_MULTI_GET {
        return Err(AppError::BadRequest("too many ids"));
    }
    // resolve pool from X-Brain-Domain header.
    let pool = handlers::resolve_domain_pool(&state.registry, domain.as_deref())
        .unwrap_or(state.pool.clone());
    // same label + record gate as `/get/{id}`
    // — the domain predicate below binds `label`, and the composite gate runs
    // over every fetched row (the recall parity).
    let label = domain
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("global")
        .to_string();
    let record_gate = crate::handlers::gate::record_read_gate(&principal.0, &state.pool);
    let ids = req.ids;
    // mask PII per row for non-admin principals (loopback/
    // opaque stays unmasked by design — has_pii_read(None)). The batch row
    // LOAD lives in the lifecycle fetch core (domain-scoped, stored forms);
    // the read seam + the per-row authz filters stay HERE at the emission
    // boundary (a batch read filters like recall searches).
    let pii_principal = principal.0.clone();
    let rows = task::spawn_blocking(move || -> Result<Vec<serde_json::Value>, AppError> {
        let conn = pool.get().map_err(|e| AppError::Internal(e.to_string()))?;
        let recs = crate::service::lifecycle::fetch::chunks_in_domain(&conn, &ids, &label)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let mut out = Vec::with_capacity(recs.len());
        for rec in recs {
            let pii_flag = rec.pii;
            let title = crate::gate::sanitize_read_opt(rec.title, pii_flag, &pii_principal);
            let heading_path =
                crate::gate::sanitize_read_opt(rec.heading_path, pii_flag, &pii_principal);
            let value = serde_json::json!({
                "id": rec.id,
                "title": title,
                "content": crate::gate::sanitize_read(&rec.content, pii_flag, &pii_principal),
                "document_id": rec.document_id,
                "chunk_index": rec.chunk_index,
                "heading_path": heading_path,
                "line_start": rec.line_start,
                "line_end": rec.line_end,
                "source_uri": rec.source_uri,
                "revision_id": rec.revision_id,
            });
            // drop (not error) rows whose domain the
            // principal may not read and rows the record gate denies — a
            // batch read filters like recall searches, keeping id-probing
            // of foreign rows blind rather than loud.
            if !handlers::can_read_domain(&pii_principal, &rec.domain) {
                continue;
            }
            if !record_gate.admits(&rec.owner, &rec.access_scope) {
                continue;
            }
            out.push(value);
        }
        Ok(out)
    })
    .await
    .map_err(|_| AppError::Internal("Task join error".into()))??;

    // read-event audit for batched reads (best-effort).
    // One event per request; target = the chunk count, never content.
    if crate::config::audit_read_events(principal.0.is_some())
        && let Ok(conn) = state.pool.get()
    {
        let actor = handlers::recall::principal_label(&principal.0);
        let tenant = handlers::recall::principal_tenant(&principal.0);
        crate::audit::record_read_event(
            &conn,
            crate::audit::AuditKind::Get,
            &actor,
            &format!("chunks:{}", rows.len()),
            None,
            &tenant,
        );
    }

    Ok(Json(serde_json::json!({ "chunks": rows })))
}

// ── Guard: quarantine operator endpoints ──────────────────────────
// Rows with `flagged = 1` are excluded from retrieval by default (see
// vec0_knn/fts_search). These endpoints let an operator review, approve
// (release), or purge (delete) quarantined chunks.

#[derive(Deserialize)]
pub(crate) struct QuarantineListParams {
    limit: Option<usize>,
}

/// `GET /quarantine` — list flagged (quarantined) chunks for operator review.
pub(crate) async fn list_quarantined(
    State(state): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    headers: axum::http::HeaderMap,
    Query(p): Query<QuarantineListParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    // AuthZ read gate (operator review surface), scoped to the header domain
    // (in shim mode the pool is shared, so the label also
    // scopes the SQL — a `read:<t>/global` grant must not review another
    // tenant's quarantine queue).
    let domain = handlers::domain_from_headers(&headers);
    crate::handlers::authorize(
        &principal.0,
        crate::auth::Action::Read,
        "",
        domain.as_deref().unwrap_or("global"),
    )
    .map_err(|e| AppError::Forbidden(e.inner.message))?;
    let shim_label = if state.registry.is_multi_db() {
        None
    } else {
        Some(
            domain
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("global")
                .to_string(),
        )
    };
    let limit = p.limit.unwrap_or(100).clamp(1, config::MAX_MULTI_GET);
    let pool = state.pool.clone();
    // the /quarantine list is the reviewer-facing
    // surface for flagged content — exactly where bidi smuggling is most
    // dangerous. Run title/source through the read seam. The principal is
    // copied in (loopback/opaque stay unmasked like every read surface).
    let principal_for_rows = principal.0.clone();
    let rows = task::spawn_blocking(move || -> Result<Vec<serde_json::Value>, AppError> {
        let conn = pool.get().map_err(|e| AppError::Internal(e.to_string()))?;
        // the /quarantine list is the reviewer-facing
        // surface for flagged content — exactly where bidi smuggling is most
        // dangerous. Run title/source through the read seam. The principal is
        // copied in (loopback/opaque stay unmasked like every read surface).
        let principal_ref = &principal_for_rows;
        let map_row = |r: &rusqlite::Row<'_>| -> rusqlite::Result<serde_json::Value> {
            let title = r.get::<_, Option<String>>(1)?;
            let source = r.get::<_, Option<String>>(2)?;
            Ok(serde_json::json!({
                "id": r.get::<_, i64>(0)?,
                "title": crate::gate::sanitize_read_opt(title, true, principal_ref),
                "source": crate::gate::sanitize_read_opt(source, true, principal_ref),
                "content_hash": r.get::<_, Option<String>>(3)?,
                "created_at": r.get::<_, Option<String>>(4)?,
            }))
        };
        // Shim mode binds the label into the predicate (multi-db pools
        // are territory-scoped already).
        let mut sql = String::from(
            "SELECT id, title, source, content_hash, created_at \
             FROM knowledge WHERE flagged = 1",
        );
        let mut bind: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(label) = &shim_label {
            sql.push_str(" AND domain = ?");
            bind.push(Box::new(label.clone()));
        }
        sql.push_str(" ORDER BY id DESC LIMIT ?");
        bind.push(Box::new(limit as i64));
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let out: Vec<serde_json::Value> = stmt
            .query_map(
                rusqlite::params_from_iter(bind.iter().map(|b| b.as_ref())),
                map_row,
            )
            .map_err(|e| AppError::Internal(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(out)
    })
    .await
    .map_err(|_| AppError::Internal("Task join error".into()))??;

    Ok(Json(
        serde_json::json!({ "quarantined": rows, "count": rows.len() }),
    ))
}

/// `POST /quarantine/{id}/release` — operator approves the evidence; clears the
/// flag so the chunk re-enters retrieval.
pub(crate) async fn release_quarantine(
    State(state): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, AppError> {
    // AuthZ admin gate (operator action).
    crate::handlers::authorize(&principal.0, crate::auth::Action::Admin, "", "global")
        .map_err(|e| AppError::Forbidden(e.inner.message))?;
    let pool = state.pool.clone();
    let released = task::spawn_blocking(move || -> Result<usize, AppError> {
        let conn = pool.get().map_err(|e| AppError::Internal(e.to_string()))?;
        let n = conn
            .execute(
                "UPDATE knowledge SET flagged = 0 WHERE id = ?1 AND flagged = 1",
                params![id],
            )
            .map_err(|e| AppError::Internal(e.to_string()))?;
        // Provenance: record the operator approval (hash-only, best-effort).
        if n > 0 {
            audit::record(
                &conn,
                audit::AuditKind::Ingest,
                "operator",
                &id.to_string(),
                audit::AuditStatus::Ok,
                "quarantine-release",
            );
        }
        Ok(n)
    })
    .await
    .map_err(|_| AppError::Internal("Task join error".into()))??;

    if released == 0 {
        return Err(AppError::NotFound("no quarantined chunk with that id"));
    }
    Ok(Json(
        serde_json::json!({ "ok": true, "released": released }),
    ))
}

/// `POST /quarantine/{id}/delete` — operator purges a quarantined chunk (removes
/// the knowledge row + its vec0 index entry).
pub async fn delete_quarantine(
    State(state): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, AppError> {
    // AuthZ admin gate (operator action).
    crate::handlers::authorize(&principal.0, crate::auth::Action::Admin, "", "global")
        .map_err(|e| AppError::Forbidden(e.inner.message))?;
    let pool = state.pool.clone();
    let deleted = task::spawn_blocking(move || -> Result<usize, AppError> {
        let mut conn = pool.get().map_err(|e| AppError::Internal(e.to_string()))?;
        let tx = conn
            .transaction()
            .map_err(|e| AppError::Internal(e.to_string()))?;
        // a held chunk refuses any erasure,
        // including the quarantine delete path (a held id can be flagged).
        crate::legal_hold::refuse_if_held(&tx, &[id])
            .map_err(|e| AppError::Conflict(format!("{}: {}", e.inner.code, e.inner.message)))?;
        // vec0 has no FK cascade — clean the index entry explicitly.
        // was `let _ =` — a lingering vec0 row would
        // surface a deleted chunk's vector in retrieval.
        tx.execute(
            "DELETE FROM vec_knowledge WHERE knowledge_id = ?1",
            params![id],
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;
        // Only delete if still flagged, so this endpoint can't purge live rows.
        let n = tx
            .execute(
                "DELETE FROM knowledge WHERE id = ?1 AND flagged = 1",
                params![id],
            )
            .map_err(|e| AppError::Internal(e.to_string()))?;
        if n > 0 {
            audit::record(
                &tx,
                audit::AuditKind::Ingest,
                "operator",
                &id.to_string(),
                audit::AuditStatus::Ok,
                "quarantine-delete",
            );
        }
        tx.commit().map_err(|e| AppError::Internal(e.to_string()))?;
        Ok(n)
    })
    .await
    .map_err(|_| AppError::Internal("Task join error".into()))??;

    if deleted == 0 {
        return Err(AppError::NotFound("no quarantined chunk with that id"));
    }
    Ok(Json(serde_json::json!({ "ok": true, "deleted": deleted })))
}

pub(crate) async fn get_entity(
    State(state): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    headers: axum::http::HeaderMap,
    Path(name): Path<String>,
    Query(limit_q): Query<GraphLimit>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Allow spaces: entity names are normalized per NAME_RE (^[A-Za-z0-9 _-]{1,100}$),
    // which permits spaces (e.g. note titles like "bignay fruit").
    if name.len() > 100
        || !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == ' ' || c == '_' || c == '-')
    {
        return Err(AppError::BadRequest("Invalid entity name"));
    }

    // AuthZ read gate, scoped to the requested domain.
    let domain = handlers::domain_from_headers(&headers);
    crate::handlers::authorize(
        &principal.0,
        crate::auth::Action::Read,
        "",
        domain.as_deref().unwrap_or("global"),
    )
    .map_err(|e| AppError::Forbidden(e.inner.message))?;
    // resolve pool from X-Brain-Domain header.
    let pool = handlers::resolve_domain_pool(&state.registry, domain.as_deref())
        .unwrap_or(state.pool.clone());
    let name_lower = name.to_lowercase();
    // finite edge set, clamped like the multi-get cap.
    let limit = graph_read::clamp_graph_limit(limit_q.limit);
    // in shim mode a JWT principal reads only
    // edges whose chunk provenance carries the requested domain label.
    let domain_scoped = domain.as_deref().unwrap_or("global");
    let domain_scope = handlers::graph_domain_scope(&principal.0, &state.registry, domain_scoped);

    let result = task::spawn_blocking(move || {
        let conn = pool.get().map_err(|e| AppError::Internal(e.to_string()))?;

        let entity = conn
            .query_row(
                "SELECT id, name, entity_type FROM entities WHERE name = ?1",
                params![name_lower],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .ok();

        let Some((id, name, etype)) = entity else {
            return Ok(serde_json::json!({"error": "Entity not found"}));
        };

        let relations = entity_relations(&conn, id, limit, domain_scope.as_deref())?;

        Ok(serde_json::json!({
            "name": name,
            "type": etype.unwrap_or_else(|| "concept".to_string()),
            "relations": relations
        }))
    })
    .await
    .map_err(|_| AppError::Internal("Task join error".into()))??;

    Ok(Json(result))
}

/// the edge set for an entity, capped at `limit` (newest ids
/// first — a stable, reproducible order; the KG has no histogram to rank by).
/// Extracted so the LIMIT contract is unit-testable without an HTTP stack.
/// `domain_scope` restricts edges to those whose
/// chunk provenance carries the label (shim mode, JWT principal) — an edge
/// with no knowledge link has no domain atom and is invisible to scoped
/// readers. `None` = loopback/opaque/multi-db (unrestricted, unchanged).
pub fn entity_relations(
    conn: &rusqlite::Connection,
    id: i64,
    limit: i64,
    domain_scope: Option<&str>,
) -> Result<Vec<serde_json::Value>, AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT e.name, r.relation_type, CASE WHEN r.from_entity_id = ?1 THEN 'out' ELSE 'in' END as dir
             FROM relationships r
             JOIN entities e ON (r.to_entity_id = e.id OR r.from_entity_id = e.id)
             LEFT JOIN knowledge k ON r.knowledge_id = k.id
             WHERE (r.from_entity_id = ?1 OR r.to_entity_id = ?1)
               AND r.superseded_at IS NULL
               AND (?3 IS NULL OR k.domain = ?3)
             ORDER BY r.id LIMIT ?2",
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let relations = stmt
        .query_map(params![id, limit, domain_scope], |r| {
            Ok(serde_json::json!({
                "to_entity": r.get::<_, String>(0)?,
                "relation_type": r.get::<_, String>(1)?,
                "direction": r.get::<_, String>(2)?
            }))
        })
        .map_err(|e| AppError::Internal(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect::<Vec<_>>();
    Ok(relations)
}

pub(crate) async fn get_relations(
    State(state): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    headers: axum::http::HeaderMap,
    Query(params): Query<RelationsQuery>,
    Query(limit_q): Query<GraphLimit>,
) -> Result<Json<serde_json::Value>, AppError> {
    let (param, is_from) = match (&params.from, &params.to) {
        (Some(f), None) if !f.is_empty() => (f.clone(), true),
        (None, Some(t)) if !t.is_empty() => (t.clone(), false),
        _ => return Err(AppError::BadRequest("Must specify 'from' or 'to'")),
    };

    // AuthZ read gate, scoped to the requested domain.
    let domain = handlers::domain_from_headers(&headers);
    crate::handlers::authorize(
        &principal.0,
        crate::auth::Action::Read,
        "",
        domain.as_deref().unwrap_or("global"),
    )
    .map_err(|e| AppError::Forbidden(e.inner.message))?;
    // resolve pool from X-Brain-Domain header.
    let pool = handlers::resolve_domain_pool(&state.registry, domain.as_deref())
        .unwrap_or(state.pool.clone());
    let param_lower = param.to_lowercase();
    // finite edge set, clamped like the multi-get cap.
    let limit = graph_read::clamp_graph_limit(limit_q.limit);
    // shim-mode JWT edge scoping (see
    // `graph_domain_scope`).
    let domain_scope = handlers::graph_domain_scope(
        &principal.0,
        &state.registry,
        domain.as_deref().unwrap_or("global"),
    );

    let result = task::spawn_blocking(move || {
        let conn = pool.get().map_err(|e| AppError::Internal(e.to_string()))?;
        let direction = if is_from { "out" } else { "in" };
        let results = relations_for(
            &conn,
            &param_lower,
            is_from,
            direction,
            limit,
            domain_scope.as_deref(),
        )?;
        Ok(serde_json::json!({ "relations": results }))
    })
    .await
    .map_err(|_| AppError::Internal("Task join error".into()))??;

    Ok(Json(result))
}

/// the relations fan-out/in from an entity, capped at `limit`
/// (newest ids first). Extracted for the LIMIT contract to be unit-testable.
/// `domain_scope` restricts edges by their chunk
/// provenance label (see `entity_relations`).
pub fn relations_for(
    conn: &rusqlite::Connection,
    param_lower: &str,
    is_from: bool,
    direction: &str,
    limit: i64,
    domain_scope: Option<&str>,
) -> Result<Vec<serde_json::Value>, AppError> {
    let query = if is_from {
        "SELECT e.name, r.relation_type FROM relationships r
         JOIN entities e ON r.to_entity_id = e.id
         LEFT JOIN knowledge k ON r.knowledge_id = k.id
         WHERE r.from_entity_id = (SELECT id FROM entities WHERE name = ?1)
           AND r.superseded_at IS NULL
           AND (?3 IS NULL OR k.domain = ?3)
         ORDER BY r.id LIMIT ?2"
    } else {
        "SELECT e.name, r.relation_type FROM relationships r
         JOIN entities e ON r.from_entity_id = e.id
         LEFT JOIN knowledge k ON r.knowledge_id = k.id
         WHERE r.to_entity_id = (SELECT id FROM entities WHERE name = ?1)
           AND r.superseded_at IS NULL
           AND (?3 IS NULL OR k.domain = ?3)
         ORDER BY r.id LIMIT ?2"
    };

    let mut stmt = conn
        .prepare(query)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let results = stmt
        .query_map(params![param_lower, limit, domain_scope], |r| {
            Ok(serde_json::json!({
                "entity": r.get::<_, String>(0)?,
                "relation": r.get::<_, String>(1)?,
                "direction": direction,
            }))
        })
        .map_err(|e| AppError::Internal(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect::<Vec<_>>();
    Ok(results)
}

/// `GET /graph/relationships/{id}/history` — the supersession lineage of an
/// edge.
///
/// Given any version of a (from,to,relation_type) triple, return **every**
/// version of that triple in version order (oldest → newest), each with its
/// four timestamps (valid_at, invalid_at, created_at, superseded_at) and a
/// `current` flag (`superseded_at IS NULL`). This is the historical surface
/// that a "current belief" read deliberately hides: retired versions carry
/// entity names + valid intervals that a default view redacts. It is the
/// read-side guarantee of the four-timestamp model — supersession never
/// deletes, so this surface can always reconstruct what brain believed and
/// when it stopped.
///
/// Admin-gated at the row level (the history is operator evidence, not a
/// regular read); the call is audit-recorded (`AuditKind::GraphRead`) so the
/// retrieval of retired PII-bearing labels is itself on the chain.
pub(crate) async fn get_edge_history(
    State(state): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    headers: axum::http::HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, AppError> {
    // AuthZ: Admin-level read (the supersession lineage is operator evidence).
    // The gate was `Action::Read` while every doc
    // surface (CHANGELOG §1.27.22, openapi.yaml, docs/api.md, this comment)
    // claims Admin — the retired PII-bearing labels this surface returns are
    // operator evidence, not a regular read. Code now matches the docs.
    let domain = handlers::domain_from_headers(&headers);
    crate::handlers::authorize(
        &principal.0,
        crate::auth::Action::Admin,
        "",
        domain.as_deref().unwrap_or("global"),
    )
    .map_err(|e| AppError::Forbidden(e.inner.message))?;
    let pool = handlers::resolve_domain_pool(&state.registry, domain.as_deref())
        .unwrap_or(state.pool.clone());
    let actor = principal
        .0
        .as_ref()
        .map(|p| p.sub.clone())
        .unwrap_or_else(|| "auto".to_string());

    let result = task::spawn_blocking(move || -> Result<serde_json::Value, AppError> {
        let conn = pool.get().map_err(|e| AppError::Internal(e.to_string()))?;
        // Resolve the triple from the requested version id.
        let triple: Option<(i64, i64, String)> = conn
            .query_row(
                "SELECT from_entity_id, to_entity_id, relation_type FROM relationships WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .ok();
        let Some((from_id, to_id, kind)) = triple else {
            return Err(AppError::NotFound("Relationship not found"));
        };
        // Every version of the triple, oldest → newest. Retired versions keep
        // their entity names — this surface is the read-side guarantee that
        // supersession never deletes.
        let mut stmt = conn
            .prepare(
                "SELECT e1.name, e2.name, r.relation_type, r.knowledge_id,
                        r.valid_at, r.invalid_at, r.created_at, r.superseded_at,
                        r.id
                 FROM relationships r
                 JOIN entities e1 ON r.from_entity_id = e1.id
                 JOIN entities e2 ON r.to_entity_id = e2.id
                 WHERE r.from_entity_id = ?1 AND r.to_entity_id = ?2
                   AND r.relation_type = ?3
                 ORDER BY r.id",
            )
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let versions: Vec<serde_json::Value> = stmt
            .query_map(params![from_id, to_id, kind], |r| {
                Ok(serde_json::json!({
                    "id": r.get::<_, i64>(8)?,
                    "relation_id": r.get::<_, i64>(8)?,
                    "from_entity": r.get::<_, String>(0)?,
                    "to_entity": r.get::<_, String>(1)?,
                    "relation_type": r.get::<_, String>(2)?,
                    "knowledge_id": r.get::<_, Option<i64>>(3)?,
                    "valid_at": r.get::<_, Option<String>>(4)?,
                    "invalid_at": r.get::<_, Option<String>>(5)?,
                    "created_at": r.get::<_, Option<String>>(6)?,
                    "superseded_at": r.get::<_, Option<String>>(7)?,
                    "current": r.get::<_, Option<String>>(7)?.is_none(),
                }))
            })
            .map_err(|e| AppError::Internal(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::Internal(e.to_string()))?;
        if versions.is_empty() {
            // The triple resolved but produced no versions (should not happen
            // with FK integrity, but fail closed rather than return an empty
            // lineage).
            return Err(AppError::NotFound("Relationship not found"));
        }
        let current_count = versions.iter().filter(|v| v["current"] == true).count();
        // Audit the read (the retrieval of retired, PII-bearing labels is
        // evidence). Best-effort for the response, never silent for the
        // operator: a dropped evidence row logs loudly —
        // this surface's own comment says the audit IS the guarantee.
        if crate::audit::record(
            &conn,
            crate::audit::AuditKind::GraphRead,
            &actor,
            &id.to_string(),
            crate::audit::AuditStatus::Ok,
            &format!("lineage:{from_id}:->:{to_id}:={kind} current={current_count}"),
        )
        .is_none()
        {
            tracing::warn!("edge-history audit record dropped (id {id}) — evidence gap");
        }
        Ok(serde_json::json!({
            "relation_id": versions[0]["relation_id"],
            "from_entity": versions[0]["from_entity"],
            "to_entity": versions[0]["to_entity"],
            "relation_type": kind,
            "current": versions.iter().find(|v| v["current"] == true),
            "versions": versions,
        }))
    })
    .await
    .map_err(|_| AppError::Internal("Task join error".into()))??;

    Ok(Json(result))
}

pub(crate) async fn traverse_graph(
    State(state): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    headers: axum::http::HeaderMap,
    Query(params): Query<TraverseQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    // AuthZ read gate, scoped to the requested domain.
    let domain = handlers::domain_from_headers(&headers);
    crate::handlers::authorize(
        &principal.0,
        crate::auth::Action::Read,
        "",
        domain.as_deref().unwrap_or("global"),
    )
    .map_err(|e| AppError::Forbidden(e.inner.message))?;
    let entity = params.start.unwrap_or_default();
    // hard-cap traversal depth at trace::MAX_HOPS
    // (forbidden-list rule: no unbounded graph walks).
    let depth = params.max_depth.unwrap_or(2).min(trace::MAX_HOPS as u8);
    let cross_domain = params.cross_domain;
    // structured path output (default off for back-compat).
    let explain = params.explain;

    if entity.is_empty() {
        return Err(AppError::BadRequest("Entity is required"));
    }
    if entity.len() > 100
        || !entity
            .chars()
            .all(|c| c.is_alphanumeric() || c == ' ' || c == '_' || c == '-')
    {
        return Err(AppError::BadRequest("Invalid entity name"));
    }

    // normalize the bi-temporal `at` filter to the
    // SQLite-comparable format. Reject malformed timestamps (a silent lexical
    // compare would be wrong, not just useless).
    let at_normalized: Option<String> = match params.at.as_deref().map(str::trim) {
        Some("") | None => None,
        Some(s) => Some(search_mod::normalize_since(s).map_err(|_| {
            AppError::BadRequest("invalid 'at' timestamp; expected ISO-8601 or YYYY-MM-DD")
        })?),
    };

    // normalize the `kind` filter into either an exact
    // match or a prefix match (if it ends with `:`). Empty → None (walk all).
    // The filter is applied INSIDE the recursive CTE via parameterized SQL,
    // never interpolation (forbidden-list rule).
    let kind_filter: Option<String> = params
        .kind
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    // resolve pool from X-Brain-Domain header. When `cross_domain=true`,
    // walk edges across every known domain pool.
    let header_domain = handlers::domain_from_headers(&headers);
    // Scope label: computed once (the header is moved into the target
    // list below, so the label is materialized before the move).
    let scope_label = header_domain.as_deref().unwrap_or("global").to_string();
    let entity_lower = entity.to_lowercase();

    // Build the (domain, pool) target list. Cross-domain fans out to every
    // known domain; otherwise just the resolved one.
    let mut targets: Vec<(String, crate::Pool)> = Vec::new();
    if cross_domain {
        for d in state.registry.known_domains() {
            if let Ok(p) = state.registry.pool_for(&d) {
                targets.push((d, p));
            }
        }
    }
    if targets.is_empty() {
        let d = scope_label.clone();
        let p = handlers::resolve_domain_pool(&state.registry, Some(&d))
            .unwrap_or_else(|_| state.pool.clone());
        targets.push((d, p));
    }
    // a tenant-scoped principal must not
    // walk FOREIGN pools. Drop every target whose domain it may not read —
    // the same retain `/recall` federation applies. Loopback/opaque
    // (superuser) is untouched. An emptied list is a legitimate "nothing
    // walkable", not an error.
    targets.retain(|(d, _)| handlers::can_read_domain(&principal.0, d));
    // shim-mode JWT edge scoping (the entity tables carry no
    // domain column; the chunk link is the domain atom).
    let domain_scope = handlers::graph_domain_scope(&principal.0, &state.registry, &scope_label);

    let result = task::spawn_blocking(move || -> Result<serde_json::Value, AppError> {
        // bi-temporal edge filter. When `at` is set, an
        // edge is traversable iff its valid-interval [valid_at, invalid_at)
        // contains `at`: valid_at <= at AND (invalid_at IS NULL OR invalid_at > at).
        // NULL valid_at ⇒ origin unknown ⇒ treated as always-valid (the
        // additive-migration default for pre-v1.4 edges). Parameterized, never
        // interpolated. Graphiti-validity semantics (Context7 2026-07-30).
        //
        // the CTE also bounds depth (already) and visits
        // (the recursive UNION ALL has no global visited-set; the path-based
        // cycle guard below prevents infinite loops). MAX_HOPS/MAX_VISITED are
        // enforced on the Rust side after the walk.
        //
        // the CTE now carries `relation_type` per hop so the
        // structured `paths` output can render faithful explanations
        // (`A --works_at--> B --ceo_of--> C`). The flat `traversal` array stays
        // for back-compat. Path string carries ids+rels as `id:rel:id:rel:id`.
        let valid_clause = if at_normalized.is_some() {
            " AND (valid_at IS NULL OR valid_at <= ?at) \
               AND (invalid_at IS NULL OR invalid_at > ?at)"
        } else {
            ""
        };
        // Traversal meets its own doc. The
        // module doc promised "traversal skips edges that a later same-typed
        // edge has superseded" — the code never did (it filtered only by the
        // valid-time window). This predicate makes an edge a *current belief*
        // per the true bi-temporal model (SQL:2011 / Snodgrass): it is live
        // (`superseded_at IS NULL` — transaction-time END unset) AND it is the
        // newest live version of its (from,to,relation_type) triple
        // (`NOT EXISTS` a newer live `r2.id`). Currency is a **transaction-time**
        // property, deliberately NOT a valid-time one: a corrected belief that
        // is backdated (earlier valid_at) still supersedes the old current belief
        // (see graph_supersede::backdated_overlap_preserves_both_valid_intervals),
        // so the anti-join orders by id, not by valid interval.
        //
        // It is a no-op on well-formed + legacy DBs (byte-identical default:
        // the UNIQUE idx_rels_unique historically enforced one row per triple,
        // so a lone row has no same-triple live peer and the NOT EXISTS holds).
        // On a corrupt legacy DB (multiple live rows for one triple, written
        // before the invariant), the anti-join deterministically converges to
        // the newest live edition. When `at` is present the valid-time window
        // (already served by `valid_clause`) is composed on the SAME current
        // belief — the standard bi-temporal as-of query (current beliefs whose
        // valid interval contains `at`).
        let current_clause = "
                AND r.superseded_at IS NULL
                AND NOT EXISTS (SELECT 1 FROM relationships r2
                    WHERE r2.from_entity_id = r.from_entity_id
                      AND r2.to_entity_id = r.to_entity_id
                      AND r2.relation_type = r.relation_type
                      AND r2.superseded_at IS NULL
                      AND r2.id > r.id)";
        // The seed CTE's `relationships` is aliased `rs` (it joins `knowledge`
        // for the domain scope). Qualify every correlated reference with `rs.`
        // so the NOT EXISTS subquery's `r2` does not shadow the outer row
        // (a shadowed `r2.id > id` would compare a row to itself — always false,
        // the NOT EXISTS always true, and the seed anti-join silently disabled).
        let current_seed_clause = "
                AND rs.superseded_at IS NULL
                AND NOT EXISTS (SELECT 1 FROM relationships r2
                    WHERE r2.from_entity_id = rs.from_entity_id
                      AND r2.to_entity_id = rs.to_entity_id
                      AND r2.relation_type = rs.relation_type
                      AND r2.superseded_at IS NULL
                      AND r2.id > rs.id)";
        // kind filter. Prefix match when kind ends with `:` (e.g.
        // `causes:`), exact match otherwise. Applied to BOTH the seed and the
        // recursive step so the walk stays inside the requested edge type.
        // Use named placeholders that we'll substitute to the right ?N below.
        let kind_clause_tmpl = match &kind_filter {
            Some(k) if k.ends_with(':') => " AND r.relation_type LIKE ?kind ESCAPE '\\' ",
            Some(_) => " AND r.relation_type = ?kind ",
            None => "",
        };
        // The seed clause references the columns without the `r.` alias.
        let kind_seed_clause_tmpl = match &kind_filter {
            Some(k) if k.ends_with(':') => " AND relation_type LIKE ?kind ESCAPE '\\' ",
            Some(_) => " AND relation_type = ?kind ",
            None => "",
        };
        // Compute the positional placeholder indices based on which optional
        // params are bound. ?1 = eid (always), ?2 = depth (always, in the
        // recursive step). After that: ?3 = at (if present), then the next
        // index = kind (if present). This is the bug fix: previously kind was
        // hardcoded to ?4, which only worked when `at` was also bound.
        let at_ph = "?3";
        let kind_ph = if at_normalized.is_some() { "?4" } else { "?3" };
        // the scope placeholder sits after every
        // optional at/kind param (always bound, as `Option<String>`); the
        // scope clause is compiled in only when the request is scoped, so
        // unscoped walks (loopback/opaque/multi-db) keep the byte-identical
        // query shape.
        let (scope_clause, scope_join, scope_join_rec) = match &domain_scope {
            Some(_) => {
                let ph = if at_normalized.is_some() {
                    "?5"
                } else if kind_filter.is_some() {
                    "?4"
                } else {
                    "?3"
                };
                (
                    format!(" AND (?{ph} IS NULL OR k.domain = ?{ph})"),
                    "LEFT JOIN knowledge k ON k.id = rs.knowledge_id".to_string(),
                    "LEFT JOIN knowledge k ON r.knowledge_id = k.id".to_string(),
                )
            }
            None => (String::new(), String::new(), String::new()),
        };
        let valid_clause = valid_clause.replace("?at", at_ph);
        let kind_clause = kind_clause_tmpl.replace("?kind", kind_ph);
        let kind_seed_clause = kind_seed_clause_tmpl.replace("?kind", kind_ph);
        // The current-belief clauses carry no `?at` placeholder (currency is
        // transaction-time, independent of the valid-time window); they are
        // constants, not templates.
        let current_clause = current_clause.to_string();
        let current_seed_clause = current_seed_clause.to_string();
        let query = format!(
            "WITH RECURSIVE traversal(from_id, to_id, depth, path, edge_path) AS (\
                SELECT rs.from_entity_id, rs.to_entity_id, 1, \
                       CAST(rs.from_entity_id AS TEXT), CAST(rs.relation_type AS TEXT) \
                FROM relationships rs {scope_join} \
                WHERE rs.from_entity_id = ?1{valid_clause}{kind_seed_clause}{current_seed_clause}{scope_clause} \
                UNION ALL \
                SELECT r.from_entity_id, r.to_entity_id, t.depth + 1, \
                       t.path || '->' || CAST(r.from_entity_id AS TEXT), \
                       t.edge_path || '|' || r.relation_type \
                FROM relationships r \
                JOIN traversal t ON r.from_entity_id = t.to_id \
                {scope_join_rec} \
                WHERE t.depth < ?2{valid_clause}{kind_clause}{current_clause}{scope_clause} \
            ) \
            SELECT DISTINCT e.name, t.depth, t.path, t.edge_path, \
                   (SELECT name FROM entities WHERE id = t.from_id) AS from_name \
            FROM traversal t \
            JOIN entities e ON t.to_id = e.id"
        );
        let mut all: Vec<serde_json::Value> = Vec::new();
        let mut total_visited = 0;
        for (domain, pool) in &targets {
            // Hard cap on visited nodes across the whole walk (forbidden-list
            // rule: no unbounded graph walks). Stop once breached.
            if total_visited >= trace::MAX_VISITED {
                break;
            }
            let conn = match pool.get() {
                Ok(c) => c,
                Err(e) => return Err(AppError::Internal(e.to_string())),
            };
            let entity_id: Option<i64> = conn
                .query_row(
                    "SELECT id FROM entities WHERE name = ?1",
                    params![entity_lower],
                    |r| r.get(0),
                )
                .ok();
            let Some(eid) = entity_id else { continue };
            let mut stmt = conn
                .prepare(&query)
                .map_err(|e| AppError::Internal(e.to_string()))?;
            // Build the params for the kind filter (same value used for both
            // seed and recursive step; SQLite's ?N is positional per-CTE-clause
            // but we pass it once and reference it twice via the format string).
            // For prefix matches, escape the trailing `_` and `%` wildcards in
            // the user's input so `causes:` doesn't match `causes_x` (the `_`
            // is a SQL LIKE wildcard). The escape char is `\\`.
            let kind_param: Option<String> = kind_filter.as_ref().map(|k| {
                if k.ends_with(':') {
                    // Prefix match: escape wildcards then append `%`.
                    let escaped = k.replace('_', "\\_").replace('%', "\\%");
                    format!("{escaped}%")
                } else {
                    k.clone()
                }
            });
            let rows: Vec<_> = match (
                at_normalized.as_ref(),
                kind_param.as_ref(),
                domain_scope.as_ref(),
            ) {
                (Some(at), Some(k), Some(sc)) => stmt
                    .query_map(params![eid, depth, at, k, sc], graph_read::traverse_row_mapper(domain))
                    .map_err(|e| AppError::Internal(e.to_string()))?
                    .filter_map(|r| r.ok())
                    .take(trace::MAX_VISITED.saturating_sub(total_visited))
                    .collect(),
                (Some(at), Some(k), None) => stmt
                    .query_map(params![eid, depth, at, k], graph_read::traverse_row_mapper(domain))
                    .map_err(|e| AppError::Internal(e.to_string()))?
                    .filter_map(|r| r.ok())
                    .take(trace::MAX_VISITED.saturating_sub(total_visited))
                    .collect(),
                (Some(at), None, Some(sc)) => stmt
                    .query_map(params![eid, depth, at, sc], graph_read::traverse_row_mapper(domain))
                    .map_err(|e| AppError::Internal(e.to_string()))?
                    .filter_map(|r| r.ok())
                    .take(trace::MAX_VISITED.saturating_sub(total_visited))
                    .collect(),
                (Some(at), None, None) => stmt
                    .query_map(params![eid, depth, at], graph_read::traverse_row_mapper(domain))
                    .map_err(|e| AppError::Internal(e.to_string()))?
                    .filter_map(|r| r.ok())
                    .take(trace::MAX_VISITED.saturating_sub(total_visited))
                    .collect(),
                (None, Some(k), Some(sc)) => stmt
                    .query_map(params![eid, depth, k, sc], graph_read::traverse_row_mapper(domain))
                    .map_err(|e| AppError::Internal(e.to_string()))?
                    .filter_map(|r| r.ok())
                    .take(trace::MAX_VISITED.saturating_sub(total_visited))
                    .collect(),
                (None, Some(k), None) => stmt
                    .query_map(params![eid, depth, k], graph_read::traverse_row_mapper(domain))
                    .map_err(|e| AppError::Internal(e.to_string()))?
                    .filter_map(|r| r.ok())
                    .take(trace::MAX_VISITED.saturating_sub(total_visited))
                    .collect(),
                (None, None, Some(sc)) => stmt
                    .query_map(params![eid, depth, sc], graph_read::traverse_row_mapper(domain))
                    .map_err(|e| AppError::Internal(e.to_string()))?
                    .filter_map(|r| r.ok())
                    .take(trace::MAX_VISITED.saturating_sub(total_visited))
                    .collect(),
                (None, None, None) => stmt
                    .query_map(params![eid, depth], graph_read::traverse_row_mapper(domain))
                    .map_err(|e| AppError::Internal(e.to_string()))?
                    .filter_map(|r| r.ok())
                    .take(trace::MAX_VISITED.saturating_sub(total_visited))
                    .collect(),
            };
            total_visited += rows.len();
            all.extend(rows);
        }
        // build the structured `paths` array when requested.
        // Each entry groups a traversal row into a hop chain with named entities
        // + relation types, so a consuming agent can render
        // "A --works_at--> B --ceo_of--> C" without parsing the id-string.
        let paths = if explain {
            graph_read::build_explanation_paths(&all)
        } else {
            Vec::new()
        };
        let mut out = serde_json::json!({ "traversal": all, "visited": total_visited });
        if explain {
            out["paths"] = serde_json::Value::Array(paths);
        }
        Ok(out)
    })
    .await
    .map_err(|_| AppError::Internal("Task join error".into()))??;

    Ok(Json(result))
}

/// The 1 GiB import dial (F-49a): merged AFTER the shared 1 MiB cap in
/// `mod.rs` — see the mod.rs comment for the tower-http ordering.
pub fn import_router() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/domains/{name}/import",
            post(handlers::domains::import_domain),
        )
        // F-49a: this dial is DELIBERATE and scoped to exactly this route
        // group — bulk markdown imports are Admin-gated (handler re-checks
        // before reading a byte) and stream-parsed one file at a time, so
        // the 1 GiB ceiling is an operator-scale allowance, not an anonymous
        // amplification surface. Every other route stays at the 1 MiB
        // default layered after this router.
        .layer(tower_http::limit::RequestBodyLimitLayer::new(
            1024 * 1024 * 1024,
        ))
}
