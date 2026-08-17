//! the cross-border transfer register + the
//! TIA/DPA evidence artifacts (HTTP surface).
//!
//! `POST /transfers` records a cross-border data flow (Art 30 + Art 46 evidence);
//! `GET /transfers` lists it with optional filters (mechanism/jurisdiction/
//! dataset); `GET /transfers/{id}/tia` + `GET /transfers/{id}/dpa` export the
//! Transfer Impact Assessment (Schrems II) + DPA (Art 28) templates pre-filled
//! from the register row. Every action is admin-gated and every write is
//! hash-chained into the audit (`AuditKind::Transfer`). These are **evidence
//! artifacts** — a human (DPO/legal) reviews + signs them; nothing here renders
//! legal judgment.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use std::sync::Arc;

use crate::audit::{AuditKind, AuditStatus};
use crate::handlers::auth::OptPrincipal;
use crate::handlers::HandlerError;
use crate::AppState;

/// `POST /transfers` body — the Art 30/Art 46 register entry.
#[derive(Debug, Deserialize)]
pub struct TransferRequest {
    pub dataset: String,
    pub origin_jurisdiction: String,
    pub destination_jurisdiction: String,
    pub mechanism: String,
    pub counterparty: String,
    #[serde(default)]
    pub lawful_basis: Option<String>,
    pub purpose: String,
    #[serde(default)]
    pub signed_at: Option<i64>,
    #[serde(default)]
    pub expires_at: Option<i64>,
}

/// `POST /transfers` — record a cross-border transfer. Admin + audited.
pub async fn register_transfer(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Json(req): Json<TransferRequest>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    crate::transfers::validate_register(
        &req.dataset,
        &req.origin_jurisdiction,
        &req.destination_jurisdiction,
        &req.mechanism,
        &req.counterparty,
        &req.purpose,
        req.lawful_basis.as_deref(),
        req.signed_at,
        req.expires_at,
    )?;

    let pool_for = pool.clone();
    let dataset_for = req.dataset.clone();
    let origin = req.origin_jurisdiction.clone();
    let destination = req.destination_jurisdiction.clone();
    let mechanism = req.mechanism.clone();
    let counterparty = req.counterparty.clone();
    let basis = req.lawful_basis.clone();
    let purpose = req.purpose.clone();
    // `mechanism_label` is read AFTER the closure — the closure consumes the
    // `mechanism` clone, so keep a fresh one for the audit + response.
    let mechanism_label = req.mechanism.clone();
    let mech_for_closure = mechanism_label.clone();
    let id = tokio::task::spawn_blocking(move || -> Result<i64, HandlerError> {
        let mut conn = pool_for
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let tx = conn
            .transaction()
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        let id = crate::transfers::register(
            &tx,
            &dataset_for,
            &origin,
            &destination,
            &mechanism,
            &counterparty,
            basis.as_deref(),
            &purpose,
            req.signed_at,
            req.expires_at,
        )?;
        // the Art-30 audit row lands INSIDE the
        // write transaction (nested via SAVEPOINT) so the register row and its
        // audit are atomic — a crash between commit and audit can no longer
        // leave an unmirrored register entry. Best-effort (record swallows its
        // own errors) like every audit call site, so a broken audit row never
        // rolls back the register write.
        let _ = crate::audit::record(
            &tx,
            AuditKind::Transfer,
            "api",
            &format!("transfer_register:{id}"),
            AuditStatus::Ok,
            &format!(
                "{}:{}->{}:{mechanism_label}",
                dataset_for, origin, destination
            ),
        );
        tx.commit()
            .map_err(|e| HandlerError::internal(format!("commit failed: {e}")))?;
        Ok(id)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))?;
    let id: i64 = id?;

    Ok(Json(
        serde_json::json!({ "id": id, "mechanism": mech_for_closure }),
    ))
}

/// `GET /transfers` query: optional filters + page size.
#[derive(Debug, Default, Deserialize)]
pub struct TransferListQuery {
    pub limit: Option<i64>,
    pub mechanism: Option<String>,
    pub jurisdiction: Option<String>,
    pub dataset: Option<String>,
}

/// `GET /transfers` — the register list, newest-first, bounded + filterable.
pub async fn list_transfers(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Query(q): Query<TransferListQuery>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    // The clamp is applied once, in `list` (bounds = MAX_TRANSFER_LIMIT).
    let limit = q.limit.unwrap_or(100);
    let pool_for = pool.clone();
    let m = q.mechanism.clone();
    let j = q.jurisdiction.clone();
    let d = q.dataset.clone();
    let rows =
        tokio::task::spawn_blocking(move || -> Result<Vec<serde_json::Value>, HandlerError> {
            let conn = pool_for
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            let rows =
                crate::transfers::list(&conn, limit, m.as_deref(), j.as_deref(), d.as_deref())?;
            Ok(rows
                .into_iter()
                .map(|t| serde_json::to_value(t).unwrap_or_default())
                .collect())
        })
        .await
        .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    Ok(Json(serde_json::json!({ "transfers": rows })))
}

/// `GET /transfers/{id}/tia` — the Schrems II Transfer Impact Assessment,
/// pre-filled from the register row + the destination law + surveillance table.
pub async fn get_tia(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    let pool_for = pool.clone();
    let tia = tokio::task::spawn_blocking(
        move || -> Result<Option<crate::transfers::TiaTemplate>, HandlerError> {
            let conn = pool_for
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            crate::transfers::tia_from(&conn, id)
        },
    )
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    let tia = tia.ok_or_else(|| HandlerError::not_found("no transfer with this id"))?;
    Ok(Json(serde_json::to_value(tia).unwrap_or_default()))
}

/// `GET /transfers/{id}/dpa` — the DPA (Art 28 sub-processor terms) fields,
/// pre-filled from the transfer row (the evidence a client's controller asks
/// for before authorizing the sub-processor).
pub async fn get_dpa(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    let pool_for = pool.clone();
    let value = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, HandlerError> {
        let conn = pool_for
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let t = crate::transfers::transfer_by_id(&conn, id)?
            .ok_or_else(|| HandlerError::not_found("no transfer with this id"))?;
        Ok(crate::transfers::dpa_fields(&t))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    Ok(Json(value))
}
