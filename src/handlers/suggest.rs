//! v1.9.0 "Suggest" — opt-in, non-interrupting anticipation (light cut).
//!
//! Scoped to the evidence-gated v1.9 surface in
//! `IMPLEMENTATION_ROADMAP_v1.5_to_v4.0_EVIDENCE_GATED.md` §v1.9. NOT the
//! broader Anticipate plan (sessions/SSE/decay/personalization) — the roadmap
//! forbids all of those ("unsolicited push, ranking decay, hidden
//! personalization, or SSE by default"). What ships here:
//!
//!   - `POST /suggest`         — opt-in pull. Caller supplies explicit context;
//!     server returns related-but-not-already-surfaced chunks, each tagged
//!     `reason: "anticipated"`.
//!   - `POST /suggest/feedback` — Mem0-style accept/dismiss per surfaced chunk.
//!   - `GET  /suggest/metrics`  — the false-positive rate (roadmap exit criterion).
//!
//! Research basis (Context7, verified 2026-08-02):
//!   - Mem0 (`/mem0ai/mem0`): the `feedback` API shape (`memory_id`, `feedback`,
//!     `feedback_reason?`) and "feedback analytics" track accept vs dismiss —
//!     this is the false-positive metric. Session identity is client-owned
//!     (`run_id`); the server never auto-tracks sessions.
//!   - Letta/MemGPT (`/letta-ai/letta`): anticipatory memory is *reviewable* —
//!     nothing is silently injected. `/suggest` returns labelled candidates
//!     the caller explicitly asked for; the agent chooses to use them.
//!
//! No new state machine, no background work, no push. The session is a
//! caller-supplied opaque string (Mem0 `run_id` pattern); the server does no
//! session-boundary detection, no timeout, no embedding mean.

use axum::extract::State;
use axum::response::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::audit;
use crate::handlers::auth::OptPrincipal;
use crate::handlers::{normalize_domain, HandlerError, MAX_QUERY};
use crate::{config, AppState};

// ─────────────────────────────────────────────────────────────────────────
// POST /suggest
// ─────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SuggestRequest {
    /// What the caller is working on right now. The server embeds this and
    /// returns related chunks the caller did NOT directly ask for. Required,
    /// non-empty, bounded by [`MAX_QUERY`] (same limit as `/recall`).
    pub context: String,
    /// Chunk ids already surfaced to the caller (e.g. the `/recall` hits from
    /// this turn). Excluded from the result so suggestions are supplementary,
    /// not redundant. Capped at [`config::MAX_SUGGEST_EXCLUDE`].
    #[serde(default)]
    pub exclude: Vec<i64>,
    /// Number of suggestions to return. 1..=[`config::MAX_SUGGEST_K`]; default
    /// [`config::DEFAULT_SUGGEST_K`].
    #[serde(default = "default_k")]
    pub k: u32,
    /// Optional domain scoping. Validated for shape; resolved to a pool.
    #[serde(default)]
    pub domain: Option<String>,
    /// Caller-supplied opaque session label. NOT auto-tracked — the server
    /// stores it verbatim on feedback rows for per-session metric breakdown.
    /// Mem0 `run_id` pattern: the client owns session identity.
    #[serde(default)]
    pub session: Option<String>,
}

fn default_k() -> u32 {
    config::DEFAULT_SUGGEST_K
}

/// One anticipated chunk. `provenance.reason = "anticipated"` is the contract
/// marker that tells the consuming agent "this was not directly retrieved —
/// it's a suggestion; you may ignore it."
#[derive(Debug, Serialize)]
pub struct SuggestionHit {
    pub id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub content: String,
    /// Cosine-derived similarity in `[0.0, 1.0]` (same scoring as `/recall`).
    pub score: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    pub provenance: SuggestionProvenance,
}

#[derive(Debug, Serialize)]
pub struct SuggestionProvenance {
    /// Always `"anticipated"` for `/suggest` hits. Stable machine-readable
    /// marker so the consuming agent can choose to ignore suggestions.
    pub reason: &'static str,
    /// The caller-supplied session label, echoed back for client-side bookkeeping.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SuggestResponse {
    pub suggestions: Vec<SuggestionHit>,
    pub telemetry: SuggestTelemetry,
}

#[derive(Debug, Serialize)]
pub struct SuggestTelemetry {
    pub k: u32,
    pub excluded: usize,
    pub retrieved: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
}

/// `POST /suggest` — opt-in anticipation. See module docs.
pub async fn suggest(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SuggestRequest>,
) -> Result<Json<SuggestResponse>, HandlerError> {
    if !config::brain_suggest_enabled() {
        // Roadmap kill switch: "otherwise the feature is removed." Returns 501
        // (not 404) so a configured client can distinguish "disabled" from
        // "wrong URL" without parsing the error body.
        return Err(HandlerError::internal_with(
            "suggest_disabled",
            "BRAIN_SUGGEST_ENABLED=false; the /suggest surface is disabled",
            axum::http::StatusCode::NOT_IMPLEMENTED,
        ));
    }

    let (context, k, exclude_count) = validate_suggest(&req)?;
    let domain = match &req.domain {
        Some(d) => Some(normalize_domain(d)?),
        None => None,
    };

    // Same prompt-injection defense as /recall: never embed adversarial input.
    if crate::contains_suspicious_pattern(&context) {
        return Err(HandlerError::bad_request(
            "context_rejected",
            "context matches a blocked prompt-injection pattern",
        ));
    }

    let pool = crate::handlers::resolve_domain_pool(&state.registry, domain.as_deref())
        .unwrap_or(state.pool.clone());
    let model = Arc::clone(&state.model);
    let exclude = req.exclude.clone();
    let session = req
        .session
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let domain_label = domain.clone();
    let k_for_task = k;

    let suggestions =
        tokio::task::spawn_blocking(move || -> Result<Vec<SuggestionHit>, HandlerError> {
            let conn = pool
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            // Embed once (same path as /recall).
            let qvec = model
                .encode(std::slice::from_ref(&context))
                .into_iter()
                .next()
                .unwrap_or_default();
            // Over-fetch by exclude.len() so filtering excluded ids doesn't starve
            // the result, then truncate to k. Capped at MAX_K to bound the query.
            let overfetch = k_for_task
                .saturating_add(exclude.len() as u32)
                .min(config::MAX_K as u32) as usize;
            let filters = crate::SearchFilters {
                domain: domain_label.clone(),
                ..Default::default()
            };
            // vec0_knn honors the v1.6.0 `valid_to IS NULL` default (current facts
            // only) and the v0.9.7 flagged-row exclusion — superseded + quarantined
            // chunks are never suggested.
            let mut results = crate::search::vec0_knn(&conn, &qvec, overfetch, &filters)
                .map_err(|e| HandlerError::internal(format!("vec0_knn failed: {e}")))?;
            // Drop excluded ids (linear scan; bounded by MAX_K+MAX_SUGGEST_EXCLUDE).
            if !exclude.is_empty() {
                results.retain(|r| !exclude.contains(&r.id));
            }
            results.truncate(k_for_task as usize);
            Ok(results
                .into_iter()
                .map(|r| SuggestionHit {
                    id: r.id,
                    title: r.title,
                    content: r.content,
                    score: r.score,
                    domain: domain_label.clone(),
                    provenance: SuggestionProvenance {
                        reason: "anticipated",
                        session: session.clone(),
                    },
                })
                .collect())
        })
        .await
        .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    Ok(Json(SuggestResponse {
        telemetry: SuggestTelemetry {
            k,
            excluded: exclude_count,
            retrieved: suggestions.len(),
            domain,
        },
        suggestions,
    }))
}

/// Validate a parsed `/suggest` request against the contract bounds. Returns
/// `(trimmed_context, k, exclude_len)` on success. Pure so the bounds can be
/// unit-tested without AppState or a model.
pub(super) fn validate_suggest(req: &SuggestRequest) -> Result<(String, u32, usize), HandlerError> {
    let context = req.context.trim().to_string();
    if context.is_empty() {
        return Err(HandlerError::bad_request(
            "context_empty",
            "context must not be empty",
        ));
    }
    if context.chars().count() > MAX_QUERY {
        return Err(HandlerError::bad_request(
            "context_too_long",
            format!("context exceeds {MAX_QUERY} chars"),
        ));
    }
    if !(1..=config::MAX_SUGGEST_K).contains(&req.k) {
        return Err(HandlerError::bad_request_with(
            "k_out_of_range",
            format!("k must be 1..={}", config::MAX_SUGGEST_K),
            serde_json::json!({ "min": 1, "max": config::MAX_SUGGEST_K }),
        ));
    }
    if req.exclude.len() > config::MAX_SUGGEST_EXCLUDE {
        return Err(HandlerError::bad_request(
            "exclude_too_long",
            format!("exclude exceeds {} ids", config::MAX_SUGGEST_EXCLUDE),
        ));
    }
    Ok((context, req.k, req.exclude.len()))
}

// ─────────────────────────────────────────────────────────────────────────
// POST /suggest/feedback
// ─────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct FeedbackRequest {
    pub chunk_id: i64,
    /// `"accept"` (useful) or `"dismiss"` (not useful — counts as a false
    /// positive in the metric). Anything else is rejected.
    pub feedback: String,
    /// Optional free-text reason. Stored only as a hash (never raw), mirroring
    /// the audit log's treatment of sensitive identifiers.
    #[serde(default)]
    pub reason: Option<String>,
    /// Caller-supplied session label. Labels the feedback row so the metric
    /// can be broken down per-session.
    #[serde(default)]
    pub session: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FeedbackResponse {
    pub status: &'static str,
}

/// `POST /suggest/feedback` — record an accept/dismiss outcome. The feedback
/// table IS the audit surface (append-only, hash-of-reason, tenant-scoped); no
/// duplicate `audit_events` row is written.
pub async fn feedback(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Json(req): Json<FeedbackRequest>,
) -> Result<Json<FeedbackResponse>, HandlerError> {
    if !config::brain_suggest_enabled() {
        return Err(HandlerError::internal_with(
            "suggest_disabled",
            "BRAIN_SUGGEST_ENABLED=false; the /suggest surface is disabled",
            axum::http::StatusCode::NOT_IMPLEMENTED,
        ));
    }
    let outcome = FeedbackOutcome::from_str(&req.feedback).ok_or_else(|| {
        HandlerError::bad_request_with(
            "feedback_invalid",
            "feedback must be 'accept' or 'dismiss'",
            serde_json::json!({ "allowed": ["accept", "dismiss"] }),
        )
    })?;
    // Bounded reason; hash before storage (never persist raw free text).
    let reason_hash = req
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(audit::hash);
    let session = req
        .session
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    // tenant: from the JWT principal if present, else the audit default.
    // ponytail: per-tenant isolation without a multi-DB cutover — the label
    // scopes the metric query; cross-tenant reads are blocked by AuthZ on the
    // route, not by row-level SQL (v2.0 adds row-level enforcement).
    let tenant = principal
        .0
        .as_ref()
        .map(|p| p.tenant.clone())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| audit::DEFAULT_TENANT.to_string());
    let chunk_id = req.chunk_id;
    let outcome_str = outcome.as_str();

    let pool = state.pool.clone();
    tokio::task::spawn_blocking(move || -> Result<(), HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        // chunk_id validity: refuse feedback on a non-existent chunk so the
        // metric isn't poisoned by typos. (A deleted chunk's id still counts —
        // the feedback was real when given; only never-existed ids are rejected.)
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM knowledge WHERE id = ?1)",
                rusqlite::params![chunk_id],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n != 0)
            .unwrap_or(false);
        if !exists {
            return Err(HandlerError::not_found(format!(
                "no chunk with id {chunk_id}"
            )));
        }
        conn.execute(
            "INSERT INTO suggest_feedback(chunk_id, feedback, reason_hash, ts, session, tenant_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(chunk_id, COALESCE(session, '')) DO UPDATE SET
               feedback = excluded.feedback,
               reason_hash = excluded.reason_hash,
               ts = excluded.ts",
            rusqlite::params![chunk_id, outcome_str, reason_hash, ts, session, tenant],
        )
        .map_err(|e| HandlerError::internal(format!("feedback insert failed: {e}")))?;
        Ok(())
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    Ok(Json(FeedbackResponse { status: "recorded" }))
}

/// The two outcomes that matter for the false-positive metric. Mem0's
/// `VERY_NEGATIVE` is collapsed — it adds no signal for a suggestion surface
/// (a dismiss is a dismiss). A future "report-as-harmful" path is v2.x.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedbackOutcome {
    Accept,
    Dismiss,
}

impl FeedbackOutcome {
    /// Parse `"accept"` / `"dismiss"` (case-insensitive). `None` for anything
    /// else — the caller turns that into a 400. Pure so the parsing contract
    /// can be unit-tested without a handler.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "accept" | "accepted" | "positive" | "useful" => Some(FeedbackOutcome::Accept),
            "dismiss" | "dismissed" | "negative" | "not_useful" => Some(FeedbackOutcome::Dismiss),
            _ => None,
        }
    }
    pub const fn as_str(self) -> &'static str {
        match self {
            FeedbackOutcome::Accept => "accept",
            FeedbackOutcome::Dismiss => "dismiss",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// GET /suggest/metrics
// ─────────────────────────────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
pub struct MetricsParams {
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default)]
    pub since: Option<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct MetricsResponse {
    pub total: u64,
    pub accepts: u64,
    pub dismisses: u64,
    /// `accepts / total` in `[0.0, 1.0]`. `0.0` when total == 0.
    pub accept_rate: f32,
    /// `dismisses / total` — the roadmap's false-positive rate. `0.0` when
    /// total == 0.
    pub false_positive_rate: f32,
    pub window: Option<MetricsWindow>,
}

#[derive(Debug, Serialize)]
pub struct MetricsWindow {
    pub since: Option<String>,
    pub session: Option<String>,
}

/// `GET /suggest/metrics?session=&since=` — the false-positive rate over the
/// feedback ledger. This IS the roadmap v1.9 exit criterion, made queryable.
pub async fn metrics(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    axum::extract::Query(params): axum::extract::Query<MetricsParams>,
) -> Result<Json<MetricsResponse>, HandlerError> {
    if !config::brain_suggest_enabled() {
        return Err(HandlerError::internal_with(
            "suggest_disabled",
            "BRAIN_SUGGEST_ENABLED=false; the /suggest surface is disabled",
            axum::http::StatusCode::NOT_IMPLEMENTED,
        ));
    }
    let session = params
        .session
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let since_norm = match params
        .since
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(s) => Some(
            crate::search::normalize_since(s)
                .map_err(|e| HandlerError::bad_request("since_invalid", e.to_string()))?,
        ),
        None => None,
    };
    let tenant = principal
        .0
        .as_ref()
        .map(|p| p.tenant.clone())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| audit::DEFAULT_TENANT.to_string());

    let pool = state.pool.clone();
    let session_for_task = session.clone();
    let since_for_task = since_norm.clone();
    let m = tokio::task::spawn_blocking(move || -> Result<MetricsResponse, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        // One grouped query; the (tenant_id, ts) index keeps this cheap.
        let mut sql = String::from(
            "SELECT feedback, COUNT(*) FROM suggest_feedback \
             WHERE tenant_id = ?1",
        );
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(tenant)];
        if let Some(since) = &since_for_task {
            sql.push_str(" AND ts >= ?");
            params_vec.push(Box::new(since.clone()));
        }
        if let Some(sess) = &session_for_task {
            sql.push_str(" AND session = ?");
            params_vec.push(Box::new(sess.clone()));
        }
        sql.push_str(" GROUP BY feedback");
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| HandlerError::internal(format!("metrics prepare failed: {e}")))?;
        let param_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
        let mut accepts = 0u64;
        let mut dismisses = 0u64;
        let rows = stmt
            .query_map(param_refs.as_slice(), |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })
            .map_err(|e| HandlerError::internal(format!("metrics query failed: {e}")))?;
        for row in rows.flatten() {
            match row.0.as_str() {
                "accept" => accepts = row.1.max(0) as u64,
                "dismiss" => dismisses = row.1.max(0) as u64,
                _ => {} // unknown outcome — ignore (forward-compat with future kinds)
            }
        }
        let total = accepts + dismisses;
        Ok(compute_metrics(
            total,
            accepts,
            dismisses,
            since_for_task,
            session_for_task,
        ))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    Ok(Json(m))
}

/// Pure metric computation. Kept separate so the false-positive math can be
/// unit-tested without a database. `total == 0` returns zero rates (not NaN).
pub(super) fn compute_metrics(
    total: u64,
    accepts: u64,
    dismisses: u64,
    since: Option<String>,
    session: Option<String>,
) -> MetricsResponse {
    let (accept_rate, fpr) = if total == 0 {
        (0.0, 0.0)
    } else {
        (
            accepts as f32 / total as f32,
            dismisses as f32 / total as f32,
        )
    };
    MetricsResponse {
        total,
        accepts,
        dismisses,
        accept_rate,
        false_positive_rate: fpr,
        window: (since.is_some() || session.is_some()).then_some(MetricsWindow { since, session }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── validate_suggest bounds ──────────────────────────────────────────

    fn req(context: &str, k: u32, exclude: &[i64]) -> SuggestRequest {
        SuggestRequest {
            context: context.to_string(),
            k,
            exclude: exclude.to_vec(),
            domain: None,
            session: None,
        }
    }

    #[test]
    fn validate_suggest_rejects_empty_context() {
        let r = req("   ", 5, &[]);
        assert!(validate_suggest(&r).is_err());
    }

    #[test]
    fn validate_suggest_rejects_oversized_context() {
        let big = "x".repeat(MAX_QUERY + 1);
        let r = req(&big, 5, &[]);
        assert!(validate_suggest(&r).is_err());
    }

    #[test]
    fn validate_suggest_rejects_k_out_of_range() {
        assert!(validate_suggest(&req("ok", 0, &[])).is_err());
        assert!(validate_suggest(&req("ok", config::MAX_SUGGEST_K + 1, &[])).is_err());
    }

    #[test]
    fn validate_suggest_rejects_oversized_exclude() {
        let excl: Vec<i64> = (0..(config::MAX_SUGGEST_EXCLUDE + 1) as i64).collect();
        assert!(validate_suggest(&req("ok", 5, &excl)).is_err());
    }

    #[test]
    fn validate_suggest_accepts_valid_request_and_trims_context() {
        let (ctx, k, n) = validate_suggest(&req("  hello  ", 7, &[1, 2])).expect("ok");
        assert_eq!(ctx, "hello");
        assert_eq!(k, 7);
        assert_eq!(n, 2);
    }

    // ── exclusion + truncation (the core algorithm) ──────────────────────

    #[test]
    fn exclude_then_truncate_filters_excluded_ids_and_caps_at_k() {
        // Simulate vec0_knn output: 10 candidates, ids 0..9, uniform score.
        let mut results: Vec<crate::SearchResult> = (0..10)
            .map(|i| crate::SearchResult::raw(i, 0.9 - i as f32 * 0.01, None, format!("c{i}")))
            .collect();
        let exclude = [0i64, 1, 2];
        let k = 4usize;
        // Same retain+truncate logic the handler uses.
        results.retain(|r| !exclude.contains(&r.id));
        results.truncate(k);
        // 10 - 3 excluded = 7 candidates; truncate to k=4 → 4 suggestions,
        // none of which are in the exclude set.
        assert_eq!(results.len(), 4);
        assert!(results.iter().all(|r| !exclude.contains(&r.id)));
        // Order preserved (best first): ids 3,4,5,6.
        assert_eq!(
            results.iter().map(|r| r.id).collect::<Vec<_>>(),
            vec![3, 4, 5, 6]
        );
    }

    // ── FeedbackOutcome parsing ──────────────────────────────────────────

    #[test]
    fn feedback_outcome_accepts_canonical_and_aliases() {
        assert_eq!(
            FeedbackOutcome::from_str("accept"),
            Some(FeedbackOutcome::Accept)
        );
        assert_eq!(
            FeedbackOutcome::from_str("ACCEPT"),
            Some(FeedbackOutcome::Accept)
        );
        assert_eq!(
            FeedbackOutcome::from_str("useful"),
            Some(FeedbackOutcome::Accept)
        );
        assert_eq!(
            FeedbackOutcome::from_str("positive"),
            Some(FeedbackOutcome::Accept)
        );
        assert_eq!(
            FeedbackOutcome::from_str("dismiss"),
            Some(FeedbackOutcome::Dismiss)
        );
        assert_eq!(
            FeedbackOutcome::from_str("Dismissed"),
            Some(FeedbackOutcome::Dismiss)
        );
        assert_eq!(
            FeedbackOutcome::from_str("negative"),
            Some(FeedbackOutcome::Dismiss)
        );
    }

    #[test]
    fn feedback_outcome_rejects_junk() {
        assert_eq!(FeedbackOutcome::from_str(""), None);
        assert_eq!(FeedbackOutcome::from_str("maybe"), None);
        assert_eq!(FeedbackOutcome::from_str("very_negative"), None); // Mem0's third state — intentionally unsupported
    }

    #[test]
    fn feedback_outcome_round_trips_through_as_str() {
        assert_eq!(FeedbackOutcome::Accept.as_str(), "accept");
        assert_eq!(FeedbackOutcome::Dismiss.as_str(), "dismiss");
    }

    // ── metrics math (the exit criterion) ────────────────────────────────

    #[test]
    fn metrics_compute_false_positive_rate() {
        // 70 accepts, 30 dismisses → 30% false-positive rate.
        let m = compute_metrics(100, 70, 30, None, None);
        assert_eq!(m.total, 100);
        assert_eq!(m.accepts, 70);
        assert_eq!(m.dismisses, 30);
        assert!((m.accept_rate - 0.70).abs() < 1e-6);
        assert!((m.false_positive_rate - 0.30).abs() < 1e-6);
        assert!(m.window.is_none()); // no filters → no window reported
    }

    #[test]
    fn metrics_zero_total_yields_zero_rates_not_nan() {
        let m = compute_metrics(0, 0, 0, None, None);
        assert_eq!(m.total, 0);
        assert!(!m.accept_rate.is_nan());
        assert!(!m.false_positive_rate.is_nan());
        assert_eq!(m.accept_rate, 0.0);
        assert_eq!(m.false_positive_rate, 0.0);
    }

    #[test]
    fn metrics_includes_window_when_filtered() {
        let m = compute_metrics(5, 5, 0, Some("2026-08-01".into()), Some("s1".into()));
        assert!(m.window.is_some());
        let w = m.window.unwrap();
        assert_eq!(w.since.as_deref(), Some("2026-08-01"));
        assert_eq!(w.session.as_deref(), Some("s1"));
    }
}
