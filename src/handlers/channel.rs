//! The case-scoped channel surfaces (Channel).
//!
//! - `POST /workflow/runs/{id}/notes {content}` — screened at write
//!   (empty/≤4000/blocklist), stored through the invisible-strip +
//!   markdown-ref seam; mentions (`@skill:<tag>`, `@principal`) resolve into
//!   swarm invites in the SAME transaction as the note.
//! - `GET /workflow/runs/{id}/notes?limit=&offset=` — the channel view,
//!   chronological, policy-expired rows hidden before the page split,
//!   every emitted string on the read seam.
//! - `POST /workflow/runs/{id}/notes/{invite_id}/accept` — the invitee
//!   joins the channel (Relay's accept machinery, smaller).

use axum::{
    Json,
    extract::{Path, Query, State},
};
use std::collections::HashMap;
use std::sync::Arc;

use crate::AppState;
use crate::handlers::HandlerError;
use crate::handlers::auth::OptPrincipal;
use crate::workflow::channel::{self, ChannelError};

fn channel_err(e: ChannelError) -> HandlerError {
    match e {
        ChannelError::InvalidContent(w) => HandlerError::bad_request(
            "note_invalid",
            match w {
                "empty" => "note content must not be empty",
                "too_long" => "note content exceeds 4000 chars",
                _ => "note content matches a blocked prompt-injection pattern",
            },
        ),
        ChannelError::Unresolved(u) => HandlerError::bad_request_with(
            "mentions_unresolved",
            "these mentions resolved to nobody in this domain",
            serde_json::json!({ "unresolved": u }),
        ),
        ChannelError::TooManyInvites(n) => HandlerError::bad_request_with(
            "invite_limit",
            format!(
                "a note may invite at most {} principals",
                channel::MAX_INVITES_PER_NOTE
            ),
            serde_json::json!({ "resolved": n }),
        ),
        ChannelError::NotFound(m) => HandlerError::not_found(m),
        ChannelError::Database(m) => HandlerError::internal(m),
    }
}

/// Presence rides every mutating channel tx (best-effort, never gates the
/// work) — the Relay handler's posture.
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
pub struct NoteRequest {
    pub content: String,
}

/// `POST /workflow/runs/{id}/notes`
pub async fn post_notes(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    Path(id): Path<i64>,
    Json(body): Json<NoteRequest>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    let domain = super::workflow::run_domain(&state, id).await?;
    super::authorize(&principal, crate::auth::Action::Write, "", &domain)?;

    // Screen BEFORE any write: bounds + blocklist + strip. The stored form is
    // viewer-independent; reads re-apply the read seam regardless. One clock
    // read per request — the row, its events, and the receipt share it.
    let screened = channel::screen_content(&body.content).map_err(channel_err)?;
    let actor = super::recall::principal_label(&principal);
    let mentions = channel::parse_mentions(&screened);
    let now = chrono::Utc::now().timestamp();
    let screened_in_tx = screened.clone();
    let actor_in_tx = actor.clone();

    let outcome = tokio::task::spawn_blocking(move || -> Result<_, HandlerError> {
        let pool = super::resolve_domain_pool(&state.registry, None)?;
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        let mut tx = crate::workflow::tx::WorkflowTx::begin(&mut conn)
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        // Resolution runs INSIDE the transition so the note, its invites, and
        // their lineage events commit against one consistent roster snapshot.
        let invitees = channel::resolve_mentions(tx.tx(), &domain, &mentions, &actor_in_tx)
            .map_err(channel_err)?;
        let key_suffix = format!("{now}-{}", rand::random::<u32>());
        let out = channel::insert_note(
            tx.tx(),
            &channel::NoteDraft {
                domain: &domain,
                run_id: id,
                author: &actor_in_tx,
                screened_content: &screened_in_tx,
                key_suffix: &key_suffix,
                now,
            },
            &invitees,
        )
        .map_err(channel_err)?;
        crew_touch(tx.tx(), &domain, &actor_in_tx, id);
        tx.commit()
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        Ok(out)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?;
    let out = outcome?;

    let invites: Vec<serde_json::Value> = out
        .invites
        .iter()
        .map(|(invite_id, to)| {
            serde_json::json!({
                "invite_id": invite_id,
                "to": crate::gate::sanitize_read(to, false, &principal),
            })
        })
        .collect();
    Ok(Json(serde_json::json!({
        "run_id": id,
        "note_id": out.note_id,
        "author": crate::gate::sanitize_read(&actor, false, &principal),
        "content": crate::gate::sanitize_read(&screened, false, &principal),
        "created_at": now,
        "invites": invites,
    })))
}

/// `GET /workflow/runs/{id}/notes?limit=&offset=`
pub async fn get_notes(
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
    let now = chrono::Utc::now().timestamp();

    // The SAME three-layer retention resolution as the decay path: kill-switch
    // off = nothing decays; a bound profile's block replaces the server-wide
    // map; the `case-note` kind is looked up in whichever governs this domain.
    // Resolved inside the blocking task (DB access stays off the reactor); an
    // unreadable profile degrades to the server-wide map, never to an error.
    let rows = tokio::task::spawn_blocking(move || -> Result<_, HandlerError> {
        let pool = super::resolve_domain_pool(&state.registry, None)?;
        let conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        let ttl_days: Option<i64> = if crate::config::brain_retention_enabled() {
            let profile_map = brain_server::profile::profile_for_domain(&conn, &domain)
                .ok()
                .flatten()
                .and_then(|p| p.retention_map());
            let server_wide = crate::config::retention_kind_days();
            profile_map
                .as_ref()
                .and_then(|m| m.get("case-note"))
                .or_else(|| server_wide.get("case-note"))
                .copied()
        } else {
            None
        };
        channel::list_notes(&conn, id, ttl_days, now, offset, limit).map_err(channel_err)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?;
    let rows = rows?;
    let payload: Vec<serde_json::Value> = rows
        .iter()
        .map(|n| {
            serde_json::json!({
                "id": n.id,
                "kind": n.kind,
                "author": crate::gate::sanitize_read(&n.author, false, &principal),
                "content": crate::gate::sanitize_read(&n.content, false, &principal),
                "addressed_to": n
                    .addressed_to
                    .as_deref()
                    .map(|a| crate::gate::sanitize_read(a, false, &principal)),
                "parent_note_id": n.parent_note_id,
                "state": n.state,
                "created_at": n.created_at,
            })
        })
        .collect();
    Ok(Json(serde_json::json!({
        "run_id": id,
        "count": payload.len(),
        "notes": payload,
    })))
}

/// `POST /workflow/runs/{id}/notes/{invite_id}/accept`
pub async fn post_invite_accept(
    State(state): State<Arc<AppState>>,
    principal: OptPrincipal,
    path: Path<(i64, i64)>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let (id, invite_id) = *path;
    let principal = principal.0;
    let domain = super::workflow::run_domain(&state, id).await?;
    super::authorize(&principal, crate::auth::Action::Write, "", &domain)?;
    let actor = super::recall::principal_label(&principal);

    let moved = tokio::task::spawn_blocking(move || -> Result<_, HandlerError> {
        let pool = super::resolve_domain_pool(&state.registry, None)?;
        let mut conn = pool
            .get()
            .map_err(|e| HandlerError::internal(format!("{e}")))?;
        let mut tx = crate::workflow::tx::WorkflowTx::begin(&mut conn)
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        let moved = channel::accept_invite(
            tx.tx(),
            id,
            invite_id,
            &actor,
            chrono::Utc::now().timestamp(),
        )
        .map_err(channel_err)?;
        if moved {
            crew_touch(tx.tx(), &domain, &actor, id);
        }
        tx.commit()
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        Ok(moved)
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?;
    let moved = moved?;
    Ok(Json(serde_json::json!({
        "run_id": id,
        "invite_id": invite_id,
        "moved": moved,
        "state": if moved { channel::INVITE_ACCEPTED } else { channel::INVITE_PENDING },
    })))
}
