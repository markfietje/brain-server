//! The thin pipeline: a CLOSED, operator-ratified stage vocabulary over the
//! account record, with a decision_ref-required transition core — the
//! machine-refusal law verbatim: the machine never advances a stage on its
//! own authority, and the classifier never advances one at all (the Phase 2
//! advisory is a read surface, gated on 1.32.8, not built here).
//!
//! Stages are NOT stored on the record: the current stage is derived as the
//! latest [`PIPELINE_ROW_KIND`] row's stage under the account's run id, else
//! the entry stage ([`Stage::Lead`]). No account carries a pipeline row at
//! creation, so exactly the transitions the operator makes carry a
//! decision_ref. The vocabulary widens ONLY by dated addendum.

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::workflow::tx::WorkflowTx;

/// The additive session-log kind carrying the stage timeline, written under
/// the ACCOUNT's run id.
pub(crate) const PIPELINE_ROW_KIND: &str = "pipeline";

/// The audit target each transition writes.
pub(crate) const AUDIT_PIPELINE: &str = "pipeline";

/// The closed stage vocabulary (operator-ratified 2026-09-22).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Stage {
    Lead,
    Qualified,
    Proposal,
    ClosedWon,
    ClosedLost,
}

impl Stage {
    pub(crate) const ALL: &'static [Stage] = &[
        Stage::Lead,
        Stage::Qualified,
        Stage::Proposal,
        Stage::ClosedWon,
        Stage::ClosedLost,
    ];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Stage::Lead => "lead",
            Stage::Qualified => "qualified",
            Stage::Proposal => "proposal",
            Stage::ClosedWon => "closed_won",
            Stage::ClosedLost => "closed_lost",
        }
    }

    pub(crate) fn parse(raw: &str) -> Option<Self> {
        Stage::ALL.iter().copied().find(|s| s.as_str() == raw)
    }
}

/// The entry stage: every account sits here until its first transition.
pub(crate) const ENTRY_STAGE: Stage = Stage::Lead;

/// The allowed-edge table (closed; self-transitions refuse by absence).
pub(crate) fn allowed_edge(from: Stage, to: Stage) -> bool {
    matches!(
        (from, to),
        (Stage::Lead, Stage::Qualified)
            | (Stage::Qualified, Stage::Proposal)
            | (Stage::Proposal, Stage::ClosedWon)
            | (Stage::Proposal, Stage::ClosedLost)
    )
}

/// The pure transition gate: the decision_ref is REQUIRED (the machine
/// never advances a stage on its own authority), the target must sit in the
/// closed vocabulary, and the edge must be allowed. Named refusals only.
pub(crate) fn validate_transition(
    from: Stage,
    to: Stage,
    decision_ref: &str,
) -> Result<(), String> {
    if decision_ref.trim().is_empty() {
        return Err(
            "pipeline: decision_ref required — the machine never advances a \
             stage on its own authority"
                .into(),
        );
    }
    if from == to || !allowed_edge(from, to) {
        return Err(format!(
            "illegal_stage_transition: {}→{}",
            from.as_str(),
            to.as_str()
        ));
    }
    Ok(())
}

/// The account's current stage: the latest pipeline row's stage under the
/// account's run id, else the entry stage.
pub(crate) fn current_stage(conn: &Connection, account_id: i64) -> rusqlite::Result<Stage> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT payload_json FROM agent_session_events
              WHERE run_id = ?1 AND kind = ?2 ORDER BY seq DESC LIMIT 1",
            params![account_id, PIPELINE_ROW_KIND],
            |r| r.get(0),
        )
        .optional()?;
    let Some(payload_json) = raw else {
        return Ok(ENTRY_STAGE);
    };
    let payload: serde_json::Value = serde_json::from_str(&payload_json).map_err(|e| {
        rusqlite::Error::InvalidParameterName(format!("pipeline: stored row unreadable: {e}"))
    })?;
    let stage = payload
        .get("stage")
        .and_then(serde_json::Value::as_str)
        .and_then(Stage::parse)
        .ok_or_else(|| {
            rusqlite::Error::InvalidParameterName("pipeline: stored stage unknown".into())
        })?;
    Ok(stage)
}

/// The receipt for one advance.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PipelineReceipt {
    pub created: bool,
    pub seq: i64,
    pub from: Stage,
    pub to: Stage,
}

/// Advance one account's stage: derive the current stage, gate the
/// transition, append the `pipeline` row (`{stage, decision_ref,
/// prev_stage}`) + its audited row, all inside the caller's tx. The caller
/// screens the decision_ref at the surface; the core refuses independently.
pub(crate) fn advance_pipeline(
    tx: &mut WorkflowTx<'_>,
    account_id: i64,
    target: &str,
    decision_ref: &str,
    now: i64,
) -> Result<PipelineReceipt, String> {
    let to = Stage::parse(target).ok_or("pipeline_stage_unknown")?;
    let record = super::accounts::load_account(tx.tx(), account_id)
        .map_err(|e| format!("pipeline: stored record unreadable: {e}"))?
        .ok_or("account_not_found")?;
    if record.status == super::accounts::AccountStatus::Archived {
        return Err("account_archived".into());
    }
    let from = current_stage(tx.tx(), account_id)
        .map_err(|e| format!("pipeline: stage read failed: {e}"))?;
    validate_transition(from, to, decision_ref)?;
    let payload = serde_json::json!({
        "stage": to.as_str(),
        "decision_ref": decision_ref,
        "prev_stage": from.as_str(),
    })
    .to_string();
    let n: i64 = tx
        .tx()
        .query_row(
            "SELECT COUNT(*) FROM agent_session_events WHERE run_id = ?1 AND kind = ?2",
            params![account_id, PIPELINE_ROW_KIND],
            |r| r.get(0),
        )
        .map_err(|e| format!("pipeline: count read failed: {e}"))?;
    let key = format!("account{account_id}:pipeline:{}", n + 1);
    let (created, seq) =
        super::session_log::append(tx.tx(), account_id, PIPELINE_ROW_KIND, &payload, &key, now)
            .map_err(|e| format!("pipeline: row write failed: {e}"))?;
    if created {
        // Vocabulary words only in the detail — the reference never lands
        // in the audit text.
        super::audit_write(
            tx.tx(),
            account_id,
            AUDIT_PIPELINE,
            crate::audit::AuditStatus::Ok,
            &format!("stage {}→{}", from.as_str(), to.as_str()),
        );
    }
    Ok(PipelineReceipt {
        created,
        seq,
        from,
        to,
    })
}

/// The account's pipeline timeline, oldest first (the thin pipeline view).
pub(crate) fn pipeline_timeline(
    conn: &Connection,
    account_id: i64,
) -> rusqlite::Result<Vec<serde_json::Value>> {
    let mut stmt = conn.prepare(
        "SELECT seq, payload_json FROM agent_session_events
          WHERE run_id = ?1 AND kind = ?2 ORDER BY seq",
    )?;
    let rows = stmt.query_map(params![account_id, PIPELINE_ROW_KIND], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (seq, payload_json) = row?;
        let payload: serde_json::Value = serde_json::from_str(&payload_json).map_err(|e| {
            rusqlite::Error::InvalidParameterName(format!("pipeline: stored row unreadable: {e}"))
        })?;
        out.push(serde_json::json!({ "seq": seq, "stage": payload }));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migration;
    use crate::register_sqlite_vec::register_sqlite_vec;
    use crate::workflow::tx::WorkflowTx;
    use rusqlite::Connection;

    fn db() -> Connection {
        register_sqlite_vec();
        let mut conn = Connection::open_in_memory().unwrap();
        run_migration(&mut conn, 1).unwrap();
        conn
    }

    fn account(conn: &mut Connection) -> i64 {
        let mut wtx = WorkflowTx::begin(conn).unwrap();
        let created = crate::workflow::accounts::create_account(
            &mut wtx,
            "acme",
            "Acme Limited",
            "hash:op",
            100,
        )
        .unwrap();
        wtx.commit().unwrap();
        created.run_id
    }

    fn advance(
        conn: &mut Connection,
        id: i64,
        target: &str,
        r#ref: &str,
        now: i64,
    ) -> Result<PipelineReceipt, String> {
        let mut wtx = WorkflowTx::begin(conn).unwrap();
        let receipt = advance_pipeline(&mut wtx, id, target, r#ref, now);
        if receipt.is_ok() {
            wtx.commit().unwrap();
        }
        receipt
    }

    #[test]
    fn pipeline_vocabulary_is_closed() {
        let ratified = ["lead", "qualified", "proposal", "closed_won", "closed_lost"];
        assert_eq!(Stage::ALL.len(), ratified.len());
        for raw in ratified {
            let stage = Stage::parse(raw).unwrap();
            assert_eq!(stage.as_str(), raw, "parse/as_str round-trip");
        }
        for other in [
            "Lead", "won", "lost", "closed", "archived", "active", "", "prospect",
        ] {
            assert!(Stage::parse(other).is_none(), "{other} is not a stage");
        }
    }

    #[test]
    fn pipeline_allowed_edges_match() {
        // Exhaustive over the 5×5 grid: exactly the four ratified edges.
        let mut edges = Vec::new();
        for from in Stage::ALL {
            for to in Stage::ALL {
                if allowed_edge(*from, *to) {
                    edges.push((from.as_str(), to.as_str()));
                }
            }
        }
        assert_eq!(
            edges,
            vec![
                ("lead", "qualified"),
                ("qualified", "proposal"),
                ("proposal", "closed_won"),
                ("proposal", "closed_lost"),
            ]
        );
        // Self-transitions refuse by absence.
        for stage in Stage::ALL {
            assert!(!allowed_edge(*stage, *stage));
        }
    }

    #[test]
    fn pipeline_illegal_transition_refused() {
        // Skips and selfs name source→target; no decision_ref ambiguity.
        let err = validate_transition(Stage::Lead, Stage::ClosedWon, "op-1").unwrap_err();
        assert_eq!(err, "illegal_stage_transition: lead→closed_won");
        let err = validate_transition(Stage::Lead, Stage::Lead, "op-1").unwrap_err();
        assert_eq!(err, "illegal_stage_transition: lead→lead");
        let err = validate_transition(Stage::ClosedWon, Stage::Lead, "op-1").unwrap_err();
        assert_eq!(err, "illegal_stage_transition: closed_won→lead");
        // A legal edge with a real reference passes.
        assert!(validate_transition(Stage::Lead, Stage::Qualified, "op-1").is_ok());
    }

    #[test]
    fn pipeline_transition_requires_decision_ref() {
        for blank in ["", "   ", "\t"] {
            let err = validate_transition(Stage::Lead, Stage::Qualified, blank).unwrap_err();
            assert!(
                err.starts_with("pipeline: decision_ref required"),
                "the machine-refusal law names itself: {err}"
            );
        }
    }

    #[test]
    fn pipeline_advance_needs_authority() {
        let mut conn = db();
        let id = account(&mut conn);
        // Unknown target names the vocabulary.
        assert_eq!(
            advance(&mut conn, id, "won", "op-1", 150).unwrap_err(),
            "pipeline_stage_unknown"
        );
        // Absent account.
        assert_eq!(
            advance(&mut conn, 99_999, "qualified", "op-1", 150).unwrap_err(),
            "account_not_found"
        );
        // The full legal walk: lead → qualified → proposal → closed_won.
        assert_eq!(
            advance(&mut conn, id, "qualified", "op-1", 150).unwrap(),
            PipelineReceipt {
                created: true,
                seq: 1,
                from: Stage::Lead,
                to: Stage::Qualified
            }
        );
        assert_eq!(
            advance(&mut conn, id, "proposal", "op-2", 160).unwrap(),
            PipelineReceipt {
                created: true,
                seq: 2,
                from: Stage::Qualified,
                to: Stage::Proposal
            }
        );
        assert_eq!(
            advance(&mut conn, id, "closed_won", "op-3", 170).unwrap(),
            PipelineReceipt {
                created: true,
                seq: 3,
                from: Stage::Proposal,
                to: Stage::ClosedWon
            }
        );
        // Terminal: no edge leaves closed_won.
        let err = advance(&mut conn, id, "proposal", "op-4", 180).unwrap_err();
        assert_eq!(err, "illegal_stage_transition: closed_won→proposal");
        // The timeline: payload carries {stage, decision_ref, prev_stage}.
        let timeline = pipeline_timeline(&conn, id).unwrap();
        assert_eq!(timeline.len(), 3);
        assert_eq!(timeline[0]["stage"]["stage"], "qualified");
        assert_eq!(timeline[0]["stage"]["prev_stage"], "lead");
        assert_eq!(timeline[0]["stage"]["decision_ref"], "op-1");
        assert_eq!(timeline[2]["stage"]["stage"], "closed_won");
        assert_eq!(timeline[2]["stage"]["prev_stage"], "proposal");
        assert_eq!(current_stage(&conn, id).unwrap(), Stage::ClosedWon);
        // Each append carried exactly one pipeline audit row; the chain holds.
        let audits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE kind = 'workflow' AND target_hash = ?1",
                params![crate::audit::hash(AUDIT_PIPELINE)],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(audits, 3);
        assert!(crate::audit::verify_chain(&conn));
    }

    #[test]
    fn pipeline_entry_stage_is_derived_never_stored() {
        let mut conn = db();
        let id = account(&mut conn);
        // No rows yet: the entry stage derives, nothing is written.
        assert_eq!(current_stage(&conn, id).unwrap(), Stage::Lead);
        let rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_session_events WHERE run_id = ?1 AND kind = 'pipeline'",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rows, 0, "creation writes no pipeline row");
        // The first advance goes lead → qualified WITH a reference.
        advance(&mut conn, id, "qualified", "op-1", 150).unwrap();
        assert_eq!(current_stage(&conn, id).unwrap(), Stage::Qualified);
        // An archived account refuses stage changes.
        let mut wtx = WorkflowTx::begin(&mut conn).unwrap();
        crate::workflow::accounts::archive_account(&mut wtx, id, "op-arch", 200).unwrap();
        wtx.commit().unwrap();
        assert_eq!(
            advance(&mut conn, id, "proposal", "op-2", 210).unwrap_err(),
            "account_archived"
        );
    }
}
