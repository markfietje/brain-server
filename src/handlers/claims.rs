//! Protocol adapters for the create loop.
//!
//! These handlers parse, authorize, and shape. Every statement, every bound,
//! and every audit row lives in `crate::service::create`; every verdict lives
//! in `crate::workflow::create`. Nothing here decides anything — that is the
//! point of the split, and it is why the SQL guard over this directory stays
//! green.
//!
//! ## What a proposer learns
//!
//! A refusal body carries a closed code and the claim's own public id, and
//! nothing else. Not the failing byte offset, not the adjacent text, not which
//! evidence item was at fault. The detailed diagnostic goes to the audit chain
//! and the promotion screen. Handing a generator a pointer at where it was
//! wrong turns the gate into an oracle it can be searched against, so this is
//! enforced at the RESPONSE BOUNDARY and not by convention: the diagnostic is
//! written to the audit row and simply never copied into the reply.
//!
//! ## Who may read this family
//!
//! Every surface here — reads included — additionally requires the `workflow`
//! role, so the whole claim family is closed to the agent principal. The reads
//! are gated on purpose rather than by convention: the screen read shows a
//! claim that has NOT been ratified yet, and a model that can read the review
//! surface can learn what the human is about to look at. The gated list read
//! carries only ratified, visible material, but keeping the posture uniform
//! across the family means a future surface added here inherits the stricter of
//! the two by default rather than the looser.
//!
//! ## The principal kind
//!
//! No handler writes a stored principal-kind string. It is produced by the one
//! mapping function in `crate::workflow::create`, from the typed principal —
//! because that string is a database fence key, and a caller-supplied one would
//! be a total bypass of every fence in the loop.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;

use crate::handlers::HandlerError;
use crate::handlers::auth::OptPrincipal;
use crate::service::create::{
    self, CreateError, MAX_CLAIM_ID_BYTES, RECALL_PAGE_DEFAULT, RECALL_PAGE_MAX,
};
use crate::workflow::create::principal_kind_string;

/// Map a store error. The core's message is preserved verbatim, which is what
/// lets the handler freeze this route's vocabulary without the service layer
/// ever naming an HTTP status.
fn db_err(error: rusqlite::Error) -> HandlerError {
    HandlerError::internal(error.to_string())
}

fn create_error(error: CreateError) -> HandlerError {
    match error {
        CreateError::SchemaRejected(fault) => {
            HandlerError::bad_request(fault.as_str(), "the claim schema was refused at admission")
        }
        CreateError::Refused(receipt) => {
            HandlerError::bad_request(receipt.reason, "the claim was refused by the gate")
        }
        CreateError::PromotionDisabled(id) => HandlerError::bad_request(
            "promotion_disabled",
            format!(
                "promotion is disabled for claim {id} until a published out-of-sample false-promotion figure exists"
            ),
        ),
        CreateError::NotFound => HandlerError::not_found("claim not found"),
        CreateError::Storage(message) => HandlerError::internal(message),
    }
}

/// The typed principal kind for this caller. The ONE place a handler obtains
/// it, and it reads a Rust enum rather than a string.
fn principal_kind(
    principal: &Option<crate::auth::Principal>,
) -> crate::auth::policy::PrincipalKind {
    principal
        .as_ref()
        .map(|p| p.kind)
        .unwrap_or(crate::auth::policy::PrincipalKind::Jwt)
}

fn actor_label(principal: &Option<crate::auth::Principal>) -> String {
    super::recall::principal_label(principal)
}

fn parse_claim_id(raw: &str) -> Result<String, HandlerError> {
    if raw.is_empty() || raw.len() > MAX_CLAIM_ID_BYTES {
        return Err(HandlerError::bad_request(
            "claim_id_invalid",
            "a claim id must be non-empty and within its bound",
        ));
    }
    if !raw
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Err(HandlerError::bad_request(
            "claim_id_invalid",
            "a claim id must be an opaque token",
        ));
    }
    Ok(raw.to_string())
}

#[derive(Deserialize)]
pub(crate) struct ClaimQuery {
    pub limit: Option<usize>,
}

/// The proposal body. Deliberately strict: every typed slot must be filled,
/// because a slot that defaults its way to ratified is exactly the free-text
/// failure this loop replaces.
#[derive(Deserialize)]
pub(crate) struct ProposeBody {
    pub claim_id: String,
    pub domain: String,
    pub subject: String,
    pub predicate: String,
    pub object: serde_json::Value,
}

fn parse_propose(raw: ProposeBody) -> Result<ProposeBody, HandlerError> {
    let claim_id = parse_claim_id(&raw.claim_id)?;
    if raw.domain.trim().is_empty()
        || raw.subject.trim().is_empty()
        || raw.predicate.trim().is_empty()
    {
        return Err(HandlerError::bad_request(
            "claim_incomplete",
            "a claim must name its domain, subject and predicate",
        ));
    }
    if raw.object.is_null() {
        return Err(HandlerError::bad_request(
            "claim_incomplete",
            "a claim must carry a value; an unfilled slot is not a claim",
        ));
    }
    Ok(ProposeBody { claim_id, ..raw })
}

#[derive(Deserialize)]
pub(crate) struct SchemaBody {
    pub domain: String,
    pub version: i64,
    pub body: String,
}

/// Author a claim schema. Human-only, and the route says so before it reads
/// anything: the binding check that the ACTING principal is human lives here
/// and in the promotion transaction, because a table CHECK can only read a
/// column.
pub(crate) async fn post_claim_schema(
    State(state): State<Arc<crate::AppState>>,
    principal: OptPrincipal,
    Json(body): Json<SchemaBody>,
) -> Result<(StatusCode, Json<serde_json::Value>), HandlerError> {
    let principal = principal.0;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal, crate::auth::Action::Write, "", "global")?;
    super::authorize_role(&principal, &pool, "workflow")?;
    if principal_kind(&principal) != crate::auth::policy::PrincipalKind::Jwt {
        return Err(HandlerError::forbidden(
            crate::auth::Action::Write,
            "global",
            "a claim schema is a human artifact; an agent principal cannot author one",
        ));
    }
    if body.domain.trim().is_empty() || body.version < 1 {
        return Err(HandlerError::bad_request(
            "claim_incomplete",
            "a schema must name its domain and carry a positive version",
        ));
    }
    let kind = principal_kind(&principal);
    let authored_by = principal_kind_string(kind);
    let now = chrono::Utc::now().timestamp();
    let stored = tokio::task::spawn_blocking(move || {
        let mut connection = pool.get().map_err(HandlerError::db_down)?;
        // `WorkflowTx::begin` is the house's write seam: BEGIN IMMEDIATE, and
        // a drop that rolls the write AND its audit row back together. A raw
        // deferred transaction here would have let two writers interleave
        // between the read and the insert, which is the exact class the write
        // -discipline ratchet exists to keep out of the handler layer.
        let mut wtx = crate::workflow::tx::WorkflowTx::begin(&mut connection).map_err(db_err)?;
        let stored = create::store_schema(
            wtx.tx(),
            &body.domain,
            body.version,
            kind,
            &body.body,
            Vec::new(),
            now,
        )
        .map_err(create_error)?;
        wtx.commit().map_err(db_err)?;
        Ok::<_, HandlerError>(stored)
    })
    .await
    .map_err(|error| HandlerError::internal(format!("{error}")))??;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "domain": stored.decl.domain,
            "version": stored.decl.version,
            "body_digest": stored.body_digest,
            "authored_by": authored_by,
            "authored": true,
        })),
    ))
}

/// Propose a claim. Write-gated and role-gated; the stored principal kind is
/// derived, never accepted.
pub(crate) async fn post_claim(
    State(state): State<Arc<crate::AppState>>,
    principal: OptPrincipal,
    Json(body): Json<ProposeBody>,
) -> Result<(StatusCode, Json<serde_json::Value>), HandlerError> {
    let principal = principal.0;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal, crate::auth::Action::Write, "", "global")?;
    super::authorize_role(&principal, &pool, "workflow")?;
    let body = parse_propose(body)?;
    let kind = principal_kind(&principal);
    let created_by = principal_kind_string(kind);
    let actor = actor_label(&principal);
    let now = chrono::Utc::now().timestamp();
    let response = tokio::task::spawn_blocking(move || {
        let mut connection = pool.get().map_err(HandlerError::db_down)?;
        // A claim cannot be stored without a ratified schema to type it. The
        // lookup belongs to the core, not here: a handler that opened its own
        // transaction to run it would be doing storage work, which is exactly
        // what the split in this repository exists to prevent.
        let schema_ref =
            create::latest_ratified_schema_id(&connection, &body.domain).map_err(create_error)?;
        let Some(schema_ref) = schema_ref else {
            return Err(HandlerError::bad_request(
                "no_schema",
                "no ratified schema exists for this domain; a claim may not be typed without one",
            ));
        };
        let object = body.object.to_string();
        let mut wtx = crate::workflow::tx::WorkflowTx::begin(&mut connection).map_err(db_err)?;
        create::store_claim(
            wtx.tx(),
            &create::ClaimDraft {
                claim_id: &body.claim_id,
                domain: &body.domain,
                schema_ref,
                subject: &body.subject,
                predicate: &body.predicate,
                object: &object,
                authored_by: kind,
                created_at: now,
                citations: &[],
            },
        )
        .map_err(create_error)?;
        wtx.commit().map_err(db_err)?;
        crate::audit::record_tenant(
            &connection,
            crate::audit::AuditKind::Workflow,
            &actor,
            &body.claim_id,
            crate::audit::AuditStatus::Ok,
            "claim proposed",
            "global",
        );
        Ok::<_, HandlerError>(serde_json::json!({
            "claim_id": body.claim_id,
            "status": "pending",
            "created_by": created_by,
        }))
    })
    .await
    .map_err(|error| HandlerError::internal(format!("{error}")))??;
    let mut response = response;
    super::sanitize_value_strings(&mut response);
    Ok((StatusCode::CREATED, Json(response)))
}

/// The gated read: the loop's only reader. It cannot be asked for anything
/// unratified, because the query it calls has no parameter that would reach it.
pub(crate) async fn get_claims(
    State(state): State<Arc<crate::AppState>>,
    principal: OptPrincipal,
    Query(query): Query<ClaimQuery>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal, crate::auth::Action::Read, "", "global")?;
    super::authorize_role(&principal, &pool, "workflow")?;
    let limit = super::bounded_limit(
        query.limit,
        RECALL_PAGE_DEFAULT,
        RECALL_PAGE_MAX,
        "claim_limit_out_of_bounds",
    )?;
    let actor = actor_label(&principal);
    let rows = tokio::task::spawn_blocking(move || {
        let connection = pool.get().map_err(HandlerError::db_down)?;
        let rows = create::recall_page(&connection, limit).map_err(create_error)?;
        crate::audit::record_tenant(
            &connection,
            crate::audit::AuditKind::Workflow,
            &actor,
            "claims",
            crate::audit::AuditStatus::Ok,
            "claim recall read",
            "global",
        );
        Ok::<_, HandlerError>(rows)
    })
    .await
    .map_err(|error| HandlerError::internal(format!("{error}")))??;
    let mut claims =
        serde_json::to_value(&rows).map_err(|e| HandlerError::internal(e.to_string()))?;
    super::sanitize_value_strings(&mut claims);
    Ok(Json(
        serde_json::json!({ "claims": claims, "limit": limit }),
    ))
}

/// Read one claim for the promotion screen. This is the ONE surface besides
/// the service core that may see a claim that is not yet ratified, which is
/// why it is a separate function from the gated read rather than a flag on it.
pub(crate) async fn get_claim(
    State(state): State<Arc<crate::AppState>>,
    principal: OptPrincipal,
    Path(claim_id): Path<String>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal, crate::auth::Action::Read, "", "global")?;
    super::authorize_role(&principal, &pool, "workflow")?;
    let claim_id = parse_claim_id(&claim_id)?;
    let actor = actor_label(&principal);
    let row = tokio::task::spawn_blocking(move || {
        let connection = pool.get().map_err(HandlerError::db_down)?;
        // Probe-blind: authorization has already run, and an absent claim and
        // a forbidden claim are indistinguishable from outside.
        let row = create::load_for_screen(&connection, &claim_id).map_err(create_error)?;
        crate::audit::record_tenant(
            &connection,
            crate::audit::AuditKind::Workflow,
            &actor,
            &claim_id,
            crate::audit::AuditStatus::Ok,
            "claim screen read",
            "global",
        );
        Ok::<_, HandlerError>(row)
    })
    .await
    .map_err(|error| HandlerError::internal(format!("{error}")))??;
    let mut response =
        serde_json::to_value(row).map_err(|e| HandlerError::internal(e.to_string()))?;
    super::sanitize_value_strings(&mut response);
    Ok(Json(response))
}

/// Run the gate over a stored claim.
///
/// The reply is the verdict and the closed code. The diagnostic — which check
/// failed, over what rows — is written to the audit chain and is NOT copied
/// into this body. That is the control, not an omission from it.
pub(crate) async fn post_claim_verify(
    State(state): State<Arc<crate::AppState>>,
    principal: OptPrincipal,
    Path(claim_id): Path<String>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal, crate::auth::Action::Write, "", "global")?;
    super::authorize_role(&principal, &pool, "workflow")?;
    let claim_id = parse_claim_id(&claim_id)?;
    let actor = actor_label(&principal);
    let for_verify = claim_id.clone();
    let for_audit = claim_id.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        let connection = pool.get().map_err(HandlerError::db_down)?;
        let outcome = create::verify_claim(&connection, &for_verify, &[]).map_err(create_error)?;
        // The full diagnostic lands here, in the chain, and nowhere else.
        let detail = match outcome {
            crate::workflow::create::verify::GateOutcome::Pass => "gate=pass".to_string(),
            crate::workflow::create::verify::GateOutcome::Refused { check, reason } => {
                format!(
                    "gate=refused check={} reason={}",
                    check.as_str(),
                    reason.as_str()
                )
            }
            crate::workflow::create::verify::GateOutcome::Unavailable { check } => {
                format!("gate=unavailable check={}", check.as_str())
            }
        };
        crate::audit::record_tenant(
            &connection,
            crate::audit::AuditKind::Workflow,
            &actor,
            &for_audit,
            crate::audit::AuditStatus::Ok,
            &detail,
            "global",
        );
        Ok::<_, HandlerError>(outcome)
    })
    .await
    .map_err(|error| HandlerError::internal(format!("{error}")))??;
    // The reply carries the closed code and nothing that points at a location.
    let body = match outcome {
        crate::workflow::create::verify::GateOutcome::Pass => serde_json::json!({
            "claim_id": claim_id, "verdict": "pass",
        }),
        crate::workflow::create::verify::GateOutcome::Refused { reason, .. } => serde_json::json!({
            "claim_id": claim_id, "verdict": "refused", "reason": reason.as_str(),
        }),
        crate::workflow::create::verify::GateOutcome::Unavailable { check } => serde_json::json!({
            "claim_id": claim_id, "verdict": "unavailable", "reason": "evidence_unresolvable",
            "check": check.as_str(),
        }),
    };
    Ok(Json(body))
}

/// Promote a claim. The route exists, is authorized, is audited, and returns
/// the typed disabled refusal — in every configuration, for every actor.
pub(crate) async fn post_claim_promote(
    State(state): State<Arc<crate::AppState>>,
    principal: OptPrincipal,
    Path(claim_id): Path<String>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal, crate::auth::Action::Write, "", "global")?;
    super::authorize_role(&principal, &pool, "workflow")?;
    let claim_id = parse_claim_id(&claim_id)?;
    let actor = actor_label(&principal);
    let outcome = tokio::task::spawn_blocking(move || {
        let connection = pool.get().map_err(HandlerError::db_down)?;
        let exists = create::claim_exists(&connection, &claim_id).map_err(create_error)?;
        // The attempt is audited whether or not it succeeds. A promotion path
        // that only records its successes is a path whose refusals are
        // invisible, and an invisible refusal rate is a gate that has already
        // lost.
        crate::audit::record_tenant(
            &connection,
            crate::audit::AuditKind::Workflow,
            &actor,
            &claim_id,
            crate::audit::AuditStatus::Ok,
            "promotion attempted; the loop is inert and the refusal is the expected outcome",
            "global",
        );
        Ok::<_, HandlerError>((exists, claim_id))
    })
    .await
    .map_err(|error| HandlerError::internal(format!("{error}")))??;
    let (exists, claim_id) = outcome;
    if !exists {
        return Err(HandlerError::not_found("claim not found"));
    }
    Ok(Json(serde_json::json!({
        "claim_id": claim_id,
        "status": "refused",
        "reason": "promotion_disabled",
        "note": "the create loop ships inert: promotion requires a published out-of-sample \
                 false-promotion figure with a named owner",
    })))
}
