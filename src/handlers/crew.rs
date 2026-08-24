//! The Crew surfaces — colleagues become visible.
//!
//! - `GET /ops/crew?domain=&now=` — the roster view: TTL-decayed presence,
//!   Watchbill site badges, role + skills tags. Presence shows WHAT KIND of
//!   act, never case content; hidden entirely when the DPO switch is off or
//!   unreadable.
//! - `POST /ops/skills` — propose a skills change. The ONLY write path to
//!   `principal_skills` is the approval of the resulting
//!   `crew_skills_update` proposal (HITL); this endpoint never touches the
//!   skills table.
//! - `POST /ops/crew/config` — the DPO switch (Admin; audited).

use axum::{
    Json,
    extract::{Query, State},
};
use std::collections::HashMap;
use std::sync::Arc;

use crate::AppState;
use crate::handlers::HandlerError;
use crate::handlers::auth::OptPrincipal;
use crate::workflow::crew::{self, CrewError};

fn crew_err(e: CrewError) -> HandlerError {
    match e {
        CrewError::InvalidActivity(_) => {
            HandlerError::bad_request("activity_invalid", e.to_string())
        }
        CrewError::InvalidPrincipal(_) | CrewError::InvalidSkills(_) => {
            HandlerError::bad_request("skills_change_invalid", e.to_string())
        }
        CrewError::TooManySkills => HandlerError::conflict("skills_cap_reached"),
        CrewError::ProposalNotFound => HandlerError::not_found("proposal not found"),
        CrewError::ProposalNotPending => {
            HandlerError::conflict("proposal already decided by a concurrent action")
        }
        CrewError::Database(m) => HandlerError::internal(m),
    }
}

/// `GET /ops/crew?domain=&now=` — the roster (Read on the domain).
pub async fn get_ops_crew(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = params
        .get("domain")
        .cloned()
        .unwrap_or_else(|| "global".into());
    let now = match params.get("now").map(|s| s.parse::<i64>()) {
        Some(Ok(t)) => t,
        Some(Err(_)) => {
            return Err(HandlerError::bad_request(
                "now_invalid",
                "now must be epoch seconds",
            ));
        }
        None => chrono::Utc::now().timestamp(),
    };
    super::authorize(&principal, crate::auth::Action::Read, "", &domain)?;
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    let out = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, HandlerError> {
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        let enabled = crew::presence_enabled(&conn, &domain);
        let members = crew::roster(&conn, &domain, now).map_err(crew_err)?;
        let members: Vec<serde_json::Value> = members
            .iter()
            .map(|m| {
                serde_json::json!({
                    "principal": m.principal,
                    "state": m.state.as_str(),
                    "activity_kind": m.activity_kind,
                    "current_case_ref": m.current_case_ref,
                    "site": m.site,
                    "roles": m.roles,
                    "skills": m.skills,
                })
            })
            .collect();
        Ok(serde_json::json!({
            "now": now,
            "domain": domain,
            "presence_enabled": enabled,
            "members": members,
        }))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?;
    Ok(Json(out?))
}

/// `POST /ops/skills` — propose a skills change (Write). Creates ONE pending
/// `crew_skills_update` proposal; validation happens here AND again at
/// approval time inside the applying transaction.
pub async fn post_ops_skills(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = body
        .get("domain")
        .and_then(|v| v.as_str())
        .unwrap_or("global")
        .to_string();
    let target = body
        .get("principal")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            HandlerError::bad_request("skills_change_invalid", "principal is required")
        })?;
    let empty = Vec::new();
    let add: Vec<String> = body
        .get("add")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or(empty);
    let remove: Vec<String> = body
        .get("remove")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let change = crew::SkillsChange {
        domain,
        principal: target.to_string(),
        add,
        remove,
    };
    if change.add.len() > crew::MAX_SKILLS || change.remove.len() > crew::MAX_SKILLS {
        return Err(HandlerError::bad_request(
            "skills_change_invalid",
            format!("at most {} tags per change side", crew::MAX_SKILLS),
        ));
    }
    crew::apply_skills_change_probe(&change).map_err(crew_err)?;
    super::authorize(&principal, crate::auth::Action::Write, "", &change.domain)?;
    let actor = super::recall::principal_label(&principal);
    let content =
        serde_json::to_string(&change).map_err(|e| HandlerError::internal(e.to_string()))?;
    let audit_domain = change.domain.clone();
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    let id: Result<i64, HandlerError> = tokio::task::spawn_blocking(move || {
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        // RAII immediate tx: a panic or early error rolls the proposal +
        // audit pair back on drop — an open transaction can never leak back
        // into the pool (the DropBehavior::Rollback default).
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        let id: i64 = tx
            .query_row(
                "INSERT INTO proposals(kind, content, novelty, salience, created_at, owner)
                 VALUES (?1, ?2, 0.5, 0.5, ?3, ?4) RETURNING id",
                rusqlite::params![
                    crew::KIND_SKILLS_UPDATE,
                    content,
                    chrono::Utc::now().timestamp(),
                    actor
                ],
                |r| r.get(0),
            )
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        crate::audit::record_tenant(
            &tx,
            crate::audit::AuditKind::Workflow,
            actor.trim(),
            &format!("proposal:{id}"),
            crate::audit::AuditStatus::Ok,
            "crew/skills/propose",
            &audit_domain,
        );
        tx.commit()
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        Ok(id)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?;
    let id = id?;
    Ok(Json(serde_json::json!({
        "proposal_id": id,
        "kind": crew::KIND_SKILLS_UPDATE,
        "status": "pending",
    })))
}

/// `POST /ops/crew/config {domain?, presence_enabled}` — the DPO switch
/// (Admin; audited; flip + audit ride one tx).
pub async fn post_ops_crew_config(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = body
        .get("domain")
        .and_then(|v| v.as_str())
        .unwrap_or("global")
        .to_string();
    let enabled = body
        .get("presence_enabled")
        .and_then(|v| v.as_bool())
        .ok_or_else(|| {
            HandlerError::bad_request("presence_config_invalid", "presence_enabled bool required")
        })?;
    // Toggling people-visibility is governance, not content work — Admin.
    super::authorize(&principal, crate::auth::Action::Admin, "", &domain)?;
    let actor = super::recall::principal_label(&principal);
    let pool = super::resolve_domain_pool(&state.registry, None)?;
    let (domain_tx, domain_resp) = (domain.clone(), domain.clone());
    tokio::task::spawn_blocking(move || -> Result<(), HandlerError> {
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        crew::set_presence_enabled(&tx, &domain_tx, enabled, chrono::Utc::now().timestamp())
            .map_err(crew_err)?;
        crate::audit::record_tenant(
            &tx,
            crate::audit::AuditKind::Workflow,
            actor.trim(),
            &format!("crew-config:{domain_tx}"),
            crate::audit::AuditStatus::Ok,
            &format!("crew/config/{}", if enabled { "enable" } else { "disable" }),
            &domain_tx,
        );
        tx.commit()
            .map_err(|e| HandlerError::internal(format!("{e}")))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))??;
    Ok(Json(serde_json::json!({
        "domain": domain_resp,
        "presence_enabled": enabled,
    })))
}
