//! Durable-step primitives, part 3: CAS state transitions.
//!
//! Governed workflow is an optimistic-locking problem: the `*-core` engine
//! crates hold a local `state_revision`, mutate their `state_json`, and write
//! it back — but only if the stored revision still matches. [`cas_update`]
//! is that compare-and-swap in one statement, returning the same conflict
//! vocabulary the crates expose (`DI_*_CONFLICT`-style): `Stale` when a
//! concurrent writer advanced the run, `Gone` when the run no longer exists.

use super::audit_write;
use crate::audit::AuditStatus;
use rusqlite::{params, Connection};

#[derive(Debug, PartialEq)]
pub(crate) enum CasError {
    /// The run was deleted between read and write.
    Gone,
    /// `expected_revision` did not match the stored `state_revision` — a
    /// concurrent transition won; the caller must re-read and re-diff.
    Stale { actual_revision: i64 },
    /// The underlying SQLite write failed (not a conflict — an infrastructure
    /// error the caller should surface, not retry optimistically).
    Database(String),
}

impl From<rusqlite::Error> for CasError {
    fn from(e: rusqlite::Error) -> Self {
        CasError::Database(e.to_string())
    }
}

/// Atomically advance a run's state iff the caller's view is current.
/// Returns the run's new id on success (mirrors [`WorkflowTx::commit`]).
pub(crate) fn cas_update(
    conn: &Connection,
    run_id: i64,
    expected_revision: i64,
    new_state_json: &str,
    new_status: &str,
    now: i64,
) -> Result<i64, CasError> {
    let updated = conn.execute(
        "UPDATE workflow_runs
            SET state_json = ?3, status = ?4, state_revision = ?2 + 1, updated_at = ?5
          WHERE id = ?1 AND state_revision = ?2",
        rusqlite::params![run_id, expected_revision, new_state_json, new_status, now],
    )?;
    if updated == 0 {
        // Nothing matched. Distinguish deleted from stale for a useful conflict.
        let actual: Option<i64> = conn
            .query_row(
                "SELECT state_revision FROM workflow_runs WHERE id = ?1",
                params![run_id],
                |r| r.get(0),
            )
            .ok();
        let (status, detail) = match actual {
            Some(rev) => (
                CasError::Stale {
                    actual_revision: rev,
                },
                format!("cas_stale:expected={expected_revision}:actual={rev}"),
            ),
            None => (CasError::Gone, format!("cas_gone:{run_id}")),
        };
        // A rejected transition is evidence too — audit it as denied so the
        // chain records the contention, not just the wins.
        audit_write(
            conn,
            run_id,
            &format!("run:{run_id}"),
            AuditStatus::Denied,
            &detail,
        );
        return Err(status);
    }
    audit_write(
        conn,
        run_id,
        &format!("run:{run_id}"),
        AuditStatus::Ok,
        &format!("cas:{expected_revision}->{}", expected_revision + 1),
    );
    Ok(run_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use brain_server::migration::run_migration;
    use brain_server::register_sqlite_vec::register_sqlite_vec;

    fn seed() -> Connection {
        register_sqlite_vec();
        let mut conn = Connection::open_in_memory().unwrap();
        run_migration(&mut conn, 1).unwrap();
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('global', 'interview', '{\"v\":1}', 0, 'active', 1, 1)",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn cas_update_rejects_stale() {
        let conn = seed();

        // A stale writer (revision 0 when the run is already at 1) is rejected.
        cas_update(&conn, 1, 0, r#"{"v":2}"#, "active", 2).unwrap();
        let err = cas_update(&conn, 1, 0, r#"{"v":3}"#, "active", 3).unwrap_err();
        assert_eq!(err, CasError::Stale { actual_revision: 1 });

        // The current view still succeeds.
        cas_update(&conn, 1, 1, r#"{"v":4}"#, "complete", 4).unwrap();
        let (json, rev): (String, i64) = conn
            .query_row(
                "SELECT state_json, state_revision FROM workflow_runs WHERE id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(json, r#"{"v":4}"#);
        assert_eq!(rev, 2);
    }

    #[test]
    fn cas_update_rejects_gone() {
        let conn = seed();
        conn.execute("DELETE FROM workflow_runs WHERE id = 1", [])
            .unwrap();
        assert_eq!(
            cas_update(&conn, 1, 0, "{}", "active", 5).unwrap_err(),
            CasError::Gone
        );
    }
}
