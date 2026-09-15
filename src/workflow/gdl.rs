//! The GDL case machine — the 7-phase governed troubleshooting loop.
//!
//! One case is one scientific experiment with a customer on the line
//! (TROUBLESHOOTING_METHODOLOGY.md, the line's source of record):
//! `Intake → Triage → Hypothesize → Plan → Act → Verify → Handoff`.
//! The PHASE MACHINE is deterministic Rust — the model proposes a phase
//! artifact as JSON, a pure arbiter ([`parse_and_gate`]) decides, and a
//! rejected artifact is retried bounded-then-routed, never waved through.
//! This is the active-reasoning posture the 2026 RCA literature converges
//! on: the loop drives evidence through a hypothesis structure instead of
//! labeling an incident post-hoc.
//!
//! Persistence per phase-pass is ONE [`WorkflowTx`]: the phase's
//! `workflow_steps` row (Act adds one sub-row per executed test-log row),
//! the CAS run-state advance (with its own audit row), and one audit row
//! per inserted step — all-or-nothing, hash-chained. The session narrative
//! (instructions, artifacts, gate verdicts) rides the loop substrate's
//! append-only `agent_session_events`; the plan strip is rendered into
//! every phase instruction at the CONTEXT END (dynamic content last so
//! the cached prefix survives — the kernel's cache law outranks the
//! plan's literal "system-prompt front-matter" wording, recorded in the
//! 1.32.1 plan).
//!
//! The 9 binding laws (methodology §3) are enforced where they are
//! mechanically checkable; every gate failure CITES ITS LAW in the error
//! string so a rejection is an auditable process fact, not a shrug:
//! L1 evidence before action · L2 one variable at a time · L3 known-good
//! comparison · L4 what-changed first · L5 least-invasive ladder · L6
//! verify under failing conditions · L7 no premature closure · L8
//! escalation = evidence handoff · L9 no fix from memory.
//!
//! What this deliberately does NOT do: no subagent fan-out or adversarial
//! verification (1.32.2), no follow-the-sun handoff policy (1.32.3), no
//! provider code (the loopback fixture carries the tests), no live
//! routing claims, and no auto-publish of anything the loop captures —
//! capture lands as proposals on the human review queue or not at all.

use std::sync::Arc;

use brain_engine_sdk::env::{ExecutionEnv, ToolDef};
use brain_engine_sdk::harness::AgentHarness;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::Pool;
use crate::agentloop::provider::LlmProvider;
use crate::agentloop::run_loop::{LoopConfig, LoopDriver, LoopError, RunOutcome};
use crate::workflow::host::SqliteWorkflowHost;
use crate::workflow::session_log;

/// Bounded corrective retries per phase: one original ask plus this many
/// gate-error re-asks. Exhausting them ROUTES the case (route, not
/// resolve — a case the model cannot artifact honestly is a human's case).
pub(crate) const MAX_PHASE_ATTEMPTS: u32 = 3;

/// The plan strip is bounded (bounds law + cache discipline): a strip that
/// grows without bound is context the model pays for on every phase.
pub(crate) const PLAN_STRIP_MAX_LINES: usize = 24;

/// The verify stability window floor (methodology A6: a 15-minute
/// stability window is part of verification, not optional polish).
pub(crate) const VERIFY_STABILITY_WINDOW_MIN: u32 = 15;

/// The cache-stable GDL method prompt (≤ 20 lines, prompt discipline).
pub(crate) const GDL_METHOD_PROMPT: &str = "\
You are a governed troubleshooting agent running the 7-phase method:\n\
Intake, Triage, Hypothesize, Plan, Act, Verify, Handoff.\n\
Laws you cannot break: evidence before action (L1); one variable at a\n\
time (L2); known-good comparison (L3); what-changed first (L4);\n\
least-invasive reversible fixes (L5); verify under the failing\n\
conditions (L6); no premature closure (L7); escalation is an evidence\n\
handoff (L8); no fix from memory — every action cites its playbook\n\
step (L9).\n\
Each phase hands you a JSON contract. Answer with ONE JSON object and\n\
nothing else. A rejected artifact comes back with the laws it broke:\n\
fix the artifact, do not argue with the gate.\n";

// ── the phase vocabulary ───────────────────────────────────────────────────

/// The 7 phases in order. The order is LAW: forward transitions are the
/// only way through, and the machine never skips (a case that cannot
/// satisfy a phase routes or escalates instead).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) enum GdlPhase {
    Intake,
    Triage,
    Hypothesize,
    Plan,
    Act,
    Verify,
    Handoff,
}

impl GdlPhase {
    pub(crate) const ALL: [GdlPhase; 7] = [
        GdlPhase::Intake,
        GdlPhase::Triage,
        GdlPhase::Hypothesize,
        GdlPhase::Plan,
        GdlPhase::Act,
        GdlPhase::Verify,
        GdlPhase::Handoff,
    ];

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            GdlPhase::Intake => "intake",
            GdlPhase::Triage => "triage",
            GdlPhase::Hypothesize => "hypothesize",
            GdlPhase::Plan => "plan",
            GdlPhase::Act => "act",
            GdlPhase::Verify => "verify",
            GdlPhase::Handoff => "handoff",
        }
    }
}

// ── phase artifacts (the model's proposals; the arbiter's inputs) ──────────

/// One KT IS/IS-NOT row: `(is, is_not)`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct IsNotRow {
    #[serde(default)]
    pub is: String,
    #[serde(default)]
    pub is_not: String,
}

/// The KT problem statement (methodology §2 A1): WHAT/WHERE/WHEN/EXTENT,
/// each with an IS and an IS-NOT column. "Not why" — the table bounds the
/// problem; explanation belongs to Hypothesize.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct IsNotTable {
    #[serde(default)]
    pub what: IsNotRow,
    #[serde(default, rename = "where")]
    pub place: IsNotRow,
    #[serde(default, rename = "when")]
    pub time: IsNotRow,
    #[serde(default)]
    pub extent: IsNotRow,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct IntakeArtifact {
    #[serde(default)]
    pub is_not: IsNotTable,
    /// Telemetry captured BEFORE anything is touched (L1): ticket verbatim,
    /// TSR/SEL exports, first-symptom logs. Empty = the case routes.
    #[serde(default)]
    pub telemetry_refs: Vec<String>,
    /// The what-changed window (L4): 7 days of firmware/BIOS/driver/config/
    /// network changes. "unknown" is a VALID recorded value; absent is not.
    #[serde(default)]
    pub what_changed: String,
    /// The known-good comparison (L3): sibling node, same-model baseline,
    /// yesterday's config. "none available" is valid recorded; absent is not.
    #[serde(default)]
    pub known_good: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct TriageArtifact {
    /// P1–P4 per the routing matrix (A0 stabilize-first classification).
    #[serde(default)]
    pub priority: String,
    /// Containment/restore performed BEFORE diagnosis (A0) — logged as such.
    #[serde(default)]
    pub stabilized: bool,
    /// Knowledge/playbook candidates surfaced by search-first (A2/KCS step 1).
    #[serde(default)]
    pub search_hits: Vec<String>,
    /// accept | defer | unknown — defer/unknown escalates with the bundle.
    #[serde(default)]
    pub verdict: String,
}

/// One hypothesis: a statement with a falsifiable prediction. Confirmation
/// is NOT the model's to claim — the arbiter confirms only at ≥2 distinct
/// evidence sources (A4 triangulation, L7).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Hypothesis {
    #[serde(default)]
    pub statement: String,
    #[serde(default)]
    pub prediction: String,
    /// Independent evidence sources currently supporting the statement.
    #[serde(default)]
    pub sources: Vec<String>,
    /// The model's stated confidence, if any — recorded as typed
    /// `confidence` evidence (never trusted as confirmation: triangulation
    /// is arbiter-computed from distinct sources).
    #[serde(default)]
    pub confidence: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct HypothesizeArtifact {
    #[serde(default)]
    pub hypotheses: Vec<Hypothesis>,
}

/// One planned step (methodology §10's envelope, per-step shape).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct PlanStep {
    #[serde(default)]
    pub order: i64,
    /// check | gate | action.
    #[serde(default)]
    pub kind: String,
    /// L0..L3 — who may ACT on this step (every tier SEES every step).
    #[serde(default)]
    pub skill_gate: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub command: String,
    /// The falsifiable expectation — a step without an expected outcome is
    /// a test that cannot fail, and a test that cannot fail proves nothing.
    #[serde(default)]
    pub expected: String,
    /// Next order on fail (the fail_action); absent = dead-end at this step.
    #[serde(default)]
    pub fail_action: Option<i64>,
    /// 0=observe · 1=reversible config · 2=replace/reseat · 3=destructive
    /// (L5: plan action steps must ascend this ladder, ties allowed).
    #[serde(default)]
    pub invasiveness: u8,
    #[serde(default)]
    pub justification: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct VerifyStepSpec {
    /// The EXACT failing scenario to re-run (L6) — the customer's operation,
    /// not "looks fine".
    #[serde(default)]
    pub re_run: String,
    #[serde(default)]
    pub pass_condition: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct DeadEnd {
    #[serde(default)]
    pub escalate_to: String,
    #[serde(default)]
    pub required_evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct PlanArtifact {
    #[serde(default)]
    pub steps: Vec<PlanStep>,
    #[serde(default)]
    pub verify_step: VerifyStepSpec,
    #[serde(default)]
    pub dead_end: DeadEnd,
}

/// The DTFVC discipline on one executed step: Diagnose→Test→Fix→Verify→
/// Capture (methodology §2 A3/A5–A7). Diagnose+Test are the floor; a
/// check row stops there; an action row carries fix/verify forward.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Dtfvc {
    #[serde(default)]
    pub diagnose: String,
    #[serde(default)]
    pub test: String,
    #[serde(default)]
    pub fix: Option<String>,
    #[serde(default)]
    pub verify: Option<String>,
    #[serde(default)]
    pub capture: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Verdict {
    #[default]
    Pending,
    Pass,
    Fail,
    Done,
    Rejected,
    /// An off-playbook step taken under recorded justification (L9): logged
    /// as experimental, rejected for gold, and reported as a taxonomy gap.
    Experimental,
}

/// One executed test-log row (the case's audit trail, methodology §4).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct TestLogRow {
    #[serde(default)]
    pub order: i64,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub description: String,
    /// The playbook step this row cites (L9). None + no justification = gate
    /// fail; None + justification = experimental (flagged, not silent).
    #[serde(default)]
    pub playbook_ref: Option<String>,
    /// Exactly ONE variable per row (L2) — two changes in one row invalidate
    /// the test.
    #[serde(default)]
    pub variables: Vec<String>,
    #[serde(default)]
    pub expected: String,
    #[serde(default)]
    pub actual: Option<String>,
    #[serde(default)]
    pub verdict: Verdict,
    #[serde(default)]
    pub evidence_ref: Option<String>,
    #[serde(default)]
    pub dtfvc: Dtfvc,
    #[serde(default)]
    pub invasiveness: u8,
    #[serde(default)]
    pub justification: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ActArtifact {
    #[serde(default)]
    pub rows: Vec<TestLogRow>,
    /// True when the plan's steps are all executed and Act is finished.
    #[serde(default)]
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct VerifyArtifact {
    #[serde(default)]
    pub re_run: String,
    #[serde(default)]
    pub pass: bool,
    #[serde(default)]
    pub stability_window_min: u32,
    /// No collateral in SEL/logs post-fix (A6's negative check).
    #[serde(default)]
    pub negative_check: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct CaptureArtifact {
    /// Symptom → confirmed root cause → fix → verify (A7, KCS in-stream).
    #[serde(default)]
    pub resolution: String,
    #[serde(default)]
    pub bundle_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct HandoffArtifact {
    #[serde(default)]
    pub capture: CaptureArtifact,
}

// ── the case state (workflow_runs.state_json for kind='troubleshoot') ──────

/// The live case. Serialized whole into the run row on every phase-pass
/// CAS; the audit chain + session log reconstruct how it got here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct GdlCase {
    pub phase: GdlPhase,
    /// The ticket verbatim (A1: the original words are evidence).
    pub ticket: String,
    #[serde(default)]
    pub intake: Option<IntakeArtifact>,
    #[serde(default)]
    pub triage: Option<TriageArtifact>,
    #[serde(default)]
    pub hypotheses: Vec<Hypothesis>,
    #[serde(default)]
    pub plan: Vec<PlanStep>,
    #[serde(default)]
    pub verify_step: Option<VerifyStepSpec>,
    #[serde(default)]
    pub dead_end: Option<DeadEnd>,
    #[serde(default)]
    pub test_log: Vec<TestLogRow>,
    #[serde(default)]
    pub verify: Option<VerifyArtifact>,
    #[serde(default)]
    pub capture: Option<CaptureArtifact>,
}

impl GdlCase {
    pub(crate) fn fresh(ticket: &str) -> GdlCase {
        GdlCase {
            phase: GdlPhase::Intake,
            ticket: ticket.to_string(),
            intake: None,
            triage: None,
            hypotheses: Vec::new(),
            plan: Vec::new(),
            verify_step: None,
            dead_end: None,
            test_log: Vec::new(),
            verify: None,
            capture: None,
        }
    }

    /// The escalation bundle (L8): IS/NOT + telemetry + differential +
    /// test-log census — Rust-assembled from case state, never a
    /// model-summary sentence. `complete` is the L8 MINIMUM the receiving
    /// tier can work from (a bounded problem statement plus captured
    /// telemetry); `test_log_rows` rides for the receiver to judge the
    /// "empty log at escalation" send-back condition, which is
    /// phase-relative (a triage defer has no log yet by construction).
    pub(crate) fn escalation_bundle(&self) -> EscalationBundle {
        EscalationBundle {
            is_not: self.intake.as_ref().map(|i| i.is_not.clone()),
            telemetry_refs: self
                .intake
                .as_ref()
                .map(|i| i.telemetry_refs.clone())
                .unwrap_or_default(),
            hypotheses: self.hypotheses.clone(),
            test_log_rows: self.test_log.len(),
            complete: self.intake.is_some(),
        }
    }
}

/// The Rust-assembled escalation packet (L8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct EscalationBundle {
    pub is_not: Option<IsNotTable>,
    pub telemetry_refs: Vec<String>,
    pub hypotheses: Vec<Hypothesis>,
    pub test_log_rows: usize,
    pub complete: bool,
}

// ── the pure arbiter ───────────────────────────────────────────────────────

/// What the arbiter decided about a proposed artifact.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Gate {
    /// The artifact passes; apply it and advance.
    Pass,
    /// The artifact violates named laws; re-ask bounded, then route.
    Fail(Vec<String>),
    /// The case must NOT proceed through this phase — route or escalate.
    Route(String),
}

fn err(law: &str, detail: &str) -> String {
    format!("{law}: {detail}")
}

/// KT intake completeness (the plan's gate-every-case item): all four
/// IS/IS-NOT rows present with a non-empty IS column; telemetry captured
/// (L1); what-changed recorded, "unknown" allowed (L4); known-good
/// recorded, "none available" allowed (L3).
fn intake_gate(a: &IntakeArtifact) -> Vec<String> {
    let mut errors = Vec::new();
    for (name, row) in [
        ("what", &a.is_not.what),
        ("where", &a.is_not.place),
        ("when", &a.is_not.time),
        ("extent", &a.is_not.extent),
    ] {
        if row.is.trim().is_empty() {
            errors.push(err(
                "KT",
                &format!("is_not.{name}.is is empty — an unbounded problem statement"),
            ));
        }
    }
    if a.telemetry_refs.is_empty() {
        errors.push(err(
            "L1",
            "no telemetry captured — evidence before action is law, a reboot \
             without a TSR is a destroyed diagnosis",
        ));
    }
    if a.what_changed.trim().is_empty() {
        errors.push(err(
            "L4",
            "what_changed not recorded — 'unknown' is a valid recorded value, \
             absence is not",
        ));
    }
    if a.known_good.trim().is_empty() {
        errors.push(err(
            "L3",
            "known_good not recorded — 'none available' is a valid recorded \
             value, absence is not",
        ));
    }
    errors
}

/// Triage: P1–P4 classified, search-first honored (A2/KCS step 1 — accept
/// requires ≥1 knowledge candidate; the differential IS the checklist, and
/// a checklist without the answer escalates rather than guesses).
fn triage_gate(a: &TriageArtifact) -> Vec<String> {
    let mut errors = Vec::new();
    if !matches!(a.priority.as_str(), "P1" | "P2" | "P3" | "P4") {
        errors.push(err(
            "A0",
            &format!(
                "priority '{}' not in P1..P4 — classify before anything",
                a.priority
            ),
        ));
    }
    if a.verdict == "accept" && a.search_hits.is_empty() {
        errors.push(err(
            "L9",
            "accept with zero search hits — search-first is KCS step 1; a case \
             the knowledge layer does not cover escalates with the bundle",
        ));
    }
    if !matches!(a.verdict.as_str(), "accept" | "defer" | "unknown") {
        errors.push(err(
            "A2",
            &format!("verdict '{}' not in accept|defer|unknown", a.verdict),
        ));
    }
    errors
}

/// Hypotheses: at least one live statement, each with a falsifiable
/// prediction (a test that cannot fail proves nothing — the scientific
/// core). Confirmation is arbiter-computed downstream, never claimed here
/// (L7: one line = hypothesis, not root cause).
fn hypothesize_gate(a: &HypothesizeArtifact) -> Vec<String> {
    let mut errors = Vec::new();
    if a.hypotheses.is_empty() {
        errors.push(err(
            "A3",
            "no hypotheses — the differential is the checklist; escalate \
             rather than proceed empty",
        ));
    }
    for (i, h) in a.hypotheses.iter().enumerate() {
        if h.statement.trim().is_empty() {
            errors.push(err("A3", &format!("hypothesis[{i}].statement empty")));
        }
        if h.prediction.trim().is_empty() {
            errors.push(err(
                "A3",
                &format!(
                    "hypothesis[{i}].prediction empty — a hypothesis \
                          without a falsifiable prediction cannot be tested"
                ),
            ));
        }
    }
    errors
}

/// The plan (methodology §10): contiguous ordered steps, each with a kind,
/// a skill gate, and a non-empty expected outcome; action steps ascend the
/// invasiveness ladder (L5); the verify step names the exact failing
/// scenario (L6); the dead-end escalation is defined, never improvised.
fn plan_gate(a: &PlanArtifact) -> Vec<String> {
    let mut errors = Vec::new();
    if a.steps.is_empty() {
        errors.push(err(
            "§10",
            "empty step plan — the spine emits the plan, the agent never invents a sequence",
        ));
    }
    for (i, s) in a.steps.iter().enumerate() {
        if s.order != (i as i64) + 1 {
            errors.push(err(
                "§10",
                &format!(
                    "step[{i}].order {} breaks contiguous 1..=n ordering",
                    s.order
                ),
            ));
        }
        if !matches!(s.kind.as_str(), "check" | "gate" | "action") {
            errors.push(err(
                "§10",
                &format!("step[{i}].kind '{}' not in check|gate|action", s.kind),
            ));
        }
        if !matches!(s.skill_gate.as_str(), "L0" | "L1" | "L2" | "L3") {
            errors.push(err(
                "§10",
                &format!("step[{i}].skill_gate '{}' not in L0..L3", s.skill_gate),
            ));
        }
        if s.expected.trim().is_empty() {
            errors.push(err(
                "A3",
                &format!("step[{i}].expected empty — a test that cannot fail proves nothing"),
            ));
        }
        if s.kind == "action" && s.invasiveness > 3 {
            errors.push(err(
                "L5",
                &format!("step[{i}].invasiveness {} not in 0..=3", s.invasiveness),
            ));
        }
    }
    // L5: action steps ascend the invasiveness ladder unless justified.
    let mut last_rank: Option<u8> = None;
    for (i, s) in a.steps.iter().enumerate() {
        if s.kind != "action" {
            continue;
        }
        if let Some(prev) = last_rank
            && s.invasiveness < prev
            && s.justification.as_deref().is_none_or(str::is_empty)
        {
            errors.push(err(
                "L5",
                &format!(
                    "step[{i}] invasiveness {} drops below the pending {} — \
                     the least-invasive ladder ascends; a cheaper step after a \
                     more invasive one needs a recorded justification",
                    s.invasiveness, prev
                ),
            ));
        }
        last_rank = Some(s.invasiveness);
    }
    if a.verify_step.re_run.trim().is_empty() {
        errors.push(err(
            "L6",
            "verify_step.re_run empty — verification re-runs the EXACT failing \
             scenario, 'looks fine' is not evidence",
        ));
    }
    if a.dead_end.escalate_to.trim().is_empty() {
        errors.push(err(
            "L8",
            "dead_end.escalate_to empty — the dead-end escalation is defined, \
             never improvised",
        ));
    }
    errors
}

/// Act: each executed row follows the plan's order, tests ONE variable
/// (L2), cites its playbook step or is explicitly experimental (L9), and
/// the case had telemetry before the first action (L1 — enforced at
/// intake; re-checked here so a hand-seeded case cannot skip it).
fn act_gate(a: &ActArtifact, case: &GdlCase) -> Vec<String> {
    let mut errors = Vec::new();
    if case
        .intake
        .as_ref()
        .is_none_or(|i| i.telemetry_refs.is_empty())
    {
        errors.push(err(
            "L1",
            "the case carries no telemetry — action without captured evidence \
             is law-broken at intake, not retryable here",
        ));
    }
    for (i, r) in a.rows.iter().enumerate() {
        if r.order != (case.test_log.len() + i + 1) as i64 {
            errors.push(err(
                "§4",
                &format!(
                    "row[{i}].order {} not contiguous after the existing log \
                     (expected {})",
                    r.order,
                    case.test_log.len() + i + 1
                ),
            ));
        }
        if r.variables.len() != 1 {
            errors.push(err(
                "L2",
                &format!(
                    "row[{i}] changes {} variables — one variable at a time; \
                     multi-variable changes prove nothing",
                    r.variables.len()
                ),
            ));
        }
        if r.expected.trim().is_empty() {
            errors.push(err(
                "A3",
                &format!("row[{i}].expected empty — predicted vs actual is the test"),
            ));
        }
        let unjustified = r.justification.as_deref().is_none_or(str::is_empty);
        match (&r.playbook_ref, unjustified) {
            (None, true) => errors.push(err(
                "L9",
                &format!(
                    "row[{i}] cites no playbook step and carries no \
                     justification — no fix from memory"
                ),
            )),
            (None, false) if r.verdict != Verdict::Experimental => {
                errors.push(err(
                    "L9",
                    &format!(
                        "row[{i}] is off-playbook with justification but \
                             verdict is {:?} — an experimental step is flagged \
                             experimental, never laundered",
                        r.verdict
                    ),
                ));
            }
            _ => {}
        }
        if r.dtfvc.diagnose.trim().is_empty() || r.dtfvc.test.trim().is_empty() {
            errors.push(err(
                "DTFVC",
                &format!(
                    "row[{i}] missing its diagnose/test floor — every step is \
                     Diagnose→Test before Fix"
                ),
            ));
        }
    }
    errors
}

/// Verify (A6/L6): the re-run IS the plan's failing scenario, the
/// stability window is at least the floor, the negative check ran, and
/// only a PASS closes a case.
fn verify_gate(a: &VerifyArtifact, case: &GdlCase) -> Vec<String> {
    let mut errors = Vec::new();
    let spec = case.verify_step.as_ref().expect(
        "verify_step is set by the Plan phase pass; the machine cannot reach Verify without it",
    );
    if a.re_run.trim() != spec.re_run.trim() {
        errors.push(err(
            "L6",
            &format!(
                "re_run {:?} is not the planned failing scenario {:?} — \
                 verify under the conditions that failed",
                a.re_run, spec.re_run
            ),
        ));
    }
    if a.stability_window_min < VERIFY_STABILITY_WINDOW_MIN {
        errors.push(err(
            "A6",
            &format!(
                "stability_window {}min under the {}min floor",
                a.stability_window_min, VERIFY_STABILITY_WINDOW_MIN
            ),
        ));
    }
    if !a.negative_check {
        errors.push(err(
            "A6",
            "negative_check false — no collateral in SEL/logs post-fix is \
             part of verification",
        ));
    }
    errors
}

/// Handoff: the capture artifact is present (A7 — a resolution nobody can
/// retrieve later is a second failure).
fn handoff_gate(a: &HandoffArtifact) -> Vec<String> {
    let mut errors = Vec::new();
    if a.capture.resolution.trim().is_empty() {
        errors.push(err(
            "A7",
            "capture.resolution empty — solve AND capture in the stream is \
             the KCS discipline",
        ));
    }
    errors
}

/// The arbiter: parse the model's proposal for `phase` against `case` and
/// decide. PURE — no DB, no clock, no provider; every input is an argument
/// so the whole law book is unit-testable without a runtime.
pub(crate) fn parse_and_gate(
    phase: GdlPhase,
    case: &GdlCase,
    text: &str,
) -> (Gate, Option<String>) {
    let parsed: serde_json::Value = match serde_json::from_str(text.trim()) {
        Ok(v) => v,
        Err(e) => {
            return (
                Gate::Fail(vec![format!(
                    "JSON: artifact is not a JSON object ({e}) — emit ONE \
                     JSON object and nothing else"
                )]),
                None,
            );
        }
    };
    match phase {
        GdlPhase::Intake => match serde_json::from_value::<IntakeArtifact>(parsed.clone()) {
            Ok(a) => {
                let errors = intake_gate(&a);
                let gate = if errors.is_empty() {
                    Gate::Pass
                } else {
                    Gate::Fail(errors)
                };
                (gate, serde_json::to_string(&a).ok())
            }
            Err(e) => (
                Gate::Fail(vec![format!("JSON: intake artifact shape: {e}")]),
                None,
            ),
        },
        GdlPhase::Triage => match serde_json::from_value::<TriageArtifact>(parsed.clone()) {
            Ok(a) => {
                if matches!(a.verdict.as_str(), "defer" | "unknown") {
                    return (
                        Gate::Route(format!(
                            "triage verdict '{}' — escalate with the bundle",
                            a.verdict
                        )),
                        serde_json::to_string(&a).ok(),
                    );
                }
                let errors = triage_gate(&a);
                let gate = if errors.is_empty() {
                    Gate::Pass
                } else {
                    Gate::Fail(errors)
                };
                (gate, serde_json::to_string(&a).ok())
            }
            Err(e) => (
                Gate::Fail(vec![format!("JSON: triage artifact shape: {e}")]),
                None,
            ),
        },
        GdlPhase::Hypothesize => {
            match serde_json::from_value::<HypothesizeArtifact>(parsed.clone()) {
                Ok(a) => {
                    let errors = hypothesize_gate(&a);
                    let gate = if errors.is_empty() {
                        Gate::Pass
                    } else {
                        Gate::Fail(errors)
                    };
                    (gate, serde_json::to_string(&a).ok())
                }
                Err(e) => (
                    Gate::Fail(vec![format!("JSON: hypothesize artifact shape: {e}")]),
                    None,
                ),
            }
        }
        GdlPhase::Plan => match serde_json::from_value::<PlanArtifact>(parsed.clone()) {
            Ok(a) => {
                let errors = plan_gate(&a);
                let gate = if errors.is_empty() {
                    Gate::Pass
                } else {
                    Gate::Fail(errors)
                };
                (gate, serde_json::to_string(&a).ok())
            }
            Err(e) => (
                Gate::Fail(vec![format!("JSON: plan artifact shape: {e}")]),
                None,
            ),
        },
        GdlPhase::Act => match serde_json::from_value::<ActArtifact>(parsed.clone()) {
            Ok(a) => {
                let errors = act_gate(&a, case);
                let gate = if errors.is_empty() {
                    Gate::Pass
                } else {
                    Gate::Fail(errors)
                };
                (gate, serde_json::to_string(&a).ok())
            }
            Err(e) => (
                Gate::Fail(vec![format!("JSON: act artifact shape: {e}")]),
                None,
            ),
        },
        GdlPhase::Verify => match serde_json::from_value::<VerifyArtifact>(parsed.clone()) {
            Ok(a) => {
                let errors = verify_gate(&a, case);
                let gate = if errors.is_empty() {
                    Gate::Pass
                } else {
                    Gate::Fail(errors)
                };
                (gate, serde_json::to_string(&a).ok())
            }
            Err(e) => (
                Gate::Fail(vec![format!("JSON: verify artifact shape: {e}")]),
                None,
            ),
        },
        GdlPhase::Handoff => match serde_json::from_value::<HandoffArtifact>(parsed.clone()) {
            Ok(a) => {
                let errors = handoff_gate(&a);
                let gate = if errors.is_empty() {
                    Gate::Pass
                } else {
                    Gate::Fail(errors)
                };
                (gate, serde_json::to_string(&a).ok())
            }
            Err(e) => (
                Gate::Fail(vec![format!("JSON: handoff artifact shape: {e}")]),
                None,
            ),
        },
    }
}

/// Apply a passed artifact to the case (pure; the driver persists).
pub(crate) fn apply(case: &mut GdlCase, phase: GdlPhase, artifact_json: &str) {
    match phase {
        GdlPhase::Intake => {
            if let Ok(a) = serde_json::from_str::<IntakeArtifact>(artifact_json) {
                case.intake = Some(a);
            }
        }
        GdlPhase::Triage => {
            if let Ok(a) = serde_json::from_str::<TriageArtifact>(artifact_json) {
                case.triage = Some(a);
            }
        }
        GdlPhase::Hypothesize => {
            if let Ok(a) = serde_json::from_str::<HypothesizeArtifact>(artifact_json) {
                case.hypotheses = a.hypotheses;
            }
        }
        GdlPhase::Plan => {
            if let Ok(a) = serde_json::from_str::<PlanArtifact>(artifact_json) {
                case.plan = a.steps;
                case.verify_step = Some(a.verify_step);
                case.dead_end = Some(a.dead_end);
            }
        }
        GdlPhase::Act => {
            if let Ok(a) = serde_json::from_str::<ActArtifact>(artifact_json) {
                case.test_log.extend(a.rows);
            }
        }
        GdlPhase::Verify => {
            if let Ok(a) = serde_json::from_str::<VerifyArtifact>(artifact_json) {
                case.verify = Some(a);
            }
        }
        GdlPhase::Handoff => {
            if let Ok(a) = serde_json::from_str::<HandoffArtifact>(artifact_json) {
                case.capture = Some(a.capture);
            }
        }
    }
}

/// A hypothesis's arbiter-computed status: ≥2 distinct evidence sources
/// confirm it (triangulation); anything less stays a hypothesis (L7 —
/// one line is a hypothesis, not a root cause).
pub(crate) fn hypothesis_status(h: &Hypothesis) -> HypothesisStatus {
    let mut distinct = h.sources.clone();
    distinct.sort();
    distinct.dedup();
    if distinct.len() >= 2 {
        HypothesisStatus::Confirmed
    } else {
        HypothesisStatus::Hypothesis
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HypothesisStatus {
    Hypothesis,
    Confirmed,
}

// ── the instruction assembler (deterministic; the strip rides the END) ─────

/// Render the live plan strip (bounded): the current phase, remaining plan
/// steps, and live hypotheses — the thing L1 SEES instead of hidden state.
pub(crate) fn plan_strip(case: &GdlCase) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!(
        "GDL PLAN STRIP (live) — phase: {}",
        case.phase.as_str()
    ));
    let executed: std::collections::HashSet<i64> = case.test_log.iter().map(|r| r.order).collect();
    for s in &case.plan {
        let mark = if executed.contains(&s.order) {
            "done"
        } else {
            "pending"
        };
        lines.push(format!(
            "[{}] {} ({}, skill {}) — {}",
            s.order, mark, s.kind, s.skill_gate, s.description
        ));
    }
    for h in &case.hypotheses {
        lines.push(format!(
            "H: {} [{}] — predicts: {}",
            h.statement,
            match hypothesis_status(h) {
                HypothesisStatus::Confirmed => "confirmed",
                HypothesisStatus::Hypothesis => "hypothesis",
            },
            h.prediction
        ));
    }
    if lines.len() > PLAN_STRIP_MAX_LINES {
        lines.truncate(PLAN_STRIP_MAX_LINES);
        lines.push(format!(
            "… strip truncated at {PLAN_STRIP_MAX_LINES} lines (bounds law)"
        ));
    }
    lines.join("\n")
}

/// The phase instruction: strip first, then the phase task and its JSON
/// contract, then (on retry) the named law violations to fix.
pub(crate) fn phase_instruction(
    phase: GdlPhase,
    case: &GdlCase,
    retry_errors: &[String],
) -> String {
    let contract = match phase {
        GdlPhase::Intake => {
            "Emit ONE JSON object: \
{\"is_not\":{\"what\":{\"is\":\"..\",\"is_not\":\"..\"},\"where\":{..},\"when\":{..},\"extent\":{..}},\
\"telemetry_refs\":[\"tsr://..\"],\"what_changed\":\".. or unknown\",\"known_good\":\".. or none available\"}"
        }
        GdlPhase::Triage => {
            "Emit ONE JSON object: \
{\"priority\":\"P1..P4\",\"stabilized\":bool,\"search_hits\":[\"playbook ids\"],\"verdict\":\"accept|defer|unknown\"}"
        }
        GdlPhase::Hypothesize => {
            "Emit ONE JSON object: \
{\"hypotheses\":[{\"statement\":\"..\",\"prediction\":\"falsifiable expectation\",\"sources\":[\"evidence refs\"]}]}"
        }
        GdlPhase::Plan => {
            "Emit ONE JSON object: \
{\"steps\":[{\"order\":1,\"kind\":\"check|gate|action\",\"skill_gate\":\"L0..L3\",\"description\":\"..\",\
\"command\":\"..\",\"expected\":\"..\",\"fail_action\":2,\"invasiveness\":0,\"justification\":null}],\
\"verify_step\":{\"re_run\":\"the exact failing scenario\",\"pass_condition\":\"..\"},\
\"dead_end\":{\"escalate_to\":\"..\",\"required_evidence\":[\"..\"]}}"
        }
        GdlPhase::Act => {
            "Emit ONE JSON object: \
{\"rows\":[{\"order\":1,\"kind\":\"check\",\"description\":\"..\",\"playbook_ref\":\"P-.. or null\",\
\"variables\":[\"the one variable\"],\"expected\":\"..\",\"actual\":\"..\",\"verdict\":\"pass|fail|rejected|done|experimental\",\
\"evidence_ref\":\"..\",\"dtfvc\":{\"diagnose\":\"..\",\"test\":\"..\",\"fix\":null,\"verify\":null,\"capture\":null},\
\"invasiveness\":0,\"justification\":null}],\"complete\":bool}"
        }
        GdlPhase::Verify => {
            "Emit ONE JSON object: \
{\"re_run\":\"the exact failing scenario from the plan\",\"pass\":bool,\"stability_window_min\":15,\"negative_check\":bool}"
        }
        GdlPhase::Handoff => {
            "Emit ONE JSON object: \
{\"capture\":{\"resolution\":\"symptom -> confirmed root cause -> fix -> verify\",\"bundle_hash\":\"..\"}}"
        }
    };
    let mut parts = vec![
        plan_strip(case),
        format!("PHASE {}: produce this phase's artifact.", phase.as_str()),
        contract.to_string(),
    ];
    if !retry_errors.is_empty() {
        parts.push(format!(
            "Your previous artifact was REJECTED for: {}. Fix these — do not repeat them.",
            retry_errors.join("; ")
        ));
    }
    parts.join("\n\n")
}

/// The typed evidence a phase pass emits (the 8-type vocabulary's emit
/// points): Hypothesize → hypothesis (+confidence when stated); Act →
/// test/expected/actual per row; Verify → verification; Handoff → capture.
/// Intake/Triage/Plan emit NOTHING typed — their outputs are case state
/// (a bounded problem statement, a routing verdict, a step plan), not
/// evidence claims, and pretending otherwise would inflate the
/// evidence-per-phase signal the eval reports.
pub(crate) fn typed_evidence_for(
    phase: GdlPhase,
    artifact_json: &str,
    now: i64,
) -> Vec<super::evidence::TypedEvidence> {
    use super::evidence::{EvidenceKind, TypedEvidence};
    let line = |kind, claim: &str, evidence: String, source: &str, confidence: f64| TypedEvidence {
        kind,
        claim: claim.to_string(),
        evidence,
        source: source.to_string(),
        confidence,
        ts: now,
    };
    match phase {
        GdlPhase::Hypothesize => {
            let Ok(a) = serde_json::from_str::<HypothesizeArtifact>(artifact_json) else {
                return Vec::new();
            };
            let mut out = Vec::new();
            for h in &a.hypotheses {
                out.push(line(
                    EvidenceKind::Hypothesis,
                    &h.statement,
                    h.prediction.clone(),
                    "gdl",
                    0.5,
                ));
                if let Some(c) = h.confidence {
                    out.push(line(
                        EvidenceKind::Confidence,
                        &h.statement,
                        format!("{c}"),
                        "model",
                        c,
                    ));
                }
            }
            out
        }
        GdlPhase::Act => {
            let Ok(a) = serde_json::from_str::<ActArtifact>(artifact_json) else {
                return Vec::new();
            };
            let mut out = Vec::new();
            for r in &a.rows {
                let source = r
                    .playbook_ref
                    .clone()
                    .unwrap_or_else(|| "experimental".into());
                out.push(line(
                    EvidenceKind::Test,
                    &r.description,
                    r.dtfvc.test.clone(),
                    &source,
                    0.5,
                ));
                out.push(line(
                    EvidenceKind::Expected,
                    &r.description,
                    r.expected.clone(),
                    &source,
                    0.5,
                ));
                if let Some(actual) = &r.actual {
                    out.push(line(
                        EvidenceKind::Actual,
                        &r.description,
                        actual.clone(),
                        &source,
                        0.9,
                    ));
                }
            }
            out
        }
        GdlPhase::Verify => {
            let Ok(a) = serde_json::from_str::<VerifyArtifact>(artifact_json) else {
                return Vec::new();
            };
            vec![line(
                EvidenceKind::Verification,
                &a.re_run,
                format!(
                    "pass={} window={}min negative={}",
                    a.pass, a.stability_window_min, a.negative_check
                ),
                "gdl",
                if a.pass { 0.9 } else { 0.4 },
            )]
        }
        GdlPhase::Handoff => {
            let Ok(a) = serde_json::from_str::<HandoffArtifact>(artifact_json) else {
                return Vec::new();
            };
            vec![line(
                EvidenceKind::Capture,
                &a.capture.resolution,
                a.capture.bundle_hash.clone(),
                "gdl",
                0.9,
            )]
        }
        _ => Vec::new(),
    }
}

// ── the outcome vocabulary ─────────────────────────────────────────────────

/// How a case ended. Only [`GdlOutcome::Resolved`] closed it; everything
/// else is a routing decision — the loop's safe posture is route-not-
/// resolve, and a routed case's run row STAYS `active` for the human who
/// now owns it (review-default posture governs loop writes).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum GdlOutcome {
    /// Verify passed under the failing conditions and the capture landed.
    Resolved {
        phases: u32,
        verify: VerifyArtifact,
        capture: CaptureArtifact,
    },
    /// A gate said this case is not the loop's to resolve.
    Routed {
        at: GdlPhase,
        reason: String,
    },
    /// Dead-end/defer/verify-fail: handed to Eng with the Rust-assembled
    /// bundle (L8 — escalation IS the bundle).
    Escalated {
        at: GdlPhase,
        bundle: EscalationBundle,
    },
    /// Verification ran and FAILED — back to a human, not silently retried.
    VerifyFailed {
        at: GdlPhase,
        bundle: EscalationBundle,
    },
    Canceled,
    Capped {
        at: GdlPhase,
        reason: &'static str,
    },
}

// ── the driver ─────────────────────────────────────────────────────────────

/// The GDL driver: one case, seven phases, every dependency injected.
/// Composes the loop-line's [`LoopDriver`] per phase (the 5-step loop IS the phase
/// engine — the GDL machine only decides what a phase accepts).
pub(crate) struct GdlDriver {
    pool: Pool,
    loop_driver: LoopDriver,
}

impl GdlDriver {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        pool: Pool,
        host: Arc<SqliteWorkflowHost>,
        provider: Arc<dyn LlmProvider>,
        tools: Vec<ToolDef>,
        env: ExecutionEnv,
        config: LoopConfig,
    ) -> Self {
        let harness = Arc::new(AgentHarness::new(host.clone(), "gdl", GDL_METHOD_PROMPT));
        let loop_driver = LoopDriver::new(
            pool.clone(),
            host,
            harness,
            provider,
            tools,
            env,
            config,
            "",
        );
        GdlDriver { pool, loop_driver }
    }

    /// Run one case from its current phase to a terminal outcome. The
    /// ticket is the verbatim intake input; a fresh run starts at Intake,
    /// a resumed run picks up at the persisted phase.
    pub(crate) async fn run_case(
        &self,
        run_id: i64,
        ticket: &str,
        cancel: &CancellationToken,
    ) -> Result<GdlOutcome, LoopError> {
        let mut case = self.load_case(run_id, ticket).await?;
        let start = case.phase;
        let mut phases_done: u32 = 0;
        for phase in GdlPhase::ALL {
            if phase < start {
                continue;
            }
            case.phase = phase;
            let mut retry_errors: Vec<String> = Vec::new();
            let mut attempts = 0u32;
            let artifact_json = loop {
                attempts += 1;
                let instruction = phase_instruction(phase, &case, &retry_errors);
                let outcome = self
                    .loop_driver
                    .run_turns(run_id, &instruction, cancel)
                    .await?;
                match outcome {
                    RunOutcome::Canceled => return Ok(GdlOutcome::Canceled),
                    RunOutcome::TurnCapReached { .. } => {
                        return Ok(GdlOutcome::Capped {
                            at: phase,
                            reason: "turn_cap",
                        });
                    }
                    RunOutcome::BudgetExceeded { .. } => {
                        return Ok(GdlOutcome::Capped {
                            at: phase,
                            reason: "budget",
                        });
                    }
                    RunOutcome::Completed { .. } => {}
                }
                let text = self.last_assistant_text(run_id).await?;
                let (gate, artifact) = parse_and_gate(phase, &case, &text);
                match gate {
                    Gate::Pass => break artifact,
                    Gate::Route(reason) => {
                        self.append_gate_event(
                            run_id,
                            phase,
                            "route",
                            attempts,
                            std::slice::from_ref(&reason),
                        )
                        .await?;
                        // defer/unknown at triage: the knowledge layer does
                        // not cover this case — hand it over WITH the bundle.
                        let bundle = case.escalation_bundle();
                        return Ok(GdlOutcome::Escalated { at: phase, bundle });
                    }
                    Gate::Fail(errors) => {
                        self.append_gate_event(run_id, phase, "fail", attempts, &errors)
                            .await?;
                        if attempts >= MAX_PHASE_ATTEMPTS {
                            return Ok(GdlOutcome::Routed {
                                at: phase,
                                reason: format!(
                                    "gate exhausted after {attempts} attempts: {}",
                                    errors.join("; ")
                                ),
                            });
                        }
                        retry_errors = errors;
                    }
                }
            };
            let Some(artifact_json) = artifact_json else {
                // Pass always carries the normalized artifact; unreachable
                // by construction, fail loud if construction ever changes.
                return Ok(GdlOutcome::Routed {
                    at: phase,
                    reason: "gate passed without an artifact".into(),
                });
            };
            // Verify failure is terminal-but-active: the case goes back to
            // a human with the bundle, never silently re-acted.
            if phase == GdlPhase::Verify
                && let Ok(a) = serde_json::from_str::<VerifyArtifact>(&artifact_json)
                && !a.pass
            {
                apply(&mut case, phase, &artifact_json);
                self.persist_phase(run_id, phase, &case, &artifact_json, "active")
                    .await?;
                let bundle = case.escalation_bundle();
                return Ok(GdlOutcome::VerifyFailed { at: phase, bundle });
            }
            apply(&mut case, phase, &artifact_json);
            self.append_gate_event(run_id, phase, "pass", attempts, &[])
                .await?;
            // A6/verify-once-clear: only the Handoff pass CLOSES the run
            // row (`resolved`); every intermediate phase persists `active`.
            // Routed/escalated cases stay `active` — the human who now
            // owns them decides (review-default posture).
            let status = if phase == GdlPhase::Handoff {
                "resolved"
            } else {
                "active"
            };
            self.persist_phase(run_id, phase, &case, &artifact_json, status)
                .await?;
            phases_done += 1;
        }
        let verify = case
            .verify
            .clone()
            .expect("the machine cannot reach the end without Verify passing");
        let capture = case
            .capture
            .clone()
            .expect("Handoff passed, so capture is set");
        Ok(GdlOutcome::Resolved {
            phases: phases_done,
            verify,
            capture,
        })
    }

    async fn load_case(&self, run_id: i64, ticket: &str) -> Result<GdlCase, LoopError> {
        let pool = self.pool.clone();
        let json = tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| LoopError::Persist(format!("pool: {e}")))?;
            super::state::read_state_and_revision(&conn, run_id)
                .map_err(|e| LoopError::Persist(e.to_string()))?
                .map(|(json, _rev)| json)
                .ok_or_else(|| LoopError::Persist(format!("run {run_id} gone")))
        })
        .await
        .map_err(|e| LoopError::Persist(format!("load join failed: {e}")))??;
        match serde_json::from_str::<GdlCase>(&json) {
            Ok(case) => Ok(case),
            Err(_) => Ok(GdlCase::fresh(ticket)),
        }
    }

    async fn last_assistant_text(&self, run_id: i64) -> Result<String, LoopError> {
        let pool = self.pool.clone();
        let text = tokio::task::spawn_blocking(move || -> Result<String, LoopError> {
            let conn = pool
                .get()
                .map_err(|e| LoopError::Persist(format!("pool: {e}")))?;
            let events = session_log::replay(&conn, run_id, session_log::REPLAY_CAP)
                .map_err(|e| LoopError::Persist(e.to_string()))?;
            Ok(events
                .into_iter()
                .rev()
                .find(|e| e.kind == "assistant")
                .and_then(|e| {
                    serde_json::from_str::<serde_json::Value>(&e.payload_json)
                        .ok()?
                        .get("text")?
                        .as_str()
                        .map(str::to_string)
                })
                .unwrap_or_default())
        })
        .await
        .map_err(|e| LoopError::Persist(format!("replay join failed: {e}")))??;
        Ok(text)
    }

    async fn append_gate_event(
        &self,
        run_id: i64,
        phase: GdlPhase,
        verdict: &str,
        attempt: u32,
        errors: &[String],
    ) -> Result<(), LoopError> {
        let payload = serde_json::json!({
            "phase": phase.as_str(),
            "verdict": verdict,
            "attempt": attempt,
            "errors": errors,
        })
        .to_string();
        crate::agentloop::run_loop::append_session_events(
            &self.pool,
            run_id,
            vec![(
                "gdl_gate".into(),
                payload,
                format!("gdl:{}:{verdict}:{attempt}", phase.as_str()),
            )],
        )
        .await
    }

    /// Persist a phase pass in ONE WorkflowTx: the phase's step row (plus
    /// Act's per-row sub-rows), one audit row per inserted step, and the
    /// CAS state advance. All-or-nothing.
    async fn persist_phase(
        &self,
        run_id: i64,
        phase: GdlPhase,
        case: &GdlCase,
        artifact_json: &str,
        status: &str,
    ) -> Result<(), LoopError> {
        let pool = self.pool.clone();
        let case_json =
            serde_json::to_string(case).map_err(|e| LoopError::Persist(e.to_string()))?;
        let phase_name = phase.as_str();
        let artifact_json = artifact_json.to_string();
        let status = status.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool
                .get()
                .map_err(|e| LoopError::Persist(format!("pool: {e}")))?;
            let mut wtx =
                super::tx::WorkflowTx::begin(&mut conn).map_err(|e| LoopError::Persist(e.to_string()))?;
            let now = chrono::Utc::now().timestamp();
            wtx.tx()
                .execute(
                    "INSERT INTO workflow_steps(run_id, phase, step_key, state_json)
                     VALUES (?1, ?2, ?2, ?3)",
                    rusqlite::params![run_id, phase_name, artifact_json],
                )
                .map_err(|e| LoopError::Persist(e.to_string()))?;
            let phase_row_id = wtx.tx().last_insert_rowid();
            super::audit_write(
                wtx.tx(),
                run_id,
                &format!("step:{phase_name}"),
                crate::audit::AuditStatus::Ok,
                &format!("gdl phase {phase_name} passed"),
            );
            // Act's test-log rows persist as durable sub-rows so the
            // escalation bundle and QA scorer read the log from storage,
            // not from a model's memory of it.
            if phase_name == "act"
                && let Ok(rows) =
                    serde_json::from_str::<ActArtifact>(&artifact_json)
            {
                for r in &rows.rows {
                    let row_json = serde_json::to_string(r)
                        .map_err(|e| LoopError::Persist(e.to_string()))?;
                    wtx.tx()
                        .execute(
                            "INSERT INTO workflow_steps(run_id, phase, step_key, state_json, parent_step_id)
                             VALUES (?1, 'act', ?2, ?3, ?4)",
                            rusqlite::params![
                                run_id,
                                format!("act-{}", r.order),
                                row_json,
                                phase_row_id
                            ],
                        )
                        .map_err(|e| LoopError::Persist(e.to_string()))?;
                    super::audit_write(
                        wtx.tx(),
                        run_id,
                        &format!("step:act-{}", r.order),
                        crate::audit::AuditStatus::Ok,
                        &format!("test-log row {} persisted", r.order),
                    );
                }
            }
            // The phase's typed evidence lands in the SAME tx — one
            // mutation, one chain. Handoff's capture still reads an
            // 'active' run row here: the closing CAS below is what seals
            // it, in this same transaction.
            let batch = typed_evidence_for(phase, &artifact_json, now);
            if !batch.is_empty() {
                super::evidence::record(wtx.tx(), run_id, &batch, None)
                    .map_err(|e| LoopError::Persist(format!("evidence: {e:?}")))?;
            }
            let (state_json, rev) = super::state::read_state_and_revision(wtx.tx(), run_id)
                .map_err(|e| LoopError::Persist(e.to_string()))?
                .ok_or_else(|| LoopError::Persist(format!("run {run_id} gone")))?;
            // The fresh run row '{}' (never a GdlCase) is the expected
            // starting state; any other mismatch is contention the CAS
            // will name loudly as Stale.
            let expected = if state_json.trim() == "{}" { 0 } else { rev };
            super::state::cas_update(wtx.tx(), run_id, expected, &case_json, &status, now)
                .map_err(|e| LoopError::Persist(e.to_string()))?;
            wtx.commit()
                .map_err(|e| LoopError::Persist(e.to_string()))?;
            Ok(())
        })
        .await
        .map_err(|e| LoopError::Persist(format!("persist join failed: {e}")))?
    }
}

// ── the QA bridge (skipped verify classifies as AGENT cause, §Verification) ─

/// Build the pure scorer's [`RunArtifacts`] view of a finished (or routed)
/// case: a skipped verify is an AGENT failure, never a system ceiling.
pub(crate) fn run_artifacts_of(
    case: &GdlCase,
    audit_ok: bool,
    open_contradictions: usize,
) -> brain_engine_sdk::pure::qa_score::RunArtifacts {
    use brain_engine_sdk::pure::qa_score::{RunArtifacts, StepRow};
    let steps = case
        .test_log
        .iter()
        .map(|r| StepRow {
            expected: r.expected.clone(),
            actual: r.actual.clone().unwrap_or_default(),
            skipped_verify: r.dtfvc.verify.as_deref().is_none_or(str::is_empty),
            abstained: matches!(r.verdict, Verdict::Rejected),
            guidance_accepted: None,
        })
        .collect();
    RunArtifacts {
        steps,
        findings: case
            .hypotheses
            .iter()
            .map(|h| h.statement.clone())
            .collect(),
        contradictions: open_contradictions,
        audit_ok,
        repeat_contact: false,
        handoff_complete: case.escalation_bundle().complete,
        verified: case.verify.as_ref().is_some_and(|v| v.pass),
        escalation_honored: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agentloop::provider::{LoopbackProvider, Usage, scripted_text};
    use crate::audit::verify_chain;
    use crate::config;
    use crate::migration::run_migration;
    use crate::register_sqlite_vec::register_sqlite_vec;
    use brain_engine_sdk::env::DenyAll;
    use brain_engine_sdk::pure::qa_score::{Cause, RunArtifacts as SdkRunArtifacts};
    use rusqlite::Connection;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    struct Fixture {
        driver: GdlDriver,
        provider: Arc<LoopbackProvider>,
        tmp: tempfile::NamedTempFile,
    }

    fn fixture(script: Vec<Vec<crate::agentloop::provider::StreamEvent>>) -> Fixture {
        register_sqlite_vec();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mgr = r2d2_sqlite::SqliteConnectionManager::file(tmp.path());
        let pool: Pool = r2d2::Pool::builder().max_size(4).build(mgr).unwrap();
        run_migration(&mut pool.get().unwrap(), config::DB_MMAP_SIZE_MIB).unwrap();
        pool.get()
            .unwrap()
            .execute(
                "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
                 VALUES ('acme', 'troubleshoot', '{}', 0, 'active', 1, 1)",
                [],
            )
            .unwrap();
        let host = Arc::new(SqliteWorkflowHost::new(pool.clone()));
        let provider = LoopbackProvider::new("loopback", script);
        let driver = GdlDriver::new(
            pool,
            host,
            provider.clone(),
            vec![],
            ExecutionEnv {
                fs: Arc::new(DenyAll),
                read_only: true,
                allow_process: false,
                root: "/".into(),
                allowed_commands: vec![],
            },
            LoopConfig::default(),
        );
        Fixture {
            driver,
            provider,
            tmp,
        }
    }

    fn step_rows(path: &std::path::Path) -> Vec<(i64, String, String, Option<i64>)> {
        let conn = Connection::open(path).unwrap();
        let mut stmt = conn
            .prepare("SELECT id, phase, step_key, parent_step_id FROM workflow_steps ORDER BY id")
            .unwrap();
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap();
        rows.collect::<Result<_, _>>().unwrap()
    }

    // The happy-path per-phase artifacts — the JSON contracts the phase
    // instructions name, one scripted model turn per phase.
    const INTAKE_JSON: &str = r#"{"is_not":{"what":{"is":"PERC H740P write-cache write-through","is_not":"read cache"},"where":{"is":"node-042 RAID-10 VDs","is_not":"node-041"},"when":{"is":"since 03:12 during rebuild","is_not":"before 03:12"},"extent":{"is":"VD 5 only","is_not":"all VDs"}},"telemetry_refs":["tsr://node-042","sel://events"],"what_changed":"fw 2.10 flashed last week","known_good":"node-041 same fw"}"#;
    const TRIAGE_JSON: &str = r#"{"priority":"P3","stabilized":false,"search_hits":["P-STORAGE-0104"],"verdict":"accept"}"#;
    const HYPOTHESIZE_JSON: &str = r#"{"hypotheses":[{"statement":"PERC battery dead","prediction":"racadm battery state reports Failed","sources":["SEL event 0x42","racadm get storageservices.battery"],"confidence":0.7}]}"#;
    const PLAN_JSON: &str = r#"{"steps":[{"order":1,"kind":"check","skill_gate":"L1","description":"query battery state","command":"racadm get storageservices.battery","expected":"Ready","fail_action":2,"invasiveness":0,"justification":null},{"order":2,"kind":"action","skill_gate":"L2","description":"replace battery ring 3","command":"hw replace battery","expected":"battery Ready","fail_action":null,"invasiveness":2,"justification":null}],"verify_step":{"re_run":"rebuild rate on VD 5 under the customer load","pass_condition":">10%/h"},"dead_end":{"escalate_to":"eng-storage","required_evidence":["TSR","test log"]}}"#;
    const ACT_JSON: &str = r#"{"rows":[{"order":1,"kind":"check","description":"query battery state","playbook_ref":"P-STORAGE-0104","variables":["battery state"],"expected":"Ready","actual":"Failed","verdict":"fail","evidence_ref":"TSR p.12","dtfvc":{"diagnose":"battery fault hypothesis","test":"racadm query","fix":"replace battery ring 3","verify":"rebuild rate 14%/h","capture":"battery replacement row"},"invasiveness":2,"justification":null}],"complete":true}"#;
    const VERIFY_JSON: &str = r#"{"re_run":"rebuild rate on VD 5 under the customer load","pass":true,"stability_window_min":15,"negative_check":true}"#;
    const HANDOFF_JSON: &str = r#"{"capture":{"resolution":"write-through during rebuild -> dead PERC battery -> replaced ring 3 -> verified 14%/h","bundle_hash":"h0"}}"#;

    fn happy_script() -> Vec<Vec<crate::agentloop::provider::StreamEvent>> {
        vec![
            scripted_text(INTAKE_JSON),
            scripted_text(TRIAGE_JSON),
            scripted_text(HYPOTHESIZE_JSON),
            scripted_text(PLAN_JSON),
            scripted_text(ACT_JSON),
            scripted_text(VERIFY_JSON),
            scripted_text(HANDOFF_JSON),
        ]
    }

    #[test]
    fn gdl_case_runs_all_seven_phases_end_to_end() {
        let f = fixture(happy_script());
        let cancel = CancellationToken::new();
        let outcome = rt()
            .block_on(f.driver.run_case(1, "node-042 rebuild is slow", &cancel))
            .unwrap();
        // Resolved ONLY through a passed verify; 7 phases completed.
        assert!(
            matches!(
                &outcome,
                GdlOutcome::Resolved { phases: 7, verify, .. } if verify.pass && verify.re_run.contains("VD 5")
            ),
            "the happy path resolves through verify: {outcome:?}"
        );
        // Durable step rows: 7 phase rows + the act test-log sub-row,
        // the act sub-row parented to the act phase row.
        let rows = step_rows(f.tmp.path());
        assert_eq!(rows.len(), 8, "7 phase rows + 1 act sub-row");
        assert_eq!(
            rows.iter()
                .map(|(_, p, _, _)| p.as_str())
                .collect::<Vec<_>>(),
            vec![
                "intake",
                "triage",
                "hypothesize",
                "plan",
                "act",
                "act",
                "verify",
                "handoff"
            ],
            "phase rows in order, act carries its sub-row"
        );
        assert!(rows[4].3.is_none(), "phase rows have no parent");
        assert_eq!(
            rows[5].3,
            Some(rows[4].0),
            "the act sub-row is parented to the act phase row's id"
        );
        // The run row CLOSED at handoff (verify-once-clear).
        let conn = Connection::open(f.tmp.path()).unwrap();
        let (status, state_json): (String, String) = conn
            .query_row(
                "SELECT status, state_json FROM workflow_runs WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "resolved");
        let case: GdlCase = serde_json::from_str(&state_json).unwrap();
        assert_eq!(case.phase, GdlPhase::Handoff);
        assert_eq!(case.test_log.len(), 1);
        assert!(case.capture.is_some());
        // The audit chain verifies end to end (audit-per-write held).
        assert!(verify_chain(&conn), "every phase write is chain-verified");
        // The session narrative carries the gate verdicts and the model's
        // artifacts; the strip rode every instruction.
        let events = session_log::replay(&conn, 1, session_log::REPLAY_CAP).unwrap();
        let gate_kinds: Vec<(usize, &str)> = events
            .iter()
            .enumerate()
            .filter(|(_, e)| e.kind == "gdl_gate")
            .map(|(i, e)| (i, e.payload_json.as_str()))
            .collect();
        assert_eq!(gate_kinds.len(), 7, "one pass verdict per phase");
        assert!(gate_kinds.iter().all(|(_, p)| p.contains("\"pass\"")));
        // The provider saw the plan strip in every instruction after Plan.
        let requests = f.provider.requests();
        assert_eq!(requests.len(), 7, "one model exchange per phase");
        assert!(
            requests[3]
                .messages
                .iter()
                .any(|m| m.text.contains("GDL PLAN STRIP"))
        );
        assert!(
            requests[6]
                .messages
                .iter()
                .any(|m| m.text.contains("[1] done"))
        );
        // The system prompt is the pinned method prompt (cache-stable).
        assert!(requests[0].system_prompt.contains("7-phase method"));
        // Typed evidence landed in the findings table, one batch per
        // emitting phase: hypothesis + confidence (Hypothesize), test +
        // expected + actual (Act), verification (Verify), capture
        // (Handoff) — 7 lines, deterministic order, zero contradictions.
        let claims: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT claim FROM findings WHERE run_id = 1 ORDER BY id")
                .unwrap();
            let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
            rows.collect::<Result<_, _>>().unwrap()
        };
        assert_eq!(
            claims,
            vec![
                "confidence: PERC battery dead",
                "hypothesis: PERC battery dead",
                "actual: query battery state",
                "expected: query battery state",
                "test: query battery state",
                "verification: rebuild rate on VD 5 under the customer load",
                "capture: write-through during rebuild -> dead PERC battery -> replaced ring 3 -> verified 14%/h",
            ],
            "phase-batch order, claim-sorted within each batch, kinds on prefixes"
        );
        assert_eq!(super::super::evidence::open_contradictions(&conn, 1), 0);
    }

    #[test]
    fn resolved_case_rejects_unjustified_evidence_writes() {
        // The revisit law, end to end: after the machine resolves a case
        // (verify passed, run row closed), a further evidence write is
        // denied unless it carries a recorded justification.
        let f = fixture(happy_script());
        let cancel = CancellationToken::new();
        let outcome = rt()
            .block_on(f.driver.run_case(1, "node-042 rebuild is slow", &cancel))
            .unwrap();
        assert!(matches!(outcome, GdlOutcome::Resolved { .. }));
        let mut conn = Connection::open(f.tmp.path()).unwrap();
        let before: i64 = conn
            .query_row("SELECT COUNT(*) FROM findings WHERE run_id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        let mut wtx = crate::workflow::tx::WorkflowTx::begin(&mut conn).unwrap();
        let denied = super::super::evidence::record(
            wtx.tx(),
            1,
            &[super::super::evidence::TypedEvidence {
                kind: super::super::evidence::EvidenceKind::Actual,
                claim: "sneaked late row".into(),
                evidence: "Ready".into(),
                source: "gdl".into(),
                confidence: 0.5,
                ts: 9,
            }],
            None,
        );
        assert_eq!(
            denied.unwrap_err(),
            super::super::evidence::EvidenceError::RevisitDenied
        );
        drop(wtx); // roll back the denied attempt entirely
        let after: i64 = conn
            .query_row("SELECT COUNT(*) FROM findings WHERE run_id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(before, after, "nothing persisted on the denial");
        assert!(
            crate::audit::verify_chain(&conn),
            "the denied audit row keeps the chain green"
        );
    }

    #[test]
    fn intake_missing_is_not_routes_not_resolves() {
        // The model never produces a valid intake: three attempts of
        // half-filled KT tables exhaust the gate — the case ROUTES and the
        // run row stays active for the human who owns it now.
        let bad = r#"{"is_not":{"what":{"is":"","is_not":""},"where":{"is":"","is_not":""},"when":{"is":"","is_not":""},"extent":{"is":"","is_not":""}},"telemetry_refs":[],"what_changed":"","known_good":""}"#;
        let f = fixture(vec![
            scripted_text(bad),
            scripted_text(bad),
            scripted_text(bad),
        ]);
        let cancel = CancellationToken::new();
        let outcome = rt()
            .block_on(f.driver.run_case(1, "server is slow", &cancel))
            .unwrap();
        match &outcome {
            GdlOutcome::Routed { at, reason } => {
                assert_eq!(*at, GdlPhase::Intake);
                assert!(reason.contains("gate exhausted"), "{reason}");
                assert!(reason.contains("KT"), "the route names the law: {reason}");
            }
            other => panic!("a case without an intake artifact routes: {other:?}"),
        }
        let conn = Connection::open(f.tmp.path()).unwrap();
        let status: String = conn
            .query_row("SELECT status FROM workflow_runs WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(status, "active", "a routed case stays open for the human");
        let events = session_log::replay(&conn, 1, session_log::REPLAY_CAP).unwrap();
        let fails = events.iter().filter(|e| e.kind == "gdl_gate").count();
        assert_eq!(fails, 3, "every failed attempt is a durable gate event");
        // The retry instruction carried the named violations back to the
        // model (the corrective loop is in the narrative).
        let requests = f.provider.requests();
        assert!(
            requests[1]
                .messages
                .iter()
                .any(|m| { m.text.contains("REJECTED") && m.text.contains("L1") })
        );
    }

    #[test]
    fn intake_gate_names_every_law_it_enforces() {
        let a = IntakeArtifact {
            is_not: IsNotTable::default(),
            telemetry_refs: vec![],
            what_changed: String::new(),
            known_good: String::new(),
        };
        let errors = intake_gate(&a);
        let joined = errors.join(" | ");
        for law in ["KT", "L1", "L4", "L3"] {
            assert!(joined.contains(law), "{law} must be cited: {joined}");
        }
        // Recorded-unknown is VALID (L4's "record 'unknown', never assume").
        let honest = IntakeArtifact {
            is_not: IsNotTable {
                what: IsNotRow {
                    is: "slow".into(),
                    is_not: String::new(),
                },
                place: IsNotRow {
                    is: "node-042".into(),
                    is_not: String::new(),
                },
                time: IsNotRow {
                    is: "since 03:12".into(),
                    is_not: String::new(),
                },
                extent: IsNotRow {
                    is: "VD 5".into(),
                    is_not: String::new(),
                },
            },
            telemetry_refs: vec!["tsr://x".into()],
            what_changed: "unknown".into(),
            known_good: "none available".into(),
        };
        assert!(
            intake_gate(&honest).is_empty(),
            "recorded-unknown is valid KT honesty: {:?}",
            intake_gate(&honest)
        );
    }

    #[test]
    fn triage_without_search_hits_cannot_accept_and_defer_escalates() {
        // Pure: accept with zero knowledge coverage is L9-broken.
        let mut a = serde_json::from_str::<TriageArtifact>(TRIAGE_JSON).unwrap();
        a.search_hits.clear();
        let errors = triage_gate(&a);
        assert!(errors.iter().any(|e| e.starts_with("L9")), "{errors:?}");
        // E2E: a defer verdict escalates WITH the bundle (L8).
        let defer = r#"{"priority":"P2","stabilized":true,"search_hits":[],"verdict":"defer"}"#;
        let f = fixture(vec![scripted_text(INTAKE_JSON), scripted_text(defer)]);
        let cancel = CancellationToken::new();
        let outcome = rt()
            .block_on(f.driver.run_case(1, "odd case", &cancel))
            .unwrap();
        match &outcome {
            GdlOutcome::Escalated { at, bundle } => {
                assert_eq!(*at, GdlPhase::Triage);
                assert!(bundle.complete, "the bundle carries IS/NOT + telemetry");
                assert_eq!(bundle.telemetry_refs.len(), 2);
            }
            other => panic!("defer escalates with the bundle: {other:?}"),
        }
    }

    #[test]
    fn one_line_is_hypothesis_two_distinct_sources_confirm() {
        // L7/A4: root cause is confirmed ONLY by triangulation — ≥2
        // DISTINCT evidence lines. One line, twice-cited, is still one.
        let one = Hypothesis {
            statement: "battery dead".into(),
            prediction: "state Failed".into(),
            sources: vec!["SEL event".into(), "SEL event".into()],
            confidence: None,
        };
        assert_eq!(hypothesis_status(&one), HypothesisStatus::Hypothesis);
        let two = Hypothesis {
            sources: vec!["SEL event".into(), "racadm state".into()],
            ..one
        };
        assert_eq!(hypothesis_status(&two), HypothesisStatus::Confirmed);
    }

    #[test]
    fn plan_gate_enforces_order_contract_and_ladder() {
        let mut a = serde_json::from_str::<PlanArtifact>(PLAN_JSON).unwrap();
        assert!(plan_gate(&a).is_empty(), "the happy plan passes");
        // Ladder violation: a rank-2 action BEFORE a rank-0 one, no
        // justification — the least-invasive ladder ascends.
        a.steps[0].kind = "action".into();
        a.steps[0].invasiveness = 2;
        a.steps[1].invasiveness = 0;
        let errors = plan_gate(&a);
        assert!(
            errors.iter().any(|e| e.starts_with("L5")),
            "the ladder drop is named: {errors:?}"
        );
        // Justification unmutes it (a recorded reason is a process fact).
        a.steps[1].justification = Some("vendor ER mandated order".into());
        assert!(plan_gate(&a).iter().all(|e| !e.starts_with("L5")));
        // The dead-end escalation is defined, never improvised (L8).
        a.dead_end.escalate_to = String::new();
        assert!(plan_gate(&a).iter().any(|e| e.starts_with("L8")));
    }

    #[test]
    fn act_gate_enforces_one_variable_playbook_citation_and_dtfvc() {
        let case = {
            let mut c = GdlCase::fresh("t");
            c.intake = Some(serde_json::from_str(INTAKE_JSON).unwrap());
            c
        };
        let mut a = serde_json::from_str::<ActArtifact>(ACT_JSON).unwrap();
        assert!(act_gate(&a, &case).is_empty(), "the happy act passes");
        // L2: two variables in one row invalidate the test.
        a.rows[0].variables = vec!["battery".into(), "firmware".into()];
        let errors = act_gate(&a, &case);
        assert!(errors.iter().any(|e| e.starts_with("L2")), "{errors:?}");
        // L9: off-playbook without justification is a gate fail; WITH
        // justification it must be FLAGGED experimental, never laundered.
        a.rows[0].variables = vec!["battery".into()];
        a.rows[0].playbook_ref = None;
        a.rows[0].justification = None;
        assert!(act_gate(&a, &case).iter().any(|e| e.starts_with("L9")));
        a.rows[0].justification = Some("emergency containment".into());
        a.rows[0].verdict = Verdict::Pass;
        assert!(
            act_gate(&a, &case)
                .iter()
                .any(|e| e.starts_with("L9") && e.contains("experimental")),
            "an off-playbook pass is rejected-for-gold, loudly"
        );
        a.rows[0].verdict = Verdict::Experimental;
        assert!(act_gate(&a, &case).is_empty());
        // L1: a hand-seeded case with no telemetry cannot act.
        let mut bare = GdlCase::fresh("t");
        bare.intake = Some(IntakeArtifact {
            telemetry_refs: vec![],
            ..serde_json::from_str(INTAKE_JSON).unwrap()
        });
        assert!(act_gate(&a, &bare).iter().any(|e| e.starts_with("L1")));
    }

    #[test]
    fn verify_gate_requires_the_planned_failing_scenario() {
        let case = {
            let mut c = GdlCase::fresh("t");
            let plan: PlanArtifact = serde_json::from_str(PLAN_JSON).unwrap();
            c.verify_step = Some(plan.verify_step);
            c
        };
        let mut a = serde_json::from_str::<VerifyArtifact>(VERIFY_JSON).unwrap();
        assert!(verify_gate(&a, &case).is_empty());
        a.re_run = "looks fine to me".into();
        assert!(
            verify_gate(&a, &case).iter().any(|e| e.starts_with("L6")),
            "'looks fine' is not verification"
        );
        a.re_run = case.verify_step.clone().unwrap().re_run;
        a.stability_window_min = 5;
        assert!(verify_gate(&a, &case).iter().any(|e| e.starts_with("A6")));
    }

    #[test]
    fn failed_verify_hands_back_with_bundle_not_silent_retry() {
        let fail = r#"{"re_run":"rebuild rate on VD 5 under the customer load","pass":false,"stability_window_min":15,"negative_check":true}"#;
        let f = fixture(vec![
            scripted_text(INTAKE_JSON),
            scripted_text(TRIAGE_JSON),
            scripted_text(HYPOTHESIZE_JSON),
            scripted_text(PLAN_JSON),
            scripted_text(ACT_JSON),
            scripted_text(fail),
        ]);
        let cancel = CancellationToken::new();
        let outcome = rt().block_on(f.driver.run_case(1, "t", &cancel)).unwrap();
        match &outcome {
            GdlOutcome::VerifyFailed { bundle, .. } => {
                assert!(bundle.complete, "the hand-back carries the full bundle");
            }
            other => panic!("a failed verify is terminal-but-active: {other:?}"),
        }
        let conn = Connection::open(f.tmp.path()).unwrap();
        let status: String = conn
            .query_row("SELECT status FROM workflow_runs WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(status, "active");
    }

    #[test]
    fn skipped_verify_is_agent_cause_not_system_ceiling() {
        // The plan's Verification item: a skipped verify classifies as
        // AGENT cause via the pure scorer — never a silent system ceiling.
        let mut case = GdlCase::fresh("t");
        case.intake = Some(serde_json::from_str(INTAKE_JSON).unwrap());
        case.test_log = serde_json::from_str::<ActArtifact>(ACT_JSON).unwrap().rows;
        let artifacts = run_artifacts_of(&case, true, 0);
        // The happy act row carries dtfvc.verify — no skip.
        assert!(!artifacts.steps[0].skipped_verify);
        assert_eq!(
            brain_engine_sdk::pure::qa_score::classify_cause(&artifacts),
            Cause::Agent,
            "a case whose verify never ran is an agent-cause outcome"
        );
        // And the direct scorer vocabulary still holds for a real skip.
        let skipped = SdkRunArtifacts {
            steps: vec![brain_engine_sdk::pure::qa_score::StepRow {
                expected: "Ready".into(),
                actual: "Failed".into(),
                skipped_verify: true,
                abstained: false,
                guidance_accepted: None,
            }],
            findings: vec![],
            contradictions: 0,
            audit_ok: true,
            repeat_contact: false,
            handoff_complete: true,
            verified: false,
            escalation_honored: true,
        };
        assert_eq!(
            brain_engine_sdk::pure::qa_score::classify_cause(&skipped),
            Cause::Agent
        );
    }

    #[test]
    fn escalation_bundle_completeness_is_measured_not_assumed() {
        let fresh = GdlCase::fresh("t");
        assert!(!fresh.escalation_bundle().complete);
        let mut full = GdlCase::fresh("t");
        full.intake = Some(serde_json::from_str(INTAKE_JSON).unwrap());
        full.test_log = serde_json::from_str::<ActArtifact>(ACT_JSON).unwrap().rows;
        let bundle = full.escalation_bundle();
        assert!(bundle.complete, "IS/NOT + telemetry + test log = complete");
        assert_eq!(bundle.test_log_rows, 1);
    }

    #[test]
    fn phase_order_is_law() {
        assert_eq!(
            GdlPhase::ALL.map(|p| p.as_str()),
            [
                "intake",
                "triage",
                "hypothesize",
                "plan",
                "act",
                "verify",
                "handoff"
            ]
        );
        assert!(GdlPhase::Intake < GdlPhase::Handoff);
        assert_eq!(MAX_PHASE_ATTEMPTS, 3, "one ask + two corrective re-asks");
    }

    #[test]
    fn method_prompt_is_bounded_and_strip_is_bounded() {
        assert!(
            GDL_METHOD_PROMPT.lines().count() <= 20,
            "prompt discipline: the method prompt stays cache-stable small"
        );
        let mut case = GdlCase::fresh("t");
        case.intake = Some(serde_json::from_str(INTAKE_JSON).unwrap());
        case.hypotheses = serde_json::from_str::<HypothesizeArtifact>(HYPOTHESIZE_JSON)
            .unwrap()
            .hypotheses;
        let strip = plan_strip(&case);
        assert!(strip.contains("GDL PLAN STRIP (live)"));
        assert!(strip.lines().count() <= PLAN_STRIP_MAX_LINES + 1);
    }

    #[test]
    fn canceled_case_settles_clean() {
        let f = fixture(vec![scripted_text(INTAKE_JSON)]);
        let cancel = CancellationToken::new();
        cancel.cancel();
        let outcome = rt().block_on(f.driver.run_case(1, "t", &cancel)).unwrap();
        assert_eq!(outcome, GdlOutcome::Canceled);
        let conn = Connection::open(f.tmp.path()).unwrap();
        assert!(
            verify_chain(&conn),
            "cancel settlement keeps the chain green"
        );
    }

    #[test]
    fn malformed_artifact_is_a_named_gate_failure() {
        let case = GdlCase::fresh("t");
        let (gate, artifact) = parse_and_gate(GdlPhase::Intake, &case, "I am just text");
        assert!(matches!(gate, Gate::Fail(ref e) if e[0].starts_with("JSON")));
        assert!(artifact.is_none());
        // Usage import stays live for the outcome shapes below.
        let _ = Usage::default();
    }
}
