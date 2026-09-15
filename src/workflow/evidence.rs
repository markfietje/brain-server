//! The GDL evidence vocabulary — 8 epistemic types over the SDK reducer.
//!
//! The kernel's findings/contradictions tables store UNTYPED
//! [`brain_engine_sdk::pure::evidence::Finding`] rows; this module is the
//! typed GDL face over them: a CLOSED 8-type vocabulary (hypothesis, test,
//! expected, actual, confidence, contradiction, verification, capture —
//! the GDL spec's set, never merged, never extended quietly) mapped onto
//! findings rows by a stable claim prefix, reduced by the SDK's existing
//! [`reduce`] — dedup + false-merge guard + contradiction surfacing —
//! WITHOUT touching the merge law (the SDK is not edited; its oracle pins
//! re-run unchanged and bridge tests here pin the mapping's behavior).
//!
//! Determinism: the SDK reducer orders output claim-sorted
//! (BTreeMap canonical keys); a typed batch therefore persists in a
//! deterministic order and replaying the same batch yields the same rows.
//!
//! The revisit law (the GDL spec's "revisit after verify requires
//! justification"): once a run's row is CLOSED (`resolved`/`completed`/
//! `closed` — resolution implies verification passed), recording further
//! evidence against it is REFUSED with an audit `denied` row unless the
//! caller carries a recorded justification. The refusal is the point: a
//! verified case's diagnostic record does not mutate silently after the
//! fact; when it legitimately must (a reopened RCA), the justification is
//! a durable, auditable process fact.
//!
//! What this deliberately does NOT do: no schema change (the kind rides
//! the claim prefix — `findings` has no kind column and adding one is a
//! declared schema move, not this release's), no evidence deletion or
//! mutation (append-only like everything the chain reconstructs), and no
//! second reducer — one merge law, the SDK's.

use rusqlite::{Connection, params};

use super::audit_write;
use crate::audit::AuditStatus;
use brain_engine_sdk::pure::evidence::{Finding, reduce};

/// The closed 8-type GDL evidence vocabulary. Exactly eight, pinned by
/// test; a ninth type is a spec change, not a refactor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EvidenceKind {
    Hypothesis,
    Test,
    Expected,
    Actual,
    Confidence,
    Contradiction,
    Verification,
    Capture,
}

impl EvidenceKind {
    pub(crate) const ALL: [EvidenceKind; 8] = [
        EvidenceKind::Hypothesis,
        EvidenceKind::Test,
        EvidenceKind::Expected,
        EvidenceKind::Actual,
        EvidenceKind::Confidence,
        EvidenceKind::Contradiction,
        EvidenceKind::Verification,
        EvidenceKind::Capture,
    ];

    /// The stable claim prefix that carries the kind into the untyped
    /// findings row (the schema stays untouched; the prefix is the type).
    pub(crate) fn prefix(self) -> &'static str {
        match self {
            EvidenceKind::Hypothesis => "hypothesis",
            EvidenceKind::Test => "test",
            EvidenceKind::Expected => "expected",
            EvidenceKind::Actual => "actual",
            EvidenceKind::Confidence => "confidence",
            EvidenceKind::Contradiction => "contradiction",
            EvidenceKind::Verification => "verification",
            EvidenceKind::Capture => "capture",
        }
    }

    /// Parse the kind back off a persisted finding's claim. Unknown
    /// prefixes (pre-GDL findings rows, foreign writers) map to None —
    /// untyped rows are the kernel's, not ours to reinterpret.
    pub(crate) fn of_claim(claim: &str) -> Option<EvidenceKind> {
        let prefix = claim.split(':').next()?;
        EvidenceKind::ALL.into_iter().find(|k| k.prefix() == prefix)
    }
}

/// One typed evidence line, pre-reduction. Callers construct; the writer
/// only reads.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TypedEvidence {
    pub kind: EvidenceKind,
    /// The claim in GDL terms ("battery state"). The writer composes the
    /// persisted claim as `<prefix>: <claim>`.
    pub claim: String,
    /// The locator/digest/observation backing the claim (the false-merge
    /// guard's key: different evidence on one claim NEVER merges).
    pub evidence: String,
    pub source: String,
    pub confidence: f64,
    pub ts: i64,
}

impl TypedEvidence {
    /// The finding this line persists as. PURE — the mapping is the
    /// typed/untyped bridge, and its stability is what replay depends on.
    pub(crate) fn to_finding(&self) -> Finding {
        Finding {
            claim: format!("{}: {}", self.kind.prefix(), self.claim),
            evidence: self.evidence.clone(),
            source: self.source.clone(),
            confidence: self.confidence,
            ts: self.ts,
        }
    }

    /// Rebuild the typed view off a persisted finding row (the inverse of
    /// [`to_finding`]); None for untyped rows.
    pub(crate) fn from_finding(f: &Finding) -> Option<TypedEvidence> {
        let kind = EvidenceKind::of_claim(&f.claim)?;
        let claim = f
            .claim
            .strip_prefix(&format!("{}: ", kind.prefix()))?
            .to_string();
        Some(TypedEvidence {
            kind,
            claim,
            evidence: f.evidence.clone(),
            source: f.source.clone(),
            confidence: f.confidence,
            ts: f.ts,
        })
    }
}

/// What one recorded batch produced, in reduction order.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RecordOutcome {
    /// Inserted findings row ids (reduction order — deterministic).
    pub findings: Vec<i64>,
    /// Surfaced contradiction pairs as findings-row id pairs.
    pub contradictions: Vec<(i64, i64)>,
}

/// The typed writer's failure vocabulary. Loud and typed; the caller
/// decides policy — a revisit denial is a gate outcome, not a crash.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum EvidenceError {
    Db(String),
    /// The run is closed (verified/resolved) and no justification rode
    /// the write. The audit `denied` row already landed in-tx.
    RevisitDenied,
}

/// Record one typed batch against `run_id` INSIDE the caller's
/// [`super::tx::WorkflowTx`]: reduce → insert findings → insert open
/// contradiction rows for surfaced pairs → one audit row for the batch.
/// The revisit law applies when the run row is closed.
pub(crate) fn record(
    conn: &Connection,
    run_id: i64,
    batch: &[TypedEvidence],
    justification: Option<&str>,
) -> Result<RecordOutcome, EvidenceError> {
    // The revisit gate reads the run row's status inside the caller's tx:
    // resolution implies verify passed, and a closed case's record is a
    // revisit. Unjustified → durable denied audit + refusal.
    let status: Option<String> = conn
        .query_row(
            "SELECT status FROM workflow_runs WHERE id = ?1",
            params![run_id],
            |r| r.get(0),
        )
        .map_err(|e| EvidenceError::Db(e.to_string()))
        .ok()
        .flatten();
    if let Some(status) = status
        && matches!(status.as_str(), "resolved" | "completed" | "closed")
    {
        let Some(reason) = justification.filter(|j| !j.trim().is_empty()) else {
            audit_write(
                conn,
                run_id,
                &format!("evidence:{run_id}"),
                AuditStatus::Denied,
                "revisit_after_verify: evidence on a closed run requires a recorded justification",
            );
            return Err(EvidenceError::RevisitDenied);
        };
        audit_write(
            conn,
            run_id,
            &format!("evidence:{run_id}"),
            AuditStatus::Ok,
            &format!("revisit_after_verify justified: {reason}"),
        );
    }

    let reduction = reduce(batch.iter().map(TypedEvidence::to_finding).collect());
    let mut ids = Vec::with_capacity(reduction.findings.len());
    for f in &reduction.findings {
        conn.execute(
            "INSERT INTO findings(run_id, claim, evidence, source, confidence, ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![run_id, f.claim, f.evidence, f.source, f.confidence, f.ts],
        )
        .map_err(|e| EvidenceError::Db(e.to_string()))?;
        ids.push(conn.last_insert_rowid());
    }
    let mut pairs = Vec::with_capacity(reduction.contradictions.len());
    for (a, b) in &reduction.contradictions {
        conn.execute(
            "INSERT INTO contradictions(run_id, finding_a_id, finding_b_id, state)
             VALUES (?1, ?2, ?3, 'open')",
            params![run_id, ids[*a], ids[*b]],
        )
        .map_err(|e| EvidenceError::Db(e.to_string()))?;
        pairs.push((ids[*a], ids[*b]));
    }
    // One audit row per batch mutation (audit-per-write; the row carries
    // the census so the chain reconstructs the shape of the write).
    let kinds: Vec<&str> = batch.iter().map(|t| t.kind.prefix()).collect();
    audit_write(
        conn,
        run_id,
        &format!("evidence:{run_id}"),
        AuditStatus::Ok,
        &format!(
            "recorded {} typed lines ({}), {} contradiction(s) surfaced",
            ids.len(),
            kinds.join(","),
            pairs.len()
        ),
    );
    Ok(RecordOutcome {
        findings: ids,
        contradictions: pairs,
    })
}

/// Resolve one surfaced contradiction by naming the finding that settles
/// it (the A4 discipline: contradictions are surfaced and resolved with
/// evidence, never merged away).
pub(crate) fn resolve_contradiction(
    conn: &Connection,
    run_id: i64,
    contradiction_id: i64,
    resolved_by_finding_id: i64,
) -> Result<(), EvidenceError> {
    let updated = conn
        .execute(
            "UPDATE contradictions SET state = 'resolved', resolved_by_finding_id = ?3
              WHERE id = ?1 AND run_id = ?2 AND state = 'open'",
            params![contradiction_id, run_id, resolved_by_finding_id],
        )
        .map_err(|e| EvidenceError::Db(e.to_string()))?;
    if updated == 0 {
        return Err(EvidenceError::Db(format!(
            "contradiction {contradiction_id} not open for run {run_id}"
        )));
    }
    audit_write(
        conn,
        run_id,
        &format!("contradiction:{contradiction_id}"),
        AuditStatus::Ok,
        &format!("resolved by finding {resolved_by_finding_id}"),
    );
    Ok(())
}

/// Open contradiction count for a run — the QA bridge's contradiction
/// signal (degraded reads count zero; the scorer treats missing data as
/// no-signal, the same posture as the handoff packet's legal-holds read).
pub(crate) fn open_contradictions(conn: &Connection, run_id: i64) -> usize {
    conn.query_row(
        "SELECT COUNT(*) FROM contradictions WHERE run_id = ?1 AND state = 'open'",
        params![run_id],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n as usize)
    .unwrap_or(0)
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
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('acme', 'troubleshoot', '{}', 0, 'active', 1, 1)",
            [],
        )
        .unwrap();
        conn
    }

    fn line(kind: EvidenceKind, claim: &str, evidence: &str, confidence: f64) -> TypedEvidence {
        TypedEvidence {
            kind,
            claim: claim.into(),
            evidence: evidence.into(),
            source: "gdl".into(),
            confidence,
            ts: 1,
        }
    }

    fn findings(conn: &Connection) -> Vec<(String, String)> {
        let mut stmt = conn
            .prepare("SELECT claim, evidence FROM findings ORDER BY id")
            .unwrap();
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        rows.collect::<Result<_, _>>().unwrap()
    }

    #[test]
    fn evidence_kind_set_is_exactly_eight() {
        assert_eq!(EvidenceKind::ALL.len(), 8);
        let prefixes: Vec<&str> = EvidenceKind::ALL.iter().map(|k| k.prefix()).collect();
        assert_eq!(
            prefixes,
            vec![
                "hypothesis",
                "test",
                "expected",
                "actual",
                "confidence",
                "contradiction",
                "verification",
                "capture"
            ],
            "the GDL spec's closed set, pinned"
        );
        // Round-trip: every prefix parses back to its kind, foreign
        // prefixes do not.
        for k in EvidenceKind::ALL {
            assert_eq!(
                EvidenceKind::of_claim(&format!("{}: x", k.prefix())),
                Some(k)
            );
        }
        assert_eq!(EvidenceKind::of_claim("vendor: x"), None);
    }

    #[test]
    fn typed_rows_ride_the_sdk_reducer_unchanged() {
        // The bridge must not bend the merge law: same typed claim +
        // identical evidence dedups; same claim + DIFFERENT evidence
        // stays separate and surfaces a contradiction (the false-merge
        // guard, on typed rows).
        let mut conn = db();
        let batch = vec![
            line(EvidenceKind::Actual, "battery state", "Failed", 0.9),
            line(EvidenceKind::Actual, "BATTERY   state", "Failed", 0.4),
            line(EvidenceKind::Actual, "battery state", "Ready", 0.6),
        ];
        let mut wtx = WorkflowTx::begin(&mut conn).unwrap();
        let out = record(wtx.tx(), 1, &batch, None).unwrap();
        wtx.commit().unwrap();
        assert_eq!(out.findings.len(), 2, "identical evidence dedups");
        assert_eq!(out.contradictions.len(), 1, "Failed vs Ready surfaces");
        let rows = findings(&conn);
        assert_eq!(
            rows,
            vec![
                ("actual: battery state".into(), "Failed".into()),
                ("actual: battery state".into(), "Ready".into()),
            ],
            "deterministic order: highest confidence first, claim-sorted"
        );
        // The round-trip view is lossless.
        let typed = TypedEvidence::from_finding(&Finding {
            claim: rows[0].0.clone(),
            evidence: rows[0].1.clone(),
            source: "gdl".into(),
            confidence: 0.9,
            ts: 1,
        })
        .unwrap();
        assert_eq!(typed.kind, EvidenceKind::Actual);
        assert_eq!(typed.claim, "battery state");
    }

    #[test]
    fn different_kinds_never_merge_or_contradict() {
        // An `expected` and an `actual` about the same check are different
        // CLAIMS (the prefix is the type) — they never group, never
        // surface as a contradiction. Contradiction means two answers to
        // the SAME typed question.
        let mut conn = db();
        let batch = vec![
            line(EvidenceKind::Expected, "battery state", "Ready", 0.5),
            line(EvidenceKind::Actual, "battery state", "Failed", 0.9),
        ];
        let mut wtx = WorkflowTx::begin(&mut conn).unwrap();
        let out = record(wtx.tx(), 1, &batch, None).unwrap();
        wtx.commit().unwrap();
        assert_eq!(out.findings.len(), 2);
        assert!(out.contradictions.is_empty());
    }

    #[test]
    fn record_persists_audit_and_open_contradictions() {
        let mut conn = db();
        let batch = vec![
            line(
                EvidenceKind::Hypothesis,
                "battery dead",
                "predicts state Failed",
                0.5,
            ),
            line(
                EvidenceKind::Verification,
                "rebuild under load",
                "pass 14%/h",
                0.9,
            ),
        ];
        let mut wtx = WorkflowTx::begin(&mut conn).unwrap();
        record(wtx.tx(), 1, &batch, None).unwrap();
        wtx.commit().unwrap();
        assert_eq!(open_contradictions(&conn, 1), 0);
        // Resolve flow: surface one, then settle it by naming a finding.
        let mut wtx = WorkflowTx::begin(&mut conn).unwrap();
        let out = record(
            wtx.tx(),
            1,
            &[
                line(EvidenceKind::Actual, "battery state", "Ready", 0.8),
                line(EvidenceKind::Actual, "battery state", "Failed", 0.7),
            ],
            None,
        )
        .unwrap();
        wtx.commit().unwrap();
        assert_eq!(open_contradictions(&conn, 1), 1);
        let cid: i64 = conn
            .query_row(
                "SELECT id FROM contradictions WHERE run_id = 1 AND state = 'open'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let mut wtx = WorkflowTx::begin(&mut conn).unwrap();
        resolve_contradiction(wtx.tx(), 1, cid, out.findings[0]).unwrap();
        wtx.commit().unwrap();
        assert_eq!(open_contradictions(&conn, 1), 0);
        // Double-resolve refuses (already settled).
        let mut wtx = WorkflowTx::begin(&mut conn).unwrap();
        assert!(resolve_contradiction(wtx.tx(), 1, cid, out.findings[0]).is_err());
        wtx.commit().unwrap();
    }

    #[test]
    fn revisit_after_verify_without_justification_is_denied() {
        let mut conn = db();
        conn.execute(
            "UPDATE workflow_runs SET status = 'resolved' WHERE id = 1",
            [],
        )
        .unwrap();
        let mut wtx = WorkflowTx::begin(&mut conn).unwrap();
        let err = record(
            wtx.tx(),
            1,
            &[line(EvidenceKind::Capture, "late write", "h1", 0.5)],
            None,
        )
        .unwrap_err();
        wtx.commit().unwrap();
        assert_eq!(err, EvidenceError::RevisitDenied);
        // The denial is a durable audit fact (details land hashed — the
        // chain stores tamper evidence, not payloads), and nothing
        // persisted.
        let denial_detail = crate::audit::hash(
            "revisit_after_verify: evidence on a closed run requires a recorded justification",
        );
        let denials: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE status = 'denied' AND detail_hash = ?1",
                params![denial_detail],
                |r| r.get(0),
            )
            .unwrap_or(0);
        assert!(denials >= 1, "the revisit denial is audited");
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM findings WHERE run_id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(n, 0, "the unjustified write refused");
        // A recorded justification reopens the door — and audits itself.
        let mut wtx = WorkflowTx::begin(&mut conn).unwrap();
        record(
            wtx.tx(),
            1,
            &[line(EvidenceKind::Capture, "reopened rca", "h2", 0.5)],
            Some("repeater RCA reopened by eng"),
        )
        .unwrap();
        wtx.commit().unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM findings WHERE run_id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(n, 1);
        let just_detail =
            crate::audit::hash("revisit_after_verify justified: repeater RCA reopened by eng");
        let just: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE status = 'ok' AND detail_hash = ?1",
                params![just_detail],
                |r| r.get(0),
            )
            .unwrap_or(0);
        assert!(just >= 1, "the justification is audited (hashed)");
    }

    #[test]
    fn active_runs_record_without_justification() {
        // The gate is about CLOSED runs; an open case records freely.
        let mut conn = db();
        let mut wtx = WorkflowTx::begin(&mut conn).unwrap();
        record(
            wtx.tx(),
            1,
            &[line(EvidenceKind::Test, "query battery", "racadm get", 0.5)],
            None,
        )
        .unwrap();
        wtx.commit().unwrap();
        assert_eq!(findings(&conn).len(), 1);
    }
}
