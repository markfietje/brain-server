//! Erasure reach for governed-workflow data: DSAR sweeps cover the
//! workflow tables per domain, and an active legal hold freezes a run
//! exactly as it freezes a chunk.
//!
//! Hold convention: a hold row whose `knowledge_id` is negative holds
//! `workflow_runs` id `-knowledge_id` (chunk ids are positive, so no chunk
//! path can collide). A frozen run is DEFERRED by a DSAR — listed on the
//! certificate — never silently deleted.

use std::collections::HashSet;

use crate::handlers::HandlerError;

/// Workflow-run ids frozen by an active legal hold (`knowledge_id = -run_id`).
pub(crate) fn frozen_runs(conn: &rusqlite::Connection) -> Result<HashSet<i64>, HandlerError> {
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT -knowledge_id FROM legal_holds
             WHERE released_at IS NULL AND knowledge_id < 0",
        )
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    let rows = stmt
        .query_map([], |r| r.get::<_, i64>(0))
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    Ok(rows.flatten().collect())
}

/// What one domain's subject sweep did.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct SweepReport {
    /// Runs matched AND deleted.
    pub runs_deleted: usize,
    /// Dependent rows removed with them (steps/outbox/findings/contradictions).
    pub dependent_rows: usize,
    /// Frozen runs left in place (run id + active reasons), certificate-listed.
    pub deferred: Vec<(i64, Vec<String>)>,
    /// Total runs matched (deleted + deferred) — the honest footprint number.
    pub runs_matched: usize,
}

/// Sweep every workflow table in this pool for rows carrying `subject`,
/// deleting matched runs with their dependents in the caller's transaction.
/// Frozen runs are skipped and reported. Best-effort over-match posture,
/// same as the trace/proposal sweeps: erasure-safe direction.
pub(crate) fn sweep_subject(
    tx: &rusqlite::Transaction<'_>,
    subject: &str,
) -> Result<SweepReport, HandlerError> {
    let mut report = SweepReport::default();
    if subject.is_empty() {
        return Ok(report);
    }
    let pattern = format!("%{subject}%");
    let mut stmt = tx
        .prepare("SELECT id FROM workflow_runs WHERE state_json LIKE ?1 ORDER BY id")
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    let targets: Vec<i64> = stmt
        .query_map(rusqlite::params![pattern], |r| r.get(0))
        .map_err(|e| HandlerError::internal(e.to_string()))?
        .flatten()
        .collect();
    drop(stmt);
    report.runs_matched = targets.len();

    let frozen = frozen_runs(tx)?;
    let held =
        crate::legal_hold::active_reasons(tx, &targets.iter().map(|r| -r).collect::<Vec<_>>())?;
    for run_id in targets {
        if frozen.contains(&run_id) {
            report
                .deferred
                .push((run_id, held.get(&-run_id).cloned().unwrap_or_default()));
            continue;
        }
        // Contradictions referencing this run's findings (either side or resolver).
        report.dependent_rows += tx
            .execute(
                "DELETE FROM contradictions WHERE id IN (
                     SELECT c.id FROM contradictions c
                     LEFT JOIN findings fa ON fa.id = c.finding_a_id
                     LEFT JOIN findings fb ON fb.id = c.finding_b_id
                     WHERE c.run_id = ?1 OR fa.run_id = ?1 OR fb.run_id = ?1)",
                rusqlite::params![run_id],
            )
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        report.dependent_rows += tx
            .execute(
                "DELETE FROM findings WHERE run_id = ?1",
                rusqlite::params![run_id],
            )
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        report.dependent_rows += tx
            .execute(
                "DELETE FROM workflow_steps WHERE run_id = ?1",
                rusqlite::params![run_id],
            )
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        report.dependent_rows += tx
            .execute(
                "DELETE FROM outbox WHERE run_id = ?1",
                rusqlite::params![run_id],
            )
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        report.runs_deleted += 1;
        tx.execute(
            "DELETE FROM workflow_runs WHERE id = ?1",
            rusqlite::params![run_id],
        )
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;
    use brain_server::migration::run_migration;
    use brain_server::register_sqlite_vec::register_sqlite_vec;

    fn db() -> (
        r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
        tempfile::NamedTempFile,
    ) {
        register_sqlite_vec();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mgr = r2d2_sqlite::SqliteConnectionManager::file(tmp.path());
        let pool = r2d2::Pool::builder().max_size(2).build(mgr).unwrap();
        run_migration(&mut pool.get().unwrap(), config::DB_MMAP_SIZE_MIB).unwrap();
        (pool, tmp)
    }

    fn seed_run(conn: &rusqlite::Connection, domain: &str, state: &str) -> i64 {
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, status, created_at, updated_at)
             VALUES (?1, 'interview', ?2, 'active', 1, 1)",
            rusqlite::params![domain, state],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn count(conn: &rusqlite::Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn sweep_deletes_matching_runs_and_dependents() {
        let (pool, _tmp) = db();
        let mut conn = pool.get().unwrap();
        let run = seed_run(&conn, "acme", r#"{"subject":"jane@example.com"}"#);
        let other = seed_run(&conn, "acme", r#"{"subject":"bob@example.com"}"#);
        {
            let tx = conn.transaction().unwrap();
            tx.execute(
                "INSERT INTO workflow_steps(run_id, phase, step_key, state_json) VALUES (?1,'p','s','{}')",
                rusqlite::params![run],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO findings(run_id, claim, evidence, source, confidence, ts)
                 VALUES (?1,'claim','ev','src',0.9,1)",
                rusqlite::params![run],
            )
            .unwrap();
            let rep = sweep_subject(&tx, "jane@example.com").unwrap();
            assert_eq!(rep.runs_deleted, 1);
            assert_eq!(rep.runs_matched, 1);
            assert!(rep.dependent_rows >= 2);
            tx.commit().unwrap();
        }
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM workflow_runs"),
            1,
            "only the non-matching run survives"
        );
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM workflow_steps"), 0);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM findings"), 0);
        assert_eq!(
            count(
                &conn,
                &format!("SELECT COUNT(*) FROM workflow_runs WHERE id={other}")
            ),
            1
        );
    }

    #[test]
    fn legal_hold_freezes_run_from_dsar_sweep() {
        let (pool, _tmp) = db();
        let mut conn = pool.get().unwrap();
        let run = seed_run(&conn, "acme", r#"{"who":"jane"}"#);
        conn.execute(
            "INSERT INTO legal_holds(knowledge_id, reason, held_by, held_at)
             VALUES (-?1, 'case-42 litigation', 'dpo', 1)",
            rusqlite::params![run],
        )
        .unwrap();
        let tx = conn.transaction().unwrap();
        let rep = sweep_subject(&tx, "jane").unwrap();
        assert_eq!(rep.runs_deleted, 0);
        assert_eq!(rep.runs_matched, 1);
        assert_eq!(
            rep.deferred,
            vec![(run, vec!["case-42 litigation".to_string()])]
        );
        tx.commit().unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM workflow_runs"), 1);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM legal_holds"), 1);
    }

    #[test]
    fn empty_subject_and_no_match_are_noops() {
        let (pool, _tmp) = db();
        let mut conn = pool.get().unwrap();
        seed_run(&conn, "acme", r#"{"a":1}"#);
        let tx = conn.transaction().unwrap();
        let empty = sweep_subject(&tx, "").unwrap();
        assert_eq!(empty, SweepReport::default());
        let none = sweep_subject(&tx, "missing-subject").unwrap();
        assert_eq!(none.runs_matched, 0);
        tx.commit().unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM workflow_runs"), 1);
    }
}
