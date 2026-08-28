//! Durable-step primitives, part 3: CAS state transitions.
//!
//! Governed workflow is an optimistic-locking problem: the `*-core` engine
//! crates hold a local `state_revision`, mutate their `state_json`, and write
//! it back — but only if the stored revision still matches. [`cas_update`]
//! is that compare-and-swap in one statement, returning the SDK's conflict
//! vocabulary ([`brain_engine_sdk::host::CasError`]): `Stale` when a
//! concurrent writer advanced the run, `Gone` when the run no longer exists.

pub use brain_engine_sdk::host::CasError;

use super::audit_write;
use crate::audit::AuditStatus;
use rusqlite::{Connection, OptionalExtension, params};

fn db_err(e: rusqlite::Error) -> CasError {
    CasError::Database(e.to_string())
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
    let updated = conn
        .execute(
            "UPDATE workflow_runs
            SET state_json = ?3, status = ?4, state_revision = ?2 + 1, updated_at = ?5
          WHERE id = ?1 AND state_revision = ?2",
            rusqlite::params![run_id, expected_revision, new_state_json, new_status, now],
        )
        .map_err(db_err)?;
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

// ── run-row reads: the projections the engine surfaces serve ──────────────

/// The stored run row: (id, domain, kind, status, state_json, created_at,
/// updated_at). Wire shaping stays handler-side.
pub(crate) type RunRowTuple = (i64, String, String, String, String, i64, i64);

/// The stored step row: (id, run_id, phase, step_key, state_json, revision,
/// parent_step_id).
pub(crate) type StepRowTuple = (i64, i64, String, String, String, i64, Option<i64>);

/// The full run row `GET /workflow/runs/{id}` serves.
pub(crate) fn run_row(conn: &Connection, run_id: i64) -> rusqlite::Result<Option<RunRowTuple>> {
    conn.query_row(
        "SELECT id,domain,kind,status,state_json,created_at,updated_at FROM workflow_runs WHERE id=?1",
        params![run_id],
        |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ))
        },
    )
    .optional()
}

/// The run's domain label, or None when the run is gone (the caller owns
/// the probe-blind 404).
pub(crate) fn run_domain_of(conn: &Connection, run_id: i64) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT domain FROM workflow_runs WHERE id=?1",
        params![run_id],
        |r| r.get(0),
    )
    .optional()
}

/// (domain, state_json) — the suggestions surface's read (domain for the
/// authz gate, state for the reuse query). One statement so the row is
/// resolved once, BEFORE authorization, exactly as the 404 order demands.
pub(crate) fn run_domain_and_state(
    conn: &Connection,
    run_id: i64,
) -> rusqlite::Result<Option<(String, String)>> {
    conn.query_row(
        "SELECT domain,state_json FROM workflow_runs WHERE id=?1",
        params![run_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
}

/// The engine state read: (state_json, state_revision), None when the run
/// is gone. Shared by the bare-connection state view (whose caller audits
/// the read) and the in-tx CAS sequences (answer/rewind).
pub(crate) fn read_state_and_revision(
    conn: &Connection,
    run_id: i64,
) -> rusqlite::Result<Option<(String, i64)>> {
    conn.query_row(
        "SELECT state_json, state_revision FROM workflow_runs WHERE id=?1",
        params![run_id],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
    )
    .optional()
}

/// The run's step rows in id order: (id, run_id, phase, step_key,
/// state_json, revision, parent_step_id). Stored forms — the read seam
/// stays handler-side.
pub(crate) fn steps_of_run(conn: &Connection, run_id: i64) -> rusqlite::Result<Vec<StepRowTuple>> {
    let mut stmt = conn.prepare(
        "SELECT id,run_id,phase,step_key,state_json,revision,parent_step_id FROM workflow_steps WHERE run_id=?1 ORDER BY id",
    )?;
    let rows = stmt.query_map(params![run_id], |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
            r.get(6)?,
        ))
    })?;
    rows.collect()
}

/// Open a run: the row write + id resolution inside the CALLER'S
/// transaction ([`super::tx::WorkflowTx`]). The caller owes the `open`
/// audit row and the presence touch, in the same tx.
pub(crate) fn open_run(
    conn: &Connection,
    domain: &str,
    kind: &str,
    state_json: &str,
    now: i64,
) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
         VALUES (?1, ?2, ?3, 0, 'active', ?4, ?4)",
        params![domain, kind, state_json, now],
    )?;
    Ok(conn.last_insert_rowid())
}

/// The handoff head: (kind, status, created_at, state_json), None when gone.
pub(crate) fn run_head(
    conn: &Connection,
    run_id: i64,
) -> rusqlite::Result<Option<(String, String, i64, String)>> {
    conn.query_row(
        "SELECT kind, status, created_at, state_json FROM workflow_runs WHERE id=?1",
        params![run_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )
    .optional()
}

/// Step labels for the handoff packet (`step_key:phase`), id order, bounded
/// at 200 (row errors skip — the packet is best-effort assembled).
pub(crate) fn step_labels(conn: &Connection, run_id: i64) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT step_key || ':' || phase FROM workflow_steps
          WHERE run_id=?1 ORDER BY id LIMIT 200",
    )?;
    let it = stmt.query_map(params![run_id], |r| r.get::<_, String>(0))?;
    Ok(it.filter_map(Result::ok).collect())
}

/// Count of unreleased legal holds (GLOBAL by schema — holds are not
/// per-row). The caller decides the failure posture; the handoff packet's
/// documented fail-open reads `unwrap_or(0)` ("no hold" on a degraded DB).
pub(crate) fn active_legal_holds(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM legal_holds WHERE released_at IS NULL",
        [],
        |r| r.get(0),
    )
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
