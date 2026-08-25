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
    /// Crew rows erased with the subject: presence rows, skills tags, and
    /// shift-roster memberships rewritten to drop the subject.
    pub crew_rows: usize,
    /// Channel rows erased with the subject: notes and invites authored by
    /// or addressed to them (exact-principal match, crew posture).
    pub channel_rows: usize,
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
        // Channel + handover + CRM-linkage rows are FK children of the run:
        // they MUST go before the run row or the delete violates the foreign
        // key and the whole erasure fails (offers predated the case_notes
        // sweep; the Bridges linkage predates both — all three families clear
        // here, before their parent row; the external CRM case itself is out
        // of reach by design, only this server's link row dies).
        report.dependent_rows += tx
            .execute(
                "DELETE FROM case_notes WHERE run_id = ?1",
                rusqlite::params![run_id],
            )
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        report.dependent_rows += tx
            .execute(
                "DELETE FROM handover_offers WHERE run_id = ?1",
                rusqlite::params![run_id],
            )
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        report.dependent_rows += tx
            .execute(
                "UPDATE crm_cases SET run_id = NULL WHERE run_id = ?1",
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

    // ── Crew sweep: a principal id is
    // personal data wherever it sits. Presence + skills rows go by exact
    // principal; shift rosters are REWRITTEN to drop the subject (the shift
    // itself survives — it is schedule evidence, not subject data).
    report.crew_rows += tx
        .execute(
            "DELETE FROM presence WHERE principal = ?1",
            rusqlite::params![subject],
        )
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    report.crew_rows += tx
        .execute(
            "DELETE FROM principal_skills WHERE principal = ?1",
            rusqlite::params![subject],
        )
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    // Channel sweep: a note or invite carries the subject's personal data
    // BOTH as authorship/addressee ids and possibly in content — the exact-
    // principal sweep covers rows on ANY run (the over-match, erasure-safe
    // direction; run-level matches above took their dependents already).
    report.channel_rows += tx
        .execute(
            "DELETE FROM case_notes WHERE author = ?1 OR addressed_to = ?1",
            rusqlite::params![subject],
        )
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    let mut stmt = tx
        .prepare("SELECT id, roster_json FROM shifts WHERE roster_json LIKE ?1")
        .map_err(|e| HandlerError::internal(e.to_string()))?;
    let rostered: Vec<(i64, String)> = stmt
        .query_map(rusqlite::params![format!("%{subject}%")], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .map_err(|e| HandlerError::internal(e.to_string()))?
        .flatten()
        .collect();
    drop(stmt);
    for (id, json) in rostered {
        let Ok(roster) = serde_json::from_str::<Vec<String>>(&json) else {
            // A corrupt roster cell fails closed: the whole DSAR errors (tx
            // rolls back) rather than certifying an erasure that skipped it.
            return Err(HandlerError::internal(format!(
                "shift {id} roster_json is corrupt; erasure refused"
            )));
        };
        let kept: Vec<String> = roster
            .iter()
            .filter(|p| p.as_str() != subject)
            .cloned()
            .collect();
        if kept.len() != roster.len() {
            report.crew_rows += roster.len() - kept.len();
            let new_json =
                serde_json::to_string(&kept).map_err(|e| HandlerError::internal(e.to_string()))?;
            tx.execute(
                "UPDATE shifts SET roster_json = ?1 WHERE id = ?2",
                rusqlite::params![new_json, id],
            )
            .map_err(|e| HandlerError::internal(e.to_string()))?;
        }
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

    #[test]
    fn dsar_sweep_erases_crew_rows_and_scrubs_rosters() {
        use crate::workflow::crew::touch;
        use crate::workflow::shifts::{ShiftDraft, insert_shift};
        let (pool, _tmp) = db();
        let mut conn = pool.get().unwrap();
        // Presence + skills for two principals; only jane's must go.
        touch(&conn, "acme", "jane", "cranking", Some("run:1"), &[], 10).unwrap();
        touch(&conn, "acme", "bob", "reviewing", None, &[], 11).unwrap();
        conn.execute(
            "INSERT INTO principal_skills(domain, principal, skill, created_at)
             VALUES ('acme','jane','networking',1), ('acme','bob','voip',1)",
            [],
        )
        .unwrap();
        // One shift rostering both; one rostering only bob.
        let both = ["jane".to_string(), "bob".to_string()];
        let bob_only = ["bob".to_string()];
        insert_shift(
            &conn,
            &ShiftDraft {
                domain: "acme",
                site: "manila",
                tz: "UTC",
                start_epoch: 0,
                end_epoch: 100,
                overlap_minutes: 0,
                roster: &both,
            },
        )
        .unwrap();
        insert_shift(
            &conn,
            &ShiftDraft {
                domain: "acme",
                site: "ams",
                tz: "UTC",
                start_epoch: 100,
                end_epoch: 200,
                overlap_minutes: 0,
                roster: &bob_only,
            },
        )
        .unwrap();

        let tx = conn.transaction().unwrap();
        let rep = sweep_subject(&tx, "jane").unwrap();
        tx.commit().unwrap();
        assert_eq!(
            rep.crew_rows, 3,
            "presence row + skill tag + one roster membership"
        );
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM presence WHERE principal='jane'"
            ),
            0
        );
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM presence WHERE principal='bob'"),
            1
        );
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM principal_skills WHERE principal='jane'"
            ),
            0
        );
        let roster: String = conn
            .query_row(
                "SELECT roster_json FROM shifts WHERE site='manila'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            roster, r#"["bob"]"#,
            "the shift survives; the subject does not"
        );
        let untouched: String = conn
            .query_row("SELECT roster_json FROM shifts WHERE site='ams'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(untouched, r#"["bob"]"#);
    }

    /// dsar_sweep_erases_channel_rows_and_fk_children_of_the_run
    #[test]
    fn dsar_sweep_erases_channel_rows_and_fk_children_of_the_run() {
        let (pool, _tmp) = db();
        let mut conn = pool.get().unwrap();
        // A run with EVERY dependent family: a handover offer, channel notes,
        // and a Bridges CRM-case link (each references workflow_runs without
        // a cascade — any one left behind fails the whole erasure).
        let run = seed_run(&conn, "acme", r#"{"subject":"jane@example.com"}"#);
        conn.execute(
            "INSERT INTO handover_offers(domain, run_id, from_principal, to_principal,
                 state, sla_deadline, created_at)
             VALUES ('acme', ?1, 'jane', 'bob', 'offered', 9999, 1)",
            rusqlite::params![run],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO case_notes(domain, run_id, author, kind, content, state, created_at)
             VALUES ('acme', ?1, 'jane', 'note', 'working the queue', 'visible', 1)",
            rusqlite::params![run],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO crm_cases(case_ref, source, org_id, case_id, run_id, status, updated_rev)
             VALUES ('CR-1', 'github', 'o', 'c-1', ?1, 'open', 'r1')",
            rusqlite::params![run],
        )
        .unwrap();
        // A note authored by the subject on a DIFFERENT principal's run goes
        // too (exact-principal arm), but that run itself survives.
        let other = seed_run(&conn, "acme", r#"{"subject":"bob@example.com"}"#);
        conn.execute(
            "INSERT INTO case_notes(domain, run_id, author, kind, content, state, created_at)
             VALUES ('acme', ?1, 'jane', 'note', 'lenders context', 'visible', 2)",
            rusqlite::params![other],
        )
        .unwrap();

        let tx = conn.transaction().unwrap();
        let rep = sweep_subject(&tx, "jane").unwrap();
        tx.commit().unwrap();
        assert_eq!(rep.runs_deleted, 1);
        assert_eq!(
            rep.channel_rows, 1,
            "the cross-run authored note; the run-dependent pair counted as dependents"
        );
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM case_notes"),
            0,
            "every channel row carrying the subject is gone"
        );
        assert_eq!(
            count(
                &conn,
                &format!("SELECT COUNT(*) FROM workflow_runs WHERE id={other}")
            ),
            1,
            "bob's run survives"
        );
        // The external CRM case outlives its erased run: the LINK unlinks
        // (nullable by design), the sync row survives.
        let linked: Option<i64> = conn
            .query_row(
                "SELECT run_id FROM crm_cases WHERE case_ref='CR-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(linked, None, "the erased run's linkage is gone");
    }
}
