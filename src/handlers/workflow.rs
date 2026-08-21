use axum::{
    Json,
    extract::{Path, State},
};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::AppState;
use crate::auth::Principal;
use crate::handlers::HandlerError;

#[derive(Serialize)]
struct RunRow {
    id: i64,
    domain: String,
    kind: String,
    status: String,
    state_json: String,
    created_at: i64,
    updated_at: i64,
}

pub async fn get_run(
    State(state): State<Arc<AppState>>,
    axum::Extension(principal): axum::Extension<Option<Principal>>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let pool = state.pool.clone();
    let row: Option<RunRow> = tokio::task::spawn_blocking(move || {
        let conn = pool.get().map_err(|e| format!("{e}"))?;
        conn.query_row(
            "SELECT id,domain,kind,status,state_json,created_at,updated_at FROM workflow_runs WHERE id=?1",
            rusqlite::params![id],
            |r| Ok(RunRow{
                id: r.get(0)?, domain: r.get(1)?, kind: r.get(2)?, status: r.get(3)?,
                state_json: r.get(4)?, created_at: r.get(5)?, updated_at: r.get(6)?,
            }),
        ).optional().map_err(|e| format!("{e}"))
    }).await.map_err(|e| HandlerError::internal(format!("{e}")))?.map_err(HandlerError::internal)?;
    let row = row.ok_or_else(|| HandlerError::not_found("workflow run not found"))?;
    crate::handlers::authorize(&principal, crate::auth::Action::Read, "", &row.domain)?;
    Ok(Json(serde_json::to_value(&row).unwrap()))
}

pub async fn list_steps(
    State(state): State<Arc<AppState>>,
    axum::Extension(principal): axum::Extension<Option<Principal>>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let pool = state.pool.clone();
    let (domain, steps): (Option<String>, Vec<serde_json::Value>) = tokio::task::spawn_blocking(move || -> Result<_, String> {
        let conn = pool.get().map_err(|e| format!("{e}"))?;
        let domain: Option<String> = conn.query_row("SELECT domain FROM workflow_runs WHERE id=?1", rusqlite::params![id], |r| r.get(0)).optional().map_err(|e| format!("{e}"))?;
        let Some(ref _d) = domain else { return Ok((None, vec![])) };
        let mut stmt = conn.prepare("SELECT id,run_id,phase,step_key,state_json,revision,parent_step_id FROM workflow_steps WHERE run_id=?1 ORDER BY id").map_err(|e| format!("{e}"))?;
        let rows = stmt.query_map(rusqlite::params![id], |r| {
            Ok(serde_json::json!({
                "id": r.get::<_,i64>(0)?,
                "run_id": r.get::<_,i64>(1)?,
                "phase": r.get::<_,String>(2)?,
                "step_key": r.get::<_,String>(3)?,
                "state_json": r.get::<_,String>(4)?,
                "revision": r.get::<_,i64>(5)?,
                "parent_step_id": r.get::<_,Option<i64>>(6)?,
            }))
        }).map_err(|e| format!("{e}"))?;
        let mut out = Vec::new();
        for r in rows { out.push(r.map_err(|e| format!("{e}"))?); }
        Ok((domain, out))
    }).await.map_err(|e| HandlerError::internal(format!("{e}")))?.map_err(HandlerError::internal)?;
    let Some(domain) = domain else {
        return Err(HandlerError::not_found("workflow run not found"));
    };
    crate::handlers::authorize(&principal, crate::auth::Action::Read, "", &domain)?;
    Ok(Json(serde_json::json!({"steps": steps})))
}

#[derive(Deserialize)]
pub struct SteeringRequest {
    pub message: String,
}

pub async fn post_steering(
    State(state): State<Arc<AppState>>,
    axum::Extension(principal): axum::Extension<Option<Principal>>,
    Path(id): Path<i64>,
    Json(body): Json<SteeringRequest>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    if body.message.len() > 4000 {
        return Err(HandlerError::bad_request(
            "message_too_long",
            "message exceeds 4000 chars",
        ));
    }
    if body.message.trim().is_empty() {
        return Err(HandlerError::bad_request(
            "message_empty",
            "message must not be empty",
        ));
    }
    let sanitized = crate::gate::sanitize_read(&body.message, false, &principal);
    let payload = serde_json::json!({"message": sanitized}).to_string();
    let pool = state.pool.clone();
    let principal2 = principal.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let conn = pool.get().map_err(|e| format!("{e}"))?;
        let domain: String = conn.query_row("SELECT domain FROM workflow_runs WHERE id=?1", rusqlite::params![id], |r| r.get(0)).optional().map_err(|e| format!("{e}"))?.ok_or("workflow run not found".to_string())?;
        // auth inside blocking would need principal; we already authorized via sanitized? do check via is_authorized logic without pool
        // authorize read/write check is done outside; re-check via principal2
        if let Some(p) = &principal2
            && !crate::auth::is_authorized(p, crate::auth::Action::Write, &p.tenant, &domain)
        {
            return Err("forbidden".to_string());
        }
        let cnt: i64 = conn.query_row("SELECT COUNT(*) FROM outbox WHERE run_id=?1 AND topic='steering'", rusqlite::params![id], |r| r.get(0)).map_err(|e| format!("{e}"))?;
        if cnt >= 100 {
            conn.execute("DELETE FROM outbox WHERE id IN (SELECT id FROM outbox WHERE run_id=?1 AND topic='steering' ORDER BY id ASC LIMIT 1)", rusqlite::params![id]).map_err(|e| format!("{e}"))?;
        }
        let now = chrono::Utc::now().timestamp();
        let key = format!("steering-{id}-{now}-{}", rand::random::<u32>());
        crate::workflow::outbox::enqueue(&conn, id, "steering", &payload, &key, now).map_err(|e| format!("{e}"))?;
        crate::audit::record_tenant(&conn, crate::audit::AuditKind::Workflow, "api", &format!("workflow:{id}"), crate::audit::AuditStatus::Ok, "steering", &domain);
        Ok(())
    }).await.map_err(|e| HandlerError::internal(format!("{e}")))?.map_err(|e| if e=="forbidden" { HandlerError::forbidden(crate::auth::Action::Write, "", "workflow") } else if e=="workflow run not found" { HandlerError::not_found(e) } else { HandlerError::internal(e) })?;
    Ok(Json(serde_json::json!({"ok": true})))
}

pub async fn get_suggestions(
    State(state): State<Arc<AppState>>,
    axum::Extension(principal): axum::Extension<Option<Principal>>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let pool = state.pool.clone();
    let st = state.clone();
    let (domain, state_json): (String, String) =
        tokio::task::spawn_blocking(move || -> Result<_, String> {
            let conn = pool.get().map_err(|e| format!("{e}"))?;
            let row: Option<(String, String)> = conn
                .query_row(
                    "SELECT domain,state_json FROM workflow_runs WHERE id=?1",
                    rusqlite::params![id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(|e| format!("{e}"))?;
            row.ok_or("workflow run not found".to_string())
        })
        .await
        .map_err(|e| HandlerError::internal(format!("{e}")))?
        .map_err(|e| {
            if e == "workflow run not found" {
                HandlerError::not_found(e)
            } else {
                HandlerError::internal(e)
            }
        })?;
    crate::handlers::authorize(&principal, crate::auth::Action::Read, "", &domain)?;
    let q: String = serde_json::from_str::<serde_json::Value>(&state_json)
        .ok()
        .and_then(|v| v.get("q").and_then(|x| x.as_str()).map(|s| s.to_string()))
        .unwrap_or_else(|| state_json.chars().take(200).collect());
    let q = q.trim().to_string();
    // do a recall scoped to domain; if q empty, zero hits
    let hits: Vec<serde_json::Value> = if q.is_empty() {
        vec![]
    } else {
        // use search directly via spawn_blocking
        let pool2 = st.pool.clone();
        let domain2 = domain.clone();
        let q2 = q.clone();
        // simple FTS search; fallback to empty
        tokio::task::spawn_blocking(move || -> Vec<serde_json::Value> {
            let Ok(conn) = pool2.get() else { return vec![] };
            let mut out = Vec::new();
            let mut stmt = match conn.prepare("SELECT id,title,content FROM knowledge WHERE domain=?1 AND content LIKE ?2 LIMIT 5") {
                Ok(s) => s,
                Err(_) => return vec![],
            };
            let pat = format!("%{}%", q2.chars().take(50).collect::<String>());
            let rows = match stmt.query_map(rusqlite::params![domain2, pat], |r| Ok(serde_json::json!({"id": r.get::<_,i64>(0)?, "title": r.get::<_,Option<String>>(1)?, "snippet": r.get::<_,String>(2)?.chars().take(200).collect::<String>()}))) {
                Ok(r) => r,
                Err(_) => return vec![],
            };
            for r in rows.flatten() { out.push(r); }
            out
        }).await.unwrap_or_default()
    };
    if hits.is_empty() {
        let pool3 = st.pool.clone();
        tokio::task::spawn_blocking(move || {
            if let Ok(conn) = pool3.get() {
                let now = chrono::Utc::now().timestamp();
                let _ = conn.execute("INSERT INTO findings(run_id,claim,evidence,source,confidence,ts) VALUES (?1,'abstention','no hits','copilot',0,?2)", rusqlite::params![id, now]);
            }
        }).await.ok();
        return Ok(Json(serde_json::json!({"suggestions": []})));
    }
    let suggestions: Vec<serde_json::Value> = hits
        .into_iter()
        .map(|h| serde_json::json!({"hit": h}))
        .collect();
    Ok(Json(serde_json::json!({"suggestions": suggestions})))
}
