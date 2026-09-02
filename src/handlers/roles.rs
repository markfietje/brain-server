//! the role API.
//!
//! `GET /roles` (list, seeded presets + operator-defined), `GET
//! /roles/{name}`, and `POST /roles/{name}` (define/clone/edit, Admin +
//! audited). Reads are `Read` at global scope (roles are server config, not
//! domain data); writes are Admin + audited. Role *binding* to a principal
//! happens in the IdP (`roles` claim) or an admin — there is no `principals`
//! table; the honest ceiling in the plan (a managed directory is v2.x SCIM).
//!
//! The data + action gates read the same store: `handlers::authorize_role` and
//! `handlers::gate::role_retrieval_gate` resolve the bundles held by a JWT
//! principal, so an edit here takes effect at the next request (no restart).

use axum::Json;
use axum::extract::{Path, State};
use serde::Deserialize;
use std::sync::Arc;

use crate::AppState;
use crate::handlers::HandlerError;
use crate::handlers::auth::OptPrincipal;

fn map_err(e: String) -> HandlerError {
    HandlerError::internal(format!("role store: {e}"))
}

/// `GET /roles` — every role, name-ordered (the operator pick list).
pub async fn list_roles(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
) -> Result<Json<serde_json::Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Read, "", "global")?;
    let pool = state.pool.clone();
    let roles = tokio::task::spawn_blocking(move || -> Result<_, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        brain_server::role::list(&conn).map_err(map_err)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    Ok(Json(serde_json::json!({ "roles": roles })))
}

/// `GET /roles/{name}` — one role (404 when unknown).
pub async fn get_role(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Read, "", "global")?;
    if !brain_server::role::is_valid_role_name(&name) {
        return Err(HandlerError::bad_request(
            "role_invalid",
            "role name must be lowercase alnum + hyphen (max 63)",
        ));
    }
    let pool = state.pool.clone();
    let name_for_err = name.clone();
    let r = tokio::task::spawn_blocking(move || -> Result<_, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        brain_server::role::load(&conn, &name).map_err(map_err)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    match r {
        Some(r) => Ok(Json(serde_json::to_value(r).unwrap_or_default())),
        None => Err(HandlerError::not_found(format!(
            "no role named '{name_for_err}'"
        ))),
    }
}

/// `POST /roles/{name}` body — a Role minus the name (the path is the name; a
/// `name` field in the body, if present, is ignored). Absent vector fields
/// default to empty (deny-by-default for reads + actions).
#[derive(Debug, Default, Deserialize)]
pub struct RoleUpsertRequest {
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub owner_filter: Option<String>,
    #[serde(default)]
    pub can: Vec<String>,
    #[serde(default)]
    pub panels_default: Option<Vec<String>>,
    #[serde(default)]
    pub panels_hidden: Option<Vec<String>>,
    #[serde(default)]
    pub tools_allowed: Option<Vec<String>>,
}

/// `POST /roles/{name}` — define or edit a role (Admin + audited). Editing a
/// seeded preset by name overwrites it (they are starting points, not locked)
/// and the edit survives re-migrations (seeded via upsert).
pub async fn upsert_role(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(name): Path<String>,
    Json(req): Json<RoleUpsertRequest>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    let r = brain_server::role::Role {
        name,
        description: req.description,
        scopes: req.scopes,
        owner_filter: req.owner_filter.unwrap_or_else(|| "self".to_string()),
        can: req.can,
        panels_default: req.panels_default,
        panels_hidden: req.panels_hidden,
        tools_allowed: req.tools_allowed,
    };
    brain_server::role::validate(&r).map_err(|e| HandlerError::bad_request("role_invalid", e))?;
    let actor = super::recall::principal_label(&principal.0);
    let pool = state.pool.clone();
    let stored = tokio::task::spawn_blocking(move || -> Result<_, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        brain_server::role::upsert(&conn, &r).map_err(map_err)?;
        crate::audit::record(
            &conn,
            crate::audit::AuditKind::Reconcile,
            &actor,
            &format!("role:{}", r.name),
            crate::audit::AuditStatus::Ok,
            "role_upserted",
        );
        brain_server::role::load(&conn, &r.name).map_err(map_err)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    match stored {
        Some(r) => Ok(Json(serde_json::to_value(r).unwrap_or_default())),
        None => Err(HandlerError::internal("role vanished after upsert")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_request_maps_from_plan_example() {
        let req: RoleUpsertRequest = serde_json::from_str(
            r#"{"scopes":["team"],"owner_filter":"reports",
               "can":["approve","reject","release_quarantine","dsar_export"],
               "tools_allowed":["ump.recall"]}"#,
        )
        .expect("plan example parses");
        assert_eq!(req.scopes, vec!["team"]);
        assert_eq!(req.owner_filter.as_deref(), Some("reports"));
        assert!(req.can.contains(&"approve".to_string()));
    }
}
