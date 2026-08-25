//! The Mesh surfaces: signed agent cards + agent→agent delegation.
//!
//! - `POST /ops/agents/cards` — provision (or re-sign) an agent's card with
//!   the UMP operator key (Admin on the domain).
//! - `GET /ops/agents/cards[?domain=]` and `/ops/agents/cards/{principal}`
//!   — verified-at-read card views; a card whose signature fails refuses.
//! - `POST /workflow/runs/{id}/delegations {to_principal, task}` — verify the
//!   target's card, then one row + lineage event + audit in-tx.
//! - `GET /workflow/runs/{id}/delegations` — the delegation view.
//! - `POST /workflow/runs/{id}/delegations/{delegation_id}/result` — the
//!   delegated agent's exactly-once result.

use axum::{
    Json,
    extract::{Path, Query, State},
};
use std::collections::HashMap;
use std::sync::Arc;

use crate::AppState;
use crate::handlers::HandlerError;
use crate::handlers::auth::OptPrincipal;
use crate::workflow::channel;
use crate::workflow::mesh::{self, MeshError};

fn mesh_err(e: MeshError) -> HandlerError {
    match e {
        MeshError::NoOperatorKey => HandlerError::conflict_with(
            "operator_key_missing",
            "no operator signing key — agent cards refuse to provision or verify",
            serde_json::json!({ "note": "provision BRAIN_UMP_KEY_DIR to enable Mesh" }),
        ),
        MeshError::CardTampered(p) => HandlerError::bad_request_with(
            "card_tampered",
            "the agent card fails signature verification",
            serde_json::json!({ "principal": crate::gate::sanitize_read(&p, false, &None) }),
        ),
        MeshError::CardUnknown(p) => HandlerError::bad_request_with(
            "agent_unknown",
            "no verified agent card for this principal in this domain",
            serde_json::json!({ "principal": crate::gate::sanitize_read(&p, false, &None) }),
        ),
        MeshError::InvalidInput(what, why) => {
            HandlerError::bad_request("input_invalid", format!("invalid {what}: {why}"))
        }
        MeshError::DelegationsFull => HandlerError::conflict_with(
            "delegations_full",
            "this run reached its delegation ceiling — close out pending work first",
            serde_json::json!({ "cap": mesh::MAX_DELEGATIONS_PER_RUN }),
        ),
        MeshError::NotFound(w) => HandlerError::not_found(w),
        MeshError::NotDelegatee(_) => HandlerError::bad_request(
            "not_delegatee",
            "only the delegated agent may submit the result",
        ),
        MeshError::AlreadyCompleted => {
            HandlerError::conflict("this delegation already returned its result")
        }
        MeshError::Database(m) => HandlerError::internal(m),
    }
}

/// Presence rides every mutating mesh tx (best-effort, never gates).
fn crew_touch(conn: &rusqlite::Connection, domain: &str, actor: &str, run_id: i64) {
    if let Err(e) = crate::workflow::crew::touch(
        conn,
        domain,
        actor,
        "cranking",
        Some(&format!("run:{run_id}")),
        &[],
        chrono::Utc::now().timestamp(),
    ) {
        tracing::warn!(run = run_id, "presence touch failed: {e}");
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CardRequest {
    pub domain: String,
    pub principal: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_capabilities")]
    pub capabilities: serde_json::Value,
}

fn default_capabilities() -> serde_json::Value {
    serde_json::json!({})
}

/// `POST /ops/agents/cards`
pub async fn post_card(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Json(body): Json<CardRequest>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    super::authorize(&principal, crate::auth::Action::Admin, "", &body.domain)?;
    let capabilities_json = if body.capabilities.is_object() {
        body.capabilities.to_string()
    } else {
        return Err(HandlerError::bad_request(
            "capabilities_invalid",
            "capabilities must be a JSON object",
        ));
    };
    let now = chrono::Utc::now().timestamp();
    let card = tokio::task::spawn_blocking(move || -> Result<_, HandlerError> {
        let draft = mesh::CardDraft {
            domain: &body.domain,
            principal: &body.principal,
            name: &body.name,
            description: &body.description,
            capabilities_json: &capabilities_json,
        };
        let pool = super::resolve_domain_pool(&state.registry, None)?;
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        let mut tx = crate::workflow::tx::WorkflowTx::begin(&mut conn)
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        let card = mesh::provision_card(tx.tx(), &draft, now).map_err(mesh_err)?;
        tx.commit()
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        Ok(card)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))??;

    Ok(Json(card_view(&card, &principal)))
}

fn card_view(card: &mesh::AgentCard, viewer: &Option<crate::auth::Principal>) -> serde_json::Value {
    let s = |v: &str| crate::gate::sanitize_read(v, false, viewer);
    serde_json::json!({
        "principal": s(&card.principal),
        "domain": s(&card.domain),
        "name": s(&card.name),
        "description": s(&card.description),
        "capabilities": serde_json::from_str::<serde_json::Value>(&card.capabilities_json)
            .unwrap_or(serde_json::json!({})),
        "signed_by": s(&card.signed_by),
        // A2A-shaped manifest + its signature: consumers re-verify independently.
        "card": serde_json::from_str::<serde_json::Value>(&card.card_json)
            .unwrap_or(serde_json::json!({})),
        "signature": card.signature_hex,
    })
}

/// `GET /ops/agents/cards?domain=`
pub async fn get_cards(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = params
        .get("domain")
        .cloned()
        .unwrap_or_else(|| "global".to_string());
    super::authorize(&principal, crate::auth::Action::Read, "", &domain)?;
    let cards = tokio::task::spawn_blocking(move || -> Result<_, HandlerError> {
        let pool = super::resolve_domain_pool(&state.registry, None)?;
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        let cards = mesh::list_cards(&conn, &domain).map_err(mesh_err)?;
        Ok((domain, cards))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?;
    let (domain, cards) = cards?;
    let payload: Vec<serde_json::Value> = cards.iter().map(|c| card_view(c, &principal)).collect();
    Ok(Json(serde_json::json!({
        "domain": crate::gate::sanitize_read(&domain, false, &principal),
        "count": payload.len(),
        "cards": payload,
    })))
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationRequest {
    pub to_principal: String,
    pub task: String,
}

/// `POST /workflow/runs/{id}/delegations`
pub async fn post_delegation(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
    Json(body): Json<DelegationRequest>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = super::workflow::run_domain(&state, id).await?;
    super::authorize(&principal, crate::auth::Action::Write, "", &domain)?;
    let screened = channel::screen_content(&body.task).map_err(super::channel::channel_err)?;
    let actor = super::recall::principal_label(&principal);
    let now = chrono::Utc::now().timestamp();

    let outcome = tokio::task::spawn_blocking(move || -> Result<_, HandlerError> {
        let pool = super::resolve_domain_pool(&state.registry, None)?;
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        let mut tx = crate::workflow::tx::WorkflowTx::begin(&mut conn)
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        let key_suffix = format!("{now}-{}", rand::random::<u32>());
        let out = mesh::request_delegation(
            tx.tx(),
            &mesh::DelegationDraft {
                domain: &domain,
                run_id: id,
                from_principal: &actor,
                to_principal: &body.to_principal,
                screened_task: &screened,
                key_suffix: &key_suffix,
                now,
            },
        )
        .map_err(mesh_err)?;
        let to_label = body.to_principal.clone();
        crew_touch(tx.tx(), &domain, &actor, id);
        tx.commit()
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        Ok((out, to_label))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?;
    let (out, to_label) = outcome?;

    Ok(Json(serde_json::json!({
        "run_id": id,
        "delegation_id": out.delegation_id,
        "event_id": out.event_id,
        "to": crate::gate::sanitize_read(&to_label, false, &principal),
        "agent_name": crate::gate::sanitize_read(&out.card.name, false, &principal),
        "state": mesh::STATE_REQUESTED,
    })))
}

/// `GET /workflow/runs/{id}/delegations?limit=&offset=`
pub async fn get_delegations(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = super::workflow::run_domain(&state, id).await?;
    super::authorize(&principal, crate::auth::Action::Read, "", &domain)?;
    let parse_num = |k: &str| -> Result<Option<i64>, HandlerError> {
        match params.get(k).map(|s| s.parse::<i64>()) {
            Some(Ok(n)) => Ok(Some(n)),
            Some(Err(_)) => Err(HandlerError::bad_request(
                "param_invalid",
                format!("{k} must be an integer"),
            )),
            None => Ok(None),
        }
    };
    let limit = parse_num("limit")?.unwrap_or(200);
    let offset = parse_num("offset")?.unwrap_or(0);

    let rows = tokio::task::spawn_blocking(move || -> Result<_, HandlerError> {
        let pool = super::resolve_domain_pool(&state.registry, None)?;
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        mesh::list_delegations(&conn, id, offset, limit).map_err(mesh_err)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?;
    let rows = rows?;
    let payload: Vec<serde_json::Value> = rows
        .iter()
        .map(|d| {
            serde_json::json!({
                "id": d.id,
                "from": crate::gate::sanitize_read(&d.from_principal, false, &principal),
                "to": crate::gate::sanitize_read(&d.to_principal, false, &principal),
                "task": crate::gate::sanitize_read(&d.task, false, &principal),
                "state": d.state,
                "result": d.result.as_deref().map(|r| crate::gate::sanitize_read(r, false, &principal)),
                "created_at": d.created_at,
                "decided_at": d.decided_at,
            })
        })
        .collect();
    Ok(Json(serde_json::json!({
        "run_id": id,
        "count": payload.len(),
        "delegations": payload,
    })))
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationResultRequest {
    pub result: String,
}

/// `POST /workflow/runs/{id}/delegations/{delegation_id}/result`
pub async fn post_delegation_result(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    path: Path<(i64, i64)>,
    Json(body): Json<DelegationResultRequest>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let (id, delegation_id) = *path;
    let principal = principal.0;
    let domain = super::workflow::run_domain(&state, id).await?;
    super::authorize(&principal, crate::auth::Action::Write, "", &domain)?;
    let screened = channel::screen_content(&body.result).map_err(super::channel::channel_err)?;
    let actor = super::recall::principal_label(&principal);
    let now = chrono::Utc::now().timestamp();

    let event_id = tokio::task::spawn_blocking(move || -> Result<_, HandlerError> {
        let pool = super::resolve_domain_pool(&state.registry, None)?;
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        let mut tx = crate::workflow::tx::WorkflowTx::begin(&mut conn)
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        let event_id = mesh::submit_result(tx.tx(), id, delegation_id, &actor, &screened, now)
            .map_err(mesh_err)?;
        crew_touch(tx.tx(), &domain, &actor, id);
        tx.commit()
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        Ok(event_id)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?;

    Ok(Json(serde_json::json!({
        "run_id": id,
        "delegation_id": delegation_id,
        "event_id": event_id?,
        "state": mesh::STATE_COMPLETED,
    })))
}
