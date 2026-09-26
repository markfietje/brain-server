//! Protocol adapters for the delivery loop's four run writes.
//!
//! The handler parses and authorizes only. Vocabulary validation, the phase
//! machine, the CAS, the trace and budget writes, and the fail-closed audit
//! live in `crate::workflow::delivery`.
//!
//! The gate order is the same on all four routes and is not an accident:
//! `run_domain` (probe-blind 404 on an absent or foreign run) → `authorize`
//! (Write on the run's OWN domain) → pool → `authorize_role` (the `workflow`
//! role, which reads the role store from the pool) → the core. Authorize
//! before the lookup, and never let an error distinguish "absent" from
//! "someone else's".
//!
//! The read seam runs once on each assembled response, at the emission
//! boundary. Nothing here holds SQL.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
};
use serde::Deserialize;

use crate::AppState;
use crate::handlers::HandlerError;
use crate::handlers::auth::OptPrincipal;
use crate::workflow::delivery::{self, BudgetCeiling, DeliveryError};

/// Open a delivery run. The only route with no run id: it is the admission.
/// The client names the domain and the scope gate is checked against it — the
/// same admission shape the case-launch route uses, so a principal cannot open
/// a run in a domain it cannot write to.
#[derive(Debug, Deserialize)]
pub struct CreateRunBody {
    pub domain: String,
    pub goal: String,
    pub tier: String,
    #[serde(default)]
    pub policy_digest: Option<String>,
    #[serde(default)]
    pub config_digest: Option<String>,
    #[serde(default)]
    pub budgets: Vec<BudgetCeiling>,
}

pub async fn post_delivery_run(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Json(body): Json<CreateRunBody>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    super::authorize(&principal, crate::auth::Action::Write, "", &body.domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize_role(&principal, &pool, "workflow")?;

    let who = super::recall::principal_label(&principal);
    let now = chrono::Utc::now().timestamp();
    let CreateRunBody {
        domain,
        goal,
        tier,
        policy_digest,
        config_digest,
        budgets,
    } = body;

    let created = tokio::task::spawn_blocking(move || {
        let mut conn = pool.get().map_err(HandlerError::db_down)?;
        delivery::create_run(
            &mut conn,
            &delivery::CreateRun {
                domain: &domain,
                goal: &goal,
                tier: &tier,
                policy_digest: policy_digest.as_deref(),
                config_digest: config_digest.as_deref(),
                budgets: &budgets,
                now,
            },
        )
        .map_err(delivery_error)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))??;

    let mut response =
        serde_json::to_value(created).map_err(|error| HandlerError::internal(error.to_string()))?;
    response["principal"] = serde_json::Value::String(who);
    super::sanitize_value_strings(&mut response);
    Ok(Json(response))
}

/// Advance one phase: the step row, the CAS, the trace, and the audit in one
/// transaction.
#[derive(Debug, Deserialize)]
pub struct AdvanceBody {
    pub expected_revision: i64,
    pub to_phase: String,
    #[serde(default)]
    pub artifact_refs: Vec<String>,
    /// The typed artifact this pass carries, if any. Absent is the previous
    /// request body unchanged — the field is additive and defaults to absent,
    /// so an existing client sends exactly the same bytes.
    #[serde(default)]
    pub artifact: Option<ArtifactBody>,
}

/// The typed artifact over the wire. The body carries the id, the content, and
/// the checkpoint gate; the SHA-256 digest is NOT accepted from the caller —
/// it is derived server-side by the shipped engine, so a client cannot name the
/// digest of an artifact the server did not derive.
#[derive(Debug, Deserialize)]
pub struct ArtifactBody {
    pub id: String,
    pub content: String,
    #[serde(default)]
    pub quality_gate: Option<String>,
}

pub async fn post_delivery_advance(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
    Json(body): Json<AdvanceBody>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    // Authorize before the core opens a transaction. The domain read is the
    // only thing that precedes it, and it is probe-blind.
    let domain = super::workflow::run_domain(&state, id).await?;
    super::authorize(&principal, crate::auth::Action::Write, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize_role(&principal, &pool, "workflow")?;

    let actor = super::recall::principal_label(&principal);
    let now = chrono::Utc::now().timestamp();

    // The artifact is UNTRUSTED input at this boundary: it is executor-produced
    // and arrives from a client, so it is screened exactly as `/propose`
    // screens its content. Reject is a 400 and quarantine is a 409 — the same
    // two answers the propose seam gives, for the same reason.
    let artifact = body
        .artifact
        .map(|a| {
            let verdict = crate::screen::screen(&a.content, &a.id);
            if verdict == crate::screen::ScreenResult::Reject {
                return Err(HandlerError::bad_request(
                    "artifact_screened_reject",
                    "the artifact content was refused by the content screen",
                ));
            }
            if verdict == crate::screen::ScreenResult::Quarantine {
                return Err(HandlerError::conflict("artifact_screened_quarantine"));
            }
            Ok(delivery::DeliveryArtifact {
                id: a.id,
                content: a.content,
                quality_gate: a.quality_gate,
            })
        })
        .transpose()?;

    let advanced = tokio::task::spawn_blocking(move || {
        let mut conn = pool.get().map_err(HandlerError::db_down)?;
        delivery::advance(
            &mut conn,
            &delivery::Advance {
                run_id: id,
                expected_revision: body.expected_revision,
                to_phase: &body.to_phase,
                artifact_refs: &body.artifact_refs,
                artifact: artifact.as_ref(),
                actor: &actor,
                now,
            },
        )
        .map_err(delivery_error)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))??;

    let mut response = serde_json::to_value(advanced)
        .map_err(|error| HandlerError::internal(error.to_string()))?;
    super::sanitize_value_strings(&mut response);
    Ok(Json(response))
}

/// Answer the run's pending question.
#[derive(Debug, Deserialize)]
pub struct AnswerBody {
    pub expected_revision: i64,
    pub answer: String,
}

pub async fn post_delivery_answer(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
    Json(body): Json<AnswerBody>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = super::workflow::run_domain(&state, id).await?;
    super::authorize(&principal, crate::auth::Action::Write, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize_role(&principal, &pool, "workflow")?;

    let actor = super::recall::principal_label(&principal);
    let now = chrono::Utc::now().timestamp();
    let answered = tokio::task::spawn_blocking(move || {
        let mut conn = pool.get().map_err(HandlerError::db_down)?;
        delivery::answer(
            &mut conn,
            &delivery::Answer {
                run_id: id,
                expected_revision: body.expected_revision,
                answer: &body.answer,
                actor: &actor,
                now,
            },
        )
        .map_err(delivery_error)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))??;

    let mut response = serde_json::to_value(answered)
        .map_err(|error| HandlerError::internal(error.to_string()))?;
    super::sanitize_value_strings(&mut response);
    Ok(Json(response))
}

/// Evaluate the phase gate. A disposition, never a mutation.
#[derive(Debug, Deserialize)]
pub struct GatesBody {
    #[serde(default)]
    pub to_phase: Option<String>,
}

pub async fn post_delivery_gates(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
    Json(body): Json<GatesBody>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = super::workflow::run_domain(&state, id).await?;
    super::authorize(&principal, crate::auth::Action::Write, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize_role(&principal, &pool, "workflow")?;

    let actor = super::recall::principal_label(&principal);
    let now = chrono::Utc::now().timestamp();
    let verdict = tokio::task::spawn_blocking(move || {
        let mut conn = pool.get().map_err(HandlerError::db_down)?;
        delivery::gates(
            &mut conn,
            &delivery::Gates {
                run_id: id,
                to_phase: body.to_phase.as_deref(),
                actor: &actor,
                now,
            },
        )
        .map_err(delivery_error)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))??;

    let mut response =
        serde_json::to_value(verdict).map_err(|error| HandlerError::internal(error.to_string()))?;
    super::sanitize_value_strings(&mut response);
    Ok(Json(response))
}

/// The typed error → HTTP mapping. Absence is 404 on every route, and a
/// non-delivery run reads as absent rather than as a wrong-kind error, so the
/// mapping never becomes an existence oracle.
fn delivery_error(error: DeliveryError) -> HandlerError {
    match error {
        DeliveryError::RunAbsent => HandlerError::not_found("delivery run not found"),
        DeliveryError::UnknownVocabulary { .. } => HandlerError::bad_request(
            "delivery_unknown_vocabulary",
            "a value is outside its closed vocabulary",
        ),
        DeliveryError::Stale { .. } => HandlerError::conflict("delivery_gate_stale_revision"),
        DeliveryError::IllegalPhaseTransition { .. } => {
            HandlerError::conflict("delivery_illegal_phase_transition")
        }
        DeliveryError::TerminalPhase { .. } => HandlerError::conflict("delivery_terminal_phase"),
        DeliveryError::NoPendingQuestion => HandlerError::conflict("delivery_no_pending_question"),
        DeliveryError::QuestionPending => HandlerError::conflict("delivery_question_pending"),
        DeliveryError::TooLong { .. } => HandlerError::bad_request(
            "delivery_input_too_long",
            "a bounded input exceeded its cap",
        ),
        DeliveryError::TooMany { .. } => HandlerError::bad_request(
            "delivery_input_too_many",
            "a bounded collection exceeded its cap",
        ),
        DeliveryError::QualityGate { .. } => {
            HandlerError::conflict("delivery_quality_gate_refused")
        }
        DeliveryError::Storage(detail) => HandlerError::internal(detail),
    }
}
