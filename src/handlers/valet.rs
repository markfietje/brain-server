//! Valet surfaces: the personal assistant's cranks + views.
//!
//! Handlers are protocol adapters ONLY: parse → authorize → one
//! `spawn_blocking` into [`crate::workflow::valet`] (the domain core) →
//! read-seam shaping → response. No daemon, no scheduler: `due` is a crank
//! invoked by cron via the CLI; the cron recipe IS the scheduler.

use axum::{Json, extract::State};
use std::sync::Arc;

use crate::AppState;
use crate::handlers::HandlerError;
use crate::workflow::valet as core;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DueRequest {
    /// Evaluate "now" at this unix-seconds timestamp (testing/replay).
    #[serde(default)]
    pub now: Option<i64>,
}

/// POST /workflow/valet/due — the crank. Fires every due valet envelope
/// (bounded batch), each in its own audited tx; a repeat re-arms its next
/// envelope. Exactly-once per envelope via the outbox idempotency key.
pub async fn post_due(
    State(state): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    body: Option<Json<DueRequest>>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    super::authorize(&principal, crate::auth::Action::Write, "", "global")?;
    super::authorize_role(&principal, &state.pool, "workflow")?;
    let now = body
        .and_then(|b| b.0.now)
        .unwrap_or_else(|| chrono::Utc::now().timestamp());
    let pool = state.pool.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let mut conn = pool.get().map_err(|e| format!("{e}"))?;
        let items = {
            let items = core::due(&conn, now);
            if items.len() >= core::MAX_DUE_BATCH {
                return Err(format!(
                    "due backlog at cap {} — drain before adding more",
                    core::MAX_DUE_BATCH
                ));
            }
            items
        };
        let mut fired = 0usize;
        let mut suppressed = 0usize;
        let mut already = 0usize;
        for item in items {
            // One tx per envelope: a transition and its evidence commit or
            // roll back together.
            let tx = conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|e| format!("{e}"))?;
            match core::fire(&tx, &item, now).map_err(|e| format!("{e}"))? {
                core::FireOutcome::Fired { .. } => fired += 1,
                core::FireOutcome::SuppressedNoConsent { .. } => suppressed += 1,
                core::FireOutcome::AlreadyFired => already += 1,
            }
            tx.commit().map_err(|e| format!("{e}"))?;
        }
        Ok(serde_json::json!({
            "ok": true,
            "fired": fired,
            "suppressed_no_consent": suppressed,
            "already_fired": already,
        }))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?
    .map_err(HandlerError::internal)?;
    Ok(Json(result))
}

/// GET /workflow/valet/brief — today's derived context: due/overdue runs,
/// drafts pending approval with their lint scores, and the evening-capture
/// notes from the trailing window (the Engine Diary raw material). Read-only,
/// read-seam sanitized.
pub async fn get_brief(
    State(state): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    super::authorize(&principal, crate::auth::Action::Read, "", "global")?;
    super::authorize_role(&principal, &state.pool, "workflow")?;
    let pool = state.pool.clone();
    let now = chrono::Utc::now().timestamp();
    let brief = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let conn = pool.get().map_err(|e| format!("{e}"))?;
        let due: Vec<serde_json::Value> = core::due(&conn, now)
            .into_iter()
            .map(|i| {
                serde_json::json!({
                    "run_id": i.run_id,
                    "kind": i.kind,
                    "what": i.state.what,
                    "due_at": i.state.due_at,
                    "overdue_secs": now - i.state.due_at,
                })
            })
            .collect();

        // Drafts pending approval + their advisory lint scores.
        let mut drafts: Vec<serde_json::Value> = Vec::new();
        for (id, content, lint) in core::pending_drafts(&conn).map_err(|e| format!("{e}"))? {
            let lint: serde_json::Value = lint
                .as_deref()
                .and_then(|l| serde_json::from_str(l).ok())
                .unwrap_or(serde_json::json!(null));
            drafts.push(serde_json::json!({
                "proposal_id": id,
                "excerpt": crate::gate::sanitize_stored(&content, false, &None),
                "lint": lint,
            }));
        }

        // Evening captures: notes on valet runs in the trailing 24h.
        let mut notes: Vec<serde_json::Value> = Vec::new();
        for (run_id, content) in
            core::evening_notes(&conn, now - 86_400).map_err(|e| format!("{e}"))?
        {
            notes.push(serde_json::json!({
                "run_id": run_id,
                "content": crate::gate::sanitize_stored(&content, false, &None),
            }));
        }

        let consented = core::consent_in_force(&conn, core::SOLE_SUBJECT, "signal")
            .map_err(|e| format!("{e}"))?;
        Ok(serde_json::json!({
            "now": now,
            "due": due,
            "drafts_pending": drafts,
            "evening_notes": notes,
            "signal_consent_in_force": consented,
        }))
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?
    .map_err(HandlerError::internal)?;
    Ok(Json(brief))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsentRequest {
    pub granted: bool,
    #[serde(default = "default_subject")]
    pub subject: String,
    #[serde(default = "default_channel")]
    pub channel: String,
}

fn default_subject() -> String {
    core::SOLE_SUBJECT.to_string()
}

fn default_channel() -> String {
    "signal".to_string()
}

/// PUT /workflow/valet/consent — the Outreach-lite registry write path.
/// One subject, one channel; anything else refuses loudly.
pub async fn put_consent(
    State(state): State<Arc<AppState>>,
    principal: crate::handlers::auth::OptPrincipal,
    Json(body): Json<ConsentRequest>,
) -> Result<Json<serde_json::Value>, HandlerError> {
    let principal = principal.0;
    super::authorize(&principal, crate::auth::Action::Write, "", "global")?;
    super::authorize_role(&principal, &state.pool, "workflow")?;
    let pool = state.pool.clone();
    let now = chrono::Utc::now().timestamp();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let conn = pool.get().map_err(|e| format!("{e}"))?;
        let res = if body.granted {
            core::consent_grant(&conn, &body.subject, &body.channel, now)
        } else {
            core::consent_revoke(&conn, &body.subject, &body.channel, now)
        };
        if let Err(e) = res {
            return Err(format!("consent_refused:{e}"));
        }
        crate::audit::record_tenant(
            &conn,
            crate::audit::AuditKind::Workflow,
            super::recall::principal_label(&principal).as_str(),
            "valet-consent",
            crate::audit::AuditStatus::Ok,
            &format!(
                "valet/consent {} {}={}",
                body.channel, body.subject, body.granted
            ),
            "global",
        );
        Ok(())
    })
    .await
    .map_err(|e| HandlerError::internal(format!("{e}")))?
    .map_err(|e| {
        if let Some(msg) = e.strip_prefix("consent_refused:") {
            HandlerError::bad_request("consent_refused", msg.to_string())
        } else {
            HandlerError::internal(e)
        }
    })?;
    Ok(Json(
        serde_json::json!({"ok": true, "granted": body.granted}),
    ))
}
