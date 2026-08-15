//! v1.27.1 "Clients" — the BPO operating register (HTTP surface).
//!
//! `POST /clients` registers an operating client (name / isolation domain /
//! jurisdiction / bound profile); `GET /clients` lists the register; `GET
//! /clients/{name}` resolves one row. Every write is Admin-gated + hash-chained
//! into the audit (`AuditKind::Client`). This is the evidence/identity register
//! only — it does not gate enforcement (that is v1.27.x + v2.x).

use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use std::sync::Arc;

use crate::audit::{AuditKind, AuditStatus};
use crate::handlers::auth::OptPrincipal;
use crate::handlers::HandlerError;
use crate::AppState;

/// `POST /clients` body. `profile` is optional (the bound profile is an R2+
/// concern; here it is recorded verbatim when supplied).
#[derive(Debug, Deserialize)]
pub struct CreateClientRequest {
    pub name: String,
    pub domain: String,
    pub jurisdiction: String,
    #[serde(default)]
    pub profile: Option<String>,
}

/// `POST /clients` — register an operating client. Admin + audited.
pub async fn register_client(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Json(req): Json<CreateClientRequest>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    crate::clients::validate_new_client(&req.name, &req.domain, &req.jurisdiction)?;

    // Scaffold the client's domain before writing the row — `pool_for` creates
    // + migrates the domain DB (multi-db) or touches the shared pool (shim).
    // The optional profile bind is the v1.21 seam; `register` (via the compose
    // fn) makes the `clients` row. Composition only, no new logic.
    let st = state.clone();
    let now = chrono::Utc::now().timestamp();
    let name_for = req.name.clone();
    let domain_for = req.domain.clone();
    let jurisdiction_for = req.jurisdiction.clone();
    let profile_for = req.profile.clone();
    tokio::task::spawn_blocking(move || -> Result<(), HandlerError> {
        crate::clients::scaffold_and_register(
            &st.registry,
            &st.pool,
            &name_for,
            &domain_for,
            &jurisdiction_for,
            profile_for.as_deref(),
            now,
        )
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    if let Ok(conn) = state.pool.get() {
        crate::audit::record(
            &conn,
            AuditKind::Client,
            "api",
            &format!("client:{}", req.name.trim().to_ascii_lowercase()),
            AuditStatus::Ok,
            &format!(
                "register:{}:{}",
                req.jurisdiction.trim().to_ascii_lowercase(),
                req.domain.trim().to_ascii_lowercase()
            ),
        );
    }
    Ok(Json(serde_json::json!({ "name": req.name })))
}

/// `GET /clients` — the full register, ordered by name. Admin read.
pub async fn list_clients(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    let pool_for = pool.clone();
    let rows =
        tokio::task::spawn_blocking(move || -> Result<Vec<serde_json::Value>, HandlerError> {
            let conn = pool_for
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            Ok(crate::clients::list(&conn)?
                .into_iter()
                .map(|c| serde_json::to_value(c).unwrap_or_default())
                .collect())
        })
        .await
        .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    Ok(Json(serde_json::json!({ "clients": rows })))
}

/// `GET /clients/{name}` — resolve one client. Admin read; 404 when absent.
pub async fn get_client(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    let pool_for = pool.clone();
    let value =
        tokio::task::spawn_blocking(move || -> Result<Option<serde_json::Value>, HandlerError> {
            let conn = pool_for
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            Ok(crate::clients::by_name(&conn, &name)?
                .map(|c| serde_json::to_value(c).unwrap_or_default()))
        })
        .await
        .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    Ok(Json(value.ok_or_else(|| {
        HandlerError::not_found("client not found")
    })?))
}

/// `POST /clients/{name}/dpa` — set Art 28 sub-processor terms (the evidence a
/// client's controller checks). Admin + audited. 404 when the client is
/// unknown (via the update's affected-row count, no second query).
pub async fn set_client_dpa(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(name): Path<String>,
    Json(req): Json<crate::clients::DpaTerms>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    crate::clients::validate_dpa_terms(&req)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    let pool_for = pool.clone();
    let key = name.trim().to_ascii_lowercase();
    let key_for = key.clone();
    let terms = req;
    let changed = tokio::task::spawn_blocking(move || -> Result<usize, HandlerError> {
        let mut conn = pool_for
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let tx = conn
            .transaction()
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        let n = crate::clients::set_dpa_terms(&tx, &key_for, &terms)?;
        tx.commit()
            .map_err(|e| HandlerError::internal(format!("commit failed: {e}")))?;
        Ok(n)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    if changed == 0 {
        return Err(HandlerError::not_found("client not found"));
    }
    if let Ok(conn) = state.pool.get() {
        crate::audit::record(
            &conn,
            AuditKind::Client,
            "api",
            &format!("client:{key}"),
            AuditStatus::Ok,
            "dpa_terms_set",
        );
    }
    Ok(Json(serde_json::json!({ "name": key })))
}

/// `GET /clients/{name}/dpa` — read the stored terms; `null` when set never.
/// Admin read. 404 when the client is unknown.
pub async fn get_client_dpa(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    let pool_for = pool.clone();
    let key = name.trim().to_ascii_lowercase();
    let value = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, HandlerError> {
        let conn = pool_for
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        match crate::clients::by_name(&conn, &key)? {
            None => Err(HandlerError::not_found("client not found")),
            Some(c) => Ok(c
                .dpa_terms
                .map(|t| serde_json::to_value(t).unwrap_or_default())
                .unwrap_or(serde_json::Value::Null)),
        }
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))?;
    Ok(Json(value?))
}
