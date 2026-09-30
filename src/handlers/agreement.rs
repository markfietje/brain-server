//! The agreement-labelling path's surfaces: the reviewer's own queue of real
//! run rows, the verdict capture bound to one, and the agreement report.
//! Protocol adapter only — every read and write lives in the domain core
//! ([`crate::workflow::agreement`], which owns the machine verdict, the closed
//! verdict vocabulary, the reviewer requirement, the exactly-once label store,
//! and both report halves), and this file carries no statements of its own (the
//! no-SQL-in-handlers law counts this file live).
//!
//! Gate posture, carried from the bench's precedent: the queue and the label
//! capture demand the Write scope on the global domain PLUS the `calibrate`
//! capability — the ratified reviewer gate; the `qa-specialist` and `dpo`
//! presets already carry it, no new role. The report is the round's
//! exfiltration surface: the DPO dual gate (Admin scope AND the DPO role) PLUS
//! the `calibrate` capability, with an audited read per call.
//!
//! **The reviewer is named, and the machine's verdict is not shown.** The
//! reviewer id rides the write (it is required and stored); the machine verdict
//! the reviewer is judging is absent from the queue rows by type, so the
//! blindness is the core's output type and not a discipline. What the report
//! emits is agreement WITH THE OPERATOR — never "with humans", never
//! "consensus": the panel is one member, and the rater is the system's author.

use axum::{
    Json,
    extract::{Query, State},
};
use serde::Deserialize;
use std::sync::Arc;

use crate::AppState;
use crate::handlers::HandlerError;
use crate::handlers::auth::OptPrincipal;

/// The bounded page: `limit` must land inside 1..=500 (named 400 outside),
/// default 100 — the corpus-page posture.
fn validate_agreement_limit(limit: Option<usize>) -> Result<usize, HandlerError> {
    let limit = limit.unwrap_or(100);
    if (1..=500).contains(&limit) {
        return Ok(limit);
    }
    Err(HandlerError::internal_with(
        "agreement_limit_out_of_bounds",
        "limit must land inside 1..=500 — the bench pages are bounded",
        axum::http::StatusCode::BAD_REQUEST,
    ))
}

/// The machinery's refusals surface named — the pinned error vocabulary at
/// the wire: a verdict outside the closed vocabulary is a 400, a missing or
/// invalid reviewer is a 400 (the label is refused before any write), an
/// absent subject is the SAME probe-blind 404 as any other absence, and
/// anything else is internal (never a silent drop).
fn agreement_err(e: String) -> HandlerError {
    match e.as_str() {
        "agreement: verdict required" => HandlerError::bad_request(
            "verdict_required",
            "a verdict is required — choose one of confirmed | overturned | uncertain",
        ),
        s if s.starts_with("agreement: verdict invalid") => {
            HandlerError::bad_request("verdict_invalid", s.to_string())
        }
        "agreement: reviewer_id required" => HandlerError::bad_request(
            "reviewer_required",
            "a reviewer id is required — a label without one is not a measurement",
        ),
        s if s.starts_with("agreement: reviewer_id invalid") => {
            HandlerError::bad_request("reviewer_invalid", s.to_string())
        }
        s if s.starts_with("agreement: reviewer_slot unknown") => {
            HandlerError::bad_request("reviewer_invalid", s.to_string())
        }
        "label_subject_absent" => HandlerError::not_found("no such run row"),
        other => HandlerError::internal(other.to_string()),
    }
}

#[derive(Debug, Deserialize)]
pub struct AgreementQueueQuery {
    pub limit: Option<usize>,
}

/// `GET /workflow/agreement/queue?limit=` — the reviewer's own queue of REAL
/// run rows (`delivery_traces` carrying a populated `model_ref`), oldest first,
/// bounded. The rows carry the trace's metadata (masked through the read seam)
/// and the reviewer's OWN latest verdict. They never carry the machine's
/// verdict: that field does not exist on the type a reviewer reads, so no
/// construction of it can show them what they are judging.
pub async fn get_agreement_queue(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Query(q): Query<AgreementQueueQuery>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal, crate::auth::Action::Write, "", "global")?;
    super::authorize_role(&principal, &pool, "calibrate")?;
    let limit = validate_agreement_limit(q.limit)?;
    let who = principal
        .as_ref()
        .map(|p| p.sub.clone())
        .unwrap_or_else(|| "loopback".into());
    let slot = crate::workflow::agreement::slot_for_principal(&who);
    let reviewer = who.clone();
    let rows = tokio::task::spawn_blocking(
        move || -> Result<Vec<crate::workflow::agreement::AgreementTuple>, String> {
            let conn = pool.get().map_err(|e| format!("{e}"))?;
            crate::workflow::agreement::agreement_queue(&conn, &who, limit)
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
                "subject_id": t.subject_id,
                "stage": t.stage,
                "phase": t.phase,
                "tier": t.tier,
                "model_ref": t.model_ref,
                "my_verdict": t.my_verdict,
                "my_verdict_seq": t.my_verdict_seq,
            })
        })
        .collect();
    let count = rows.len();
    Ok(Json(serde_json::json!({
        "slot": slot,
        "reviewer_id": reviewer,
        "rows": rows,
        "count": count,
    })))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgreementLabelBody {
    pub run_id: i64,
    pub subject_id: String,
    pub verdict: String,
}

/// `POST /workflow/agreement/labels` — bind one verdict to one real run row.
/// The body names the run row and the verdict; the reviewer is the
/// authenticated principal and the slot derives from it, so the client never
/// names the judge. The write is the core's exactly-once, append-only store: a
/// re-submitted latest verdict is the no-op receipt, a changed verdict
/// appends a supersession row, and the audit rows land inside the caller's
/// transaction.
pub async fn post_agreement_label(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Json(body): Json<AgreementLabelBody>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal, crate::auth::Action::Write, "", "global")?;
    super::authorize_role(&principal, &pool, "calibrate")?;
    let who = principal
        .as_ref()
        .map(|p| p.sub.clone())
        .unwrap_or_else(|| "loopback".into());
    let slot = crate::workflow::agreement::slot_for_principal(&who);
    let now = chrono::Utc::now().timestamp();
    let run_id = body.run_id;
    let subject_id = body.subject_id.clone();
    let reviewer_id = who.clone();
    let receipt = tokio::task::spawn_blocking(
        move || -> Result<crate::workflow::agreement::AgreementLabelReceipt, HandlerError> {
            let mut conn = pool
                .get()
                .map_err(|e| HandlerError::internal(format!("{e}")))?;
            let mut tx = crate::workflow::tx::WorkflowTx::begin(&mut conn)
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            let receipt = crate::workflow::agreement::write_agreement_label(
                &mut tx,
                run_id,
                &subject_id,
                &body.verdict,
                &who,
                slot,
                now,
            )
            .map_err(agreement_err)?;
            tx.commit()
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            Ok(receipt)
        },
    )
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))??;
    Ok(Json(serde_json::json!({
        "run_id": body.run_id,
        "subject_id": body.subject_id,
        "reviewer_id": reviewer_id,
        "slot": slot,
        "created": receipt.created,
        "seq": receipt.seq,
        "supersession": receipt.supersession,
        "machine_verdict": receipt.machine_verdict,
        "audited": receipt.created,
    })))
}

#[derive(Debug, Deserialize)]
pub struct AgreementReportQuery {
    pub limit: Option<usize>,
}

/// `GET /workflow/agreement/report` — the agreement cells over the labeled
/// set. THE exfiltration surface: the DPO dual gate plus the `calibrate`
/// capability, and an audited global row per call naming the principal and the
/// counts.
///
/// The body carries BOTH halves and keeps them apart: the per-reviewer raw
/// agreement cells (what one reviewer produced against the machine), and the
/// inter-rater κ pairs (empty in a single-rater era — the absence is the
/// datum). `distinct_reviewers` rides the reviewer cells so a reader can see
/// the single-rater era from the data. Every number here is DATA; nothing
/// gates on it.
pub async fn get_agreement_report(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Query(q): Query<AgreementReportQuery>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal, crate::auth::Action::Admin, "", "global")?;
    crate::handlers::breaches::require_dpo_role(&principal, &pool)?;
    super::authorize_role(&principal, &pool, "calibrate")?;
    let limit = validate_agreement_limit(q.limit)?;
    let who = principal
        .as_ref()
        .map(|p| p.sub.clone())
        .unwrap_or_else(|| "loopback".into());
    let report = tokio::task::spawn_blocking(
        move || -> Result<(Vec<crate::workflow::agreement::AgreementCell>, Vec<crate::workflow::agreement::AgreementPairCell>), HandlerError> {
            let conn = pool
                .get()
                .map_err(|e| HandlerError::internal(format!("{e}")))?;
            let cells = crate::workflow::agreement::agreement_report(&conn)
                .map_err(HandlerError::internal)?;
            let pairs = crate::workflow::agreement::agreement_pair_report(&conn)
                .map_err(HandlerError::internal)?;
            // EVERY report call lands an audit row: who, the counts. The read
            // and its audit share one connection so the counts describe
            // exactly the emitted page.
            crate::audit::record_tenant(
                &conn,
                crate::audit::AuditKind::Workflow,
                crate::workflow::ACTOR,
                crate::workflow::agreement::AUDIT_AGREEMENT_REPORT,
                crate::audit::AuditStatus::Ok,
                &format!(
                    "principal={who} cells={} pairs={}",
                    cells.len(),
                    pairs.len()
                ),
                "global",
            );
            Ok((cells, pairs))
        },
    )
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))??;
    let (cells, pairs) = report;
    let cells: Vec<serde_json::Value> = cells
        .into_iter()
        .take(limit)
        .map(|c| {
            serde_json::json!({
                "domain": c.domain,
                "reviewer_id": c.reviewer_id,
                "n_labeled": c.n_labeled,
                "n_confirmed": c.n_confirmed,
                "n_overturned": c.n_overturned,
                "n_uncertain": c.n_uncertain,
                "raw_agreement_units": c.raw_agreement_units,
                "distinct_reviewers": c.distinct_reviewers,
            })
        })
        .collect();
    let pairs: Vec<serde_json::Value> = pairs
        .into_iter()
        .take(limit)
        .map(|p| {
            serde_json::json!({
                "domain": p.domain,
                "reviewer_a": p.reviewer_a,
                "reviewer_b": p.reviewer_b,
                "n_joint": p.n_joint,
                "kappa_units": p.kappa_units,
                "kappa_note": p.kappa_note,
            })
        })
        .collect();
    let count = cells.len();
    let pair_count = pairs.len();
    Ok(Json(serde_json::json!({
        "measure": "agreement-with-the-operator",
        "rows": cells,
        "count": count,
        "pairs": pairs,
        "pair_count": pair_count,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bounds and the refusal mapping — the pinned error vocabulary at
    /// the wire.
    #[test]
    fn agreement_limit_bounds_and_error_mapping_are_pinned() {
        assert!(validate_agreement_limit(None).is_ok());
        assert_eq!(validate_agreement_limit(None).unwrap(), 100);
        assert!(validate_agreement_limit(Some(1)).is_ok());
        assert!(validate_agreement_limit(Some(500)).is_ok());
        assert_eq!(
            validate_agreement_limit(Some(0)).unwrap_err().inner.code,
            "agreement_limit_out_of_bounds"
        );
        assert_eq!(
            validate_agreement_limit(Some(501)).unwrap_err().status,
            axum::http::StatusCode::BAD_REQUEST
        );
        // The named refusals surface with their statuses.
        let required = agreement_err("agreement: verdict required".into());
        assert_eq!(required.status, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(required.inner.code, "verdict_required");
        let invalid = agreement_err("agreement: verdict invalid: maybe".into());
        assert_eq!(invalid.status, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(invalid.inner.code, "verdict_invalid");
        // A missing reviewer is a 400, never a silent accept.
        let no_reviewer = agreement_err("agreement: reviewer_id required".into());
        assert_eq!(no_reviewer.status, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(no_reviewer.inner.code, "reviewer_required");
        let bad_reviewer = agreement_err("agreement: reviewer_id invalid: 999 chars".into());
        assert_eq!(bad_reviewer.status, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(bad_reviewer.inner.code, "reviewer_invalid");
        // An absent subject is the probe-blind 404.
        let absent = agreement_err("label_subject_absent".into());
        assert_eq!(absent.status, axum::http::StatusCode::NOT_FOUND);
        assert_eq!(absent.inner.code, "not_found");
        // Anything else stays internal — never a silent drop.
        assert_eq!(
            agreement_err("agreement: something else".into()).status,
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    /// The slot derivation rides the core and stays in the roster.
    #[test]
    fn agreement_slot_derivation_is_pure_and_in_roster() {
        use crate::workflow::agreement::slot_for_principal;
        assert_eq!(slot_for_principal("user:a"), slot_for_principal("user:a"));
        assert!(slot_for_principal("user:a") < 2);
        assert!(slot_for_principal("user:b") < 2);
    }
}
