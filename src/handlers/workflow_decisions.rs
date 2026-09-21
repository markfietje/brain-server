//! The operator decision surfaces (the loop closeout).
//!
//! - `POST /workflow/runs/{id}/handoff/decision` — the operator delivers or
//!   cancels a handoff. A decision-required transition never moves without a
//!   decision reference (the machine-refusal law), and the law is enforced at
//!   the surface as `400 decision_ref_required` — the machine cannot close a
//!   handoff on its own authority.
//! - `POST /workflow/runs/{id}/back-referral/return` — the receiver's
//!   release: the operator's report + decision reference releases a return
//!   contract. A report missing a required field surfaces the machinery's B3
//!   refusal as a named 400, an absent contract answers 404 probe-blind, and
//!   `late` is computed at the server clock (the client never supplies it).
//!
//! Both clone the relay precedent end to end: the run's domain resolves
//! first (404 probe-blind on an absent or foreign run), Write on the run's
//! domain plus the `workflow` role gate, and ONE `WorkflowTx` carries the
//! write and its audit row — the chain proves who decided what, when.

use axum::{
    Json,
    extract::{Path, State},
};
use std::sync::Arc;

use crate::AppState;
use crate::handlers::HandlerError;
use crate::handlers::auth::OptPrincipal;
use crate::workflow::gdl::HandoffTransition;

/// The decision reference is the audit-recovery handle for the operator's
/// call — screened, bounded, no free-text pass-through.
const MAX_DECISION_REF_LEN: usize = 256;

fn validate_decision_ref(raw: &str) -> Result<String, HandlerError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(HandlerError::bad_request(
            "decision_ref_required",
            "an operator decision reference is required — the machine never \
             closes a handoff or releases a return contract on its own \
             authority",
        ));
    }
    if trimmed.len() > MAX_DECISION_REF_LEN
        || trimmed
            .chars()
            .any(|c| c.is_control() || crate::strip_invisible::is_invisible(c))
    {
        return Err(HandlerError::bad_request(
            "decision_ref_invalid",
            format!(
                "decision_ref must be 1..={MAX_DECISION_REF_LEN} chars with no \
                 control or invisible characters"
            ),
        ));
    }
    Ok(trimmed.to_string())
}

fn validate_contract_key(raw: &str) -> Result<String, HandlerError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_DECISION_REF_LEN {
        return Err(HandlerError::bad_request(
            "contract_key_required",
            format!("contract_key must be 1..={MAX_DECISION_REF_LEN} chars"),
        ));
    }
    if trimmed
        .chars()
        .any(|c| c.is_control() || crate::strip_invisible::is_invisible(c))
    {
        return Err(HandlerError::bad_request(
            "contract_key_required",
            "contract_key must not contain control or invisible characters",
        ));
    }
    Ok(trimmed.to_string())
}

/// The machinery's refusals surface named: an absent contract answers 404
/// probe-blind, the B3 report-field law answers `400 report_incomplete` with
/// the missing list, and anything else is internal (never a silent drop).
fn return_err(e: crate::agentloop::run_loop::LoopError) -> HandlerError {
    match e {
        crate::agentloop::run_loop::LoopError::Persist(m)
            if m == "back-referral contract absent" =>
        {
            HandlerError::not_found("back-referral contract not found")
        }
        crate::agentloop::run_loop::LoopError::Persist(m) if m.starts_with("B3:") => {
            HandlerError::bad_request_with(
                "report_incomplete",
                "the report is missing required fields — the contract's B3 \
                 law holds at the surface",
                serde_json::json!({ "missing": m.split("; ").collect::<Vec<_>>() }),
            )
        }
        other => HandlerError::internal(other.to_string()),
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffDecisionBody {
    pub transition: String,
    #[serde(default)]
    pub decision_ref: Option<String>,
}

/// `POST /workflow/runs/{id}/handoff/decision`
pub async fn post_handoff_decision(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
    Json(body): Json<HandoffDecisionBody>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = super::workflow::run_domain(&state, id).await?;
    super::authorize(&principal, crate::auth::Action::Write, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize_role(&principal, &pool, "workflow")?;

    let transition = match body.transition.as_str() {
        "delivered" => HandoffTransition::Delivered,
        "cancelled" => HandoffTransition::Cancelled,
        _ => {
            return Err(HandlerError::bad_request(
                "unknown_transition",
                "transition must be delivered | cancelled",
            ));
        }
    };
    let raw_ref = body.decision_ref.as_deref().unwrap_or("");
    let decision_ref = validate_decision_ref(raw_ref)?;
    let actor = super::recall::principal_label(&principal);
    let echo = crate::gate::sanitize_read(&decision_ref, false, &principal);

    let outcome = tokio::task::spawn_blocking(move || -> Result<(), HandlerError> {
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        let mut tx = crate::workflow::tx::WorkflowTx::begin(&mut conn)
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        let detail = serde_json::json!({ "route": "workflow_decisions" });
        crate::workflow::gdl::write_handoff_transition(
            tx.tx(),
            id,
            &actor,
            transition,
            Some(&decision_ref),
            &detail,
            chrono::Utc::now().timestamp(),
        )
        .map_err(|e| HandlerError::internal(e.to_string()))?;
        tx.commit()
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        Ok(())
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?;
    outcome?;
    Ok(Json(serde_json::json!({
        "run_id": id,
        "transition": transition.as_str(),
        "decision_ref": echo,
        "audited": true,
    })))
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackReferralReturnBody {
    #[serde(default)]
    pub contract_key: Option<String>,
    #[serde(default)]
    pub report: Option<serde_json::Value>,
    #[serde(default)]
    pub decision_ref: Option<String>,
}

/// `POST /workflow/runs/{id}/back-referral/return`
pub async fn post_back_referral_return(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
    Json(body): Json<BackReferralReturnBody>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = super::workflow::run_domain(&state, id).await?;
    super::authorize(&principal, crate::auth::Action::Write, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize_role(&principal, &pool, "workflow")?;

    let raw_key = body.contract_key.as_deref().unwrap_or("");
    let contract_key = validate_contract_key(raw_key)?;
    let report = match body.report {
        Some(v) if v.is_object() => v,
        _ => {
            return Err(HandlerError::bad_request(
                "report_invalid",
                "report must be a JSON object carrying the contract's \
                 required fields",
            ));
        }
    };
    let raw_ref = body.decision_ref.as_deref().unwrap_or("");
    let decision_ref = validate_decision_ref(raw_ref)?;
    let echo_key = echo_of(&contract_key, &principal);

    let outcome = tokio::task::spawn_blocking(move || -> Result<_, HandlerError> {
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        let mut tx = crate::workflow::tx::WorkflowTx::begin(&mut conn)
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        let now = chrono::Utc::now().timestamp();
        crate::workflow::gdl::write_back_referral_return(
            tx.tx(),
            id,
            &contract_key,
            report,
            Some(&decision_ref),
            now,
        )
        .map_err(return_err)?;
        // The receipt's `late` flag rides the row the release just appended;
        // the server clock decided it, the reply only reports it. The read
        // lives in the core (the no-SQL-in-handlers law).
        let late = crate::workflow::gdl::latest_back_referral_late_flag(
            tx.tx(),
            id,
            &contract_key,
        )
        .map_err(|e| HandlerError::internal(e.to_string()))?
        .unwrap_or(false);
        tx.commit()
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        Ok(late)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?;
    let late = outcome?;
    Ok(Json(serde_json::json!({
        "run_id": id,
        "contract_key": echo_key,
        "status": "returned",
        "late": late,
    })))
}

/// The key is caller-supplied identity text: echoed sanitized (the relay
/// to_principal precedent), never stored raw beyond the machinery's own
/// payload writers.
fn echo_of(key: &str, principal: &Option<crate::auth::Principal>) -> String {
    crate::gate::sanitize_read(key, false, principal)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// decision_ref_bounds_and_screening
    #[test]
    fn decision_ref_bounds_and_screening() {
        assert_eq!(
            validate_decision_ref("  op-2026-09-22#7 ").unwrap(),
            "op-2026-09-22#7"
        );
        let err = validate_decision_ref("   ").expect_err("empty refuses");
        assert_eq!(err.inner.code, "decision_ref_required");
        let err = validate_decision_ref("").expect_err("absent refuses");
        assert_eq!(err.inner.code, "decision_ref_required");
        let err =
            validate_decision_ref(&"x".repeat(MAX_DECISION_REF_LEN + 1)).expect_err("over bound");
        assert_eq!(err.inner.code, "decision_ref_invalid");
        let err = validate_decision_ref("op\u{200B}-7").expect_err("invisible refuses");
        assert_eq!(err.inner.code, "decision_ref_invalid");
        let err = validate_decision_ref("op\n-7").expect_err("control refuses");
        assert_eq!(err.inner.code, "decision_ref_invalid");
    }

    /// contract_key_required_nonempty_and_bounded
    #[test]
    fn contract_key_required_nonempty_and_bounded() {
        assert_eq!(
            validate_contract_key(" run1:back_referral:owner ").unwrap(),
            "run1:back_referral:owner"
        );
        let err = validate_contract_key("").expect_err("empty refuses");
        assert_eq!(err.inner.code, "contract_key_required");
        let err =
            validate_contract_key(&"k".repeat(MAX_DECISION_REF_LEN + 1)).expect_err("over bound");
        assert_eq!(err.inner.code, "contract_key_required");
        let err = validate_contract_key("k\u{202E}").expect_err("invisible refuses");
        assert_eq!(err.inner.code, "contract_key_required");
    }

    /// The machinery's refusals surface named — never a silent drop, never a
    /// bypass: absent → 404, B3 → 400 with the missing list.
    #[test]
    fn return_errors_surface_named() {
        let absent = return_err(crate::agentloop::run_loop::LoopError::Persist(
            "back-referral contract absent".into(),
        ));
        assert_eq!(absent.status, axum::http::StatusCode::NOT_FOUND);
        let b3 = return_err(crate::agentloop::run_loop::LoopError::Persist(
            "B3: return contract released without the required report field \
             `finding`; B3: return contract released without the required \
             report field `follow_up`"
                .into(),
        ));
        assert_eq!(b3.status, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(b3.inner.code, "report_incomplete");
        let other = return_err(crate::agentloop::run_loop::LoopError::Persist(
            "sql down".into(),
        ));
        assert_eq!(other.status, axum::http::StatusCode::INTERNAL_SERVER_ERROR);
    }
}
