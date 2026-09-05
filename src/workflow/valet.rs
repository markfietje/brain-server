//! Valet: the personal assistant's scheduler-as-cases core.
//!
//! A reminder is NOT a new concept — it is an ordinary governed run
//! (`kind = 'valet/reminder' | 'valet/digest'`) whose state carries
//! `{what, due_at, repeat, channel}` and whose deadline rides the same
//! `sla_deadline` convention every other envelope consumer reads
//! (`workflow_lineage`, `relay`). Firing is a crank (`brain valet due` →
//! `POST /workflow/valet/due`): request-scoped, daemon-free, idempotent.
//!
//! Laws this module encodes:
//! - **A double cron never double-fires.** The alert outbox row carries the
//!   idempotency key `valet-{run}-{due_at}`; `INSERT OR IGNORE` makes the
//!   second fire a no-op, and the CAS on `state_revision` makes concurrent
//!   firers serialize.
//! - **`repeat` re-arms a NEW envelope** (next `due_at` in the SAME row's
//!   state) via CAS — never by resurrecting a fired one.
//! - **No consent, no send.** The one-subject Outreach-lite registry gates
//!   the Signal channel: without an in-force grant the reminder still fires
//!   (the run's own lifecycle completes) but NOTHING is enqueued for
//!   delivery, and the suppression is audited + counted.
//! - **Metadata-only envelopes.** The bus payload carries what was screened
//!   at WRITE time (the reminder label passed the injection screen when the
//!   run was created); nothing unscreened ever enters the outbox here.
//! - **Audit-per-write**: every fire/suppress/re-arm emits its audit row
//!   inside the caller's transaction.

use crate::audit::AuditStatus;
use rusqlite::{Connection, OptionalExtension, params};

pub(crate) const KIND_REMINDER: &str = "valet/reminder";
pub(crate) const KIND_DIGEST: &str = "valet/digest";

/// The lineage topic valet due-events ride; the drain worker maps this topic
/// family onto the dedicated `valet/due` alert kind.
pub(crate) const TOPIC_VALET_DUE: &str = "workflow/valet-due";

/// Bounds law (pinned below).
pub(crate) const MAX_DUE_SCAN: i64 = 1_000;
pub(crate) const MAX_DUE_BATCH: usize = 100;
pub(crate) const MAX_WHAT_LEN: usize = 500;

pub(crate) const REPEAT_NONE: &str = "none";
pub(crate) const REPEAT_DAILY: &str = "daily";
pub(crate) const REPEAT_WEEKLY: &str = "weekly";

const DAILY_SECS: i64 = 86_400;
const WEEKLY_SECS: i64 = 7 * DAILY_SECS;

/// Outreach-lite subject: exactly one (the operator). Pinned by the test
/// asserting no other subject can hold an in-force grant.
pub(crate) const SOLE_SUBJECT: &str = "owner";

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ValetState {
    pub what: String,
    pub due_at: i64,
    #[serde(rename = "repeat")]
    pub repeat: String,
    pub channel: String,
    #[serde(default)]
    pub fire_count: i64,
    #[serde(default)]
    pub last_fired_at: Option<i64>,
}

/// Stamp the canonical valet state JSON (including the `sla_deadline`
/// mirror other envelope consumers already read).
pub(crate) fn stamp_state(what: &str, due_at: i64, repeat: &str) -> Result<String, String> {
    if what.trim().is_empty() {
        return Err("what_empty".into());
    }
    if what.len() > MAX_WHAT_LEN {
        return Err("what_too_long".into());
    }
    if !matches!(repeat, REPEAT_NONE | REPEAT_DAILY | REPEAT_WEEKLY) {
        return Err("repeat_invalid".into());
    }
    let st = ValetState {
        what: what.to_string(),
        due_at,
        repeat: repeat.to_string(),
        channel: "signal".to_string(),
        fire_count: 0,
        last_fired_at: None,
    };
    // sla_deadline mirrors due_at so lineage/relay readers see ONE convention.
    let mut v = serde_json::to_value(&st).map_err(|e| e.to_string())?;
    v["sla_deadline"] = serde_json::json!(due_at);
    serde_json::to_string(&v).map_err(|e| e.to_string())
}

pub(crate) fn parse_state(raw: &str) -> Option<ValetState> {
    serde_json::from_str(raw).ok()
}

fn next_due(st: &ValetState) -> Option<i64> {
    match st.repeat.as_str() {
        REPEAT_DAILY => Some(st.due_at + DAILY_SECS),
        REPEAT_WEEKLY => Some(st.due_at + WEEKLY_SECS),
        _ => None,
    }
}

#[derive(Debug)]
pub struct ValetDue {
    pub run_id: i64,
    pub revision: i64,
    pub kind: String,
    pub state: ValetState,
}

/// Runs whose envelope came due: status active, kind `valet/%`,
/// `due_at <= now`. SQL only fetches candidates (bounded scan); the Rust-side
/// arbiter decides fate (bounds law — SQL never decides a row's fate).
/// Overdue ranks reminders before digests, then earliest deadline first.
pub(crate) fn due(conn: &Connection, now: i64) -> Vec<ValetDue> {
    let rows: Vec<(i64, i64, String, String)> = conn
        .prepare(
            "SELECT id, state_revision, kind, state_json FROM workflow_runs
              WHERE status = 'active' AND kind LIKE 'valet/%'
              ORDER BY id ASC LIMIT ?1",
        )
        .and_then(|mut s| {
            s.query_map(params![MAX_DUE_SCAN], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .and_then(|it| it.collect())
        })
        .unwrap_or_default();
    let mut out: Vec<ValetDue> = rows
        .into_iter()
        .filter_map(|(id, rev, kind, raw)| {
            parse_state(&raw)
                .filter(|st| st.due_at <= now)
                .map(|st| ValetDue {
                    run_id: id,
                    revision: rev,
                    kind,
                    state: st,
                })
        })
        .collect();
    out.sort_by(|a, b| {
        let pa = i32::from(a.kind == KIND_DIGEST);
        let pb = i32::from(b.kind == KIND_DIGEST);
        pa.cmp(&pb).then(a.state.due_at.cmp(&b.state.due_at))
    });
    out.truncate(MAX_DUE_BATCH);
    out
}

#[derive(Debug, PartialEq, Eq)]
pub enum FireOutcome {
    /// Fired now; a repeat re-armed with this next due_at.
    Fired { rearmed_due_at: Option<i64> },
    /// This exact envelope already fired (idempotency key seen) — no-op.
    AlreadyFired,
    /// Fired locally but suppressed delivery: no in-force consent.
    SuppressedNoConsent { rearmed_due_at: Option<i64> },
}

/// Fire ONE due envelope inside the caller's transaction. Exactly-once per
/// `(run, due_at)` via the outbox idempotency key + CAS on state_revision.
pub(crate) fn fire(
    conn: &Connection,
    item: &ValetDue,
    now: i64,
) -> Result<FireOutcome, rusqlite::Error> {
    let st = &item.state;
    let key = format!("valet-{}-{}", item.run_id, st.due_at);
    // Idempotency FIRST: if this envelope's outbox row exists, a previous
    // crank already handled it — touch nothing (a double cron is safe).
    let dup: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM outbox WHERE idempotency_key = ?1",
            params![key],
            |r| r.get::<_, i64>(0),
        )
        .map(|c| c > 0)?;
    if dup {
        return Ok(FireOutcome::AlreadyFired);
    }
    let rearm = next_due(st);
    let mut next = st.clone();
    next.fire_count += 1;
    next.last_fired_at = Some(now);
    if let Some(d) = rearm {
        next.due_at = d;
    }
    let next_json = {
        let mut v = serde_json::to_value(&next).map_err(|e| {
            rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(e.to_string())))
        })?;
        v["sla_deadline"] = serde_json::json!(next.due_at);
        serde_json::to_string(&v).map_err(|e| {
            rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(e.to_string())))
        })?
    };
    let status = if rearm.is_some() { "active" } else { "fired" };
    crate::workflow::state::cas_update(conn, item.run_id, item.revision, &next_json, status, now)
        .map_err(|e| {
        rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(1),
            Some(format!("valet fire cas failed: {e}")),
        )
    })?;

    // Consent gate (Outreach-lite, one subject, one channel): no in-force
    // grant → nothing leaves for Signal. The lifecycle still completed.
    let consented = consent_in_force(conn, SOLE_SUBJECT, &st.channel)?;
    if !consented {
        audit_write(
            conn,
            item.run_id,
            AuditStatus::Denied,
            &format!("valet/fire suppressed-no-consent key={key}"),
        );
        return Ok(FireOutcome::SuppressedNoConsent {
            rearmed_due_at: rearm,
        });
    }
    // Metadata-only envelope: the label was injection-screened at write time
    // (brain valet add / POST /workflow/runs callers screen `what`).
    let payload = serde_json::json!({
        "topic": TOPIC_VALET_DUE,
        "run_id": item.run_id,
        "kind": item.kind,
        "what": st.what,
        "due_at": st.due_at,
        "channel": st.channel,
    })
    .to_string();
    let inserted =
        crate::workflow::outbox::enqueue(conn, item.run_id, TOPIC_VALET_DUE, &payload, &key, now)?
            .0;
    audit_write(
        conn,
        item.run_id,
        AuditStatus::Ok,
        &format!("valet/fire key={key}"),
    );
    if !inserted {
        return Ok(FireOutcome::AlreadyFired);
    }
    Ok(FireOutcome::Fired {
        rearmed_due_at: rearm,
    })
}

fn audit_write(conn: &Connection, run_id: i64, status: AuditStatus, detail: &str) {
    crate::workflow::audit_write(conn, run_id, &format!("run:{run_id}"), status, detail);
}

// ── Outreach-lite: the one-subject consent registry ────────────────────────

/// Grant consent. The ONLY writer is this function (callers go through the
/// authenticated route); subjects are hashed at rest like the full outreach
/// registry.
pub(crate) fn consent_grant(
    conn: &Connection,
    subject: &str,
    channel: &str,
    now: i64,
) -> Result<(), String> {
    validate_subject_channel(subject, channel)?;
    let hash = crate::audit::hash(subject);
    conn.execute(
        "INSERT INTO valet_consents(subject_hash, channel, granted_at, revoked_at)
         VALUES (?1, ?2, ?3, NULL)
         ON CONFLICT(subject_hash, channel) DO UPDATE SET granted_at = ?3, revoked_at = NULL",
        params![hash, channel, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub(crate) fn consent_revoke(
    conn: &Connection,
    subject: &str,
    channel: &str,
    now: i64,
) -> Result<(), String> {
    validate_subject_channel(subject, channel)?;
    let hash = crate::audit::hash(subject);
    let n = conn
        .execute(
            "UPDATE valet_consents SET revoked_at = ?3
              WHERE subject_hash = ?1 AND channel = ?2 AND revoked_at IS NULL",
            params![hash, channel, now],
        )
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Err("consent_not_found".into());
    }
    Ok(())
}

pub(crate) fn consent_in_force(
    conn: &Connection,
    subject: &str,
    channel: &str,
) -> Result<bool, rusqlite::Error> {
    let hash = crate::audit::hash(subject);
    let row: Option<i64> = conn
        .query_row(
            "SELECT granted_at FROM valet_consents
              WHERE subject_hash = ?1 AND channel = ?2 AND revoked_at IS NULL",
            params![hash, channel],
            |r| r.get(0),
        )
        .optional()?;
    Ok(row.is_some())
}

fn validate_subject_channel(subject: &str, channel: &str) -> Result<(), String> {
    if subject != SOLE_SUBJECT {
        return Err("subject_out_of_scope".into());
    }
    if channel != "signal" {
        return Err("channel_out_of_scope".into());
    }
    Ok(())
}

// ── the morning brief's projections ──────────────────────────────────────

/// One pending draft: (proposal id, content, lint_json).
pub(crate) type PendingDraft = (i64, String, Option<String>);

/// Drafts pending approval, newest first, bounded at 20 — the brief's
/// HITL queue with their advisory lint scores (the lint JSON is the
/// stored form; the excerpt sanitize stays handler-side).
pub(crate) fn pending_drafts(conn: &Connection) -> rusqlite::Result<Vec<PendingDraft>> {
    let mut stmt = conn.prepare(
        "SELECT id, content, lint_json FROM proposals
          WHERE status='pending' AND kind='draft' ORDER BY id DESC LIMIT 20",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    rows.collect()
}

/// Evening-capture notes on valet runs inside the trailing window:
/// (run id, content), newest first, bounded at 50. The valet-kind join is
/// the point — notes on other runs never leak into the diary view.
pub(crate) fn evening_notes(conn: &Connection, since: i64) -> rusqlite::Result<Vec<(i64, String)>> {
    let mut stmt = conn.prepare(
        "SELECT n.run_id, n.content FROM case_notes n
          JOIN workflow_runs r ON r.id = n.run_id
          WHERE r.kind LIKE 'valet/%' AND n.created_at > ?1
          ORDER BY n.id DESC LIMIT 50",
    )?;
    let rows = stmt.query_map(params![since], |r| Ok((r.get(0)?, r.get(1)?)))?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migration;
    use crate::register_sqlite_vec::register_sqlite_vec;

    fn seed() -> Connection {
        register_sqlite_vec();
        let mut conn = Connection::open_in_memory().unwrap();
        run_migration(&mut conn, 1).unwrap();
        conn
    }

    fn add_reminder(conn: &Connection, what: &str, due_at: i64, repeat: &str) -> i64 {
        consent_grant(conn, SOLE_SUBJECT, "signal", 0).unwrap();
        let state = stamp_state(what, due_at, repeat).unwrap();
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('personal', ?1, ?2, 0, 'active', 1, 1)",
            params![KIND_REMINDER, state],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn begin(conn: &mut Connection) -> rusqlite::Transaction<'_> {
        conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap()
    }

    /// A double cron never double-fires: the second fire of the same
    /// envelope is AlreadyFired and adds NO second outbox row.
    #[test]
    fn due_fires_once_per_envelope_idempotently() {
        let mut conn = seed();
        add_reminder(&conn, "draft pillar post", 100, REPEAT_NONE);
        let now = 200;
        let items = due(&conn, now);
        assert_eq!(items.len(), 1);
        let tx = begin(&mut conn);
        assert_eq!(
            fire(&tx, &items[0], now).unwrap(),
            FireOutcome::Fired {
                rearmed_due_at: None
            }
        );
        tx.commit().unwrap();
        // Second cron pass over the same now: the run is no longer active…
        assert!(due(&conn, now).is_empty());
        // …but even a stale caller refiring the SAME envelope is a no-op.
        let tx = begin(&mut conn);
        assert_eq!(
            fire(&tx, &items[0], now).unwrap(),
            FireOutcome::AlreadyFired
        );
        tx.commit().unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM outbox WHERE topic = ?1",
                params![TOPIC_VALET_DUE],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "exactly one envelope per (run, due_at)");
    }

    /// A weekly repeat does not die when it fires: CAS re-arms a NEW envelope
    /// (next due_at, bumped fire_count) and the run stays active.
    #[test]
    fn repeat_rearms_new_envelope() {
        let mut conn = seed();
        let id = add_reminder(&conn, "pillar post", 100, REPEAT_WEEKLY);
        let items = due(&conn, 150);
        assert_eq!(items.len(), 1);
        let tx = begin(&mut conn);
        let out = fire(&tx, &items[0], 150).unwrap();
        tx.commit().unwrap();
        assert_eq!(
            out,
            FireOutcome::Fired {
                rearmed_due_at: Some(100 + WEEKLY_SECS)
            }
        );
        let st: String = conn
            .query_row(
                "SELECT state_json FROM workflow_runs WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        let parsed = parse_state(&st).unwrap();
        assert_eq!(parsed.due_at, 100 + WEEKLY_SECS);
        assert_eq!(parsed.fire_count, 1);
        let status: String = conn
            .query_row(
                "SELECT status FROM workflow_runs WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "active");
        // The re-armed envelope comes due exactly one week later — not before.
        assert!(due(&conn, 100 + WEEKLY_SECS - 1).is_empty());
        assert_eq!(due(&conn, 100 + WEEKLY_SECS).len(), 1);
    }

    /// Overdue ranking: reminders rank before digests, then earliest
    /// deadline first.
    #[test]
    fn overdue_ranks_by_priority_then_deadline() {
        let conn = seed();
        add_reminder(&conn, "late reminder", 50, REPEAT_NONE);
        add_reminder(&conn, "later reminder", 90, REPEAT_NONE);
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('personal', ?1, ?2, 0, 'active', 1, 1)",
            params![KIND_DIGEST, &stamp_state("digest", 10, REPEAT_NONE).unwrap()],
        )
        .unwrap();
        let items = due(&conn, 200);
        let whats: Vec<&str> = items.iter().map(|i| i.state.what.as_str()).collect();
        assert_eq!(whats, ["late reminder", "later reminder", "digest"]);
    }

    /// Two crons back-to-back (the launchd overlap case) are collectively
    /// safe: nothing double-enqueues, nothing errors.
    #[test]
    fn cron_double_invocation_is_safe() {
        let mut conn = seed();
        add_reminder(&conn, "one-shot", 100, REPEAT_NONE);
        for _ in 0..2 {
            let items = due(&conn, 200);
            let tx = begin(&mut conn);
            for it in &items {
                let _ = fire(&tx, it, 200).unwrap();
            }
            tx.commit().unwrap();
        }
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM outbox WHERE topic = ?1",
                params![TOPIC_VALET_DUE],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }

    /// No consent, no send: the envelope fires its lifecycle but NOTHING is
    /// enqueued for the Signal relay.
    #[test]
    fn no_consent_suppresses_delivery() {
        let mut conn = seed();
        add_reminder(&conn, "quiet", 100, REPEAT_NONE);
        consent_revoke(&conn, SOLE_SUBJECT, "signal", 50).unwrap();
        let items = due(&conn, 200);
        let tx = begin(&mut conn);
        let out = fire(&tx, &items[0], 200).unwrap();
        tx.commit().unwrap();
        assert!(matches!(out, FireOutcome::SuppressedNoConsent { .. }));
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM outbox WHERE topic = ?1",
                params![TOPIC_VALET_DUE],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0);
    }

    /// With an in-force grant the same envelope delivers; after revoke it
    /// does not. Only the sole subject / signal channel are in scope.
    #[test]
    fn consent_registry_gates_signal_and_is_single_subject() {
        let conn = seed();
        assert!(!consent_in_force(&conn, SOLE_SUBJECT, "signal").unwrap());
        consent_grant(&conn, SOLE_SUBJECT, "signal", 10).unwrap();
        assert!(consent_in_force(&conn, SOLE_SUBJECT, "signal").unwrap());
        assert!(consent_grant(&conn, "someone-else", "signal", 10).is_err());
        assert!(consent_grant(&conn, SOLE_SUBJECT, "email", 10).is_err());
        consent_revoke(&conn, SOLE_SUBJECT, "signal", 20).unwrap();
        assert!(!consent_in_force(&conn, SOLE_SUBJECT, "signal").unwrap());
        assert!(consent_revoke(&conn, SOLE_SUBJECT, "signal", 30).is_err());
    }

    /// State bounds: empty/too-long labels and unknown repeats refuse.
    #[test]
    fn stamp_state_enforces_bounds() {
        assert!(stamp_state("", 1, REPEAT_NONE).is_err());
        assert!(stamp_state(&"x".repeat(MAX_WHAT_LEN + 1), 1, REPEAT_NONE).is_err());
        assert!(stamp_state("ok", 1, "hourly").is_err());
        let st = stamp_state("ok", 42, REPEAT_WEEKLY).unwrap();
        assert!(st.contains("\"sla_deadline\":42"));
    }
    // ── the brief surface, end-to-end (moved from handlers/valet with its
    // seeds riding the cores — call path changed, assertions did not) ──

    fn brief_state() -> (tempfile::TempDir, std::sync::Arc<crate::AppState>) {
        crate::register_sqlite_vec::register_sqlite_vec();
        let dir = tempfile::TempDir::new().expect("temp dir");
        let db_path = dir.path().join("brain.db");
        let mgr = r2d2_sqlite::SqliteConnectionManager::file(&db_path);
        let pool: crate::Pool = r2d2::Pool::builder().build(mgr).expect("pool");
        run_migration(&mut pool.get().expect("conn"), 0).expect("migration");
        let state = std::sync::Arc::new(crate::AppState {
            token_store: crate::auth::TokenStore::new(),
            jwt_middleware_state: std::sync::Arc::new(
                crate::server::router::auth::JwtMiddlewareState::opaque_for_tests(
                    pool.clone(),
                    db_path.clone(),
                ),
            ),
            cors: tower_http::cors::CorsLayer::new(),
            model: std::sync::Arc::new(
                crate::embed::StaticEmbedder::new(crate::config::MODEL_ID).expect("model"),
            ),
            registry: crate::domain_registry::DomainRegistry::new(pool.clone(), &db_path, false),
            pool,
            db_path,
            connection_tracker: std::sync::Arc::new(crate::http_limit::ConnectionTracker::new()),
            rate_limiter: std::sync::Arc::new(crate::http_limit::RateLimiter::new()),
            snapshot: crate::integrity::SnapshotState::default(),
            audit_chain_cache: std::sync::Arc::new(std::sync::Mutex::new(None)),
            auth_mode: crate::auth::AuthMode::Opaque,
            key_store: crate::auth::jwks::KeyStore::default(),
            revocation_cache: std::sync::Arc::new(crate::auth::revocation::RevocationCache::new()),
            jwt_issuer: String::new(),
            jwt_audience: String::new(),
            oidc_config: crate::handlers::well_known::OidcConfig::unconfigured(),
            ump_events: tokio::sync::broadcast::channel(16).0,
            alert_events: tokio::sync::broadcast::channel(16).0,
            alert_seq: std::sync::atomic::AtomicU64::new(0),
            chain_watch: crate::alert::ChainWatchState::default(),
            concurrency: &crate::concurrency::CONCURRENCY,
        });
        (dir, state)
    }

    fn brief_seed(conn: &Connection, now: i64) {
        // One overdue + one future reminder.
        for (what, due) in [("overdue post", now - 3600), ("future post", now + 3600)] {
            crate::workflow::state::open_run(
                conn,
                "personal",
                KIND_REMINDER,
                &stamp_state(what, due, REPEAT_NONE).unwrap(),
                1,
            )
            .unwrap();
        }
        // One pending draft with a lint report + one decided draft (must NOT appear).
        let lint = crate::valet_style::LintReport {
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
        let (_dir, state) = brief_state();
        {
            let conn = state.pool.get().unwrap();
            let now = chrono::Utc::now().timestamp();
            brief_seed(&conn, now);
        }
        let res = crate::handlers::valet::get_brief(
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
        let (_dir, state) = brief_state();
        let res = crate::handlers::valet::get_brief(
            axum::extract::State(state.clone()),
            crate::handlers::auth::OptPrincipal(None),
        )
        .await
        .expect("brief");
        assert_eq!(res.0["signal_consent_in_force"], false);
    }
}
