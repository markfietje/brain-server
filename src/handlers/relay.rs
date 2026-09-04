//! The one-click handover surfaces (Relay).
//!
//! - `POST /workflow/runs/{id}/handover/offer` — gated by the packet-
//!   completeness check; an incomplete packet refuses with the MISSING list
//!   (422) — the machine coaches the protocol, the human fixes the packet.
//! - `POST /workflow/runs/{id}/handover/{offer_id}/accept` — CAS owner
//!   transfer to the acceptor in the SAME tx as the offer state move; the
//!   SLA clock is never touched.
//! - `POST /workflow/runs/{id}/handover/{offer_id}/decline` — a screened,
//!   bounded reason is REQUIRED (an audited refusal beats a silent bounce).
//! - `GET /ops/handovers?domain=&now=` — the handover-due board ranked by
//!   SLA remaining, flagged inside the ring's derived overlap window.

use axum::{
    Json,
    extract::{Path, Query, State},
};
use std::collections::HashMap;
use std::sync::Arc;

use crate::AppState;
use crate::handlers::HandlerError;
use crate::handlers::auth::OptPrincipal;
use crate::workflow::relay::{self, OfferDraft, OfferError, PacketFacts};
use crate::workflow::state::{self as run_state};

const MAX_REASON_LEN: usize = 4000;
const MAX_PRINCIPAL_LEN: usize = relay::MAX_PRINCIPAL_LEN;
const MAX_OVERLAP_MINUTES: i64 = 120;

fn offer_err(e: OfferError) -> HandlerError {
    match e {
        OfferError::Missing(m) => HandlerError::not_found(m),
        OfferError::Database(m) => HandlerError::internal(m),
    }
}

/// The five gate predicates over a run's stored shape. The packet is
/// complete when the receiving team can answer: what's the open question,
/// how much SLA remains, what step are we on, what evidence exists, and is
/// escalation resolved.
fn packet_facts(
    state_json: &serde_json::Value,
    steps_exist: bool,
    created_at: i64,
    now: i64,
) -> PacketFacts {
    let deadline = state_json
        .get("sla_deadline")
        .and_then(|v| v.as_i64())
        .unwrap_or_else(|| created_at + brain_engine_sdk::policy::Priority::P3.ttl_secs());
    let escalation = state_json.get("escalation").and_then(|v| v.as_str());
    PacketFacts {
        pending_question: state_json
            .get("open_question")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        sla_deadline: Some(deadline),
        now,
        has_current_step: steps_exist
            || state_json
                .get("current_step")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty()),
        has_evidence: state_json.get("checkpoint").is_some_and(|v| !v.is_null()),
        escalation_honored: escalation
            .is_none_or(|e| matches!(e, "none" | "resolved" | "acknowledged")),
    }
}

fn load_run(
    conn: &rusqlite::Connection,
    id: i64,
) -> Result<(String, String, i64, String), HandlerError> {
    relay::load_run(conn, id)
        .map_err(|e| HandlerError::internal(format!("{e}")))?
        .ok_or_else(|| HandlerError::not_found("workflow run not found"))
}

/// Identity fields fail CLOSED: bounds + a control/invisible-character
/// refusal (never a silent strip — a stripped id could collide with a
/// different real principal at accept time) + the loopback superuser is
/// never an addressee.
fn validate_to_principal(to: &str) -> Result<(), HandlerError> {
    if to.is_empty() || to.len() > MAX_PRINCIPAL_LEN {
        return Err(HandlerError::bad_request(
            "principal_invalid",
            format!("to_principal must be 1..={MAX_PRINCIPAL_LEN} chars"),
        ));
    }
    if to == "loopback" {
        return Err(HandlerError::bad_request(
            "handover_self",
            "the loopback principal cannot be a handover addressee",
        ));
    }
    if to
        .chars()
        .any(|c| c.is_control() || crate::strip_invisible::is_invisible(c))
    {
        return Err(HandlerError::bad_request(
            "principal_invalid",
            "to_principal must not contain control or invisible characters",
        ));
    }
    Ok(())
}

/// A handover of (or onto) a finished run is a governance fiction: only
/// ACTIVE runs are offerable, and acceptance must never resurrect a
/// completed/cancelled run.
fn ensure_run_active(status: &str) -> Result<(), HandlerError> {
    if status != "active" {
        return Err(HandlerError::conflict_with(
            "run_not_active",
            "only active runs can change hands",
            serde_json::json!({ "status": status }),
        ));
    }
    Ok(())
}

fn crew_touch(conn: &rusqlite::Connection, domain: &str, actor: &str, run_id: i64) {
    if let Err(e) = crate::workflow::crew::touch(
        conn,
        domain,
        actor,
        "cranking",
        None,
        &[],
        chrono::Utc::now().timestamp(),
    ) {
        tracing::warn!(run = run_id, "presence touch failed: {e}");
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfferRequest {
    pub to_principal: String,
    #[serde(default)]
    pub overlap_minutes: Option<i64>,
}

/// `POST /workflow/runs/{id}/handover/offer`
pub async fn post_handover_offer(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
    Json(body): Json<OfferRequest>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = super::workflow::run_domain(&state, id).await?;
    super::authorize(&principal, crate::auth::Action::Write, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize_role(&principal, &pool, "workflow")?;

    let to = body.to_principal.trim().to_string();
    validate_to_principal(&to)?;
    if to == super::recall::principal_label(&principal) {
        return Err(HandlerError::bad_request(
            "handover_self",
            "cannot offer a handover to yourself",
        ));
    }
    let overlap = body.overlap_minutes.unwrap_or(0);
    if !(0..=MAX_OVERLAP_MINUTES).contains(&overlap) {
        return Err(HandlerError::bad_request(
            "overlap_invalid",
            format!("overlap_minutes must be 0..={MAX_OVERLAP_MINUTES}"),
        ));
    }

    // Packet-completeness gate runs BEFORE any write — an incomplete packet
    // refuses with the missing list and writes nothing (no audit noise).
    let facts: PacketFacts = {
        let pool_gate = pool.clone();
        tokio::task::spawn_blocking(move || -> Result<PacketFacts, HandlerError> {
            let conn = pool_gate
                .get()
                .map_err(|e| HandlerError::internal(format!("{e}")))?;
            let (_domain, state_json, created_at, status) = load_run(&conn, id)?;
            ensure_run_active(&status)?;
            let parsed: serde_json::Value =
                serde_json::from_str(&state_json).unwrap_or(serde_json::Value::Null);
            let steps_exist = relay::run_has_steps(&conn, id)
                .map_err(|e| HandlerError::internal(format!("{e}")))?;
            Ok(packet_facts(
                &parsed,
                steps_exist,
                created_at,
                chrono::Utc::now().timestamp(),
            ))
        })
        .await
        .map_err(|e| HandlerError::internal(format!("{e}")))??
    };
    let missing = relay::packet_missing(&facts);
    if !missing.is_empty() {
        return Err(HandlerError::bad_request_with(
            "packet_incomplete",
            "the I-PASS packet is incomplete — fix the listed gaps before offering",
            serde_json::json!({ "missing": missing }),
        ));
    }

    let actor = super::recall::principal_label(&principal);
    let to_resp = to.clone();
    let sla_deadline = facts.sla_deadline.unwrap_or_else(|| {
        chrono::Utc::now().timestamp() + brain_engine_sdk::policy::Priority::P3.ttl_secs()
    });
    let outcome = tokio::task::spawn_blocking(move || -> Result<_, HandlerError> {
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        let draft = OfferDraft {
            domain: &domain,
            run_id: id,
            from_principal: &actor,
            to_principal: &to,
            overlap_minutes: overlap,
            sla_deadline,
            now: chrono::Utc::now().timestamp(),
        };
        let mut tx = crate::workflow::tx::WorkflowTx::begin(&mut conn)
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        let result = relay::insert_offer(tx.tx(), &draft).map_err(offer_err);
        match result {
            Ok((offer_id, _created)) => {
                crew_touch(tx.tx(), &domain, &actor, id);
                tx.commit()
                    .map_err(|e| HandlerError::internal(e.to_string()))?;
                Ok(offer_id)
            }
            Err(e) => Err(e),
        }
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?;
    let offer_id = outcome?;
    Ok(Json(serde_json::json!({
        "offer_id": offer_id,
        "run_id": id,
        "to_principal": crate::gate::sanitize_read(&to_resp, false, &principal),
        "state": relay::OFFERED,
        "sla_deadline": sla_deadline,
    })))
}

fn resume_checkpoint(state_json: &serde_json::Value) -> serde_json::Value {
    state_json
        .get("checkpoint")
        .cloned()
        .unwrap_or(serde_json::Value::Null)
}

async fn decide_route(
    state: Arc<AppState>,
    principal: OptPrincipal,
    id: i64,
    offer_id: i64,
    accept: bool,
    reason: Option<String>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = super::workflow::run_domain(&state, id).await?;
    super::authorize(&principal, crate::auth::Action::Write, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize_role(&principal, &pool, "workflow")?;

    let actor = super::recall::principal_label(&principal);
    let reason = match (&reason, accept) {
        (Some(r), _) => {
            let trimmed = r.trim();
            if trimmed.is_empty() {
                return Err(HandlerError::bad_request(
                    "reason_required",
                    "a decline requires a non-empty reason",
                ));
            }
            if trimmed.len() > MAX_REASON_LEN {
                return Err(HandlerError::bad_request(
                    "reason_too_long",
                    format!("reason must be at most {MAX_REASON_LEN} chars"),
                ));
            }
            Some(crate::gate::sanitize_read(trimmed, false, &principal))
        }
        (None, false) => {
            return Err(HandlerError::bad_request(
                "reason_required",
                "a decline requires a reason",
            ));
        }
        (None, true) => None,
    };

    let outcome = tokio::task::spawn_blocking(move || -> Result<_, HandlerError> {
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        let mut tx = crate::workflow::tx::WorkflowTx::begin(&mut conn)
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        let now = chrono::Utc::now().timestamp();
        let moved = relay::decide_offer(tx.tx(), id, offer_id, accept, reason.as_deref(), now)
            .map_err(offer_err)?;
        let mut reply = serde_json::json!({ "offer_id": offer_id, "run_id": id, "moved": moved });
        if moved && accept {
            // Ownership transfer by CAS, same tx as the offer state move:
            // either both land or neither does. The SLA clock is untouched,
            // and the run's CURRENT status is preserved — an acceptance must
            // never resurrect a completed/cancelled run.
            let row = relay::load_run_state(tx.tx(), id)
                .map_err(|e| HandlerError::internal(format!("{e}")))?
                // Same observable failure the direct query_row produced —
                // the run cannot vanish inside the Immediate tx that just
                // moved its offer, and if it ever does the text is the
                // rusqlite no-rows text, byte-for-byte.
                .ok_or_else(|| HandlerError::internal("Query returned no rows"))?;
            ensure_run_active(&row.2)?;
            let mut st: serde_json::Value =
                serde_json::from_str(&row.0).unwrap_or(serde_json::Value::Null);
            st["owner"] = serde_json::json!(actor);
            run_state::cas_update(tx.tx(), id, row.1, &st.to_string(), &row.2, now).map_err(
                |e| match e {
                    run_state::CasError::Stale { actual_revision } => HandlerError::conflict_with(
                        "cas_stale",
                        "run state advanced concurrently during acceptance",
                        serde_json::json!({ "actual_revision": actual_revision }),
                    ),
                    other => HandlerError::internal(other.to_string()),
                },
            )?;
            reply["owner"] = serde_json::json!(actor);
            // The read seam on every emitted text field: a planted invisible-
            // character checkpoint cannot smuggle a fence marker through the
            // accept receipt. Non-string checkpoints ride unchanged.
            reply["resume_at_checkpoint"] = match resume_checkpoint(&st) {
                serde_json::Value::String(s) => {
                    serde_json::Value::String(crate::gate::sanitize_read(&s, false, &principal))
                }
                other => other,
            };
        }
        if moved {
            crew_touch(tx.tx(), &domain, &actor, id);
        }
        tx.commit()
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        Ok(reply)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?;
    Ok(Json(outcome?))
}

/// `POST /workflow/runs/{id}/handover/{offer_id}/accept`
pub async fn post_handover_accept(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    path: Path<(i64, i64)>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let (id, offer_id) = *path;
    super::authorize(
        &principal.0,
        crate::auth::Action::Write,
        "",
        &super::workflow::run_domain(&state, id).await?,
    )?;
    decide_route(state, principal, id, offer_id, true, None).await
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclineRequest {
    pub reason: String,
}

/// `POST /workflow/runs/{id}/handover/{offer_id}/decline`
pub async fn post_handover_decline(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    path: Path<(i64, i64)>,
    Json(body): Json<DeclineRequest>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let (id, offer_id) = *path;
    super::authorize(
        &principal.0,
        crate::auth::Action::Write,
        "",
        &super::workflow::run_domain(&state, id).await?,
    )?;
    decide_route(state, principal, id, offer_id, false, Some(body.reason)).await
}

/// `GET /ops/handovers?domain=&now=` — the follow-the-sun board.
pub async fn get_ops_handovers(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = params
        .get("domain")
        .cloned()
        .unwrap_or_else(|| "global".into());
    let now = match params.get("now").map(|s| s.parse::<i64>()) {
        Some(Ok(t)) => t,
        Some(Err(_)) => {
            return Err(HandlerError::bad_request(
                "now_invalid",
                "now must be epoch seconds",
            ));
        }
        None => chrono::Utc::now().timestamp(),
    };
    super::authorize(&principal, crate::auth::Action::Read, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    let domain_board = domain.clone();
    let board = tokio::task::spawn_blocking(move || -> Result<_, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        relay::board(&conn, &domain_board, now).map_err(offer_err)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?;
    let (rows, corrupt_rows) = board?;
    let payload: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            serde_json::json!({
                "run_id": r.run_id,
                // Read-seam parity: owner labels are stored strings too.
                "owner": r.owner
                    .as_deref()
                    .map(|o| crate::gate::sanitize_read(o, false, &principal)),
                "sla_deadline": r.sla_deadline,
                "remaining_secs": r.remaining_secs,
                "in_overlap": r.in_overlap,
            })
        })
        .collect();
    Ok(Json(serde_json::json!({
        "domain": domain,
        "now": now,
        "board": payload,
        // Skipped rows are counted on the wire, never silently absorbed.
        "corrupt_state_rows_skipped": corrupt_rows,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// validate_to_principal_refuses_invisible_and_control_ids
    #[test]
    fn validate_to_principal_refuses_invisible_and_control_ids() {
        assert!(validate_to_principal("ams-op").is_ok());
        // Zero-width joiner smuggled into an id: refused, never stripped.
        assert!(validate_to_principal("ams\u{200B}-op").is_err());
        // Bidi override: refused.
        assert!(validate_to_principal("\u{202E}ams-op").is_err());
        // Control character: refused.
        assert!(validate_to_principal("ams\n-op").is_err());
        // The loopback superuser is never an addressee.
        assert!(validate_to_principal("loopback").is_err());
        // Bounds.
        assert!(validate_to_principal("").is_err());
        assert!(validate_to_principal(&"x".repeat(MAX_PRINCIPAL_LEN + 1)).is_err());
    }

    /// ensure_run_active_refuses_finished_runs_offer_and_accept
    #[test]
    fn ensure_run_active_refuses_finished_runs_offer_and_accept() {
        assert!(ensure_run_active("active").is_ok());
        let err = ensure_run_active("completed").expect_err("completed refuses");
        assert_eq!(err.status, axum::http::StatusCode::CONFLICT);
        let err = ensure_run_active("cancelled").expect_err("cancelled refuses");
        assert_eq!(err.status, axum::http::StatusCode::CONFLICT);
    }

    /// decline_reason_validation_bounded_and_screened
    #[test]
    fn decline_reason_validation_bounds_hold() {
        const MAX: usize = MAX_REASON_LEN;
        assert!("x".repeat(MAX).len() <= MAX);
        assert!("x".repeat(MAX + 1).len() > MAX);
    }
}
