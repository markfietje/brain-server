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
        let mut stmt = conn
            .prepare(
                "SELECT id, content, lint_json FROM proposals
                  WHERE status='pending' AND kind='draft' ORDER BY id DESC LIMIT 20",
            )
            .map_err(|e| format!("{e}"))?;
        let rows: Vec<(i64, String, Option<String>)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(|e| format!("{e}"))?
            .collect::<Result<_, _>>()
            .map_err(|e| format!("{e}"))?;
        for (id, content, lint) in rows {
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
        let mut stmt = conn
            .prepare(
                "SELECT n.run_id, n.content FROM case_notes n
                  JOIN workflow_runs r ON r.id = n.run_id
                  WHERE r.kind LIKE 'valet/%' AND n.created_at > ?1
                  ORDER BY n.id DESC LIMIT 50",
            )
            .map_err(|e| format!("{e}"))?;
        let nrows: Vec<(i64, String)> = stmt
            .query_map(rusqlite::params![now - 86_400], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .map_err(|e| format!("{e}"))?
            .collect::<Result<_, _>>()
            .map_err(|e| format!("{e}"))?;
        for (run_id, content) in nrows {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn test_state() -> (tempfile::TempDir, Arc<crate::AppState>) {
        crate::register_sqlite_vec();
        let dir = tempfile::TempDir::new().expect("temp dir");
        let db_path = dir.path().join("brain.db");
        let mgr = r2d2_sqlite::SqliteConnectionManager::file(&db_path);
        let pool: crate::Pool = r2d2::Pool::builder().build(mgr).expect("pool");
        brain_server::migration::run_migration(&mut pool.get().expect("conn"), 0)
            .expect("migration");
        let state = Arc::new(crate::AppState {
            model: Arc::new(
                brain_server::embed::StaticEmbedder::new(crate::config::MODEL_ID).expect("model"),
            ),
            registry: crate::domain_registry::DomainRegistry::new(pool.clone(), &db_path, false),
            pool,
            db_path,
            connection_tracker: Arc::new(crate::ConnectionTracker::new()),
            rate_limiter: Arc::new(crate::RateLimiter::new()),
            snapshot: crate::integrity::SnapshotState::default(),
            audit_chain_cache: Arc::new(std::sync::Mutex::new(None)),
            auth_mode: crate::auth::AuthMode::Opaque,
            key_store: crate::auth::jwks::KeyStore::default(),
            revocation_cache: Arc::new(crate::auth::revocation::RevocationCache::new()),
            jwt_issuer: String::new(),
            jwt_audience: String::new(),
            oidc_config: crate::handlers::well_known::OidcConfig::unconfigured(),
            ump_events: tokio::sync::broadcast::channel(16).0,
            alert_events: tokio::sync::broadcast::channel(16).0,
            alert_seq: std::sync::atomic::AtomicU64::new(0),
            chain_watch: crate::alert::ChainWatchState::default(),
        });
        (dir, state)
    }

    fn seed(conn: &rusqlite::Connection, now: i64) {
        // One overdue + one future reminder.
        for (what, due) in [("overdue post", now - 3600), ("future post", now + 3600)] {
            conn.execute(
                "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
                 VALUES ('personal', 'valet/reminder', ?1, 0, 'active', 1, 1)",
                rusqlite::params![
                    &core::stamp_state(what, due, core::REPEAT_NONE).unwrap()
                ],
            )
            .unwrap();
        }
        // One pending draft with a lint report + one decided draft (must NOT appear).
        let lint = brain_server::valet_style::LintReport {
            score: 60,
            findings: vec![],
            style_memory_hash: "h".repeat(64),
        };
        conn.execute(
            "INSERT INTO proposals(kind, content, novelty, status, lint_json, created_at)
             VALUES ('draft', 'draft body one', 0.5, 'pending', ?1, ?2)",
            rusqlite::params![serde_json::to_string(&lint).unwrap(), now - 100],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO proposals(kind, content, novelty, status, decided_at, created_at)
             VALUES ('draft', 'draft body two', 0.5, 'approved', ?1, ?2)",
            rusqlite::params![now - 50, now - 60],
        )
        .unwrap();
        // An evening-capture note on the overdue run, inside the window.
        conn.execute(
            "INSERT INTO case_notes(domain, run_id, author, content, created_at)
             VALUES ('personal', 1, 'operator', 'shipped the brief feature; learned to pin caps', ?1)",
            rusqlite::params![now - 10],
        )
        .unwrap();
    }

    /// The morning brief composes the whole picture: due/overdue (only what
    /// is actually due), drafts PENDING approval with their lint scores, and
    /// the trailing-window evening notes linked to their runs.
    #[tokio::test]
    async fn brief_includes_due_overdue_pending_with_lint_scores() {
        let (_dir, state) = test_state();
        {
            let conn = state.pool.get().unwrap();
            let now = chrono::Utc::now().timestamp();
            seed(&conn, now);
        }
        let res = get_brief(
            axum::extract::State(state.clone()),
            crate::handlers::auth::OptPrincipal(None),
        )
        .await
        .expect("brief");
        let v = res.0;
        let due = v["due"].as_array().expect("due array");
        assert_eq!(due.len(), 1, "only the overdue envelope is due");
        assert_eq!(due[0]["what"], "overdue post");
        assert!(due[0]["overdue_secs"].as_i64().unwrap() > 0);
        let drafts = v["drafts_pending"].as_array().expect("drafts array");
        assert_eq!(drafts.len(), 1, "only the PENDING draft appears");
        assert_eq!(drafts[0]["lint"]["score"], 60);
        let notes = v["evening_notes"].as_array().expect("notes array");
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0]["run_id"], 1, "notes link back to their runs");
    }

    /// The consent gate surfaces its state in the brief (no silent sends).
    #[tokio::test]
    async fn brief_reports_signal_consent_state() {
        let (_dir, state) = test_state();
        let res = get_brief(
            axum::extract::State(state.clone()),
            crate::handlers::auth::OptPrincipal(None),
        )
        .await
        .expect("brief");
        assert_eq!(res.0["signal_consent_in_force"], false);
    }
}
