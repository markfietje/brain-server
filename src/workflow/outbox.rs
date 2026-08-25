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
use rusqlite::{Connection, OptionalExtension, params};

/// Enqueue a payload for a topic. Returns the event id plus `true` if a new
/// row was created, `(existing_id, false)` when the key already replayed
/// (idempotent no-op receipt — the id is still resolved so callers can link
/// against the surviving row).
pub(crate) fn enqueue(
    conn: &Connection,
    run_id: i64,
    topic: &str,
    payload_json: &str,
    idempotency_key: &str,
    now: i64,
) -> rusqlite::Result<(bool, i64)> {
    let n = conn.execute(
        "INSERT OR IGNORE INTO outbox(run_id, topic, payload_json, status, idempotency_key, created_at)
         VALUES (?1, ?2, ?3, 'pending', ?4, ?5)",
        params![run_id, topic, payload_json, idempotency_key, now],
    )?;
    let id: i64 = conn.query_row(
        "SELECT id FROM outbox WHERE idempotency_key = ?1",
        params![idempotency_key],
        |r| r.get(0),
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
    Ok((n == 1, id))
}

/// Enqueue a CHILD event parented at `parent_id` — the Lineage release.
/// Same exactly-once discipline as [`enqueue`] (INSERT OR IGNORE + audit only
/// on first insert); the parent is recorded verbatim. Parent validity is the
/// caller's contract; [`verify_outbox_lineage`] is the integrity check.
pub(crate) fn enqueue_child(
    conn: &Connection,
    run_id: i64,
    parent_id: Option<i64>,
    topic: &str,
    payload_json: &str,
    idempotency_key: &str,
    now: i64,
) -> rusqlite::Result<(bool, i64)> {
    let n = conn.execute(
        "INSERT OR IGNORE INTO outbox(run_id, topic, payload_json, status, idempotency_key, created_at, parent_id)
         VALUES (?1, ?2, ?3, 'pending', ?4, ?5, ?6)",
        params![run_id, topic, payload_json, idempotency_key, now, parent_id],
    )?;
    let id: i64 = conn.query_row(
        "SELECT id FROM outbox WHERE idempotency_key = ?1",
        params![idempotency_key],
        |r| r.get(0),
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
    Ok((n == 1, id))
}

/// Append a lineage event at the run's CURRENT tip: the child-parent idiom
/// every governed flow shares (handover offers, case notes). The tip read
/// and the insert ride the CALLER's transaction, so a `BEGIN IMMEDIATE`
/// transition can never fork the chain. Same exactly-once discipline as
/// [`enqueue_child`]; returns the new event's row id.
pub(crate) fn append_lineage(
    conn: &Connection,
    run_id: i64,
    topic: &str,
    payload_json: &str,
    idempotency_key: &str,
    now: i64,
) -> rusqlite::Result<i64> {
    let parent: Option<i64> = conn
        .query_row(
            "SELECT MAX(id) FROM outbox WHERE run_id = ?1",
            params![run_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let (_, id) = enqueue_child(
        conn,
        run_id,
        parent,
        topic,
        payload_json,
        idempotency_key,
        now,
    )?;
    Ok(id)
}

/// Chain-integrity check beside the audit chain: every non-root `parent_id`
/// must reference an existing row of the SAME run with a SMALLER id. Smaller-id
/// parents make cycles impossible by construction; this verifies the stored
/// rows actually obey the law (orphan / cross-run / forward links fail).
pub(crate) fn verify_outbox_lineage(conn: &Connection, run_id: i64) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT COUNT(*) FROM outbox c
          WHERE c.run_id = ?1 AND c.parent_id IS NOT NULL
            AND NOT EXISTS (
                SELECT 1 FROM outbox p
                 WHERE p.id = c.parent_id
                   AND p.run_id = c.run_id
                   AND p.id < c.id
            )",
        params![run_id],
        |r| r.get::<_, i64>(0).map(|bad| bad == 0),
    )
}

/// Resolve an event id's ancestry chain (target first, root last) — the
/// `GET /workflow/runs/{id}/events?branch=` read.
pub(crate) fn branch_chain(
    conn: &Connection,
    run_id: i64,
    event_id: i64,
) -> rusqlite::Result<Vec<i64>> {
    let mut chain = Vec::new();
    let mut cur = Some(event_id);
    while let Some(id) = cur {
        let row: Option<(i64, Option<i64>)> = conn
            .query_row(
                "SELECT id, parent_id FROM outbox WHERE id = ?1 AND run_id = ?2",
                params![id, run_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?;
        match row {
            Some((id, parent)) => {
                chain.push(id);
                cur = parent;
                if chain.len() > 100_000 {
                    // Defensive bound; smaller-id parents make this
                    // unreachable on well-formed chains.
                    break;
                }
            }
            None => break,
        }
    }
    chain.reverse();
    Ok(chain)
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
        let (first, id1) = enqueue(&conn, 1, "intake", r#"{"a":1}"#, "pip-1", 1).unwrap();
        assert!(first);
        let (replay, id2) = enqueue(&conn, 1, "intake", r#"{"a":2}"#, "pip-1", 1).unwrap();
        assert!(!replay, "same key replays as a no-op");
        assert_eq!(id1, id2, "the replay resolves the surviving row's id");
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
        assert!(
            enqueue(&conn, 1, "intake", r#"{"a":1}"#, "k-9", 1)
                .unwrap()
                .0
        );
        assert!(
            !enqueue(&conn, 1, "intake", r#"{"a":2}"#, "k-9", 1)
                .unwrap()
                .0
        );
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE kind = 'workflow'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "the replayed enqueue must not add a second audit row");
    }

    /// Children parent verbatim; a replayed child key is a
    /// no-op that audits once and keeps the ORIGINAL parent (first write wins).
    #[test]
    fn enqueue_child_parents_and_audits_once() {
        let conn = db();
        let (_, root) = enqueue(&conn, 1, "workflow/log", "{}", "root-k", 1).unwrap();
        let (created, child) =
            enqueue_child(&conn, 1, Some(root), "workflow/log", "{}", "child-k", 2).unwrap();
        assert!(created);
        let stored: Option<i64> = conn
            .query_row(
                "SELECT parent_id FROM outbox WHERE idempotency_key='child-k'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, Some(root));
        // Replay with a DIFFERENT parent: the first write wins.
        let (replay, again) =
            enqueue_child(&conn, 1, None, "workflow/log", "{}2", "child-k", 3).unwrap();
        assert!(!replay);
        assert_eq!(again, child);
        let stored: Option<i64> = conn
            .query_row(
                "SELECT parent_id FROM outbox WHERE idempotency_key='child-k'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, Some(root), "replay never re-parents");
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE kind = 'workflow'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 2, "root + one child audit; the replay adds none");
    }

    /// verify_outbox_lineage_detects_orphans_and_cycles — fixture rows break
    /// each law (orphan parent, cross-run parent, forward id) and the check
    /// reports them; a well-formed chain (incl. all-NULL legacy roots) passes.
    #[test]
    fn verify_outbox_lineage_detects_orphans_and_cycles() {
        let conn = db();
        // Legacy flat run: every row a root — verify passes.
        enqueue(&conn, 1, "t", "{}", "l1", 1).unwrap();
        enqueue_child(&conn, 1, None, "t", "{}", "l2", 2).unwrap();
        assert!(verify_outbox_lineage(&conn, 1).unwrap());

        // Well-formed chain root -> child -> grandchild.
        let (_, r) = enqueue(&conn, 2, "t", "{}", "r", 1).unwrap();
        let (_, c) = enqueue_child(&conn, 2, Some(r), "t", "{}", "c", 2).unwrap();
        enqueue_child(&conn, 2, Some(c), "t", "{}", "g", 3).unwrap();
        assert!(verify_outbox_lineage(&conn, 2).unwrap());
        assert_eq!(
            branch_chain(&conn, 2, g_id(&conn, "g")).unwrap(),
            vec![r, c, g_id(&conn, "g")],
            "the ancestor chain reads root-first"
        );

        // Orphan: parent id points at nothing. (The FK would refuse this
        // write on a live path; the fixture disables it to place the corrupt
        // row and prove verify DETECTS it rather than the constraint.)
        conn.execute_batch("PRAGMA foreign_keys = OFF").unwrap();
        conn.execute(
            "INSERT INTO outbox(run_id, topic, payload_json, status, idempotency_key, created_at, parent_id)
             VALUES (3, 't', '{}', 'pending', 'orphan', 1, 99999)",
            [],
        )
        .unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        assert!(!verify_outbox_lineage(&conn, 3).unwrap());

        // Cross-run parent: same-run law violated even though the id exists.
        let (_, other) = enqueue(&conn, 5, "t", "{}", "other", 1).unwrap();
        enqueue_child(&conn, 4, Some(other), "t", "{}", "xrun", 1).unwrap();
        assert!(!verify_outbox_lineage(&conn, 4).unwrap());

        // Forward link (parent has a LARGER id): the stored rows disobey the
        // by-construction ordering, so it fails exactly like a cycle would.
        conn.execute(
            "INSERT INTO outbox(id, run_id, topic, payload_json, status, idempotency_key, created_at)
             VALUES (5001, 6, 't', '{}', 'pending', 'fwd-parent', 2)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO outbox(id, run_id, topic, payload_json, status, idempotency_key, created_at, parent_id)
             VALUES (5000, 6, 't', '{}', 'pending', 'fwd-child', 1, 5001)",
            [],
        )
        .unwrap();
        assert!(!verify_outbox_lineage(&conn, 6).unwrap());
    }

    fn g_id(conn: &Connection, key: &str) -> i64 {
        conn.query_row(
            "SELECT id FROM outbox WHERE idempotency_key=?1",
            params![key],
            |r| r.get(0),
        )
        .unwrap()
    }
}
