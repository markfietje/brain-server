//! `GET /chat/sessions`, `GET /chat/sessions/{session}/turns`,
//! `PATCH /chat/sessions/{session}`, `DELETE /chat/sessions/{session}` —
//! the chat session store's four wire surfaces.
//!
//! PROTOCOL ADAPTERS ONLY (the convergence law): parse → OptPrincipal →
//! authorize → owner-fence resolution → one `spawn_blocking` → the service
//! core → read-seam shaping → response. All SQL, bounds and invariants live
//! in `service::chat_sessions`; this file holds no statement.
//!
//! The owner fence is the review-queue's shape, reached through the SAME
//! resolver rather than a second opinion about ownership: `authorize(Read)`
//! first (the gate table's row), then `record_read_gate().owner_in` decides
//! whose rows the SQL admits — `None` (loopback/opaque/Admin) is
//! unrestricted, a narrowed principal sees and moves only its own rows. A
//! foreign key answers the SAME `chat_session_not_found` as an unknown one
//! (no existence oracle).
//!
//! Read seam: every emitted text field (labels, turn content) passes
//! `sanitize_read` here — the service returns stored forms.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use serde_json::json;

use crate::AppState;
use crate::handlers::HandlerError;
use crate::handlers::auth::OptPrincipal;
use crate::service::chat_sessions::{self, ChatSessionError, OwnerScope, SessionRow, TurnRow};

/// The wire's bounded query (the bounds law: every input bounded, the
/// Rust-side arbiter decides — never SQL).
#[derive(Debug, Deserialize)]
pub struct PageQuery {
    limit: Option<i64>,
    offset: Option<i64>,
}

fn page_params(q: &PageQuery) -> (i64, i64) {
    (q.limit.unwrap_or(0), q.offset.unwrap_or(0))
}

/// The owner stamp every chat write carries: the principal's `sub`, or the
/// loopback operator's label. Always attributable — the DSAR lesson.
fn owner_stamp(principal: &OptPrincipal) -> String {
    crate::handlers::gate::content_owner_stamp(&principal.0)
        .unwrap_or_else(|| "loopback".to_owned())
}

/// The fence the READS move through: the record gate's resolved owner set.
/// Re-resolved per request — the queue's own posture.
fn owner_scope(principal: &OptPrincipal, pool: &crate::Pool) -> Result<OwnerScope, HandlerError> {
    let gate = crate::handlers::gate::record_read_gate(&principal.0, pool);
    Ok(gate
        .owner_in
        .map_or_else(OwnerScope::unrestricted, OwnerScope::narrowed))
}

fn map_session_err(e: ChatSessionError) -> HandlerError {
    match e {
        ChatSessionError::SessionNotFound => HandlerError::not_found("chat_session_not_found"),
        ChatSessionError::InvalidKey => {
            HandlerError::bad_request("chat_session_invalid", e.to_string())
        }
        ChatSessionError::InvalidLabel(_) => {
            HandlerError::bad_request("chat_label_invalid", e.to_string())
        }
        ChatSessionError::Storage(msg) => HandlerError::internal(msg),
    }
}

fn session_json(row: &SessionRow) -> serde_json::Value {
    json!({
        "session_key": row.session_key,
        "label": row.label,
        "turn_count": row.turn_count,
        "created_at": row.created_at,
        "last_turn_at": row.last_turn_at,
    })
}

/// `GET /chat/sessions?limit&offset` — the sidebar's list. Read-gated, then
/// owner-scoped in SQL.
pub async fn list_sessions(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Query(q): Query<PageQuery>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Read, "", "global")?;
    let (limit, offset) = page_params(&q);
    let scope = owner_scope(&principal, &state.pool)?;
    let rows = tokio::task::spawn_blocking(move || -> Result<_, HandlerError> {
        let conn = state.pool.get().map_err(HandlerError::db_down)?;
        chat_sessions::list_sessions(&conn, &scope, limit, offset).map_err(map_session_err)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    let (sessions, total) = rows;
    let shaped: Vec<serde_json::Value> = sessions
        .iter()
        .map(|row| {
            let mut v = session_json(row);
            // Read seam: the label is stored text the caller typed.
            v["label"] = json!(crate::gate::sanitize_read(&row.label, false, &principal.0));
            v
        })
        .collect();
    Ok(Json(json!({ "sessions": shaped, "total": total })))
}

/// `GET /chat/sessions/{session}/turns?limit&offset` — one session's replay
/// page, oldest first. The same refusal for an unknown key and a foreign key.
pub async fn list_turns(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(session): Path<String>,
    Query(q): Query<PageQuery>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Read, "", "global")?;
    let (limit, offset) = page_params(&q);
    let scope = owner_scope(&principal, &state.pool)?;
    let rows = tokio::task::spawn_blocking(move || -> Result<_, HandlerError> {
        let conn = state.pool.get().map_err(HandlerError::db_down)?;
        chat_sessions::list_turns(&conn, &scope, &session, limit, offset).map_err(map_session_err)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    let (turns, total) = rows;
    let shaped: Vec<serde_json::Value> = turns
        .iter()
        .map(|turn: &TurnRow| {
            let content = crate::gate::sanitize_read(&turn.content, false, &principal.0);
            // The grounding event is the kernel's OWN json — re-parsed, never
            // re-shaped, so replay shows exactly what the turn was grounded on.
            let grounding = turn
                .grounding_json
                .as_deref()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());
            json!({
                "seq": turn.seq,
                "role": turn.role,
                "content": content,
                "grounding": grounding,
                "tier": turn.tier,
                "created_at": turn.created_at,
            })
        })
        .collect();
    Ok(Json(json!({ "turns": shaped, "total": total })))
}

#[derive(Debug, Deserialize)]
pub struct RenameRequest {
    label: String,
}

/// `PATCH /chat/sessions/{session}` — rename. Write-gated (the authz table's
/// Write row); the owner fence still decides whose rows the rename reaches.
pub async fn rename_session(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(session): Path<String>,
    Json(req): Json<RenameRequest>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Write, "", "global")?;
    let scope = owner_scope(&principal, &state.pool)?;
    let actor = owner_stamp(&principal);
    let pool = state.pool.clone();
    let row = tokio::task::spawn_blocking(move || -> Result<_, HandlerError> {
        let mut conn = pool.get().map_err(HandlerError::db_down)?;
        chat_sessions::rename_session(&mut conn, &scope, &session, &req.label, &actor)
            .map_err(map_session_err)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    let mut v = session_json(&row);
    v["label"] = json!(crate::gate::sanitize_read(&row.label, false, &principal.0));
    Ok(Json(v))
}

/// `DELETE /chat/sessions/{session}` — the erasure. Read-gated by design (a
/// user may erase its own transcript without holding Write); Admin/loopback
/// reach any row, a narrowed principal only its own — the owner fence, never
/// a second gate table.
pub async fn delete_session(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(session): Path<String>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    super::authorize(&principal.0, crate::auth::Action::Read, "", "global")?;
    let scope = owner_scope(&principal, &state.pool)?;
    let actor = owner_stamp(&principal);
    let pool = state.pool.clone();
    let erased = tokio::task::spawn_blocking(move || -> Result<_, HandlerError> {
        let mut conn = pool.get().map_err(HandlerError::db_down)?;
        chat_sessions::delete_session(&mut conn, &scope, &session, &actor).map_err(map_session_err)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("task join error: {e}")))??;
    Ok(Json(json!({ "ok": true, "turns_deleted": erased })))
}
