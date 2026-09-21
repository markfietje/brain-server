//! The run report: a PURE generator over the run's RECORDED rows only — the
//! run's own columns, its step-plan gate records, and its workflow audit
//! rows. No `state_json` content (the engines CAS against those exact
//! bytes), no provider calls, no wall-clock fields. The same rows plus a
//! pinned law version always render the same report — that is the whole
//! contract: a buyer's DPO can re-render any historical case at the law
//! version it named.
//!
//! Reads are deliberately NOT audited: an audited read would insert a row
//! that the next report's audit section would include, breaking the
//! byte-reproducibility this module exists to prove.

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

/// One gate record: a step-plan row, rendered gate-by-gate at intake.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GateRecord {
    pub phase: String,
    pub step_key: String,
    pub revision: i64,
    pub parent_step_id: Option<i64>,
}

/// One recorded workflow audit row. The detail text is stored hashed by the
/// audit chain — the hash is the evidence binding, never a plaintext leak.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AuditEntry {
    pub id: i64,
    pub ts: String,
    pub actor: Option<String>,
    pub status: Option<String>,
    pub detail_hash: Option<String>,
}

/// The report. Field order is the serialization order; the advisory law:
/// `law_version_mismatch` is `Some(pinned != legal head)` only when a pinned
/// version was named AND the legal DB's head is readable — `None` (absent)
/// is the honest "advisory unavailable", never a refusal, never a block.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RunReport {
    pub run_id: i64,
    pub domain: String,
    pub kind: String,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub state_revision: i64,
    /// The pinned version when the caller named one, else the run's own
    /// intake stamp.
    pub law_version: String,
    pub law_version_pinned: Option<String>,
    pub law_version_mismatch: Option<bool>,
    /// The legal DB's head pin bytes, when its file is configured+readable.
    pub legal_db_head: Option<String>,
    pub gate_records: Vec<GateRecord>,
    pub audit: Vec<AuditEntry>,
}

/// The run's rows: `None` when the run does not exist (the caller renders
/// the probe-blind 404).
pub fn build_run_report(
    conn: &Connection,
    run_id: i64,
    pinned: Option<&str>,
    legal_db_head: Option<String>,
) -> rusqlite::Result<Option<RunReport>> {
    let run: Option<(String, String, String, i64, i64, i64, String)> = conn
        .query_row(
            "SELECT domain, kind, status, created_at, updated_at, state_revision, law_version
               FROM workflow_runs WHERE id = ?1",
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
        .optional()?;
    let Some((domain, kind, status, created_at, updated_at, state_revision, stamp)) = run else {
        return Ok(None);
    };
    let mut stmt = conn.prepare(
        "SELECT phase, step_key, revision, parent_step_id
           FROM workflow_steps WHERE run_id = ?1 ORDER BY id",
    )?;
    let gate_records = stmt
        .query_map(params![run_id], |r| {
            Ok(GateRecord {
                phase: r.get(0)?,
                step_key: r.get(1)?,
                revision: r.get(2)?,
                parent_step_id: r.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let target = crate::audit::hash(&format!("run:{run_id}"));
    let mut stmt = conn.prepare(
        "SELECT id, ts, actor, status, detail_hash
           FROM audit_events WHERE kind = 'workflow' AND target_hash = ?1 ORDER BY id",
    )?;
    let audit = stmt
        .query_map(params![target], |r| {
            Ok(AuditEntry {
                id: r.get(0)?,
                ts: r.get(1)?,
                actor: r.get(2)?,
                status: r.get(3)?,
                detail_hash: r.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let law_version_mismatch = match (pinned, legal_db_head.as_deref()) {
        (Some(p), Some(h)) => Some(p != h),
        _ => None,
    };
    Ok(Some(RunReport {
        run_id,
        domain,
        kind,
        status,
        created_at,
        updated_at,
        state_revision,
        law_version: pinned.unwrap_or(&stamp).to_string(),
        law_version_pinned: pinned.map(str::to_string),
        law_version_mismatch,
        legal_db_head,
        gate_records,
        audit,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The C5 contract at the pure layer: the same recorded rows plus a
    /// pinned law version render the identical report, twice; and the
    /// mismatch advisory flips exactly when the legal DB's head moves —
    /// never a refusal, never a block.
    #[test]
    fn old_report_reproducible_at_pinned_version() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::migration::run_migration(&mut conn, 0).unwrap();
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at, law_version)
             VALUES ('personal', 'interview', '{\"a\":1}', 0, 'active', 1700000000, 1700000100, 'npc-advisory-2024-04')",
            [],
        )
        .unwrap();
        let run_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO workflow_steps(run_id, phase, step_key, state_json, revision) VALUES (?1, 'triage', 't1', '{}', 0)",
            params![run_id],
        )
        .unwrap();
        crate::workflow::audit_write(
            &conn,
            run_id,
            &format!("run:{run_id}"),
            crate::audit::AuditStatus::Ok,
            "open law_version=npc-advisory-2024-04",
        );

        // Pinned at the run's own law version, DB head equal → identical
        // twice, mismatch false.
        let head = "npc-advisory-2024-04".to_string();
        let a = build_run_report(
            &conn,
            run_id,
            Some("npc-advisory-2024-04"),
            Some(head.clone()),
        )
        .unwrap()
        .unwrap();
        let b = build_run_report(
            &conn,
            run_id,
            Some("npc-advisory-2024-04"),
            Some(head.clone()),
        )
        .unwrap()
        .unwrap();
        assert_eq!(a, b, "same rows + pinned version → identical report, twice");
        assert_eq!(a.law_version, "npc-advisory-2024-04");
        assert_eq!(a.law_version_mismatch, Some(false));
        assert_eq!(a.gate_records.len(), 1);
        assert_eq!(a.gate_records[0].phase, "triage");
        assert_eq!(a.audit.len(), 1, "the open audit row is recorded evidence");

        // The head moves (the DPO imported a newer version): the SAME rows
        // flip the advisory to true — advisory only, the report still builds.
        let moved = build_run_report(
            &conn,
            run_id,
            Some("npc-advisory-2024-04"),
            Some("moved".to_string()),
        )
        .unwrap()
        .unwrap();
        assert_eq!(moved.law_version_mismatch, Some(true));
        assert_eq!(moved.gate_records, a.gate_records);
        assert_eq!(moved.audit, a.audit);

        // No pinned version → the run's own stamp labels the view; no legal
        // head → the advisory is honestly absent, never a refusal.
        let unstamped_view = build_run_report(&conn, run_id, None, None)
            .unwrap()
            .unwrap();
        assert_eq!(unstamped_view.law_version, "npc-advisory-2024-04");
        assert_eq!(unstamped_view.law_version_mismatch, None);

        // Unknown run → None (the caller renders the probe-blind 404).
        assert!(build_run_report(&conn, 999, None, None).unwrap().is_none());
    }
}
