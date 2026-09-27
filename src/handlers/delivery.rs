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

/// The read route's body. There is NO write route for bindings, and that
/// absence is the design: a secret-file reference must never ride a request,
/// and consent to an external authority is given by configuring a binding and
/// withdrawn with `active = 0` — neither a request-time operation.
#[derive(Debug, serde::Serialize)]
pub struct BindingsView {
    pub bindings: Vec<serde_json::Value>,
    /// The pending census, split by what the read side makes of each row. They
    /// sit `pending` with no reader BY DESIGN, and reporting the split is what
    /// keeps a lost intent, an un-promoted one, and a FORGED one
    /// distinguishable.
    pub intents_pending: i64,
    pub observed_pending: i64,
    /// Delivery-family rows whose key is not a kernel mint.
    pub untrusted_pending: i64,
}

/// Read the delivery bindings configured for a domain.
///
/// Gate order matches the other delivery reads: the domain comes from the
/// QUERY (there is no run to scope it to), `authorize` Read on that domain,
/// then the `workflow` role. It is a GET because it serves stored rows and
/// takes no body — and because a body is where a caller would try to name an
/// endpoint or a secret path.
pub async fn get_delivery_bindings(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = q
        .get("domain")
        .map(String::as_str)
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .ok_or_else(|| HandlerError::bad_request("domain_required", "domain is required"))?
        .to_string();
    super::authorize(&principal, crate::auth::Action::Read, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, Some(&domain))?;
    super::authorize_role(&principal, &pool, "workflow")?;

    let rows = tokio::task::spawn_blocking(move || {
        let conn = pool.get().map_err(HandlerError::db_down)?;
        let bindings = crate::connector::delivery::list_bindings(&conn, &domain)
            .map_err(|e| HandlerError::conflict(e.to_string()))?;
        let pending = crate::connector::delivery::pending_intent_census(&conn, &domain)
            .map_err(|e| HandlerError::conflict(e.to_string()))?;
        Ok::<_, HandlerError>((bindings, pending))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))??;

    let (bindings, census) = rows;
    // The read seam is unconditional. `list_bindings` already omits the secret
    // file name, so there is nothing sensitive here to strip — but the seam
    // applies anyway, because "there is nothing sensitive today" is a property
    // a later column could change and the seam would then be the thing that
    // catches it.
    let mut response = serde_json::to_value(BindingsView {
        bindings: bindings
            .into_iter()
            .map(|b| serde_json::to_value(b).unwrap_or(serde_json::Value::Null))
            .collect(),
        intents_pending: census.intents,
        observed_pending: census.observed,
        untrusted_pending: census.untrusted,
    })
    .map_err(|error| HandlerError::internal(error.to_string()))?;
    super::sanitize_value_strings(&mut response);
    Ok(Json(response))
}

// ── the release family ──────────────────────────────────────────────────────
//
// The first route family whose writes LEAVE THE HOST. Three properties, all
// deliberate:
//
// * The agent preset is refused EXPLICITLY, before any other work (the
//   self-launch precedent: no probe oracle either). An agent holds `write:*`
//   and `authorize_role` only restricts principals that hold roles at all —
//   so without this refusal the role gate alone would admit an agent to the
//   one write whose consequences reach another system.
// * The domain is resolved BEFORE the scope gate and an absent release reads
//   exactly like a foreign one (probe-blind 404).
// * The approving principal is RECORDED from the authenticated caller, never
//   asserted from the body: the approval columns are an artifact of a
//   specific human act.

/// The agent-class refusal. Runs before any domain read, so the refusal
/// cannot be turned into an existence oracle either.
fn refuse_agent(principal: &crate::auth::policy::Principal) -> Result<(), HandlerError> {
    if principal.kind == crate::auth::policy::PrincipalKind::AgentLoopback {
        return Err(HandlerError::forbidden(
            crate::auth::Action::Write,
            &principal.tenant,
            "global",
        ));
    }
    Ok(())
}

/// The release's run's domain, resolved before authorization so an absent
/// release and a foreign one are the same 404. Returns the domain and the
/// run id (the approve/promote audit rows tenant on the domain).
async fn release_run_domain(
    state: &Arc<AppState>,
    release_id: i64,
) -> Result<(String, i64), HandlerError> {
    let pool = state.pool.clone();
    tokio::task::spawn_blocking(move || -> Result<Option<(String, i64)>, String> {
        let conn = pool.get().map_err(|e| format!("{e}"))?;
        crate::workflow::releases::release_run_domain(&conn, release_id).map_err(|e| format!("{e}"))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?
    .map_err(HandlerError::internal)?
    .ok_or_else(|| HandlerError::not_found("delivery release not found"))
}

/// File a release: the kernel names everything that binds (the artifact
/// digest from the run's own bytes, the binding from the run's own domain);
/// the caller names only the run, the target kind, and the governed ref.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateReleaseBody {
    pub run_id: i64,
    pub target_kind: String,
    #[serde(rename = "ref")]
    pub ref_name: String,
    pub environment: String,
    #[serde(default)]
    pub commit_sha: Option<String>,
}

pub async fn post_delivery_release(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Json(body): Json<CreateReleaseBody>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    if let Some(p) = &principal {
        refuse_agent(p)?;
    }
    let domain = super::workflow::run_domain(&state, body.run_id).await?;
    super::authorize(&principal, crate::auth::Action::Write, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize_role(&principal, &pool, "workflow")?;

    let who = super::recall::principal_label(&principal);
    let now = chrono::Utc::now().timestamp();
    let created = tokio::task::spawn_blocking(move || {
        let mut conn = pool.get().map_err(HandlerError::db_down)?;
        crate::workflow::releases::create_release(
            &mut conn,
            &crate::workflow::releases::CreateRelease {
                run_id: body.run_id,
                target_kind: &body.target_kind,
                ref_name: &body.ref_name,
                environment: &body.environment,
                commit_sha: body.commit_sha.as_deref(),
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

/// Record the approval. The kernel writes the three-way binding from its own
/// stored facts: the content digest from the release row, the authority
/// digest from the binding row as it is NOW, and the run's state revision as
/// it is NOW. The window starts here — `approved_at` — because the gap
/// between approve and promote is exactly what the expiry must bound.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApproveReleaseBody {
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub ttl_secs: Option<i64>,
}

pub async fn post_delivery_release_approve(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
    Json(body): Json<ApproveReleaseBody>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    if let Some(p) = &principal {
        refuse_agent(p)?;
    }
    let (domain, _run_id) = release_run_domain(&state, id).await?;
    super::authorize(&principal, crate::auth::Action::Write, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize_role(&principal, &pool, "workflow")?;

    let who = super::recall::principal_label(&principal);
    let now = chrono::Utc::now().timestamp();
    let scope = body.scope.unwrap_or_else(|| "promote".to_string());
    let approved = tokio::task::spawn_blocking(move || {
        let mut conn = pool.get().map_err(HandlerError::db_down)?;
        crate::workflow::releases::approve_release(
            &mut conn,
            &crate::workflow::releases::ApproveRelease {
                release_id: id,
                scope: &scope,
                ttl_secs: body.ttl_secs,
                principal: &who,
                now,
            },
        )
        .map_err(delivery_error)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))??;

    let mut response = serde_json::to_value(approved)
        .map_err(|error| HandlerError::internal(error.to_string()))?;
    super::sanitize_value_strings(&mut response);
    Ok(Json(response))
}

/// The promotion gate. A deny or an unanswered prompt changes nothing; a
/// permitted promotion walks the crate's transition law inside one
/// transaction, lands `promoted`, and mints the dispatch intents — the
/// network is touched later, by the crank, never inside the transaction.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromoteReleaseBody {
    #[serde(default)]
    pub confirm: Option<bool>,
}

pub async fn post_delivery_release_promote(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
    Json(body): Json<PromoteReleaseBody>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    if let Some(p) = &principal {
        refuse_agent(p)?;
    }
    let (domain, _run_id) = release_run_domain(&state, id).await?;
    super::authorize(&principal, crate::auth::Action::Write, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize_role(&principal, &pool, "workflow")?;

    let who = super::recall::principal_label(&principal);
    let now = chrono::Utc::now().timestamp();
    let confirm = body.confirm.unwrap_or(false);
    let verdict = tokio::task::spawn_blocking(move || {
        let mut conn = pool.get().map_err(HandlerError::db_down)?;
        crate::workflow::releases::promote_release(
            &mut conn,
            &crate::workflow::releases::PromoteRelease {
                release_id: id,
                confirm,
                actor: &who,
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

/// The /due crank: the valet precedent, transplanted. Request-scoped (no
/// daemon, no scheduler — the cron recipe IS the scheduler), a bounded batch
/// that DRAINS, `remaining` reported AND audited, and a HARD in-handler batch
/// cap — no route-level limiter exists, so the cap is the egress storm's only
/// gate.
///
/// The body's domain is the scope: checked immediately after authz, before
/// any pool work, the same admission shape the run-create route uses.
///
/// THREE phases, and the middle one holds NO database connection. Phase 1
/// gathers and re-verifies (read-only). Phase 2 dispatches each verified
/// intent through the pinned adapter read-egress path — the only egress the
/// tree has — with no pooled connection held, so a slow or hanging authority
/// cannot occupy the pool. Phase 3 marks each succeeded row delivered
/// through the guarded pending->delivered write (a concurrent drain is a
/// receipt) and reports the remainder. The ledger's BELIEF moves only when
/// the inbound authority observation reconciles; the crank never writes
/// `verified_at`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DueBody {
    pub domain: String,
    #[serde(default)]
    pub now: Option<i64>,
}

pub async fn post_delivery_due(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    body: Option<Json<DueBody>>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    use crate::workflow::releases::{self, DueIntent};

    let principal = principal.0;
    if let Some(p) = &principal {
        refuse_agent(p)?;
    }
    let Json(body) =
        body.ok_or_else(|| HandlerError::bad_request("domain_required", "domain is required"))?;
    let domain = body.domain.trim().to_string();
    if domain.is_empty() {
        return Err(HandlerError::bad_request(
            "domain_required",
            "domain is required",
        ));
    }
    super::authorize(&principal, crate::auth::Action::Write, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize_role(&principal, &pool, "workflow")?;

    let now = body.now.unwrap_or_else(|| chrono::Utc::now().timestamp());
    let who = super::recall::principal_label(&principal);
    // Absent config is a REFUSAL, not an empty scope: a crank whose secret
    // root cannot be resolved is one that cannot dispatch, and treating that
    // as "nothing due" would silently skip real work.
    let secret_root = crate::config::DeliveryBindingsProfile::from_env()
        .ok()
        .flatten()
        .map(|p| p.secret_root)
        .ok_or_else(|| HandlerError::conflict("delivery_authority_not_configured"))?;

    // Phase 1 — gather + verify, one blocking hop, no network.
    type Phase1 = Result<(Vec<DueIntent>, Vec<(i64, String)>), HandlerError>;
    let verify_pool = pool.clone();
    let verify_domain = domain.clone();
    let (batch, refusals) = tokio::task::spawn_blocking(move || -> Phase1 {
        let conn = verify_pool.get().map_err(HandlerError::db_down)?;
        let batch = releases::select_due_batch(&conn, &verify_domain).map_err(delivery_error)?;
        let mut verified = Vec::new();
        let mut refusals = Vec::new();
        for item in batch {
            match releases::verify_due_intent(&conn, &item, now) {
                Ok(()) => verified.push(item),
                Err(e) => refusals.push((item.outbox_id, e.to_string())),
            }
        }
        Ok((verified, refusals))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))??;

    // Phase 2 — the network, connection-free. The pinned egress path is the
    // adapter family: exact-host-refused, bearer read at the call, never
    // stored. The binding is re-resolved and cross-checked against the
    // release's own binding id — drift between the release and the live
    // binding configuration is a failure, not a dispatch.
    let mut drained = 0usize;
    let mut failed = 0usize;
    for item in batch {
        let resolve_pool = pool.clone();
        let resolve_domain = domain.clone();
        let resolve_kind = item.target_kind.clone();
        let resolved = tokio::task::spawn_blocking(
            move || -> Result<crate::connector::delivery::Binding, String> {
                let conn = resolve_pool.get().map_err(|e| format!("{e}"))?;
                crate::connector::delivery::resolve_binding(&conn, &resolve_domain, &resolve_kind)
                    .map_err(|e| e.to_string())
            },
        )
        .await
        .map_err(|e| HandlerError::internal(format!("{e}")))?;
        let binding = match resolved {
            Ok(b) if b.id == item.binding_id => b,
            Ok(_) => {
                failed += 1;
                continue;
            }
            Err(_) => {
                failed += 1;
                continue;
            }
        };
        // The one network call, with no connection held. Both adapters'
        // facts are reduced to the same outcome that matters here: contact
        // succeeded (and what the authority said, for the response).
        let contacted: Result<(String, String), crate::connector::delivery::BindingRefused> =
            match binding.target_kind.as_str() {
                "vcs" => crate::connector::delivery::vcs::fetch_vcs_facts(&binding, &secret_root)
                    .await
                    .map(|f| (f.target_ref, f.combined_status)),
                "ci" => crate::connector::delivery::ci::fetch_ci_facts(&binding, &secret_root)
                    .await
                    .map(|f| (f.target_ref, f.conclusion)),
                _ => Err(crate::connector::delivery::BindingRefused::NotAdapted),
            };
        match contacted {
            Ok(_) => {
                // Phase 3, per item — the guarded marking.
                let mark_pool = pool.clone();
                let mark_domain = domain.clone();
                let mark_item = item.clone();
                let marked = tokio::task::spawn_blocking(move || {
                    let mut conn = mark_pool.get().map_err(HandlerError::db_down)?;
                    releases::mark_intent_delivered(&mut conn, &mark_item, &mark_domain, now)
                        .map_err(delivery_error)
                })
                .await
                .map_err(|e| HandlerError::internal(format!("{e}")))??;
                if marked {
                    drained += 1;
                } else {
                    // A concurrent drain won: a receipt, not a failure.
                }
            }
            Err(_) => {
                failed += 1;
            }
        }
    }
    let refused = refusals.len();

    // The remainder: reported AND audited — a bounded batch that does not say
    // what it left behind is a silent short drain.
    let remaining_pool = pool.clone();
    let remaining_domain = domain.clone();
    let remaining = tokio::task::spawn_blocking(move || -> Result<i64, HandlerError> {
        let conn = remaining_pool.get().map_err(HandlerError::db_down)?;
        releases::count_due(&conn, &remaining_domain).map_err(delivery_error)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))??;
    if remaining > 0 {
        let actor = who.clone();
        let remaining_detail = format!(
            "delivery due crank drained the capped batch ({drained} drained, {refused}              re-verify refusals, {failed} dispatch failures); {remaining} intent(s) remain —              re-run the crank"
        );
        let audit_domain = domain.clone();
        let audit_pool = pool.clone();
        tokio::task::spawn_blocking(move || -> Result<(), HandlerError> {
            let conn = audit_pool.get().map_err(HandlerError::db_down)?;
            crate::audit::record_tenant(
                &conn,
                crate::audit::AuditKind::Workflow,
                &actor,
                "delivery/due",
                crate::audit::AuditStatus::Ok,
                &remaining_detail,
                &audit_domain,
            );
            Ok(())
        })
        .await
        .map_err(|e| HandlerError::internal(format!("{e}")))??;
    }

    let mut response = serde_json::json!({
        "ok": true,
        "domain": domain,
        "batch_cap": releases::MAX_DUE_INTENTS,
        "drained": drained,
        "refused": refused,
        "failed": failed,
        "remaining": remaining,
    });
    super::sanitize_value_strings(&mut response);
    Ok(Json(response))
}

// ── the run read census ─────────────────────────────────────────────────────
//
// Four reads that complete the DO's unassigned surface. The gate order is the
// house order on every one: the domain resolved FIRST (probe-blind 404 on an
// absent or foreign row), then Read on it, then the pool, then the `workflow`
// role, then a bounded core read, then the read seam.

/// The domain's release rows, newest first, capped. The approval columns ride
/// the row because the row IS the approval artifact; the read seam runs once
/// over the whole response.
pub async fn get_delivery_releases(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = q
        .get("domain")
        .map(String::as_str)
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .ok_or_else(|| HandlerError::bad_request("domain_required", "domain is required"))?
        .to_string();
    super::authorize(&principal, crate::auth::Action::Read, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, Some(&domain))?;
    super::authorize_role(&principal, &pool, "workflow")?;

    let releases = tokio::task::spawn_blocking(move || {
        let conn = pool.get().map_err(HandlerError::db_down)?;
        delivery::release_list(&conn, &domain).map_err(delivery_error)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))??;

    let mut response = serde_json::to_value(releases)
        .map_err(|error| HandlerError::internal(error.to_string()))?;
    response["cap"] = serde_json::json!(delivery::MAX_CENSUS_LIST);
    super::sanitize_value_strings(&mut response);
    Ok(Json(response))
}

/// The domain's delivery runs, keyset-paginated on the id: `?limit=` (clamped
/// in the core) and `?after_id=` so a caller never sees a row twice.
pub async fn get_delivery_runs(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = q
        .get("domain")
        .map(String::as_str)
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .ok_or_else(|| HandlerError::bad_request("domain_required", "domain is required"))?
        .to_string();
    super::authorize(&principal, crate::auth::Action::Read, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, Some(&domain))?;
    super::authorize_role(&principal, &pool, "workflow")?;

    let limit = q
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(delivery::DEFAULT_RUN_LIST);
    let after_id = q.get("after_id").and_then(|v| v.parse::<i64>().ok());

    let runs = tokio::task::spawn_blocking(move || {
        let conn = pool.get().map_err(HandlerError::db_down)?;
        delivery::delivery_runs_list(&conn, &domain, limit, after_id).map_err(delivery_error)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))??;

    let mut response =
        serde_json::to_value(runs).map_err(|error| HandlerError::internal(error.to_string()))?;
    response["cap"] = serde_json::json!(delivery::MAX_RUN_LIST);
    response["default_limit"] = serde_json::json!(delivery::DEFAULT_RUN_LIST);
    super::sanitize_value_strings(&mut response);
    Ok(Json(response))
}

/// One delivery run's head. The domain resolve comes first (probe-blind), and
/// a non-delivery run reads as absent rather than as a wrong-kind error.
pub async fn get_delivery_run(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = super::workflow::run_domain(&state, id).await?;
    super::authorize(&principal, crate::auth::Action::Read, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize_role(&principal, &pool, "workflow")?;

    let run = tokio::task::spawn_blocking(move || {
        let conn = pool.get().map_err(HandlerError::db_down)?;
        delivery::run_head_view(&conn, id)
            .map_err(delivery_error)?
            .ok_or_else(|| HandlerError::not_found("delivery run not found"))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))??;

    let mut response =
        serde_json::to_value(run).map_err(|error| HandlerError::internal(error.to_string()))?;
    super::sanitize_value_strings(&mut response);
    Ok(Json(response))
}

/// One delivery run's steps, in id order, capped.
pub async fn get_delivery_run_steps(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = super::workflow::run_domain(&state, id).await?;
    super::authorize(&principal, crate::auth::Action::Read, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize_role(&principal, &pool, "workflow")?;

    let steps = tokio::task::spawn_blocking(move || {
        let conn = pool.get().map_err(HandlerError::db_down)?;
        // A non-delivery run reads as absent here too: the census is the
        // delivery family's, and the probe-blind collapse is the law.
        delivery::run_head_view(&conn, id)
            .map_err(delivery_error)?
            .ok_or_else(|| HandlerError::not_found("delivery run not found"))?;
        delivery::run_steps_view(&conn, id).map_err(delivery_error)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))??;

    let mut response =
        serde_json::to_value(steps).map_err(|error| HandlerError::internal(error.to_string()))?;
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
        DeliveryError::ReleaseAbsent => HandlerError::not_found("delivery release not found"),
        DeliveryError::ReleaseRefused { reason } => HandlerError::conflict_with(
            "delivery_release_refused",
            reason,
            serde_json::json!({ "reason": reason }),
        ),
        DeliveryError::TierMismatch { state, chain } => HandlerError::conflict_with(
            "delivery_tier_mismatch",
            format!("the run's state claims {state}; the chain's signed predicate claims {chain}"),
            serde_json::json!({ "state_tier": state, "chain_tier": chain }),
        ),
        DeliveryError::Storage(detail) => HandlerError::internal(detail),
    }
}
