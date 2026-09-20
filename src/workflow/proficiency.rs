//! The L1/L2/L3 proficiency model — capability narrowing, not role strings.
//!
//! A proficiency level is TWO narrowings of the same loop, never a
//! different loop: an [`ExecutionEnv`] subset (L1 observes, L2 acts, L3
//! runs remediation processes — each level's env a MONOTONIC SUBSET of
//! the next, pinned by test) and a phase/step AUTHORITY matrix (who may
//! OWN a phase or ACT on a plan step, the methodology §10 `skill_gate`).
//! What is never narrowed: ESCALATION — a level below a phase's owner
//! escalates with the bundle (the guard's final denial posture: the
//! policy layer returns WITHOUT calling next, so the loop's answer is a
//! loud hand-off, never a silent capability wall).
//!
//! The no-auto-publish law lives here too: the capture path's only
//! write is PROPOSAL ROWS on the pending `/proposals` queue (the flywheel
//! `GapAction` vocabulary is ProposeNew/ProposeUpdate — the pure type
//! offers no direct-write variant, and the knowledge tables are asserted
//! byte-unchanged by the capture test). Humans validate every capture;
//! AI drafts and assists, it never decides.
//!
//! What this deliberately does NOT do: no per-operator identity mapping
//! (levels are loop configuration, principal law unchanged), no
//! certification/routing automation (pilot readiness is gold-set +
//! ambiguity-register measured, not self-assessed here), and no new
//! phases — the authority matrix rides the existing machine.

use brain_engine_sdk::env::ExecutionEnv;
use brain_engine_sdk::pure::qa_score::{FlywheelProposal, GapAction, flywheel_proposals};

use super::gdl::{GdlCase, GdlPhase, TestLogRow};
use super::tx::WorkflowTx;

/// The proficiency tiers (methodology §8): L1 executes, L2 verifies and
/// owns the capture decision, L3/Eng owns hypothesis and RCA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Proficiency {
    L1,
    L2,
    L3,
}

impl Proficiency {
    /// Numeric rank for the monotonic-narrowing pin and the skill-gate
    /// comparison (L0 steps are executable by everyone, per §10).
    pub(crate) fn rank(self) -> u8 {
        match self {
            Proficiency::L1 => 1,
            Proficiency::L2 => 2,
            Proficiency::L3 => 3,
        }
    }
}

/// The minimum proficiency that OWNS each phase. L1 executes the
/// observation phases and plan steps it is gated for; Hypothesize/Plan
/// are the engineer's (the differential and the step plan come from the
/// knowledge layer or a higher tier — never improvised below L3, which
/// is L9's no-fix-from-memory law applied to planning); Verify/Handoff
/// are L2's (the 2nd-verification and capture decisions).
pub(crate) fn phase_owner(phase: GdlPhase) -> Proficiency {
    match phase {
        GdlPhase::Intake | GdlPhase::Triage | GdlPhase::Act => Proficiency::L1,
        GdlPhase::Verify | GdlPhase::Handoff => Proficiency::L2,
        GdlPhase::Hypothesize | GdlPhase::Plan => Proficiency::L3,
    }
}

/// May `level` act on a plan step gated `skill_gate` ("L0".."L3")?
/// Every tier SEES every step; the gate decides who may ACT (§10).
pub(crate) fn may_execute_step(level: Proficiency, skill_gate: &str) -> bool {
    let gate = match skill_gate {
        "L0" => 0,
        "L1" => 1,
        "L2" => 2,
        "L3" => 3,
        _ => return false, // unknown gates deny — fail-closed, like the arbiter
    };
    level.rank() >= gate
}

/// The level's [`ExecutionEnv`]: capability SUBTRACTION from the base the
/// operator provisioned, never addition (the subagent ceiling, applied to
/// tiers). L1 observes (read-only, no process, no commands); L2 adds the
/// write lane for plan action steps; L3 adds the process lane for
/// remediation commands. Monotone by construction, pinned by test.
pub(crate) fn env_for(base: &ExecutionEnv, level: Proficiency) -> ExecutionEnv {
    let (write, process) = match level {
        Proficiency::L1 => (false, false),
        Proficiency::L2 => (true, false),
        Proficiency::L3 => (true, true),
    };
    ExecutionEnv {
        fs: std::sync::Arc::clone(&base.fs),
        read_only: base.read_only || !write,
        allow_process: base.allow_process && process,
        root: base.root.clone(),
        allowed_commands: if base.allow_process && process {
            base.allowed_commands.clone()
        } else {
            Vec::new()
        },
    }
}

/// Row-level authority errors for an Act artifact: rows executing plan
/// steps gated above `level` are violations — the row must wait for the
/// tier that owns it (escalation), not be laundered through a lower one.
pub(crate) fn act_authority_errors(
    case: &GdlCase,
    rows: &[TestLogRow],
    level: Proficiency,
) -> Vec<String> {
    let mut errors = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        let Some(step) = case.plan.iter().find(|s| s.order == r.order) else {
            continue; // the arbiter's own contiguous-order check covers this
        };
        if !may_execute_step(level, &step.skill_gate) {
            errors.push(format!(
                "authority: row[{i}] acts on step {} gated {} above {} — \
                 escalate to the owning tier, do not launder the step",
                step.order,
                step.skill_gate,
                match level {
                    Proficiency::L1 => "L1",
                    Proficiency::L2 => "L2",
                    Proficiency::L3 => "L3",
                }
            ));
        }
    }
    errors
}

/// The capture path's flywheel output: PROPOSALS only. The gap rule
/// (Microsoft's, applied to playbooks: <40% similar → propose new,
/// 40–80% → propose update) and the repeater rule (≥3 same-differential
/// cases in 30 days → RCA proposal) are the pure scorer's; this maps a
/// finished case onto them.
pub(crate) fn capture_proposals(
    case: &GdlCase,
    similarity_units: i32,
    repeater_count_30d: usize,
) -> Vec<FlywheelProposal> {
    flywheel_proposals(similarity_units, repeater_count_30d)
        .into_iter()
        .filter(|_| case.capture.is_some())
        .collect()
}

/// Enqueue capture proposals on the pending `/proposals` queue — the
/// HITL surface where humans approve/reject/edit. This is the capture
/// path's ONLY write: no knowledge row, no vec row, no article. Writing
/// TO THE QUEUE is not publishing; the queue deciding is the human's.
pub(crate) fn enqueue_capture_proposals(
    wtx: &mut WorkflowTx<'_>,
    run_id: i64,
    case: &GdlCase,
    similarity_units: i32,
    repeater_count_30d: usize,
    now: i64,
) -> rusqlite::Result<usize> {
    let proposals = capture_proposals(case, similarity_units, repeater_count_30d);
    let mut enqueued = 0;
    for p in &proposals {
        let (kind, content) = match p {
            FlywheelProposal::Gap(GapAction::ProposeNew) => (
                "gdl_gap_new",
                format!("run {run_id}: capture proposes a NEW playbook"),
            ),
            FlywheelProposal::Gap(GapAction::ProposeUpdate) => (
                "gdl_gap_update",
                format!("run {run_id}: capture proposes UPDATING the linked playbook"),
            ),
            FlywheelProposal::Rca => (
                "gdl_rca",
                format!("run {run_id}: repeater >=3/30d — blameless RCA proposal"),
            ),
            FlywheelProposal::ComplaintRca => (
                "gdl_complaint_rca",
                format!("run {run_id}: complaint-cluster RCA proposal"),
            ),
            // The SDK's vocabulary is #[non_exhaustive]: a future variant
            // rides the queue as a generic pending proposal, never drops.
            _ => ("gdl_proposal", format!("run {run_id}: capture proposal")),
        };
        wtx.tx().execute(
            "INSERT INTO proposals(kind, content, source, novelty, salience, status, created_at)
             VALUES (?1, ?2, 'gdl-capture', 0.5, 0.5, 'pending', ?3)",
            rusqlite::params![kind, content, now],
        )?;
        enqueued += 1;
    }
    if enqueued > 0 {
        super::audit_write(
            wtx.tx(),
            run_id,
            &format!("capture:{run_id}"),
            crate::audit::AuditStatus::Ok,
            &format!("no-auto-publish: {enqueued} proposal(s) enqueued pending human review"),
        );
    }
    Ok(enqueued)
}

/// The unmeasured-similarity sentinel: the scorer's [`gap_decision`] is
/// `None` at 8_000, so this value claims NO gap fact. A gap-new or
/// gap-update proposal requires a similarity that was actually measured,
/// which no GDL surface produces today — until such a source lands, the
/// gap variants stay helper-level-tested (the 3_000/6_000 fixtures) and
/// never driver-reachable.
const SIMILARITY_NOT_MEASURED: i32 = 8_000;

/// The domain-wide repeater census: resolved troubleshoot runs in the
/// domain over the last 30 days, window edges inclusive, the closing case
/// included (the caller runs it after the run row flips to `resolved` —
/// the kcs complaint census's exact shape: same BETWEEN window,
/// include-self, scalar COUNT, bounded by construction). Domain-level
/// grains over-trigger toward human review, never under-trigger.
fn repeater_census(conn: &rusqlite::Connection, domain: &str, now: i64) -> rusqlite::Result<usize> {
    conn.query_row(
        "SELECT COUNT(*) FROM workflow_runs
          WHERE domain = ?1 AND kind = 'troubleshoot' AND status = 'resolved'
            AND created_at BETWEEN ?2 - 30*86400 AND ?2",
        rusqlite::params![domain, now],
        |r| r.get::<_, i64>(0).map(|n| n as usize),
    )
}

/// The Handoff capture decision, wired into the closing transaction:
/// census the domain's repeaters, enqueue whatever the pure scorer
/// warrants onto the pending queue, and record the outcome when nothing
/// was warranted (the enqueue audits its own rows when something was).
/// This is the capture path's only production write — one writer, one
/// queue; `kcs::capture_on_case_close` is the complementary one.
pub(crate) fn capture_proposals_on_resolve(
    wtx: &mut WorkflowTx<'_>,
    run_id: i64,
    case: &GdlCase,
    now: i64,
) -> rusqlite::Result<usize> {
    let domain: String = wtx.tx().query_row(
        "SELECT domain FROM workflow_runs WHERE id = ?1",
        rusqlite::params![run_id],
        |r| r.get(0),
    )?;
    let census = repeater_census(wtx.tx(), &domain, now)?;
    let enqueued =
        enqueue_capture_proposals(wtx, run_id, case, SIMILARITY_NOT_MEASURED, census, now)?;
    if enqueued == 0 {
        super::audit_write(
            wtx.tx(),
            run_id,
            &format!("capture:{run_id}"),
            crate::audit::AuditStatus::Ok,
            "capture outcome: no proposal warranted (playbook similarity not measured; repeater census under threshold)",
        );
    }
    Ok(enqueued)
}

#[cfg(test)]
mod tests {
    use super::super::gdl::{CaptureArtifact, PlanStep};
    use super::*;
    use crate::migration::run_migration;
    use crate::register_sqlite_vec::register_sqlite_vec;
    use brain_engine_sdk::env::DenyAll;
    use rusqlite::Connection;
    use std::sync::Arc;

    fn base_env(process: bool, commands: Vec<String>) -> ExecutionEnv {
        ExecutionEnv {
            fs: Arc::new(DenyAll),
            read_only: false,
            allow_process: process,
            root: "/".into(),
            allowed_commands: commands,
        }
    }

    #[test]
    fn proficiency_env_is_monotonically_narrowing() {
        // THE pin: every capability of L1's env is possessed by L2's, and
        // L2's by L3's — read-only only relaxes, process only grants,
        // commands only widen. A tier can never hold a power a higher
        // tier lacks.
        let cmds = vec!["/usr/bin/racadm".to_string(), "/bin/echo".to_string()];
        let base = base_env(true, cmds.clone());
        let l1 = env_for(&base, Proficiency::L1);
        let l2 = env_for(&base, Proficiency::L2);
        let l3 = env_for(&base, Proficiency::L3);
        assert!(l1.read_only, "L1 observes");
        assert!(!l1.allow_process);
        assert!(l1.allowed_commands.is_empty());
        assert!(!l2.read_only, "L2 adds the write lane");
        assert!(!l2.allow_process, "L2 still cannot spawn processes");
        assert!(l2.allowed_commands.is_empty());
        assert!(!l3.read_only);
        assert!(l3.allow_process, "L3 adds the process lane");
        assert_eq!(l3.allowed_commands, cmds);
        // Monotone: capabilities only grow with rank.
        assert!(l1.read_only >= l2.read_only);
        assert!(!l1.allow_process || l2.allow_process);
        assert!(l1.allowed_commands.len() <= l2.allowed_commands.len());
        assert!(l2.allowed_commands.len() <= l3.allowed_commands.len());
        assert!(!l2.allow_process || l3.allow_process);
        // Subtraction, never addition: a read-only base stays read-only at
        // every tier; a process-less base never gains process.
        let ro = ExecutionEnv {
            read_only: true,
            ..base_env(false, vec![])
        };
        assert!(env_for(&ro, Proficiency::L3).read_only);
        assert!(!env_for(&ro, Proficiency::L3).allow_process);
    }

    #[test]
    fn phase_ownership_matrix_is_pinned() {
        assert_eq!(phase_owner(GdlPhase::Intake), Proficiency::L1);
        assert_eq!(phase_owner(GdlPhase::Triage), Proficiency::L1);
        assert_eq!(
            phase_owner(GdlPhase::Act),
            Proficiency::L1,
            "L1 executes plan steps it is gated for"
        );
        assert_eq!(
            phase_owner(GdlPhase::Verify),
            Proficiency::L2,
            "the 2nd verification is L2's"
        );
        assert_eq!(
            phase_owner(GdlPhase::Handoff),
            Proficiency::L2,
            "the capture decision is L2's"
        );
        assert_eq!(
            phase_owner(GdlPhase::Hypothesize),
            Proficiency::L3,
            "hypothesis is the engineer's"
        );
        assert_eq!(
            phase_owner(GdlPhase::Plan),
            Proficiency::L3,
            "the step plan is the engineer's"
        );
        // Skill gates: every tier sees every step; the gate decides who acts.
        assert!(may_execute_step(Proficiency::L1, "L0"));
        assert!(may_execute_step(Proficiency::L1, "L1"));
        assert!(!may_execute_step(Proficiency::L1, "L2"));
        assert!(may_execute_step(Proficiency::L2, "L2"));
        assert!(!may_execute_step(Proficiency::L2, "L3"));
        assert!(may_execute_step(Proficiency::L3, "L3"));
        assert!(
            !may_execute_step(Proficiency::L3, "L9"),
            "unknown gates deny"
        );
    }

    #[test]
    fn act_rows_above_the_tier_are_named_not_laundered() {
        let mut case = GdlCase::fresh("t");
        case.plan = vec![
            PlanStep {
                order: 1,
                kind: "check".into(),
                skill_gate: "L1".into(),
                description: "query".into(),
                command: String::new(),
                expected: "Ready".into(),
                fail_action: None,
                seam: None,
                invasiveness: 0,
                justification: None,
            },
            PlanStep {
                order: 2,
                kind: "action".into(),
                skill_gate: "L3".into(),
                description: "replace".into(),
                command: String::new(),
                expected: "Ready".into(),
                fail_action: None,
                seam: None,
                invasiveness: 2,
                justification: None,
            },
        ];
        let row = |order: i64| TestLogRow {
            order,
            ..TestLogRow::default()
        };
        // L1 executing the L3-gated step is a named violation.
        let errors = act_authority_errors(&case, &[row(1), row(2)], Proficiency::L1);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].starts_with("authority:") && errors[0].contains("L3"));
        // L3 executes both freely.
        assert!(act_authority_errors(&case, &[row(1), row(2)], Proficiency::L3).is_empty());
    }

    #[test]
    fn capture_proposes_and_never_publishes() {
        // No-auto-publish, behaviorally: the capture path's only writes
        // are PENDING proposal rows; the knowledge tables are
        // byte-unchanged by the enqueue.
        register_sqlite_vec();
        let mut conn = Connection::open_in_memory().unwrap();
        run_migration(&mut conn, 1).unwrap();
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('acme', 'troubleshoot', '{}', 0, 'resolved', 1, 1)",
            [],
        )
        .unwrap();
        let mut case = GdlCase::fresh("t");
        // A captured case with a low-similarity resolution (<40%: propose
        // new) and a repeater cluster (>=3/30d: RCA).
        case.capture = Some(CaptureArtifact {
            resolution: "symptom -> cause -> fix -> verify".into(),
            bundle_hash: "h1".into(),
        });
        let proposals = capture_proposals(&case, 3_000, 3);
        assert_eq!(proposals.len(), 2, "gap-new + rca");
        // Without a capture, nothing is proposed — no capture, no flywheel.
        let bare = GdlCase::fresh("t");
        assert!(capture_proposals(&bare, 3_000, 3).is_empty());

        // The enqueue: pending rows land, knowledge stays untouched.
        let knowledge_before: i64 = conn
            .query_row("SELECT COUNT(*) FROM knowledge", [], |r| r.get(0))
            .unwrap();
        let vec_before: i64 = conn
            .query_row("SELECT COUNT(*) FROM vec_knowledge", [], |r| r.get(0))
            .unwrap();
        let mut wtx = WorkflowTx::begin(&mut conn).unwrap();
        let n = enqueue_capture_proposals(&mut wtx, 1, &case, 3_000, 3, 42).unwrap();
        wtx.commit().unwrap();
        assert_eq!(n, 2);
        let pending: Vec<(String, String)> = {
            let mut stmt = conn
                .prepare("SELECT kind, status FROM proposals ORDER BY id")
                .unwrap();
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
            rows.collect::<Result<_, _>>().unwrap()
        };
        assert_eq!(
            pending,
            vec![
                ("gdl_gap_new".to_string(), "pending".to_string()),
                ("gdl_rca".to_string(), "pending".to_string()),
            ],
            "proposals queue as pending, awaiting a human"
        );
        let knowledge_after: i64 = conn
            .query_row("SELECT COUNT(*) FROM knowledge", [], |r| r.get(0))
            .unwrap();
        let vec_after: i64 = conn
            .query_row("SELECT COUNT(*) FROM vec_knowledge", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            knowledge_before, knowledge_after,
            "no knowledge row was written"
        );
        assert_eq!(vec_before, vec_after, "no embedding was written");
        assert!(
            crate::audit::verify_chain(&conn),
            "the enqueue's audit row keeps the chain green"
        );
    }

    #[test]
    fn similarity_not_measured_yields_no_gap() {
        // The sentinel's two faces at the scorer boundary: an UNMEASURED
        // similarity claims no gap fact, while a warranted repeater
        // cluster still proposes exactly the RCA.
        let mut case = GdlCase::fresh("t");
        case.capture = Some(CaptureArtifact {
            resolution: "symptom -> cause -> fix -> verify".into(),
            bundle_hash: "h1".into(),
        });
        assert!(
            capture_proposals(&case, 8_000, 0).is_empty(),
            "unmeasured similarity + no repeater: nothing"
        );
        assert_eq!(
            capture_proposals(&case, 8_000, 3),
            vec![FlywheelProposal::Rca],
            "unmeasured similarity + census 3: the RCA and no gap variant"
        );
    }

    #[test]
    fn repeater_census_counts_self_window_domain_kind_and_status() {
        register_sqlite_vec();
        let mut conn = Connection::open_in_memory().unwrap();
        run_migration(&mut conn, 1).unwrap();
        let now = 1_800_000_000_i64;
        let run = |domain: &str, kind: &str, status: &str, created_at: i64| {
            conn.execute(
                "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
                 VALUES (?1, ?2, '{}', 0, ?3, ?4, ?4)",
                rusqlite::params![domain, kind, status, created_at],
            )
            .unwrap();
        };
        // The closing case counts itself (resolved by census time).
        run("acme", "troubleshoot", "resolved", now);
        assert_eq!(repeater_census(&conn, "acme", now).unwrap(), 1);
        // Two in-window priors: 2 priors + self = 3 — the RCA threshold.
        run("acme", "troubleshoot", "resolved", now - 86_400);
        assert_eq!(repeater_census(&conn, "acme", now).unwrap(), 2);
        run("acme", "troubleshoot", "resolved", now - 7 * 86_400);
        assert_eq!(repeater_census(&conn, "acme", now).unwrap(), 3);
        // Window boundary: a prior at exactly the 30-day edge counts...
        run("acme", "troubleshoot", "resolved", now - 30 * 86_400);
        assert_eq!(repeater_census(&conn, "acme", now).unwrap(), 4);
        // ...and one second beyond it does not.
        run("acme", "troubleshoot", "resolved", now - 30 * 86_400 - 1);
        assert_eq!(repeater_census(&conn, "acme", now).unwrap(), 4);
        // A different domain's runs are another domain's business.
        run("other", "troubleshoot", "resolved", now - 86_400);
        assert_eq!(repeater_census(&conn, "acme", now).unwrap(), 4);
        // Complaint runs feed the kcs census, not this one.
        run("acme", "complaint", "resolved", now - 86_400);
        assert_eq!(repeater_census(&conn, "acme", now).unwrap(), 4);
        // A run not yet resolved is not a repeater data point.
        run("acme", "troubleshoot", "active", now - 86_400);
        assert_eq!(repeater_census(&conn, "acme", now).unwrap(), 4);
    }
}
