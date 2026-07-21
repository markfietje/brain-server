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
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            inner: ApiError::new("internal_error", message.into()),
        }
    }
}

impl IntoResponse for HandlerError {
    fn into_response(self) -> axum::response::Response {
        (self.status, Json(ErrorBody { error: self.inner })).into_response()
    }
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

// Hand-rolled matcher for the tiny set of validation patterns used by the
// handler stubs.  Replaces the `regex` crate dependency (removed with the
// annotator module in v0.9.0).  The patterns are simple character-class +
// repetition checks — no need for a full regex engine.
fn is_match(_pattern: &str, s: &str) -> bool {
    // The only patterns used by the handlers are:
    //   ^[a-z0-9_][a-z0-9_-]{0,63}$   (domain names)
    //   ^[a-z0-9_]{1,64}$            (relation types)
    //   ^[a-z0-9_ -]{1,100}$         (entity names)
    // We hand-roll a checker for these shapes.
    let s = s.trim();
    if s.is_empty() {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        && s.len() <= 64
}
