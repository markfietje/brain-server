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

/// The CLOSED run-status vocabulary: the exact set the codebase observes,
/// frozen. Kernel writers: `active` (open_run and
/// every resume path), `cancelled` (the mesh revocation drain), `fired` (the
/// valet crank's terminal), `resolved` (the workload suggestion acceptance).
/// Engine-written through the CAS seam, with shipped readers that
/// distinguish them: `completed` (kcs capture, scoreboard, relay's
/// run-not-active guard), `closed` (the scoreboard aftersales cohort).
/// `PUT /workflow/runs/{id}/state` accepts ONLY these — nothing speculative
/// may enter a run row's status. Extend this const in the same commit as the
/// kernel writer or reader that needs the new value.
pub const RUN_STATUSES: &[&str] = &[
    "active",
    "cancelled",
    "closed",
    "completed",
    "fired",
    "resolved",
];

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

/// The case-launch pre-check row (see [`launch_row`]).
pub(crate) type LaunchRowTuple = (String, String, String, String, i64);

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

/// The case-launch pre-check row: (domain, kind, status, state_json,
/// state_revision) in one read. The launch law (fresh troubleshoot run
/// only) is DECIDED by the handler; the data lives here with every other
/// run read.
pub(crate) fn launch_row(
    conn: &Connection,
    run_id: i64,
) -> rusqlite::Result<Option<LaunchRowTuple>> {
    conn.query_row(
        "SELECT domain,kind,status,state_json,state_revision FROM workflow_runs WHERE id=?1",
        params![run_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
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

/// The run's kind (`valet/…`, `interview`, …) — the X-W4 completion's
/// kind-scoped vet at the CAS seam reads it. Missing row → None.
pub(crate) fn run_kind(conn: &Connection, run_id: i64) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT kind FROM workflow_runs WHERE id=?1",
        params![run_id],
        |r| r.get::<_, String>(0),
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

/// Stamp the run's law version inside the CALLER'S transaction — the
/// server-derived intake stamp (empty = absent/unknown jurisdiction). Must
/// never touch `state_json`: the engines CAS against those exact bytes.
pub(crate) fn stamp_law_version(
    conn: &Connection,
    run_id: i64,
    law_version: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE workflow_runs SET law_version = ?1 WHERE id = ?2",
        params![law_version, run_id],
    )?;
    Ok(())
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
    use crate::migration::run_migration;
    use crate::register_sqlite_vec::register_sqlite_vec;

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

/// Test-support reads/writes for the case-launch route's tests. The
/// SQL-in-handlers law forbids SQL strings under `src/handlers/` — even
/// in test fixtures — so the fixtures and assertions live in the domain
/// core that owns the tables. Test-only: compiled nowhere else.
#[cfg(test)]
pub(crate) mod test_support {
    use super::params;
    use rusqlite::Connection;

    /// The run's stamped law_version + its stored state_json bytes (the
    /// intake-stamp pin reads both; the CAS law asserts byte-equality).
    pub(crate) fn law_version_and_state(
        conn: &Connection,
        run_id: i64,
    ) -> rusqlite::Result<(String, String)> {
        conn.query_row(
            "SELECT law_version, state_json FROM workflow_runs WHERE id = ?1",
            [run_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
    }

    /// The detail hash of the run's open workflow audit row.
    pub(crate) fn workflow_audit_detail_hash(
        conn: &Connection,
        run_id: i64,
    ) -> rusqlite::Result<String> {
        conn.query_row(
            "SELECT detail_hash FROM audit_events WHERE kind = 'workflow' AND target_hash = ?1",
            [crate::audit::hash(&format!("run:{run_id}"))],
            |r| r.get(0),
        )
    }

    /// One active, fresh troubleshoot run (the launch law's only input).
    pub(crate) fn insert_fresh_troubleshoot_run(
        conn: &Connection,
        domain: &str,
        now: i64,
    ) -> rusqlite::Result<i64> {
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES (?1, 'troubleshoot', '{}', 0, 'active', ?2, ?2)",
            params![domain, now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// One already-resolved troubleshoot run (the repeater-census input).
    pub(crate) fn insert_resolved_troubleshoot_run(
        conn: &Connection,
        domain: &str,
        created_at: i64,
    ) -> rusqlite::Result<i64> {
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES (?1, 'troubleshoot', '{}', 1, 'resolved', ?2, ?2)",
            params![domain, created_at],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// The pending capture proposals the enqueue writes (source-tagged).
    pub(crate) fn pending_capture_proposals(
        conn: &Connection,
    ) -> rusqlite::Result<Vec<(String, String)>> {
        let mut stmt = conn.prepare(
            "SELECT kind, status FROM proposals WHERE source = 'gdl-capture' ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }

    /// The knowledge/vec row counts (the no-auto-publication pin).
    pub(crate) fn knowledge_and_vec_counts(conn: &Connection) -> rusqlite::Result<(i64, i64)> {
        let knowledge: i64 = conn.query_row("SELECT COUNT(*) FROM knowledge", [], |r| r.get(0))?;
        let vec_rows: i64 =
            conn.query_row("SELECT COUNT(*) FROM vec_knowledge", [], |r| r.get(0))?;
        Ok((knowledge, vec_rows))
    }

    /// The fail-closed unknown-tool refusal receipts for a run.
    pub(crate) fn unknown_tool_receipts(conn: &Connection, run_id: i64) -> rusqlite::Result<i64> {
        conn.query_row(
            "SELECT COUNT(*) FROM agent_session_events
             WHERE run_id = ?1 AND kind = 'tool_result' AND payload_json LIKE '%unknown tool%'",
            params![run_id],
            |r| r.get(0),
        )
    }
}
