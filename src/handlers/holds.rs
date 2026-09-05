//! the legal-hold HTTP surface.
//!
//! `POST /legal-hold` (ids + reason) freezes chunks against every erasure
//! path; `POST /legal-hold/{id}/release` is the explicit un-freeze (never
//! auto); `GET /legal-holds` is the filterable registry. Every action is
//! Admin-gated + audited. Enforcement lives at the erasure seams: decay
//! (`/decayed` exclusion), `/purge` (409 `legal_hold_active`), and DSAR
//! (deferral + certificate `held_ids`).

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::AppState;
use crate::handlers::auth::OptPrincipal;
use crate::handlers::{HandlerError, MAX_LIMIT};

/// `POST /legal-hold` body: the ids to freeze + the human citation.
#[derive(Debug, Deserialize)]
pub struct HoldRequest {
    pub ids: Vec<i64>,
    pub reason: String,
    /// Domain whose pool holds these ids (defaults to `global` — the multi-db knob).
    #[serde(default)]
    pub domain: Option<String>,
}

/// `POST /legal-hold` response.
#[derive(Debug, Serialize)]
pub struct HoldResponse {
    pub held: usize,
    pub hold_ids: Vec<i64>,
}

/// `POST /legal-hold` — place a hold on the `global` (or `?domain=`) pool.
pub async fn post_legal_hold(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Json(req): Json<HoldRequest>,
) -> Result<Json<HoldResponse>, HandlerError> {
    let domain = req.domain.as_deref().unwrap_or("global");
    post_legal_hold_for_domain(state, principal, domain, req.ids, req.reason).await
}

/// Shared per-domain hold write: place one hold per id on `domain`'s pool.
/// Fails closed on unknown ids (a hold on nothing is operator error, not a
/// silent no-op). Admin + audited. Called by the `/legal-hold` route (with
/// `global` / its `?domain=`) and `/clients/{name}/hold` (with the client's
/// domain) — composition keeps the per-isolation write in one place.
pub(crate) async fn post_legal_hold_for_domain(
    state: Arc<AppState>,
    principal: OptPrincipal,
    domain: &str,
    ids: Vec<i64>,
    reason: String,
) -> Result<Json<HoldResponse>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    crate::legal_hold::validate(&ids, &reason)?;
    let reason = reason.trim().to_string();
    let held_by = principal
        .0
        .as_ref()
        .map(|p| p.sub.clone())
        .unwrap_or_else(|| "loopback".to_string());
    let pool = super::resolve_domain_pool(&state.registry, Some(domain))?;
    let pool_for = pool.clone();
    let reason_for = reason.clone();

    let (held, hold_ids) =
        tokio::task::spawn_blocking(move || -> Result<(usize, Vec<i64>), HandlerError> {
            let mut conn = pool_for.get().map_err(HandlerError::db_down)?;
            let tx = conn
                .transaction()
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            // Existence check inside the same tx as the inserts: an id deleted
            // between check + write is impossible, an unknown id refuses the
            // whole request (all-or-nothing, the purge posture).
            if let Some(missing) = crate::legal_hold::first_missing_id(&tx, &ids)
                .map_err(|e| HandlerError::internal(e.to_string()))?
            {
                return Err(HandlerError::bad_request_with(
                    "unknown_ids",
                    format!("knowledge id {missing} does not exist in this domain"),
                    serde_json::json!({ "unknown_id": missing }),
                ));
            }
            let now = chrono::Utc::now().timestamp();
            let created =
                crate::legal_hold::insert_holds(&tx, &ids, &reason_for, Some(&held_by), now)?;
            tx.commit()
                .map_err(|e| HandlerError::internal(format!("commit failed: {e}")))?;
            Ok((created.len(), created))
        })
        .await
        .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    if let Ok(conn) = pool.get() {
        crate::audit::record(
            &conn,
            crate::audit::AuditKind::Reconcile,
            "api",
            &format!("legal_hold:{held}"),
            crate::audit::AuditStatus::Ok,
            &reason,
        );
    }
    Ok(Json(HoldResponse { held, hold_ids }))
}

/// `POST /legal-hold/{id}/release` — the explicit un-freeze. 404 when the
/// hold row does not exist; releasing an already-released hold is a no-op.
/// The id stays frozen while ANY other hold on it remains active. Audited.
/// `?domain=` targets the hold's pool in multi-db mode (default `global`).
pub async fn release_legal_hold(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
    Query(q): Query<ReleaseQuery>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    let pool = super::resolve_domain_pool(&state.registry, q.domain.as_deref())?;
    // releasing a hold unfreezes erasure mid-
    // litigation — the same DPO/admin dual gate a breach close carries.
    super::breaches::require_dpo_role(&principal.0, &pool)?;
    let pool_for = pool.clone();
    let released = tokio::task::spawn_blocking(
        move || -> Result<Option<crate::legal_hold::LegalHoldRow>, HandlerError> {
            let mut conn = pool_for.get().map_err(HandlerError::db_down)?;
            let tx = conn
                .transaction()
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            let now = chrono::Utc::now().timestamp();
            let hold = crate::legal_hold::release(&tx, id, now)?;
            tx.commit()
                .map_err(|e| HandlerError::internal(format!("commit failed: {e}")))?;
            Ok(hold)
        },
    )
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    let hold = released.ok_or_else(|| HandlerError::not_found("no legal hold with this id"))?;
    if let Ok(conn) = pool.get() {
        crate::audit::record(
            &conn,
            crate::audit::AuditKind::Reconcile,
            "api",
            &format!("legal_hold_release:{id}"),
            crate::audit::AuditStatus::Ok,
            &hold.reason,
        );
    }
    Ok(Json(serde_json::json!({
        "released": true,
        "hold": hold,
        "note": "the knowledge id stays frozen while any other active hold on it remains",
    })))
}

/// `POST /legal-hold/{id}/release` optional query: the hold's pool.
#[derive(Debug, Default, Deserialize)]
pub struct ReleaseQuery {
    pub domain: Option<String>,
}

/// `GET /legal-holds` query: filter by knowledge `id` and/or `reason`
/// substring; `active=true` narrows to unreleased holds.
#[derive(Debug, Default, Deserialize)]
pub struct HoldsQuery {
    pub id: Option<i64>,
    pub reason: Option<String>,
    pub active: Option<bool>,
    pub limit: Option<i64>,
}

/// `GET /legal-holds` — the hold registry (Admin): every hold, released or
/// not, newest-first, bounded (default 100, clamped to the tombstones cap).
/// Multi-db: every pool's holds, tagged with the pool's domain (hold ids are
/// per-DB AUTOINCREMENT); shim mode is the single global pool.
pub async fn list_legal_holds(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Query(q): Query<HoldsQuery>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    let domains: Vec<String> = if state.registry.is_multi_db() {
        state.registry.known_domains()
    } else {
        vec!["global".to_string()]
    };
    let limit = q
        .limit
        .map(|l| l.clamp(1, MAX_LIMIT as i64 * 10))
        .unwrap_or(100);
    let id = q.id;
    let reason = q.reason.clone();
    let active_only = q.active.unwrap_or(false);
    let body = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, HandlerError> {
        let mut holds: Vec<serde_json::Value> = Vec::new();
        let mut total = 0;
        for d in &domains {
            let pool = super::resolve_domain_pool(&state.registry, Some(d))?;
            let conn = pool.get().map_err(HandlerError::db_down)?;
            let (rows, t) = crate::legal_hold::list_holds(&conn, id, reason.as_deref(), limit)?;
            total += t;
            for h in rows {
                if active_only && h.released_at.is_some() {
                    continue;
                }
                holds.push(serde_json::json!({ "domain": d, "hold": h }));
            }
        }
        holds.sort_by(|a, b| {
            let ra = a["hold"]["id"].as_i64().unwrap_or(0);
            let rb = b["hold"]["id"].as_i64().unwrap_or(0);
            rb.cmp(&ra)
        });
        holds.truncate(limit as usize);
        Ok(serde_json::json!({
            "holds": holds,
            "total": total,
        }))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    Ok(Json(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_query_defaults_are_bounded() {
        // The default + clamp contract, unit-checked without an HTTP stack.
        let q = HoldsQuery::default();
        assert!(q.id.is_none() && q.reason.is_none() && !q.active.unwrap_or(false));
        assert_eq!(
            q.limit
                .map(|l| l.clamp(1, MAX_LIMIT as i64 * 10))
                .unwrap_or(100),
            100
        );
    }
}
