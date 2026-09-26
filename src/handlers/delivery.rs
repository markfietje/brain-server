//! Protocol adapters for the delivery loop's four run writes and its reads.
//!
//! The handler parses and authorizes only. Vocabulary validation, the phase
//! machine, the CAS, the trace and budget writes, and the fail-closed audit
//! live in `crate::workflow::delivery`.
//!
//! The gate order is the same on all seven routes and is not an accident:
//! `run_domain` (probe-blind 404 on an absent or foreign run) → `authorize`
//! (Write on the run's OWN domain for the four writes, Read for the three
//! reads) → pool → `authorize_role` (the `workflow` role, which reads the role
//! store from the pool) → the core. Authorize before the lookup, and never let
//! an error distinguish "absent" from "someone else's".
//!
//! The read seam runs once on each assembled response, at the emission
//! boundary. Nothing here holds SQL.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
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
    /// the attestation round: the optional model binding. See [`ModelBindingBody`].
    #[serde(default)]
    pub model: Option<ModelBindingBody>,
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

/// the model-citation law: the model this pass executes under. Additive and optional —
/// absent is the previous request body unchanged. The client NAMES a binding;
/// the server resolves it through the registry and derives the digest, so a
/// client can never vouch for a model it did not run.
#[derive(Debug, Deserialize)]
pub struct ModelBindingBody {
    pub key: String,
    pub config_digest: String,
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

    let model = body.model.as_ref().map(|m| delivery::ModelBinding {
        key: m.key.clone(),
        config_digest: m.config_digest.clone(),
    });

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
                model: model.as_ref(),
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

/// Read the run's attestation chain.
///
/// The line's FIRST delivery read surface, and the gate order is the same as
/// the four writes': `run_domain` (probe-blind 404 on an absent or foreign run)
/// → `authorize` (Read on the run's OWN domain) → pool → `authorize_role` (the
/// `workflow` role) → the core.
///
/// **`?verify=1` is accepted and documented as an explicit request for the
/// IDENTICAL payload.** The chain verdict is UNCONDITIONAL: no parameter, and
/// no absence of one, can switch verification off. A non-verifying chain is
/// REPORTED per link with a named refusal, never hidden and never degraded into
/// a mark that reads as verified.
#[derive(Debug, Deserialize)]
pub struct AttestationsQuery {
    /// Accepted for explicitness. Carries no behaviour: the verdict ships
    /// whether it is present or not.
    #[serde(default)]
    pub verify: Option<String>,
}

pub async fn get_delivery_attestations(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
    Query(_query): Query<AttestationsQuery>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = super::workflow::run_domain(&state, id).await?;
    super::authorize(&principal, crate::auth::Action::Read, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize_role(&principal, &pool, "workflow")?;

    let now = chrono::Utc::now().timestamp();
    let read = tokio::task::spawn_blocking(move || {
        let conn = pool.get().map_err(HandlerError::db_down)?;
        crate::workflow::attestations::read_surface(&conn, id, now).map_err(|refusal| {
            // The chain could not be READ at all, as opposed to read and found
            // not to verify. That is a closed, typed refusal and a 409 — the
            // one case on this surface where a verdict cannot be reported, so
            // it must never be reported as a verified-looking empty chain.
            HandlerError::conflict(refusal.as_str())
        })
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))??;

    let mut response =
        serde_json::to_value(read).map_err(|error| HandlerError::internal(error.to_string()))?;
    super::sanitize_value_strings(&mut response);
    Ok(Json(response))
}

/// Re-derive the run's trace and report whether it is internally consistent.
///
/// The gate order is the same as the four writes' and the attestation read's:
/// `run_domain` (probe-blind 404 on an absent or foreign run) → `authorize`
/// (Read on the run's OWN domain) → pool → `authorize_role` (the `workflow`
/// role) → the core. It is a GET because it re-derives from stored bytes and
/// takes no body.
///
/// **What the verdict is, stated here because the route name invites more.**
/// It is tamper EVIDENCE over stored bytes: for every trace row, in ordinal
/// order, the row's content address recomputed from its own stored columns is
/// compared with the address stored beside it. It is NOT tamper-proofing — an
/// attacker who edits a column AND recomputes the address leaves no trace
/// here. It does NOT bind the row to the signed attestation chain; the chain is
/// what binds, and this checks. And it is NOT a compliance finding: a
/// byte-identical run is a statement about internal consistency, nothing more.
///
/// A mismatch is DATA. It is reported in the payload as a diff row and the
/// request still succeeds — a report that turned a finding into an error status
/// would tell a reader less than the finding itself does.
pub async fn get_delivery_replay_verify(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = super::workflow::run_domain(&state, id).await?;
    super::authorize(&principal, crate::auth::Action::Read, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize_role(&principal, &pool, "workflow")?;

    let now = chrono::Utc::now().timestamp();
    let report = tokio::task::spawn_blocking(move || {
        let conn = pool.get().map_err(HandlerError::db_down)?;
        delivery::replay_verify(&conn, id, now).map_err(delivery_error)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))??;

    let mut response =
        serde_json::to_value(report).map_err(|error| HandlerError::internal(error.to_string()))?;
    super::sanitize_value_strings(&mut response);
    Ok(Json(response))
}

/// The run's stored trace rows in ordinal order, plus the chain head and the
/// narrative appendix.
///
/// Same gate order, same read function, same seams as the verdict above — the
/// two surfaces ride ONE read so they can never disagree about what is stored.
/// Where the verdict answers "is this consistent", this answers "what is
/// actually there", which is the question a reader has when the verdict says
/// something did not line up.
pub async fn get_delivery_trace(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = super::workflow::run_domain(&state, id).await?;
    super::authorize(&principal, crate::auth::Action::Read, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize_role(&principal, &pool, "workflow")?;

    let now = chrono::Utc::now().timestamp();
    let listing = tokio::task::spawn_blocking(move || {
        let conn = pool.get().map_err(HandlerError::db_down)?;
        delivery::trace_listing(&conn, id, now).map_err(delivery_error)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))??;

    let mut response =
        serde_json::to_value(listing).map_err(|error| HandlerError::internal(error.to_string()))?;
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
        // the attestation round: the attestation layer's own closed vocabulary, mapped 1:1. A
        // refused key and an absent key are different codes because they are
        // different operator problems, and neither degrades into a 500.
        DeliveryError::AttestationRefused { .. } => {
            HandlerError::conflict("delivery_attestation_refused")
        }
        DeliveryError::ModelNotRegistered => {
            HandlerError::conflict("delivery_model_not_registered")
        }
        DeliveryError::ModelNotPromoted => HandlerError::conflict("delivery_model_not_promoted"),
        DeliveryError::ModelRetired => HandlerError::conflict("delivery_model_retired"),
        DeliveryError::ModelDigestMissing => {
            HandlerError::conflict("delivery_model_digest_missing")
        }
        DeliveryError::Storage(detail) => HandlerError::internal(detail),
    }
}
