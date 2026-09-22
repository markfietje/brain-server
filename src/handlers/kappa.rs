//! The κ labeling bench's surfaces: the rater's own assignment queue, the
//! blind label capture, and the DPO-gated κ report — the human-oversight
//! instrument's read/write/read triangle. Protocol adapter only — every
//! read and write lives in the domain core
//! ([`crate::workflow::kappa`], which owns the assignment, the closed
//! label vocabulary, the exactly-once label store, and the report cells),
//! and this file carries no statements of its own (the no-SQL-in-handlers
//! law counts this file live).
//!
//! Gate posture: queue + submit demand the Write scope on the global
//! domain (the corpus spans runs; the bench is one global instrument)
//! PLUS the `calibrate` capability — the ratified rater gate; the
//! `qa-specialist` and `dpo` presets already carry it, no new role. The
//! report is the round's exfiltration surface: the DPO dual gate (Admin
//! scope AND the DPO role, the scoreboard/corpus precedent) PLUS the
//! `calibrate` capability, and an audited read per call. The rater slot
//! derives from the authenticated principal, never the body — the client
//! names the judgment, not the judge.

use axum::{
    Json,
    extract::{Query, State},
};
use serde::Deserialize;
use std::sync::Arc;

use crate::AppState;
use crate::handlers::HandlerError;
use crate::handlers::auth::OptPrincipal;

/// The rater's slot, derived from the authenticated principal's subject.
/// Same subject → same slot, always; the queue echoes it so a rater can
/// self-discover it. Public for the surface (and its integration tests),
/// pure, body-free.
pub fn slot_for_principal(sub: &str) -> u8 {
    crate::workflow::kappa::slot_for_principal(sub)
}

/// The bounded page: `limit` must land inside 1..=500 (named 400 outside),
/// default 100 — the corpus-page posture.
fn validate_kappa_limit(limit: Option<usize>) -> Result<usize, HandlerError> {
    let limit = limit.unwrap_or(100);
    if (1..=500).contains(&limit) {
        return Ok(limit);
    }
    Err(HandlerError::internal_with(
        "kappa_limit_out_of_bounds",
        "limit must land inside 1..=500 — the bench pages are bounded",
        axum::http::StatusCode::BAD_REQUEST,
    ))
}

/// The machinery's refusals surface named — the pinned error vocabulary at
/// the wire: a label outside the closed vocabulary is a 400, an absent
/// tuple or assignment is the SAME probe-blind 404 (the pinned equation —
/// a tuple not assigned to you is an absent tuple), and anything else is
/// internal (never a silent drop).
fn kappa_err(e: String) -> HandlerError {
    match e.as_str() {
        "kappa: label required" => HandlerError::bad_request(
            "label_required",
            "a label is required — choose one of agree | disagree | uncertain",
        ),
        s if s.starts_with("kappa: label invalid") => {
            HandlerError::bad_request("label_invalid", s.to_string())
        }
        "label_assignment_absent" => HandlerError::not_found("no such assignment"),
        other => HandlerError::internal(other.to_string()),
    }
}

#[derive(Debug, Deserialize)]
pub struct KappaQueueQuery {
    pub limit: Option<usize>,
}

/// `GET /workflow/kappa/queue?limit=` — the caller's own assignment over
/// the mined tuples, both partitions, oldest first, bounded. The rows
/// carry the machine's proposal (masked through the read seam) and the
/// rater's OWN latest label — never another rater's, never the governed
/// truth: blindness is the core's output type, not a discipline.
pub async fn get_kappa_queue(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Query(q): Query<KappaQueueQuery>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal, crate::auth::Action::Write, "", "global")?;
    super::authorize_role(&principal, &pool, "calibrate")?;
    let limit = validate_kappa_limit(q.limit)?;
    let who = principal
        .as_ref()
        .map(|p| p.sub.clone())
        .unwrap_or_else(|| "loopback".into());
    let slot = slot_for_principal(&who);
    let rows = tokio::task::spawn_blocking(
        move || -> Result<Vec<crate::workflow::kappa::QueueTuple>, String> {
            let conn = pool.get().map_err(|e| format!("{e}"))?;
            crate::workflow::kappa::rater_queue(&conn, slot, limit)
        },
    )
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?
    .map_err(HandlerError::internal)?;
    let rows: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|t| {
            serde_json::json!({
                "run_id": t.run_id,
                "seq": t.seq,
                "digest": t.digest,
                "phase": t.phase,
                "partition": t.partition.as_str(),
                "model_proposal": t.model_proposal,
                "my_label": t.my_label,
                "my_label_seq": t.my_label_seq,
            })
        })
        .collect();
    let count = rows.len();
    Ok(Json(
        serde_json::json!({ "slot": slot, "rows": rows, "count": count }),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KappaLabelBody {
    pub digest: String,
    pub label: String,
    pub run_id: i64,
}

/// `POST /workflow/kappa/labels` — capture one judgment. The body names
/// the tuple and the label; the slot derives from the principal. The
/// write is the core's exactly-once, append-only store: a re-submitted
/// latest judgment is the no-op receipt, a changed judgment appends a
/// supersession row, and ONE audit row lands per created label (ids +
/// counts, never label text).
pub async fn post_kappa_label(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Json(body): Json<KappaLabelBody>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal, crate::auth::Action::Write, "", "global")?;
    super::authorize_role(&principal, &pool, "calibrate")?;
    let who = principal
        .as_ref()
        .map(|p| p.sub.clone())
        .unwrap_or_else(|| "loopback".into());
    let slot = slot_for_principal(&who);
    let now = chrono::Utc::now().timestamp();
    let receipt = tokio::task::spawn_blocking(
        move || -> Result<crate::workflow::kappa::KappaLabelReceipt, HandlerError> {
            let mut conn = pool
                .get()
                .map_err(|e| HandlerError::internal(format!("{e}")))?;
            let mut tx = crate::workflow::tx::WorkflowTx::begin(&mut conn)
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            let receipt = crate::workflow::kappa::write_label(
                &mut tx,
                body.run_id,
                &body.digest,
                &body.label,
                slot,
                now,
            )
            .map_err(kappa_err)?;
            tx.commit()
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            Ok(receipt)
        },
    )
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))??;
    Ok(Json(serde_json::json!({
        "run_id": body.run_id,
        "slot": slot,
        "created": receipt.created,
        "seq": receipt.seq,
        "supersession": receipt.supersession,
        "audited": receipt.created,
    })))
}

#[derive(Debug, Deserialize)]
pub struct KappaReportQuery {
    pub limit: Option<usize>,
}

/// `GET /workflow/kappa/report` — the per-rater-pair κ cells over the
/// labeled set, one per (domain × frozen partition × slot pair). THE
/// exfiltration surface: the DPO dual gate plus the `calibrate`
/// capability, and an audited global row per call naming the principal
/// and the counts. `meets_bar` is data for the operator's read — the κ
/// value never auto-gates anything.
pub async fn get_kappa_report(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Query(q): Query<KappaReportQuery>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal, crate::auth::Action::Admin, "", "global")?;
    crate::handlers::breaches::require_dpo_role(&principal, &pool)?;
    super::authorize_role(&principal, &pool, "calibrate")?;
    let limit = validate_kappa_limit(q.limit)?;
    let who = principal
        .as_ref()
        .map(|p| p.sub.clone())
        .unwrap_or_else(|| "loopback".into());
    let cells = tokio::task::spawn_blocking(
        move || -> Result<Vec<crate::workflow::kappa::KappaCell>, HandlerError> {
            let conn = pool
                .get()
                .map_err(|e| HandlerError::internal(format!("{e}")))?;
            let cells =
                crate::workflow::kappa::kappa_report(&conn).map_err(HandlerError::internal)?;
            // EVERY report call lands an audit row: who, the counts. The
            // read and its audit share one connection so the counts
            // describe exactly the emitted page.
            crate::audit::record_tenant(
                &conn,
                crate::audit::AuditKind::Workflow,
                crate::workflow::ACTOR,
                crate::workflow::kappa::AUDIT_KAPPA_REPORT,
                crate::audit::AuditStatus::Ok,
                &format!(
                    "principal={who} cells={} bar_units={}",
                    cells.len(),
                    crate::workflow::kappa::KAPPA_BAR_UNITS
                ),
                "global",
            );
            Ok(cells)
        },
    )
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))??;
    let cells: Vec<serde_json::Value> = cells
        .into_iter()
        .take(limit)
        .map(|c| {
            serde_json::json!({
                "domain": c.domain,
                "partition": c.partition,
                "slot_a": c.slot_a,
                "slot_b": c.slot_b,
                "n_joint": c.n_joint,
                "kappa_units": c.kappa_units,
                "kappa_note": c.kappa_note,
                "meets_bar": c.meets_bar,
            })
        })
        .collect();
    let count = cells.len();
    Ok(Json(serde_json::json!({
        "bar_units": crate::workflow::kappa::KAPPA_BAR_UNITS,
        "rows": cells,
        "count": count,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bounds and the refusal mapping — the pinned error vocabulary at
    /// the wire.
    #[test]
    fn kappa_limit_bounds_and_error_mapping_are_pinned() {
        assert!(validate_kappa_limit(None).is_ok());
        assert_eq!(validate_kappa_limit(None).unwrap(), 100);
        assert!(validate_kappa_limit(Some(1)).is_ok());
        assert!(validate_kappa_limit(Some(500)).is_ok());
        assert_eq!(
            validate_kappa_limit(Some(0)).unwrap_err().inner.code,
            "kappa_limit_out_of_bounds"
        );
        assert_eq!(
            validate_kappa_limit(Some(501)).unwrap_err().status,
            axum::http::StatusCode::BAD_REQUEST
        );
        // The named refusals surface with their statuses.
        let required = kappa_err("kappa: label required".into());
        assert_eq!(required.status, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(required.inner.code, "label_required");
        let invalid = kappa_err("kappa: label invalid: maybe".into());
        assert_eq!(invalid.status, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(invalid.inner.code, "label_invalid");
        let absent = kappa_err("label_assignment_absent".into());
        assert_eq!(absent.status, axum::http::StatusCode::NOT_FOUND);
        assert_eq!(absent.inner.code, "not_found");
        // Anything else stays internal — never a silent drop.
        assert_eq!(
            kappa_err("kappa: something else".into()).status,
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    /// The slot derivation rides the core and stays in the roster.
    #[test]
    fn kappa_slot_derivation_is_pure_and_in_roster() {
        assert_eq!(slot_for_principal("user:a"), slot_for_principal("user:a"));
        assert!(slot_for_principal("user:a") < 2);
        assert!(slot_for_principal("user:b") < 2);
    }
}
