//! the breach-notification HTTP surface.
//!
//! `POST /breach` opens an incident (the DPO role is the actor);
//! `POST /breach/{id}/event` appends a notification/assessment/note;
//! `POST /breach/{id}/close` closes it; `GET /breaches` + `GET /breaches/{id}`
//! are the DPO/audit ledger. Every action is `authorize(Admin)`-gated AND
//! role-gated to the `dpo`/`admin` bundle, and every write is
//! hash-chained into the audit (`AuditKind::Breach`). Human-opened by design —
//! automated detection is a v2.x monitoring concern.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use std::sync::Arc;

use crate::audit::{AuditKind, AuditStatus};
use crate::handlers::auth::OptPrincipal;
use crate::handlers::{HandlerError, MAX_LIMIT};
use crate::AppState;

/// The breach actors: `authorize(Admin)` is the base gate, and a JWT principal
/// that carries roles must hold a role named `dpo` OR one with the `admin`
/// capability — an `agent`/`qa` token cannot open or close a breach.
/// Whether a resolved role bundle may act on a breach: it must include the
/// `dpo` role or one carrying the `admin` capability. Pure, so the
/// role-gate test (`dpo_role_is_the_breach_actor`) pins the rule directly.
fn can_act_on_breach(roles: &[brain_server::role::Role]) -> bool {
    roles.iter().any(|r| r.name == "dpo" || r.can("admin"))
}

pub(crate) fn require_dpo_role(
    principal: &Option<crate::auth::Principal>,
    pool: &crate::Pool,
) -> Result<(), HandlerError> {
    let Some(p) = principal else {
        return Ok(()); // no JWT = loopback incumbent
    };
    let conn = pool
        .get()
        .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
    if p.roles.is_empty() {
        // a scope-only admin token passes ONLY while the
        // deployment defines no roles at all. Once the role store is populated
        // the deployment has opted into role-based governance, and a token
        // with no roles is exactly the single-token shape the dual gate
        // exists to stop (hold release / breach close are one-principal
        // decisions that must not ride a bare admin scope).
        let defined: i64 = conn
            .query_row("SELECT COUNT(*) FROM roles", [], |r| r.get(0))
            .map_err(|e| HandlerError::internal(format!("role store: {e}")))?;
        if defined == 0 {
            return Ok(());
        }
        return Err(HandlerError::forbidden(
            crate::auth::Action::Admin,
            &p.tenant,
            "breach",
        ));
    }
    let roles = brain_server::role::resolve(&conn, &p.roles)
        .map_err(|e| HandlerError::internal(format!("role store: {e}")))?;
    if can_act_on_breach(&roles) {
        Ok(())
    } else {
        Err(HandlerError::forbidden(
            crate::auth::Action::Admin,
            &p.tenant,
            "breach",
        ))
    }
}

fn actor_of(principal: &Option<crate::auth::Principal>) -> String {
    principal
        .as_ref()
        .map(|p| p.sub.clone())
        .unwrap_or_else(|| "loopback".to_string())
}

/// `POST /breach` body.
#[derive(Debug, Deserialize)]
pub struct OpenBreachRequest {
    pub scope: String,
    pub description: String,
    pub severity: String,
    /// Optional; defaults to now when absent (the DPO records actual discovery).
    #[serde(default)]
    pub discovered_at: Option<i64>,
    #[serde(default)]
    pub affected_estimate: Option<i64>,
    /// Affected jurisdictions (country codes) — drives the notification
    /// deadlines. Empty → none (undetermined; the DPO confirms).
    #[serde(default)]
    pub jurisdictions: Vec<String>,
}

/// `POST /breach` — open an incident. DPO/admin + audited.
pub async fn post_breach(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Json(req): Json<OpenBreachRequest>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    require_dpo_role(&principal.0, &pool)?;
    crate::breach::validate_open(
        &req.scope,
        &req.description,
        &req.severity,
        &req.jurisdictions,
    )?;
    if let Some(a) = req.affected_estimate {
        if a < 0 {
            return Err(HandlerError::bad_request(
                "breach_estimate_invalid",
                "affected_estimate must be >= 0",
            ));
        }
    }
    let opened_by = actor_of(&principal.0);
    let now = chrono::Utc::now().timestamp();
    let discovered_at = req.discovered_at.unwrap_or(now);

    let pool_for = pool.clone();
    let scope_for = req.scope.clone();
    let desc_for = req.description.clone();
    let sev_for = req.severity.clone();
    let jur_for = req.jurisdictions.clone();
    let by = opened_by.clone();
    let id = tokio::task::spawn_blocking(move || -> Result<i64, HandlerError> {
        let mut conn = pool_for
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let tx = conn
            .transaction()
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        let id = crate::breach::open(
            &tx,
            &scope_for,
            &desc_for,
            &sev_for,
            discovered_at,
            req.affected_estimate,
            &jur_for,
            &by,
            now,
        )?;
        tx.commit()
            .map_err(|e| HandlerError::internal(format!("commit failed: {e}")))?;
        Ok(id)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;

    if let Ok(conn) = pool.get() {
        crate::audit::record(
            &conn,
            AuditKind::Breach,
            "api",
            &format!("breach_open:{id}"),
            AuditStatus::Ok,
            &req.description,
        );
    }
    let deadlines = crate::ph::notification_deadlines(&req.jurisdictions, discovered_at);
    Ok(Json(serde_json::json!({
        "breach_id": id,
        "deadlines": deadlines,
    })))
}

/// `POST /breach/{id}/event` — append a notification/assessment/note.
#[derive(Debug, Deserialize)]
pub struct BreachEventRequest {
    pub event_type: String,
    #[serde(default)]
    pub jurisdiction: Option<String>,
    pub body: String,
}

pub async fn post_breach_event(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
    Json(req): Json<BreachEventRequest>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    require_dpo_role(&principal.0, &pool)?;
    let et = req.event_type.trim().to_lowercase();
    if !matches!(et.as_str(), "notification" | "assessment" | "note") {
        return Err(HandlerError::bad_request(
            "breach_event_type_invalid",
            "event_type must be notification | assessment | note",
        ));
    }
    if req.body.trim().is_empty() || req.body.len() > crate::breach::MAX_BREACH_EVENT {
        return Err(HandlerError::bad_request(
            "breach_event_body_invalid",
            format!(
                "body is required and ≤ {} characters",
                crate::breach::MAX_BREACH_EVENT
            ),
        ));
    }
    let noted_by = actor_of(&principal.0);
    let now = chrono::Utc::now().timestamp();
    let pool_for = pool.clone();
    let body_for = req.body.clone();
    let by = noted_by.clone();
    let et_for = et.clone();
    let jur_for = req.jurisdiction.clone();
    let jur_audit = jur_for.clone();
    let pushed = tokio::task::spawn_blocking(move || -> Result<bool, HandlerError> {
        let mut conn = pool_for
            .get()
            .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
        let tx = conn
            .transaction()
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        let ok =
            crate::breach::add_event(&tx, id, &et_for, jur_for.as_deref(), &body_for, &by, now)?;
        tx.commit()
            .map_err(|e| HandlerError::internal(format!("commit failed: {e}")))?;
        Ok(ok)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    if !pushed {
        return Err(HandlerError::not_found("no breach with this id"));
    }
    if let Ok(conn) = pool.get() {
        crate::audit::record(
            &conn,
            AuditKind::Breach,
            "api",
            &format!("breach_event:{id}"),
            AuditStatus::Ok,
            &format!("{}:{}", et, jur_audit.unwrap_or_default()),
        );
    }
    Ok(Json(
        serde_json::json!({ "appended": true, "breach_id": id }),
    ))
}

/// `POST /breach/{id}/close`.
pub async fn close_breach(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    require_dpo_role(&principal.0, &pool)?;
    let closed_by = actor_of(&principal.0);
    let now = chrono::Utc::now().timestamp();
    let pool_for = pool.clone();
    let by = closed_by.clone();
    let closed =
        tokio::task::spawn_blocking(move || -> Result<Option<Option<i64>>, HandlerError> {
            // Returns (closed_at) when the breach was found (was it newly closed?).
            let mut conn = pool_for
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            let tx = conn
                .transaction()
                .map_err(|e| HandlerError::internal(e.to_string()))?;
            let row = crate::breach::close(&tx, id, now, &by)?;
            tx.commit()
                .map_err(|e| HandlerError::internal(format!("commit failed: {e}")))?;
            Ok(row.map(|r| r.closed_at))
        })
        .await
        .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    let closed_at = closed.ok_or_else(|| HandlerError::not_found("no breach with this id"))?;
    if let Ok(conn) = pool.get() {
        crate::audit::record(
            &conn,
            AuditKind::Breach,
            "api",
            &format!("breach_close:{id}"),
            AuditStatus::Ok,
            "closed",
        );
    }
    Ok(Json(serde_json::json!({
        "closed": true,
        "breach_id": id,
        "closed_at": closed_at,
    })))
}

#[derive(Debug, Default, Deserialize)]
pub struct BreachListQuery {
    pub limit: Option<i64>,
}

/// `GET /breaches` — the DPO/audit ledger, newest-first, bounded.
pub async fn list_breaches(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Query(q): Query<BreachListQuery>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    require_dpo_role(&principal.0, &pool)?;
    let limit = q.limit.unwrap_or(100).clamp(1, MAX_LIMIT as i64 * 10);
    let pool_for = pool.clone();
    let body =
        tokio::task::spawn_blocking(move || -> Result<Vec<serde_json::Value>, HandlerError> {
            let conn = pool_for
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            let rows = crate::breach::list(&conn, limit)?;
            Ok(rows
                .into_iter()
                .map(|b| serde_json::to_value(b).unwrap_or_default())
                .collect())
        })
        .await
        .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    Ok(Json(serde_json::json!({ "breaches": body })))
}

/// `GET /breaches/{id}` — one incident + events + computed deadlines.
pub async fn get_breach(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    super::authorize(&principal.0, crate::auth::Action::Admin, "", "global")?;
    require_dpo_role(&principal.0, &pool)?;
    let pool_for = pool.clone();
    let view = tokio::task::spawn_blocking(
        move || -> Result<Option<crate::breach::BreachView>, HandlerError> {
            let conn = pool_for
                .get()
                .map_err(|e| HandlerError::internal(format!("DB connection failed: {e}")))?;
            crate::breach::get(&conn, id)
        },
    )
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    let view = view.ok_or_else(|| HandlerError::not_found("no breach with this id"))?;
    Ok(Json(serde_json::to_value(view).unwrap_or_default()))
}
#[cfg(test)]
mod tests {
    use super::*;
    use brain_server::role::Role;

    fn role(name: &str, can: &[&str]) -> Role {
        Role {
            name: name.to_string(),
            scopes: brain_server::role::ROLE_SCOPES
                .iter()
                .map(|s| s.to_string())
                .collect(),
            owner_filter: "all".to_string(),
            can: can.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn dpo_role_is_the_breach_actor() {
        // Verification 4: the `dpo` role or an `admin`-capability bundle may act
        // on a breach; an agent/qa cannot (403 via require_dpo_role's callers).
        let dpo = role("dpo", &["dsar_export"]);
        let admin = role("admin", &["admin"]);
        let agent = role("agent", &["read", "write"]);
        let executor = role("exec", &["read"]);

        assert!(
            can_act_on_breach(std::slice::from_ref(&dpo)),
            "dpo role acts on breach"
        );
        assert!(
            can_act_on_breach(std::slice::from_ref(&admin)),
            "admin capability acts"
        );
        let dpo_and_agent: Vec<brain_server::role::Role> = vec![dpo.clone(), agent.clone()];
        assert!(can_act_on_breach(&dpo_and_agent), "dpo among others");
        assert!(
            !can_act_on_breach(std::slice::from_ref(&agent)),
            "an agent token cannot act on a breach"
        );
        assert!(
            !can_act_on_breach(std::slice::from_ref(&executor)),
            "exec cannot act"
        );
    }
}
