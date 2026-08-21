//! Durable-step primitives, part 2: the outbox.
//!
//! Exactly-once event delivery **by idempotency key, not retry count**. A
//! retried `enqueue` with the same key is an idempotent no-op (`INSERT OR
//! IGNORE` is atomic against the `UNIQUE` constraint); `deliver` then advances
//! the row to delivered exactly once. At-least-once delivery + at-most-once
//! *effect* is the classic outbox contract — a duplicate replay cannot double-
//! apply because a replayed key is a no-op receipt.
//!
//! Both writes emit their [`crate::audit::AuditKind::Workflow`] row (see
//! [`super::audit_write`]); a replayed enqueue is deliberately NOT audited —
//! only real state changes reach the chain.

use super::audit_write;
use crate::audit::AuditStatus;
use rusqlite::{Connection, params};

/// Enqueue a payload for a topic. Returns `true` if a new row was created,
/// `false` if the key already existed (idempotent replay → no-op).
pub(crate) fn enqueue(
    conn: &Connection,
    run_id: i64,
    topic: &str,
    payload_json: &str,
    idempotency_key: &str,
    now: i64,
) -> rusqlite::Result<bool> {
    let n = conn.execute(
        "INSERT OR IGNORE INTO outbox(run_id, topic, payload_json, status, idempotency_key, created_at)
         VALUES (?1, ?2, ?3, 'pending', ?4, ?5)",
        params![run_id, topic, payload_json, idempotency_key, now],
    )?;
    if n == 1 {
        audit_write(
            conn,
            run_id,
            &format!("outbox:{idempotency_key}"),
            AuditStatus::Ok,
            &format!("enqueue:{topic}"),
        );
    }
    Ok(n == 1)
}

/// Mark a pending outbox row delivered (idempotent: delivering twice is a
/// no-op kept at the first delivered_at). Uses `RETURNING` so the audit row
/// needs no second lookup.
pub(crate) fn deliver(conn: &Connection, id: i64, now: i64) -> rusqlite::Result<()> {
    let run_id: Option<i64> = conn
        .query_row(
            "UPDATE outbox SET status = 'delivered', delivered_at = COALESCE(delivered_at, ?2)
              WHERE id = ?1 AND status = 'pending'
              RETURNING run_id",
            params![id, now],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    if let Some(run_id) = run_id {
        audit_write(
            conn,
            run_id,
            &format!("outbox:{id}"),
            AuditStatus::Ok,
            "delivered",
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use brain_server::migration::run_migration;
    use brain_server::register_sqlite_vec::register_sqlite_vec;

    fn db() -> Connection {
        register_sqlite_vec();
        let mut conn = Connection::open_in_memory().unwrap();
        run_migration(&mut conn, 1).unwrap();
        conn
    }

    #[test]
    fn outbox_idempotent_by_key() {
        let conn = db();
        assert!(enqueue(&conn, 1, "intake", r#"{"a":1}"#, "pip-1", 1).unwrap());
        assert!(
            !enqueue(&conn, 1, "intake", r#"{"a":2}"#, "pip-1", 1).unwrap(),
            "same key replays as a no-op"
        );
        // Only one row exists; payload unchanged (the first write wins).
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM outbox WHERE idempotency_key='pip-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "replayed key must not create a second row");
        let status: String = conn
            .query_row(
                "SELECT status FROM outbox WHERE idempotency_key='pip-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "pending");

        let id: i64 = conn
            .query_row("SELECT id FROM outbox", [], |r| r.get(0))
            .unwrap();
        deliver(&conn, id, 2).unwrap();
        deliver(&conn, id, 3).unwrap(); // second deliver is a no-op
        let (status, at): (String, i64) = conn
            .query_row(
                "SELECT status, delivered_at FROM outbox WHERE id=?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "delivered");
        assert_eq!(at, 2, "delivered_at must stay the first delivery");
    }

    #[test]
    fn outbox_enqueue_audits_once_not_on_replay() {
        let conn = db();
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('global', 'interview', '{}', 0, 'active', 1, 1)",
            [],
        )
        .unwrap();
        assert!(enqueue(&conn, 1, "intake", r#"{"a":1}"#, "k-9", 1).unwrap());
        assert!(!enqueue(&conn, 1, "intake", r#"{"a":2}"#, "k-9", 1).unwrap());
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE kind = 'workflow'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "the replayed enqueue must not add a second audit row");
    }
}
