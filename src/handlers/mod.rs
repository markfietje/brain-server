//! HTTP handlers for the brain-server plugin API (`/recall`, `/ingest`, etc.).
//!
//! Wire contract: see `API_CONTRACT.md` (the source of truth for request/response
//! shapes, validation bounds, and error envelope). These handlers implement the
//! **wire** contract; the internal logic (embed, centroid routing, sqlite-vec

// Stubs for future versions — suppress dead-code warnings until filled in.
#![allow(dead_code)]
#![allow(unused_imports)]
//! search, KG upsert) is filled in as the roadmap phases land.
//!
//! Conventions:
//!  - Serde types here are the canonical Rust shapes. Unknown JSON keys are
//!    ignored (forward-compatible).
//!  - All bounds are validated **before** any heavy work. Failures return the
//!    uniform error envelope `{ error: { code, message, details } }` with
//!    `400` and a stable machine-readable `code`.
//!  - Heavy logic that doesn't exist yet uses `unimplemented!()` with a
//!    reference to the ROADMAP phase that delivers it. The wire is real; the
//!    bodies are deliberately minimal so they can be filled in without
//!    changing the contract.

pub mod connectors;
pub mod consolidate;
pub mod domains;
pub mod forget;
pub mod ingest;
pub mod recall;
pub mod sources;
pub mod webhooks;

use axum::{http::StatusCode, response::IntoResponse, Json};
use serde::Serialize;
use serde_json::Value;

// ---------------------------------------------------------------------------
// Validation constants (mirror API_CONTRACT.md §0)
// ---------------------------------------------------------------------------

pub const DOMAIN_RE: &str = r"^[a-z0-9][a-z0-9_-]{0,62}$";
pub const NAME_RE: &str = r"^[A-Za-z0-9 _-]{1,100}$";
pub const RELTYPE_RE: &str = r"^[a-z0-9_]{1,64}$";

pub const MAX_QUERY: usize = 2_000;
pub const MAX_TITLE: usize = 500;
pub const MAX_CONTENT: usize = 1_000_000;
pub const MAX_LIMIT: u32 = 100;
pub const MIN_LIMIT: u32 = 1;
pub const MAX_ENTITIES: usize = 200;
pub const MAX_RELATIONS: usize = 200;
pub const MAX_BODY: usize = 2 * 1024 * 1024; // 2 MiB hard cap

pub const DEFAULT_RECALL_LIMIT: u32 = 5;
pub const DOMAIN_CONFIDENCE_THRESHOLD: f32 = 0.55;

// ---------------------------------------------------------------------------
// Shared serde types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HitSource {
    Vector,
    Fts,
    Both,
    Graph,
}

#[derive(Debug, Serialize)]
pub struct RecallHit {
    pub id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub content: String,
    pub score: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<HitSource>,
    /// Per-retriever ranks + fused score. Populated only when `provenance=true`
    /// on the request; absent otherwise (backward-compat).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance: Option<crate::search::Provenance>,
    /// Structured evidence (verbatim snippet window + line/heading span +
    /// source link + highlight ranges). Populated whenever present on the
    /// underlying `SearchResult`; absent for legacy/empty hits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<crate::search::Evidence>,
    /// Bounded verbatim snippet of the chunk (a window around the query terms),
    /// forwarded from the underlying `SearchResult`. Absent when the search did
    /// not compute one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    /// All recalled content is untrusted evidence (OWASP LLM01:2025). Serialized
    /// `true` so the consuming agent enforces the instruction/data boundary and
    /// never treats recalled text as commands.
    pub untrusted: bool,
    /// v0.9.8 M3.2: true when this chunk participates in a `contradicts` or
    /// `supersedes` link with another *current* chunk — i.e. the claim is
    /// contested. Absent (`None`) when not computed or when no conflict exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct RecallResponse {
    pub hits: Vec<RecallHit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domains_searched: Option<Vec<String>>,
    /// Per-stage retrieval telemetry, included when `provenance` is requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<crate::search::SearchTelemetry>,
}

#[derive(Debug, Serialize)]
pub struct IngestResponse {
    pub id: i64,
    pub status: &'static str, // "created" | "duplicate"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entities_added: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relations_added: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct ForgetResponse {
    pub deleted: bool,
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
}

// ---------------------------------------------------------------------------
// Uniform error envelope (API_CONTRACT.md §5)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: ApiError,
}

#[derive(Debug, Serialize)]
pub struct ApiError {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl ApiError {
    pub const fn new(code: &'static str, message: String) -> Self {
        Self {
            code,
            message,
            details: None,
        }
    }
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
}

/// Handler error type. Carries (status, code, message, optional details) and
/// renders the uniform `ErrorBody` envelope. Map domain failures here.
#[derive(Debug)]
pub struct HandlerError {
    pub status: StatusCode,
    pub inner: ApiError,
}

impl HandlerError {
    pub fn bad_request(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            inner: ApiError::new(code, message.into()),
        }
    }
    pub fn bad_request_with(
        code: &'static str,
        message: impl Into<String>,
        details: Value,
    ) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            inner: ApiError::new(code, message.into()).with_details(details),
        }
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            inner: ApiError::new("not_found", message.into()),
        }
    }
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            inner: ApiError::new("unauthorized", message.into()),
        }
    }
    pub fn rate_limited(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::TOO_MANY_REQUESTS,
            inner: ApiError::new("rate_limited", message.into()),
        }
    }
    pub fn payload_too_large(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            inner: ApiError::new("payload_too_large", message.into()),
        }
    }
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            inner: ApiError::new("recall_unavailable", message.into()),
        }
    }
    /// v0.9.9 "Qualify": HTTP 507 — new ingests are refused because the server
    /// is over its capacity envelope. Read routes (`/search`, `/recall`, `/get`)
    /// never return this; an over-capacity brain still answers.
    pub fn insufficient_storage(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INSUFFICIENT_STORAGE,
            inner: ApiError::new("capacity_exceeded", message.into()),
        }
    }
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            inner: ApiError::new("internal_error", message.into()),
        }
    }
}

/// v0.9.9 "Qualify": reject a write when the server is over its capacity
/// envelope. Returns `Ok(())` when writes are allowed; `Err(507)` when
/// `CapacityStatus::Exceeded`. Best-effort: if the measurement query fails, the
/// guard fails OPEN (allows the write) — a transient DB error must not turn the
/// brain read-only. Callers: every ingest path (`/add`, `/ingest`,
/// `/ingest/memory`, `/ingest/markdown`). Read routes do NOT call this.
pub fn guard_capacity(state: &crate::AppState) -> Result<(), HandlerError> {
    use brain_server::capacity::{capacity_target, CapacityEnvelope, CapacityStatus};
    // Cheap short-circuit: pool state never blocks writes here; we only need a
    // connection to count rows. If the pool is momentarily exhausted, fail open.
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
    // CRITICAL: measure the process's own RSS, not system-wide memory.
    // System::used_memory() is the whole-host figure; on any machine with a
    // real workload it would always exceed the 320 MB per-process ceiling and
    // block every write with a spurious 507.
    let mut sys = sysinfo::System::new();
    let pid = sysinfo::Pid::from_u32(std::process::id());
    sys.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&[pid]),
        false,
        sysinfo::ProcessRefreshKind::nothing().with_memory(),
    );
    let rss_mib: u64 = sys
        .process(pid)
        .map(|p| p.memory() / 1_000_000)
        .unwrap_or(0);
    let envelope = CapacityEnvelope::for_target(capacity_target());
    let status = brain_server::capacity::classify(docs, db_mib, rss_mib, &envelope);
    if status.blocks_writes() {
        return Err(HandlerError::insufficient_storage(format!(
            "capacity_exceeded: docs={docs}/{} db_mib={db_mib}/{} rss_mib={rss_mib}/{} — see BENCHMARKS.md §capacity",
            envelope.max_docs, envelope.max_db_mib, envelope.max_rss_mib
        )));
    }
    Ok(())
}

impl IntoResponse for HandlerError {
    fn into_response(self) -> axum::response::Response {
        (self.status, Json(ErrorBody { error: self.inner })).into_response()
    }
}

// ---------------------------------------------------------------------------
// Domain resolution helpers (v1.0.0 "Domains")
// ---------------------------------------------------------------------------

/// Resolve a domain name to its connection pool via the registry.
/// Defaults to `"global"` when `domain` is `None` or empty. Unknown domains
/// return a `400` whose `details` carries the list of known domains (per the
/// v1.0 plan: "Unknown domain → 400 with the list of known domains").
pub fn resolve_domain_pool(
    registry: &crate::domain_registry::DomainRegistry,
    domain: Option<&str>,
) -> Result<crate::Pool, HandlerError> {
    let d = domain.filter(|s| !s.trim().is_empty()).unwrap_or("global");
    registry.pool_for(d).map_err(|e| {
        let known = registry.known_domains();
        HandlerError::bad_request_with(
            "domain_invalid",
            format!("cannot resolve domain '{d}': {e}"),
            serde_json::json!({ "known_domains": known }),
        )
    })
}

/// Extract domain from `X-Brain-Domain` header — used by GET handlers
/// that don't have a JSON body with a `domain` field.
pub fn domain_from_headers(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get("x-brain-domain")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_lowercase())
}

// ---------------------------------------------------------------------------
// Shared validation helpers
// ---------------------------------------------------------------------------

/// Normalize a domain name: trim whitespace, lowercase, validate regex.
pub fn normalize_domain(raw: &str) -> Result<String, HandlerError> {
    let s = raw.trim().to_lowercase();
    if s.is_empty() {
        return Err(HandlerError::bad_request(
            "domain_invalid",
            "domain must not be empty",
        ));
    }
    if s.len() > 63 {
        return Err(HandlerError::bad_request(
            "domain_invalid",
            "domain exceeds 63 characters",
        ));
    }
    if !is_match(DOMAIN_RE, &s) {
        return Err(HandlerError::bad_request(
            "domain_invalid",
            "domain must match ^[a-z0-9][a-z0-9_-]{0,62}$",
        ));
    }
    Ok(s)
}

/// Normalize an entity/relation name: trim, collapse whitespace, validate.
pub fn normalize_name(raw: &str) -> Result<String, HandlerError> {
    let s: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.is_empty() {
        return Err(HandlerError::bad_request(
            "name_invalid",
            "name must not be empty",
        ));
    }
    if s.len() > 100 {
        return Err(HandlerError::bad_request(
            "name_invalid",
            "name exceeds 100 characters",
        ));
    }
    if !is_match(NAME_RE, &s) {
        return Err(HandlerError::bad_request(
            "name_invalid",
            "name must match ^[A-Za-z0-9 _-]{1,100}$",
        ));
    }
    Ok(s.to_lowercase())
}

/// Normalize a relation type: lowercase snake_case.
pub fn normalize_rel_type(raw: &str) -> Result<String, HandlerError> {
    let s = raw.trim().to_lowercase();
    if s.is_empty() || s.len() > 64 {
        return Err(HandlerError::bad_request(
            "relation_invalid",
            "relation type must be 1..=64 chars",
        ));
    }
    if !is_match(RELTYPE_RE, &s) {
        return Err(HandlerError::bad_request(
            "relation_invalid",
            "relation type must match ^[a-z0-9_]{1,64}$",
        ));
    }
    Ok(s)
}

// Hand-rolled checkers for the three patterns used by the validators.
// Replaces the `regex` crate dependency (removed with the annotator module in
// v0.9.0). Each checker enforces ONE shape — the previous single `is_match`
// ignored its `_pattern` argument and silently rejected spaces in entity
// names (breaking the canonical `vitamin d3` example).
//
// ponytail: each pattern is a tiny character-class + length check; a full
// regex engine is overkill. The shapes are pinned by the test in this module.
fn is_match(pattern: &str, s: &str) -> bool {
    match pattern {
        DOMAIN_RE => is_valid_domain(s),
        NAME_RE => is_valid_name(s),
        RELTYPE_RE => is_valid_rel_type(s),
        _ => false,
    }
}

/// `^[a-z0-9][a-z0-9_-]{0,62}$` — delegates to storage_layout (security-critical;
/// one source of truth for filename safety).
fn is_valid_domain(s: &str) -> bool {
    brain_server::storage_layout::is_valid_domain(s.trim())
}

/// `^[A-Za-z0-9 _-]{1,100}$` — entity/note names. Allows spaces (e.g. note
/// titles, multi-word entities like "vitamin d3") and uppercase. The validator
/// lowercases before insertion; this only checks the input shape.
fn is_valid_name(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty()
        && s.len() <= 100
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '_' || c == '-')
}

/// `^[a-z0-9_]{1,64}$` — relation types (snake_case, no hyphens).
fn is_valid_rel_type(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_from_headers_extracts_header_value() {
        use axum::http::HeaderMap;
        let mut headers = HeaderMap::new();
        assert_eq!(domain_from_headers(&headers), None);
        headers.insert("x-brain-domain", "health".parse().unwrap());
        assert_eq!(domain_from_headers(&headers), Some("health".to_string()));
    }

    #[test]
    fn resolve_domain_pool_falls_back_to_global() {
        use crate::domain_registry::DomainRegistry;
        use r2d2_sqlite::SqliteConnectionManager;
        let mgr = SqliteConnectionManager::memory();
        let pool: crate::Pool = r2d2::Pool::builder().build(mgr).expect("pool");
        let reg = DomainRegistry::new(pool.clone(), std::path::Path::new("/tmp/db.db"), false);
        let p = resolve_domain_pool(&reg, None).expect("no domain defaults to global");
        assert!(p.get().is_ok());
        let p = resolve_domain_pool(&reg, Some("")).expect("empty domain defaults to global");
        assert!(p.get().is_ok());
        // Invalid domain rejected.
        assert!(resolve_domain_pool(&reg, Some("../evil")).is_err());
    }

    /// Pin the three validator shapes. The previous is_match silently rejected
    /// spaces in entity names, breaking the canonical `vitamin d3` example.
    #[test]
    fn validators_match_their_documented_shapes() {
        // Domain: lowercase + digit + underscore/hyphen, 1..=63 chars.
        assert!(is_match(DOMAIN_RE, "global"));
        assert!(is_match(DOMAIN_RE, "health"));
        assert!(!is_match(DOMAIN_RE, "Health"));
        assert!(!is_match(DOMAIN_RE, "has space"));
        assert!(!is_match(DOMAIN_RE, "../evil"));
        assert!(!is_match(DOMAIN_RE, &"x".repeat(64)));

        // Name: alphanumeric + space + underscore/hyphen, 1..=100 chars.
        assert!(is_match(NAME_RE, "vitamin d3"));
        assert!(is_match(NAME_RE, "Bignay Fruit"));
        assert!(is_match(NAME_RE, "rust"));
        assert!(!is_match(NAME_RE, ""));
        assert!(!is_match(NAME_RE, "dot.in.name"));
        assert!(!is_match(NAME_RE, &"x".repeat(101)));

        // Relation type: lowercase snake_case, 1..=64 chars, no hyphens.
        assert!(is_match(RELTYPE_RE, "helps"));
        assert!(is_match(RELTYPE_RE, "relates_to"));
        assert!(!is_match(RELTYPE_RE, "Has Caps"));
        assert!(!is_match(RELTYPE_RE, "has-hyphen"));
        assert!(!is_match(RELTYPE_RE, "has space"));
    }
}
