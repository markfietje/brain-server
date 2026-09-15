//! The agent-session event log — the Loop line's session STORE.
//!
//! The harness's pending-write queue drains into the outbox; THIS table is
//! the replayable per-run session narrative (user turns, assistant turns,
//! tool results, compaction summaries) the loop rebuilds context from. It is
//! append-only and prefix-stable: rows are never mutated or reordered, seq
//! is per-run monotonic, and exactly-once holds by idempotency key — a
//! replayed append returns the original receipt, never a second row.
//!
//! Why a table and not the audit chain: `audit_events` stores hashes of
//! targets/details (tamper evidence), not payloads — a log you cannot read
//! back cannot be replayed. Every append still emits its [`AuditKind::Workflow`]
//! row inside the caller's transaction via the substrate's [`audit_write`],
//! so the chain can reconstruct THAT a session event landed, while this
//! table carries the WHAT.
//!
//! Like the rest of the substrate: SQL lives here, callers pass
//! `&Connection` (a `WorkflowTx` for multi-write atomicity), time enters as
//! an argument, foreign runs are refused by the writer's query not by schema
//! (the outbox posture — no FK on purpose).

use rusqlite::{Connection, params};

use super::audit_write;
use crate::audit::AuditStatus;

/// Default replay window (bounds law): the loop asks for at most this many
/// most-recent events; pinned so no caller can ask for "all of history"
/// without visibly raising a number.
pub(crate) const REPLAY_CAP: usize = 500;

/// Payload bound (bounds law): an event larger than this is refused, not
/// truncated — a loop that tries to persist a 10-MiB tool blob fails loud
/// and the caller decides policy. Matches the hostcall effect-output cap.
pub(crate) const PAYLOAD_CAP_BYTES: usize = 64 * 1024;

/// One replayed session event, in seq order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionEventRow {
    pub seq: i64,
    pub kind: String,
    pub payload_json: String,
    pub created_at: i64,
}

/// Append one event to the run's session log. Returns `(created, seq)`:
/// a first append created its row and drew the next per-run seq; a replay
/// of a known idempotency key is a no-op receipt returning the ORIGINAL seq.
/// The audit row for a created append lands inside the caller's transaction
/// (a replay audits nothing, mirroring the outbox's exactly-once posture).
///
/// Fails closed on: empty kind, oversized payload, or SQL failure — never
/// truncates, never drops silently.
pub(crate) fn append(
    conn: &Connection,
    run_id: i64,
    kind: &str,
    payload_json: &str,
    idempotency_key: &str,
    now: i64,
) -> rusqlite::Result<(bool, i64)> {
    if kind.is_empty() {
        return Err(rusqlite::Error::InvalidParameterName(
            "session event kind must be non-empty".into(),
        ));
    }
    if payload_json.len() > PAYLOAD_CAP_BYTES {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "session event payload exceeds cap: {} > {} bytes",
            payload_json.len(),
            PAYLOAD_CAP_BYTES
        )));
    }
    // Single statement under the caller's BEGIN IMMEDIATE: seq is computed
    // and the exactly-once guard evaluated atomically, so two racing writers
    // serialize on the write discipline instead of double-drawing a seq.
    // `INSERT OR IGNORE` is the belt to the NOT EXISTS braces — an aggregate
    // SELECT always yields a row, so the guard lives in a scalar subquery.
    let created = conn.execute(
        "INSERT OR IGNORE INTO agent_session_events(run_id, seq, idempotency_key, kind, payload_json, created_at)
         SELECT ?1,
                (SELECT COALESCE(MAX(seq), 0) + 1 FROM agent_session_events WHERE run_id = ?1),
                ?2, ?3, ?4, ?5
          WHERE NOT EXISTS (
              SELECT 1 FROM agent_session_events
               WHERE run_id = ?1 AND idempotency_key = ?2
          )",
        params![run_id, idempotency_key, kind, payload_json, now],
    )? == 1;
    let seq: i64 = conn.query_row(
        "SELECT seq FROM agent_session_events WHERE run_id = ?1 AND idempotency_key = ?2",
        params![run_id, idempotency_key],
        |r| r.get(0),
    )?;
    if created {
        audit_write(
            conn,
            run_id,
            "agent-session",
            AuditStatus::Ok,
            &format!("append:{kind}:{seq}"),
        );
    }
    Ok((created, seq))
}

/// Replay the run's session log: the `cap` MOST RECENT events, returned
/// oldest-first (prefix-stable order). A cap larger than the log returns the
/// whole log; callers wanting the full history for tooling pass an explicit
/// number and own that choice visibly.
pub(crate) fn replay(
    conn: &Connection,
    run_id: i64,
    cap: usize,
) -> rusqlite::Result<Vec<SessionEventRow>> {
    // DESC LIMIT + reverse: the tail window without scanning the prefix.
    let mut stmt = conn.prepare(
        "SELECT seq, kind, payload_json, created_at FROM agent_session_events
          WHERE run_id = ?1 ORDER BY seq DESC LIMIT ?2",
    )?;
    let mut rows: Vec<SessionEventRow> = stmt
        .query_map(params![run_id, cap as i64], |r| {
            Ok(SessionEventRow {
                seq: r.get(0)?,
                kind: r.get(1)?,
                payload_json: r.get(2)?,
                created_at: r.get(3)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();
    rows.reverse();
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::verify_chain;
    use crate::migration::run_migration;
    use crate::register_sqlite_vec::register_sqlite_vec;
    use crate::workflow::ACTOR;
    use crate::workflow::tx::WorkflowTx;

    fn db() -> Connection {
        register_sqlite_vec();
        let mut conn = Connection::open_in_memory().unwrap();
        run_migration(&mut conn, 1).unwrap();
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('acme', 'troubleshoot', '{}', 0, 'active', 1, 1)",
            [],
        )
        .unwrap();
        conn
    }

    fn kinds(conn: &Connection, run_id: i64) -> Vec<(i64, String)> {
        replay(conn, run_id, REPLAY_CAP)
            .unwrap()
            .into_iter()
            .map(|r| (r.seq, r.kind))
            .collect()
    }

    #[test]
    fn appends_are_sequential_and_audited() {
        let conn = db();
        for (i, kind) in ["user", "assistant", "tool_result"].iter().enumerate() {
            let (created, seq) =
                append(&conn, 1, kind, "{}", &format!("k-{i}"), 100 + i as i64).unwrap();
            assert!(created);
            assert_eq!(seq, i as i64 + 1, "per-run seq is monotonic from 1");
        }
        assert_eq!(
            kinds(&conn, 1),
            vec![
                (1, "user".into()),
                (2, "assistant".into()),
                (3, "tool_result".into()),
            ]
        );
        let audit: Vec<(String, bool)> = {
            let mut stmt = conn
                .prepare(
                    "SELECT actor, detail_hash IS NOT NULL FROM audit_events
                      WHERE kind = 'workflow' ORDER BY id",
                )
                .unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        };
        assert_eq!(audit.len(), 3, "one audit row per created append");
        assert!(
            audit.iter().all(|(actor, _)| actor == ACTOR),
            "substrate audit identity stamps session writes"
        );
        assert!(verify_chain(&conn), "the audit chain still verifies");
    }

    #[test]
    fn append_is_idempotent_by_key_and_audits_once() {
        let conn = db();
        let first = append(&conn, 1, "user", r#"{"q":"hi"}"#, "k-dup", 1).unwrap();
        let replayed = append(&conn, 1, "user", r#"{"q":"hi"}"#, "k-dup", 2).unwrap();
        assert!(first.0);
        assert!(!replayed.0, "a known key is a no-op receipt");
        assert_eq!(first.1, replayed.1, "the receipt returns the ORIGINAL seq");
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM agent_session_events", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(n, 1);
        let audits: i64 = conn
            .query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(audits, 1, "the replay audits nothing (exactly-once)");
    }

    #[test]
    fn append_rolls_back_with_the_transition() {
        let mut conn = db();
        {
            let mut wtx = WorkflowTx::begin(&mut conn).unwrap();
            append(wtx.tx(), 1, "user", "{}", "k-tx", 1).unwrap();
            // Drop without commit: the event AND its audit row must vanish.
        }
        assert!(
            kinds(&conn, 1).is_empty(),
            "a rolled-back append leaves no session row"
        );
        let audits: i64 = conn
            .query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(audits, 0, "and no audit row claiming it happened");
    }

    #[test]
    fn replay_returns_the_recent_window_oldest_first() {
        let conn = db();
        for i in 0..6 {
            append(&conn, 1, "user", "{}", &format!("k-{i}"), i).unwrap();
        }
        let seqs: Vec<i64> = replay(&conn, 1, 4)
            .unwrap()
            .into_iter()
            .map(|r| r.seq)
            .collect();
        assert_eq!(seqs, vec![3, 4, 5, 6], "the 4 most recent, oldest-first");
    }

    #[test]
    fn runs_do_not_share_a_log() {
        let conn = db();
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('acme', 'troubleshoot', '{}', 0, 'active', 1, 1)",
            [],
        )
        .unwrap();
        append(&conn, 1, "user", "{}", "k-one", 1).unwrap();
        append(&conn, 2, "user", "{}", "k-two", 1).unwrap();
        assert_eq!(kinds(&conn, 1).len(), 1);
        assert_eq!(kinds(&conn, 2).len(), 1, "per-run logs are isolated");
    }

    #[test]
    fn bounds_are_pinned_and_refuse_loud() {
        assert_eq!(REPLAY_CAP, 500);
        assert_eq!(PAYLOAD_CAP_BYTES, 64 * 1024);
        let conn = db();
        let oversized = "x".repeat(PAYLOAD_CAP_BYTES + 1);
        assert!(
            append(&conn, 1, "tool_result", &oversized, "k-big", 1).is_err(),
            "an oversized payload is refused, never truncated"
        );
        assert!(
            append(&conn, 1, "", "{}", "k-empty", 1).is_err(),
            "an empty kind is refused"
        );
        assert!(
            kinds(&conn, 1).is_empty(),
            "refused appends leave no rows behind"
        );
    }

    #[test]
    fn audit_rows_carry_the_runs_tenant() {
        let conn = db();
        append(&conn, 1, "user", "{}", "k-tenant", 1).unwrap();
        let tenant: String = conn
            .query_row(
                "SELECT tenant_id FROM audit_events WHERE kind = 'workflow'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            tenant, "acme",
            "session writes audit under the run's domain"
        );
    }
}
