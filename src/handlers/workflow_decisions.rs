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
        // The receipt's `late` flag rides the row the release just appended
        // (the same latest-row read the machinery and the sweep use) — the
        // server clock decided it, the reply only reports it.
        use rusqlite::OptionalExtension;
        let latest: Option<String> = tx
            .tx()
            .query_row(
                "SELECT payload_json FROM agent_session_events \
                 WHERE run_id = ?1 AND kind = 'back_referral' \
                 AND payload_json LIKE ?2 \
                 ORDER BY seq DESC LIMIT 1",
                rusqlite::params![id, format!("%\"{contract_key}\"%"),],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        let late = latest
            .as_deref()
            .and_then(|p| serde_json::from_str::<serde_json::Value>(p).ok())
            .and_then(|v| v["late"].as_bool())
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

    async fn open_run(f: &crate::handlers::case_run::tests::Fixture) -> i64 {
        let opened = crate::handlers::workflow::post_run(
            axum::extract::State(f.state.clone()),
            crate::handlers::auth::OptPrincipal(None),
            axum::Json(crate::handlers::workflow::OpenRunRequest {
                domain: "personal".to_string(),
                kind: "interview".to_string(),
                state_json: r#"{"a":1}"#.to_string(),
                jurisdiction: None,
            }),
        )
        .await
        .expect("open");
        opened.0["run_id"].as_i64().unwrap()
    }

    fn contract(required: Vec<&str>) -> crate::workflow::gdl::BackReferralContract {
        serde_json::from_value(serde_json::json!({
            "referrer": "l1:steward-dpc",
            "receiver": "eng-storage",
            "clinical_question": "confirm the battery",
            "required_report": required,
            "status": "open",
        }))
        .unwrap()
    }

    fn arm_contract(
        f: &crate::handlers::case_run::tests::Fixture,
        run_id: i64,
        required: Vec<&str>,
        armed_at: i64,
    ) -> String {
        let key = format!("run{run_id}:back_referral:owner");
        let mut conn = f.state.pool.get().unwrap();
        let mut tx = crate::workflow::tx::WorkflowTx::begin(&mut conn).unwrap();
        crate::workflow::gdl::write_back_referral_row(
            tx.tx(),
            run_id,
            "owner",
            &contract(required),
            "P3",
            armed_at,
        )
        .unwrap();
        tx.commit().unwrap();
        key
    }

    /// unknown_transition_refused_400 — the CLOSED vocabulary holds at the
    /// surface; only delivered | cancelled pass the route.
    #[tokio::test]
    async fn unknown_transition_refused_400() {
        let f = crate::handlers::case_run::tests::fixture();
        let run_id = open_run(&f).await;
        let err = post_handoff_decision(
            axum::extract::State(f.state.clone()),
            crate::handlers::auth::OptPrincipal(None),
            axum::extract::Path(run_id),
            Json(HandoffDecisionBody {
                transition: "postponed".into(),
                decision_ref: Some("op-1".into()),
            }),
        )
        .await
        .expect_err("unknown transition refuses");
        assert_eq!(err.status, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(err.inner.code, "unknown_transition");
        // Nothing landed.
        let conn = f.state.pool.get().unwrap();
        let rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_session_events \
                 WHERE run_id = ?1 AND kind = 'handoff_lifecycle'",
                [run_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rows, 0, "a refused transition writes nothing");
    }

    /// handoff_decision_requires_decision_ref — the B4/HITL law at the
    /// surface: absent, empty, and malformed references all refuse 400
    /// before any write.
    #[tokio::test]
    async fn handoff_decision_requires_decision_ref() {
        let f = crate::handlers::case_run::tests::fixture();
        let run_id = open_run(&f).await;
        for bad in [None, Some("   ".into()), Some("op\u{200B}-1".into())] {
            let err = post_handoff_decision(
                axum::extract::State(f.state.clone()),
                crate::handlers::auth::OptPrincipal(None),
                axum::extract::Path(run_id),
                Json(HandoffDecisionBody {
                    transition: "delivered".into(),
                    decision_ref: bad,
                }),
            )
            .await
            .expect_err("a decision-required transition refuses without a ref");
            assert_eq!(err.status, axum::http::StatusCode::BAD_REQUEST);
            assert!(
                err.inner.code == "decision_ref_required"
                    || err.inner.code == "decision_ref_invalid"
            );
        }
        let conn = f.state.pool.get().unwrap();
        let rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_session_events \
                 WHERE run_id = ?1 AND kind = 'handoff_lifecycle'",
                [run_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rows, 0, "nothing landed without the reference");
    }

    /// handoff_delivered_and_cancelled_land_lifecycle_and_audit — both
    /// operator arms move in ONE WorkflowTx each, the lifecycle rows carry
    /// the decision reference, and the audit rows ride the chain.
    #[tokio::test]
    async fn handoff_delivered_and_cancelled_land_lifecycle_and_audit() {
        let f = crate::handlers::case_run::tests::fixture();
        let run_id = open_run(&f).await;
        let delivered = post_handoff_decision(
            axum::extract::State(f.state.clone()),
            crate::handlers::auth::OptPrincipal(None),
            axum::extract::Path(run_id),
            Json(HandoffDecisionBody {
                transition: "delivered".into(),
                decision_ref: Some("op-2026-09-22#7".into()),
            }),
        )
        .await
        .expect("delivered");
        assert_eq!(delivered.0["transition"], serde_json::json!("delivered"));
        assert_eq!(
            delivered.0["decision_ref"],
            serde_json::json!("op-2026-09-22#7")
        );
        let cancelled = post_handoff_decision(
            axum::extract::State(f.state.clone()),
            crate::handlers::auth::OptPrincipal(None),
            axum::extract::Path(run_id),
            Json(HandoffDecisionBody {
                transition: "cancelled".into(),
                decision_ref: Some("op-2026-09-22#8".into()),
            }),
        )
        .await
        .expect("cancelled");
        assert_eq!(cancelled.0["transition"], serde_json::json!("cancelled"));

        let conn = f.state.pool.get().unwrap();
        let (lifecycles, audited): (i64, i64) = {
            let lifecycles: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM agent_session_events \
                     WHERE run_id = ?1 AND kind = 'handoff_lifecycle'",
                    [run_id],
                    |r| r.get(0),
                )
                .unwrap();
            let audited: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM audit_events WHERE target_hash = ?1",
                    [crate::audit::hash("handoff_lifecycle")],
                    |r| r.get(0),
                )
                .unwrap();
            (lifecycles, audited)
        };
        assert_eq!(lifecycles, 2, "both transitions landed as lifecycle rows");
        assert!(audited >= 2, "both transitions carry their audit rows");
        let payloads: Vec<String> = {
            let mut stmt = conn
                .prepare(
                    "SELECT payload_json FROM agent_session_events \
                     WHERE run_id = ?1 AND kind = 'handoff_lifecycle' ORDER BY seq",
                )
                .unwrap();
            stmt.query_map([run_id], |r| r.get::<_, String>(0))
                .map(|it| it.filter_map(Result::ok).collect())
                .unwrap()
        };
        assert!(payloads[0].contains("\"decision_ref\":\"op-2026-09-22#7\""));
        assert!(payloads[0].contains("\"human_edited\":true"));
        assert!(payloads[1].contains("\"decision_ref\":\"op-2026-09-22#8\""));
    }

    /// back_referral_return_requires_decision_ref_and_full_report — the B4
    /// and B3 laws hold at the surface, an absent contract answers 404
    /// probe-blind, and a complete report releases the contract `returned`.
    #[tokio::test]
    async fn back_referral_return_requires_decision_ref_and_full_report() {
        let f = crate::handlers::case_run::tests::fixture();
        let run_id = open_run(&f).await;
        let key = arm_contract(
            &f,
            run_id,
            vec!["finding", "treatment_plan", "follow_up"],
            chrono::Utc::now().timestamp(),
        );

        // B4 at the surface: no reference, no release.
        let err = post_back_referral_return(
            axum::extract::State(f.state.clone()),
            crate::handlers::auth::OptPrincipal(None),
            axum::extract::Path(run_id),
            Json(BackReferralReturnBody {
                contract_key: Some(key.clone()),
                report: Some(serde_json::json!({
                    "finding": "battery confirmed",
                    "treatment_plan": "replaced",
                    "follow_up": "72h re-check"
                })),
                decision_ref: None,
            }),
        )
        .await
        .expect_err("B4 refuses");
        assert_eq!(err.inner.code, "decision_ref_required");

        // B3 at the surface: an incomplete report names the missing fields.
        let err = post_back_referral_return(
            axum::extract::State(f.state.clone()),
            crate::handlers::auth::OptPrincipal(None),
            axum::extract::Path(run_id),
            Json(BackReferralReturnBody {
                contract_key: Some(key.clone()),
                report: Some(serde_json::json!({
                    "finding": "battery confirmed"
                })),
                decision_ref: Some("op-2026-09-22#9".into()),
            }),
        )
        .await
        .expect_err("B3 refuses");
        assert_eq!(err.status, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(err.inner.code, "report_incomplete");
        let missing = err.inner.details.expect("the missing list rides");
        assert_eq!(missing["missing"].as_array().unwrap().len(), 2);

        // Probe-blind: an absent contract answers 404.
        let err = post_back_referral_return(
            axum::extract::State(f.state.clone()),
            crate::handlers::auth::OptPrincipal(None),
            axum::extract::Path(run_id),
            Json(BackReferralReturnBody {
                contract_key: Some("run1:back_referral:nobody".into()),
                report: Some(serde_json::json!({"finding": "x"})),
                decision_ref: Some("op-1".into()),
            }),
        )
        .await
        .expect_err("absent contract refuses");
        assert_eq!(err.status, axum::http::StatusCode::NOT_FOUND);

        // The complete release: the contract flips returned, audited.
        let ok = post_back_referral_return(
            axum::extract::State(f.state.clone()),
            crate::handlers::auth::OptPrincipal(None),
            axum::extract::Path(run_id),
            Json(BackReferralReturnBody {
                contract_key: Some(key.clone()),
                report: Some(serde_json::json!({
                    "finding": "battery confirmed as root cause",
                    "treatment_plan": "replaced under warranty",
                    "follow_up": "72h re-check scheduled"
                })),
                decision_ref: Some("op-2026-09-22#9".into()),
            }),
        )
        .await
        .expect("released");
        assert_eq!(ok.0["status"], serde_json::json!("returned"));
        assert_eq!(ok.0["late"], serde_json::json!(false));
        let conn = f.state.pool.get().unwrap();
        let returned: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_session_events \
                 WHERE run_id = ?1 AND kind = 'back_referral' \
                 AND payload_json LIKE '%\"status\":\"returned\"%'",
                [run_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(returned, 1, "the release row landed");
    }

    /// late_return_flags_late_on_the_surface — the server clock decides: a
    /// contract armed past its window releases with `late: true`.
    #[tokio::test]
    async fn late_return_flags_late_on_the_surface() {
        let f = crate::handlers::case_run::tests::fixture();
        let run_id = open_run(&f).await;
        let key = arm_contract(
            &f,
            run_id,
            vec!["finding"],
            chrono::Utc::now().timestamp() - 2 * 86_400,
        );
        let ok = post_back_referral_return(
            axum::extract::State(f.state.clone()),
            crate::handlers::auth::OptPrincipal(None),
            axum::extract::Path(run_id),
            Json(BackReferralReturnBody {
                contract_key: Some(key),
                report: Some(serde_json::json!({"finding": "eventual receipt"})),
                decision_ref: Some("op-late-1".into()),
            }),
        )
        .await
        .expect("released late");
        assert_eq!(ok.0["late"], serde_json::json!(true));
    }

    /// report_must_be_an_object — the report field's shape law at the
    /// surface (absent, non-object, and empty-object cases).
    #[tokio::test]
    async fn report_must_be_an_object() {
        let f = crate::handlers::case_run::tests::fixture();
        let run_id = open_run(&f).await;
        let key = arm_contract(&f, run_id, vec!["finding"], chrono::Utc::now().timestamp());
        for bad in [
            None,
            Some(serde_json::json!("battery confirmed")),
            Some(serde_json::json!(["battery confirmed"])),
        ] {
            let err = post_back_referral_return(
                axum::extract::State(f.state.clone()),
                crate::handlers::auth::OptPrincipal(None),
                axum::extract::Path(run_id),
                Json(BackReferralReturnBody {
                    contract_key: Some(key.clone()),
                    report: bad,
                    decision_ref: Some("op-shape-1".into()),
                }),
            )
            .await
            .expect_err("a non-object report refuses");
            assert_eq!(err.inner.code, "report_invalid");
        }
    }
}
