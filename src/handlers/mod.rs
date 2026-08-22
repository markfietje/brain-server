//! HTTP handlers for the brain-server plugin API (`/recall`, `/ingest`, etc.).
//!
//! Wire contract: see `API_CONTRACT.md` (the source of truth for request/response
//! shapes, validation bounds, and error envelope). These handlers implement the
//! **wire** contract; the internal logic (embed, centroid routing, sqlite-vec

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

pub mod auth;
pub mod breaches;
pub mod clients;
#[cfg(feature = "compliance-pack")]
pub mod compliance;
pub mod connectors;
pub mod consolidate;
pub mod domains;
pub mod forget;
pub mod frontend;
pub mod gate;
pub mod govern;
pub mod holds;
pub mod ingest;
pub mod observe;
pub mod procedure;
pub mod profiles;
pub mod recall;
pub mod roles;
pub mod sources;
pub mod suggest;
pub mod transfers;
pub mod ump;
pub mod ump_ops;
pub mod verify;
pub mod webhooks;
pub mod well_known;
pub mod workflow;

use axum::{Json, http::StatusCode, response::IntoResponse};
use serde::Serialize;
use serde_json::Value;

// ---------------------------------------------------------------------------
// Validation constants (mirror API_CONTRACT.md §0)
// ---------------------------------------------------------------------------

pub const DOMAIN_RE: &str = r"^[a-z0-9][a-z0-9_-]{0,62}$";
pub const NAME_RE: &str = r"^[A-Za-z0-9 _-]{1,100}$";
/// allows an optional TRACE typed-edge prefix
/// (`update:`, `supersedes:`, `contradicts:`, `causes:`) before the base
/// relation. The prefix is a single `:` separator; the base stays snake_case.
pub const RELTYPE_RE: &str = r"^([a-z]+:)?[a-z0-9_]{1,62}$";

pub const MAX_QUERY: usize = 2_000;
pub const MAX_TITLE: usize = 500;
pub const MAX_CONTENT: usize = 1_000_000;
/// Explicit entry-count bound on `/ingest/memory` (the 1 MiB body cap alone
/// still admits thousands of micro-entries in one tx).
pub const MAX_INGEST_ENTRIES: usize = 500;
/// bound on the autoCapture `source_prompt` reviewer-facing field.
/// The plugin sends ≤ 2000 chars; the server enforces its own bound so a
/// malicious caller can't persist a multi-MiB prompt to the proposals table.
pub const MAX_SOURCE_PROMPT: usize = 2_048;
pub const MAX_LIMIT: u32 = 100;
pub const MIN_LIMIT: u32 = 1;
pub const MAX_ENTITIES: usize = 200;
pub const MAX_RELATIONS: usize = 200;
pub const DEFAULT_RECALL_LIMIT: u32 = 5;

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
    /// true when this chunk participates in a `contradicts` or
    /// `supersedes` link with another *current* chunk — i.e. the claim is
    /// contested. Absent (`None`) when not computed or when no conflict exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict: Option<bool>,
    /// deterministic stored confidence (0..1). Surfaced so
    /// the caller can see how much weight a fact deserves.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    /// `assertion_kind` (stated|observed|inferred).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assertion_kind: Option<String>,
    /// relevance tier (high|medium|low) derived from the
    /// fused score.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relevance: Option<&'static str>,
    /// true when this chunk's `expires_at` is in the past.
    /// Only present when the caller opted into decayed results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decayed: Option<bool>,
    /// stored-row provenance labels carried from the
    /// `knowledge` row so a consuming agent can attribute a hit. `ingest_kind`
    /// is the ingest path (memory/markdown/structured/manual/vault/connector),
    /// `memory_kind` the `node_kind` vocabulary (fact/procedure/...),
    /// `lawful_basis` the Art 5/6 posture when declared, `region` the data
    /// residency stamp. Labels only — no ranking or trust assertion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingest_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lawful_basis: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
}

/// calibrated abstention. When the existing
/// `HeuristicEstimator` reports `Recommendation::ClarifyQuery` (low overlap,
/// low lexical density, weak gap), `/recall` returns `low_confidence` with an
/// empty `hits` slice instead of shipping top-1 garbage. NOT a magic score
// cutoff — abstention is driven by the calibrated multi-signal `Recommendation`,
// which is what `IMPLEMENTATION_ROADMAP_v1.5_to_v4.0_EVIDENCE_GATED.md` §v1.5
// requires ("no fixed universal confidence threshold until held-out benefit
// is demonstrated").
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecallDecision {
    /// Normal retrieval; `hits` carries the ranked results.
    Ok,
    /// Calibrated abstention. `hits` is empty; the consuming agent should
    /// escalate (ask the user) or fall back to web search.
    LowConfidence,
}

#[derive(Debug, Serialize)]
pub struct RecallResponse {
    pub hits: Vec<RecallHit>,
    pub decision: RecallDecision,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// domains of the returned hits. Always present
    /// (empty array when no hits); no longer gated on `provenance`.
    pub domains_searched: Vec<String>,
    /// Per-stage retrieval telemetry, included when `provenance` is requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<crate::search::SearchTelemetry>,
    /// the audit row id for this recall's read event,
    /// when read-event audit is enabled (JWT mode default) AND `?trace=true`
    /// was requested. `/recall/{trace_id}/trace` replays the decision path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<i64>,
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
    /// a strict-posture domain storing a record
    /// with no documented lawful_basis is flagged here (purpose-limitation
    /// evidence). Absent when not flagged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compliance: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct ForgetResponse {
    pub deleted: bool,
}

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
    /// HTTP 422 — the request was well-formed JSON but a
    /// field value is rejected by the contract (e.g. an unknown `source`). Loud
    /// and early, before any DB/embed work.
    pub fn unprocessable(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            inner: ApiError::new(code, message.into()),
        }
    }
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            inner: ApiError::new("unauthorized", message.into()),
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
    /// HTTP 507 — new ingests are refused because the server
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
    /// like [`internal`](Self::internal) but with an
    /// explicit status + code. Used for the `BRAIN_SUGGEST_ENABLED=false` kill
    /// switch, which returns `501 Not Implemented` (not 500) so a configured
    /// client can distinguish "feature disabled" from "server error".
    pub fn internal_with(
        code: &'static str,
        message: impl Into<String>,
        status: StatusCode,
    ) -> Self {
        Self {
            status,
            inner: ApiError::new(code, message.into()),
        }
    }
    /// HTTP 409 — a concurrent modification won the race
    /// (e.g. two reviewers approved/rejected the same proposal simultaneously).
    /// Surfaces a clear conflict code instead of a generic 500 from the
    /// underlying UNIQUE constraint.
    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            inner: ApiError::new("conflict", message.into()),
        }
    }
    /// a 409 with an explicit code + details. Used by
    /// the legal-hold gate (`409 legal_hold_active` listing reasons).
    pub fn conflict_with(code: &'static str, message: impl Into<String>, details: Value) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            inner: ApiError::new(code, message.into()).with_details(details),
        }
    }
}

/// reject a write when the server is over its capacity
/// envelope. Returns `Ok(())` when writes are allowed; `Err(507)` when
/// `CapacityStatus::Exceeded`. Best-effort: if the measurement query fails, the
/// guard fails OPEN (allows the write) — a transient DB error must not turn the
/// brain read-only. Callers: every ingest path (`/add`, `/ingest`,
/// `/ingest/memory`, `/ingest/markdown`). Read routes do NOT call this.
pub fn guard_capacity(state: &crate::AppState) -> Result<(), HandlerError> {
    use brain_server::capacity::{CapacityEnvelope, capacity_target};
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
// AuthZ gate
// ---------------------------------------------------------------------------

/// The single AuthZ gate every handler passes through. Returns Ok(()) if the
/// principal is authorized for (action, team, domain), or a 403 HandlerError.
/// `principal = None` is the v1.1 opaque-token / no-auth back-compat path:
/// superuser, all scopes implicit. This is the lazy-Authorization retrofit —
/// one call per handler instead of refactoring every pool-resolution site.
///
/// Usage: `authorize(principal, Action::Read, &tenant, &domain)?;` before
/// resolving the pool or touching domain data.
pub fn authorize(
    principal: &Option<crate::auth::Principal>,
    action: crate::auth::Action,
    team: &str,
    domain: &str,
) -> Result<(), HandlerError> {
    match principal {
        None => Ok(()), // back-compat: no JWT = superuser
        Some(p) => {
            // The principal's tenant is the team context. If the caller passes
            // a team that doesn't match, it's a cross-tenant attempt.
            let effective_team = if team.is_empty() { &p.tenant } else { team };
            if crate::auth::is_authorized(p, action, effective_team, domain) {
                Ok(())
            } else {
                Err(HandlerError::forbidden(action, effective_team, domain))
            }
        }
    }
}

/// Log-safe principal identifier: a truncated SHA-256 of `sub`, never the raw
/// identifier (GDPR personal-data class; parity with the recall `query_hash`
/// rule). Local logs only — OTLP attributes were already hash-only.
pub fn mask_sub(sub: &str) -> String {
    let h = crate::audit::hash(sub);
    h.chars().take(12).collect()
}

/// Quarantine/decay review posture. `include_flagged`/`include_decayed` are
/// operator-review controls, not caller preferences: only a loopback/opaque
/// principal (`None`) or one authorized for Admin on its own tenant may set
/// them. Everyone else is clamped to `false` at every retrieval entry point
/// (fail-closed — quarantined content never reaches an unprivileged agent).
pub fn review_flags_allowed(principal: &Option<crate::auth::Principal>) -> bool {
    if let Some(p) = principal {
        return crate::auth::is_authorized(p, crate::auth::Action::Admin, &p.tenant, "global");
    }
    true
}

/// read-access predicate for a *target*
/// domain — distinct from `authorize`, which always checks the caller's
/// own-domain posture. A principal may read `domain` iff some scope grants
/// `read` there (`read:team/*` = read-everywhere). `None` principal
/// (loopback/opaque) = superuser, unchanged. Pure predicate — the caller
/// picks the error shape (403 vs 404 for id-probe blindness).
pub fn can_read_domain(principal: &Option<crate::auth::Principal>, domain: &str) -> bool {
    match principal {
        None => true,
        Some(p) => crate::auth::is_authorized(p, crate::auth::Action::Read, &p.tenant, domain),
    }
}

/// the action-gate layer on top of `authorize`. When the
/// principal carries a `roles` claim, the requested `can`-capability must be
/// in at least one resolved role's allowlist or the action is FORBIDDEN (403)
/// — the server enforces even if a client hid/disabled the button. A principal
/// with **no** roles (or an opaque/loopback principal) is untouched: `authorize`
/// remains the only gate (back-compat byte-identical). Deny-by-default: a role
/// whose `can` omits the capability (e.g. an `agent` calling approve) is
/// refused. Call AFTER `authorize` and AFTER the pool is resolved (the role
/// store reads from the pool).
pub fn authorize_role(
    principal: &Option<crate::auth::Principal>,
    pool: &crate::Pool,
    capability: &str,
) -> Result<(), HandlerError> {
    let Some(p) = principal else { return Ok(()) };
    if p.roles.is_empty() {
        return Ok(());
    }
    let conn = pool
        .get()
        .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
    let roles = brain_server::role::resolve(&conn, &p.roles)
        .map_err(|e| HandlerError::internal(format!("role store: {e}")))?;
    if roles.iter().any(|r| r.can(capability)) {
        Ok(())
    } else {
        Err(HandlerError::forbidden(
            crate::auth::Action::Write,
            &p.tenant,
            "global",
        ))
    }
}

/// §5.2: enforce a capability token's verbs × scope at handler
/// entry. `None` (no capability presented — the request authenticated via the
/// middleware's JWT/opaque path) is a no-op. Read ops require `read`, writes
/// `write` (`derive` also grants writes — deriving IS creating new memory),
/// export paths `export`; every other verb (incl. `admin`) is denied. Scope
/// must be `None`/empty (all projects) or `"global"` — the brain's UMP
/// surface is the global project. Call AFTER `authorize` (the capability
/// bearer has no JWT principal, so `authorize` alone would pass as superuser).
pub fn cap_gate(
    cap: &Option<brain_server::ump_integrity::CapabilityToken>,
    verb: &str,
) -> Result<(), HandlerError> {
    let Some(cap) = cap else { return Ok(()) };
    let has = |v: &str| cap.verbs.iter().any(|c| c == v);
    let ok = match verb {
        "read" => has("read"),
        "write" => has("write") || has("derive"),
        "export" => has("export"),
        _ => false,
    };
    if !ok {
        return Err(HandlerError::unauthorized(format!(
            "capability token lacks the '{verb}' verb"
        )));
    }
    if let Some(scope) = cap.scope.as_deref().filter(|s| !s.is_empty())
        && scope != "global"
    {
        return Err(HandlerError::unauthorized(format!(
            "capability token scope '{scope}' is not the global project"
        )));
    }
    Ok(())
}

/// tenant scope for the audit surface. The v1.2 matrix
/// forbids cross-tenant audit reads: a non-superuser principal may only ever
/// see their own tenant's rows. Returns the effective tenant filter to apply
/// (None = no filter, superuser only). Call AFTER `authorize(Admin)`.
pub fn audit_scope(
    principal: &Option<crate::auth::Principal>,
    requested_tenant: &Option<String>,
) -> Result<Option<String>, HandlerError> {
    match (principal, requested_tenant) {
        (Some(p), Some(t)) if t != &p.tenant => Err(HandlerError::forbidden(
            crate::auth::Action::Admin,
            &p.tenant,
            "audit",
        )),
        (Some(p), _) => Ok(Some(p.tenant.clone())),
        (None, t) => Ok(t.clone()),
    }
}

impl HandlerError {
    pub fn forbidden(action: crate::auth::Action, team: &str, domain: &str) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            inner: ApiError::new(
                "forbidden",
                format!("no scope grants {action:?} on {team}/{domain}"),
            )
            .with_details(serde_json::json!({
                "action": format!("{action:?}"),
                "team": team,
                "domain": domain,
            })),
        }
    }
}

// ---------------------------------------------------------------------------
// Domain resolution helpers
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
    // unregistered names (multi-db) map to
    // 404 `domain_unknown` — probe-blind, no file ever created. The legacy
    // `known_domains` detail list survives only for malformed names.
    registry.pool_for(d).map_err(|e| match e {
        crate::domain_registry::DomainRegistryError::Invalid(_) => {
            // No `known_domains` inventory in the error detail —
            // a probe with a malformed header must not receive the full on-disk
            // domain list (the probe-blind posture every other domain error
            // takes; the register listing on GET /domains is the legitimate
            // enumeration surface).
            HandlerError::bad_request(
                "domain_invalid",
                format!("cannot resolve domain '{d}': {e}"),
            )
        }
        other => map_domain_error(other),
    })
}

/// 404 with the `domain_unknown` code — an
/// unregistered domain name. Deliberately probe-blind: a never-registered name
/// is indistinguishable from an empty-but-real domain, and no path ever
/// creates a file for it (registered-only `pool_for`).
pub fn domain_unknown(domain: &str) -> HandlerError {
    HandlerError {
        status: StatusCode::NOT_FOUND,
        inner: ApiError::new(
            "domain_unknown",
            format!(
                "unknown domain '{domain}' — register it with POST /domains \
                 (multi-db mode opens registered domains only)"
            ),
        ),
    }
}

/// EVERY registered domain's pool, not just
/// `state.pool` — the multi-db chain sweep primitive. Shim mode collapses to
/// the single shared global pool (the domain label is a column there, not a
/// file). A domain whose pool cannot open is carried as `None` — callers
/// MUST treat that as not-ok (fail-closed, never silent) rather than skipping
/// the domain.
pub fn domain_pools(
    registry: &crate::domain_registry::DomainRegistry,
    global: &crate::Pool,
) -> Vec<(String, Option<crate::Pool>)> {
    if !registry.is_multi_db() {
        return vec![("global".to_string(), Some(global.clone()))];
    }
    registry
        .known_domains()
        .into_iter()
        .map(|d| {
            let pool = registry
                .pool_for(&d)
                .map_err(|e| tracing::warn!("audit domain sweep: pool for '{d}' unavailable: {e}"))
                .ok();
            (d, pool)
        })
        .collect()
}

/// Verify EVERY registered domain's audit chain — `true` only when
/// every domain opens AND verifies. A broken
/// second-domain chain is reported, not silently absorbed by an ok global
/// pool; a domain whose pool cannot open counts as NOT ok (fail-closed, never
/// silent). This is the blocking half — it takes the already-collected
/// `(domain, pool?)` targets (see [`domain_pools`]) so async callers can
/// gather pools on the runtime thread and move the owned vec into
/// `spawn_blocking` (a `&DomainRegistry` is not `'static`).
pub fn verify_domain_targets(targets: Vec<(String, Option<crate::Pool>)>) -> Vec<(String, bool)> {
    let out: Vec<(String, bool)> = targets
        .into_iter()
        .map(|(d, pool)| {
            let ok = pool
                .and_then(|p| p.get().ok())
                .map(|c| crate::audit::verify_chain(&c))
                .unwrap_or(false);
            (d, ok)
        })
        .collect();
    out
}

/// map a registry resolution error onto the
/// wire shape — the single seam every `pool_for`/`register` call site uses:
/// 404 `domain_unknown` for unregistered names, 400 `domain_invalid` for
/// malformed names, 507 at the registration cap, 500 for open/migration/
/// poisoned failures.
pub fn map_domain_error(e: crate::domain_registry::DomainRegistryError) -> HandlerError {
    match e {
        crate::domain_registry::DomainRegistryError::Unknown(d) => domain_unknown(&d),
        crate::domain_registry::DomainRegistryError::Invalid(d) => {
            HandlerError::bad_request("domain_invalid", format!("invalid domain name: {d}"))
        }
        crate::domain_registry::DomainRegistryError::Capacity(n) => {
            HandlerError::insufficient_storage(format!(
                "domain DB registration cap ({n}) reached — remove unused per-domain \
                 files or raise BRAIN_MAX_DOMAIN_DBS"
            ))
        }
        other => HandlerError::internal(format!("cannot resolve domain pool: {other}")),
    }
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

/// the graph edge-read scope. Entity tables carry
/// no domain column, so in shim mode (one shared pool) the chunk link on a
/// relationship is the domain atom: a JWT principal's graph reads restrict to
/// edges whose `knowledge_id` chunk carries `label`; an unlinked edge is
/// invisible to scoped readers. Loopback/opaque (omniscient, unchanged) and
/// multi-db mode (the pool IS the domain's territory) pass `None`.
pub fn graph_domain_scope(
    principal: &Option<crate::auth::Principal>,
    registry: &crate::domain_registry::DomainRegistry,
    label: &str,
) -> Option<String> {
    if principal.is_some() && !registry.is_multi_db() {
        Some(label.to_string())
    } else {
        None
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

// Hand-rolled checkers for the three patterns used by the validators.
// Replaces the `regex` crate dependency (removed with the annotator module in
// the previous single `is_match`
// ignored its `_pattern` argument and silently rejected spaces in entity
// names (breaking the canonical `vitamin d3` example).
//
// ponytail: each pattern is a tiny character-class + length check; a full
// regex engine is overkill. The shapes are pinned by the test in this module.
pub(crate) fn is_match(pattern: &str, s: &str) -> bool {
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
    // allow one TRACE typed-edge prefix (update: | supersedes: |
    // contradicts: | causes:) before the base relation. The prefix is lowercase
    // letters followed by a single `:`; the base is snake_case as before.
    let s = s.trim();
    if s.is_empty() || s.len() > 64 {
        return false;
    }
    let base = if let Some((prefix, rest)) = s.split_once(':') {
        if prefix.is_empty() || rest.is_empty() {
            return false;
        }
        if !prefix.chars().all(|c| c.is_ascii_lowercase()) {
            return false;
        }
        rest
    } else {
        s
    };
    base.chars()
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

    // §5.2: the capability-token gate. Verbs: read ops need
    // `read`, writes `write` (or `derive`), export paths `export`; admin is
    // never grantable. Scope must be None/empty or "global". No capability
    // (None — JWT/opaque-authenticated request) is always a pass.
    #[test]
    fn cap_gate_enforces_verbs_scope_and_never_admin() {
        use brain_server::ump_integrity::CapabilityToken;
        let cap = |verbs: &[&str], scope: Option<&str>| CapabilityToken {
            alg: "EdDSA".into(),
            iss: "did:key:z6MkTest".into(),
            verbs: verbs.iter().map(|s| s.to_string()).collect(),
            scope: scope.map(|s| s.to_string()),
            exp: u64::MAX,
            jti: None,
        };
        // None = no capability presented: no-op.
        assert!(cap_gate(&None, "read").is_ok());

        let read = Some(cap(&["read"], None));
        assert!(cap_gate(&read, "read").is_ok());
        assert!(cap_gate(&read, "write").is_err());
        assert!(cap_gate(&read, "export").is_err());
        assert!(cap_gate(&read, "admin").is_err());

        let write = Some(cap(&["write"], None));
        assert!(cap_gate(&write, "write").is_ok());
        assert!(cap_gate(&write, "read").is_err());

        // derive grants writes (deriving IS creating new memory).
        let derive = Some(cap(&["derive"], None));
        assert!(cap_gate(&derive, "write").is_ok());
        assert!(cap_gate(&derive, "read").is_err());

        let export = Some(cap(&["export"], None));
        assert!(cap_gate(&export, "export").is_ok());
        assert!(cap_gate(&export, "read").is_err());

        // Scope: "global" passes, any other project is denied on this surface.
        let scoped_global = Some(cap(&["read"], Some("global")));
        assert!(cap_gate(&scoped_global, "read").is_ok());
        let scoped_other = Some(cap(&["read"], Some("acme")));
        assert!(cap_gate(&scoped_other, "read").is_err());
        let scoped_empty = Some(cap(&["read"], Some("")));
        assert!(cap_gate(&scoped_empty, "read").is_ok());

        // Admin cannot be granted by any verb set.
        let adminish = Some(cap(&["read", "write", "derive", "export"], None));
        assert!(cap_gate(&adminish, "admin").is_err());
    }

    // idempotency property — normalizing a domain twice
    // yields the same result as normalizing once.
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn proptest_normalize_domain_is_idempotent(raw in "[a-z0-9_-]{1,63}") {
            let once = normalize_domain(&raw);
            let twice = normalize_domain(once.as_deref().unwrap_or(""));
            // Idempotent: both must succeed with the same value, or both fail.
            if once.is_ok() && twice.is_ok() {
                prop_assert_eq!(once.unwrap(), twice.unwrap());
            }
        }
    }
}
