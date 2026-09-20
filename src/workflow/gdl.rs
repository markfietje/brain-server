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

use rusqlite::OptionalExtension;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use brain_engine_sdk::env::{ExecutionEnv, ToolDef};
use brain_engine_sdk::harness::AgentHarness;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::Pool;
use crate::agentloop::hooks::LoopHooks;
use crate::agentloop::provider::LlmProvider;
use crate::agentloop::run_loop::{LoopConfig, LoopDriver, LoopError, RunOutcome};
use crate::audit::AuditStatus;
use crate::workflow::host::SqliteWorkflowHost;
use crate::workflow::session_log;

#[path = "gdl_checkpoint.rs"]
mod checkpoint;

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

/// The adversarial re-check child's own system prompt: a falsifier, not a
/// supporter. The child sees NO tools; its input is the hypothesis and
/// the captured evidence rows, its output the agreed JSON verdict.
const ADVERSARIAL_RECHECK_PROMPT: &str = "\
You are an adversarial reviewer. Your only job is to try to BREAK the \
confirmed hypothesis using the evidence given in the task. You have no \
tools and no authority to change anything. Reply with exactly one JSON \
object: {\"contradicted\": bool, \"reason\": \"the specific evidence \
and why it does or does not falsify the hypothesis\"}. If the evidence \
does not contradict the hypothesis, say so — never invent a \
contradiction.";

/// The soft-handoff predicate (integer law): a case at or above the 80
/// percent confidence line is in the soft-handoff band. Integer-only —
/// no float comparison, no conversion through a float.
pub(crate) const SOFT_HANDOFF_THRESHOLD_PCT: i64 = 80;

pub(crate) fn soft_handoff(confidence_pct: i64) -> bool {
    confidence_pct >= SOFT_HANDOFF_THRESHOLD_PCT
}

/// The named violation: a fired soft-handoff without a recorded
/// justification (the latch law).
pub(crate) const SOFT_HANDOFF_VIOLATION: &str = "soft-handoff fired without justification";

/// The pinned P-class SLA table: integer seconds from the Triage pass to
/// the deadline. The literals are pinned by test and preregistered — a
/// change is a dated addendum to the registered decision, never a tweak.
pub(crate) fn sla_seconds(priority: &str) -> Option<i64> {
    match priority {
        "P1" => Some(3_600),
        "P2" => Some(14_400),
        "P3" => Some(86_400),
        "P4" => Some(604_800),
        _ => None,
    }
}

/// The once-per-case latch: the first fire records itself; a handoff
/// after a fire without a recorded justification is the named violation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SoftHandoffLatch {
    fired: bool,
    #[serde(default)]
    justification: Option<String>,
}

impl SoftHandoffLatch {
    /// Evaluate a handoff against the latch. `Ok(true)` = the handoff
    /// fires (and is recorded); `Err(violation)` = a handoff after a
    /// fired latch with no justification — the named violation.
    pub(crate) fn evaluate(
        &mut self,
        confidence_pct: i64,
        justification: Option<&str>,
    ) -> Result<bool, String> {
        if !soft_handoff(confidence_pct) {
            return Ok(false);
        }
        if self.fired {
            match justification {
                Some(j) if !j.trim().is_empty() => Ok(true),
                _ => Err("soft-handoff fired without justification".into()),
            }
        } else {
            self.fired = true;
            self.justification = justification.map(str::to_string);
            Ok(true)
        }
    }
}

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
    /// The diagnostic channels this case CAN observe (idrac, SEL export,
    /// packet capture, …): a plan step may REQUIRE one, and a required
    /// seam that intake never declared routes instead of improvising.
    /// Bounded: at most 16 declarations, each 1..=64 lowercase chars.
    #[serde(default)]
    pub diagnostic_seams: Vec<String>,
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
    /// The diagnostic channel this step REQUIRES — must have been declared
    /// at intake, or the plan routes instead of improvising an observation
    /// the case cannot make.
    #[serde(default)]
    pub seam: Option<String>,
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
    /// model-summary sentence. `complete` is MEASURED, never assumed: the
    /// intake artifact is the minimum, and every phase strictly before
    /// the escalation point (the case's current phase — the driver never
    /// escalates at any other) must hold its gate-validated artifact: a
    /// non-defer triage verdict, live hypotheses, a step plan, executed
    /// rows for Act, and a verify attempt once the escalation lies beyond
    /// Verify. `test_log_rows` rides for the receiver to judge the
    /// "empty log at escalation" send-back condition, which is
    /// phase-relative (a triage defer has no log yet by construction).
    pub(crate) fn escalation_bundle(&self) -> EscalationBundle {
        let phase_artifact = |p: &GdlPhase| match p {
            GdlPhase::Intake => self.intake.is_some(),
            GdlPhase::Triage => self.triage.as_ref().is_some_and(|t| t.verdict != "defer"),
            GdlPhase::Hypothesize => !self.hypotheses.is_empty(),
            GdlPhase::Plan => !self.plan.is_empty(),
            GdlPhase::Act => !self.test_log.is_empty(),
            GdlPhase::Verify => self.verify.is_some(),
            GdlPhase::Handoff => self.capture.is_some(),
        };
        let complete = self.intake.is_some()
            && GdlPhase::ALL
                .iter()
                .filter(|p| **p < self.phase)
                .all(phase_artifact);
        EscalationBundle {
            is_not: self.intake.as_ref().map(|i| i.is_not.clone()),
            telemetry_refs: self
                .intake
                .as_ref()
                .map(|i| i.telemetry_refs.clone())
                .unwrap_or_default(),
            hypotheses: self.hypotheses.clone(),
            test_log_rows: self.test_log.len(),
            complete,
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
/// IS/IS-NOT rows present with both columns non-empty; telemetry captured
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
        if row.is_not.trim().is_empty() {
            errors.push(err(
                "KT",
                &format!("is_not.{name}.is_not is empty — the comparison must be recorded"),
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
    if a.diagnostic_seams.len() > 16 {
        errors.push(err(
            "L1",
            &format!(
                "diagnostic_seams declares {} channels — at most 16 may be \
                 declared (bounds law)",
                a.diagnostic_seams.len()
            ),
        ));
    }
    for s in &a.diagnostic_seams {
        if s.is_empty() || s.chars().count() > 64 || s.chars().any(char::is_uppercase) {
            errors.push(err(
                "L1",
                &format!(
                    "diagnostic_seams entry {s:?} is malformed — a seam is \
                     1..=64 lowercase chars, the channel a plan step can be \
                     held to"
                ),
            ));
        }
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

/// A hypothesis source is `{kind-prefix}:{locator}` over the closed
/// 8-kind evidence vocabulary (the findings rows' own prefix law), the
/// locator 1..=96 chars. Malformed sources never count toward
/// confirmation — corroboration is measured over evidence KINDS, not
/// over strings that merely differ.
fn kind_source(s: &str) -> Option<super::evidence::EvidenceKind> {
    let (prefix, locator) = s.split_once(':')?;
    if locator.trim().is_empty() || locator.chars().count() > 96 {
        return None;
    }
    super::evidence::EvidenceKind::ALL
        .into_iter()
        .find(|k| k.prefix() == prefix)
}

/// Hypotheses: at least one live statement, each with a falsifiable
/// prediction (a test that cannot fail proves nothing — the scientific
/// core). Sources are kind-prefixed and bounded (≤8, each a
/// `{kind}:{locator}`); a malformed source is a named L7 failure, never
/// silently counted. Confirmation is arbiter-computed downstream, never
/// claimed here (L7: one line = hypothesis, not root cause).
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
        if h.sources.len() > 8 {
            errors.push(err(
                "L7",
                &format!(
                    "hypothesis[{i}] cites {} sources — at most 8 may be \
                     cited (bounds law)",
                    h.sources.len()
                ),
            ));
        }
        for s in &h.sources {
            if kind_source(s).is_none() {
                errors.push(err(
                    "L7",
                    &format!(
                        "hypothesis[{i}] source {s:?} is malformed — a \
                         source is an independent kind-prefixed locator \
                         (kind:locator over the 8-kind evidence vocabulary, \
                         locator 1..=96 chars)"
                    ),
                ));
            }
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
/// intake; re-checked here so a hand-seeded case cannot skip it). A
/// complete artifact is BOUND to the plan: every row's order exists in
/// the plan, the plan's every step is executed exactly once (required
/// step coverage), and a Pass/Fail/Done row carries its actual outcome
/// and an evidence reference — unsupported evidence does not close.
fn act_gate(a: &ActArtifact, case: &GdlCase) -> Vec<String> {
    let mut errors = Vec::new();
    if !a.complete || a.rows.is_empty() {
        errors.push(err(
            "§4",
            "Act must contain executed rows and be complete before Verify",
        ));
    }
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
    if a.complete {
        let plan_len = case.plan.len() as i64;
        let mut executed: std::collections::BTreeMap<i64, u32> = std::collections::BTreeMap::new();
        for r in case.test_log.iter().chain(a.rows.iter()) {
            *executed.entry(r.order).or_insert(0) += 1;
        }
        for (i, r) in a.rows.iter().enumerate() {
            if r.order < 1 || r.order > plan_len {
                errors.push(err(
                    "§4",
                    &format!(
                        "row[{i}].order {} is not a plan step (the plan has \
                         {plan_len} steps) — executed rows bind to the plan",
                        r.order
                    ),
                ));
            }
        }
        for order in 1..=plan_len {
            match executed.get(&order) {
                None => errors.push(err(
                    "§4",
                    &format!(
                        "plan step {order} has no executed row — required step \
                         coverage: every plan step executes or Act is not done"
                    ),
                )),
                Some(n) if *n > 1 => errors.push(err(
                    "§4",
                    &format!(
                        "plan step {order} executed {n} times — one row per \
                         plan step; duplicate execution proves nothing"
                    ),
                )),
                _ => {}
            }
        }
        for (i, r) in a.rows.iter().enumerate() {
            if matches!(r.verdict, Verdict::Pass | Verdict::Fail | Verdict::Done)
                && (r.actual.as_deref().is_none_or(|s| s.trim().is_empty())
                    || r.evidence_ref
                        .as_deref()
                        .is_none_or(|s| s.trim().is_empty() || s.chars().count() > 128))
            {
                errors.push(err(
                    "A4",
                    &format!(
                        "row[{i}] verdict {:?} without its actual outcome and \
                         evidence_ref — unsupported evidence never closes a step",
                        r.verdict
                    ),
                ));
            }
        }
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
///
/// The registered reconciliation (docs/LOOP_AUTOCLOSE_RECONCILIATION.md):
/// "the machine never auto-closes" means no terminal path reaches
/// Resolved without BOTH (a) a passing, law-clean verify artifact
/// (verify_gate: L6 re-run match + A6 floor + negative check) and (b) a
/// present handoff capture (this gate). The machine settling a case as
/// resolved IS permitted — it is the evidence-gated closure; knowledge
/// publication stays proposal-only, and a failed or missing verify is the
/// terminal VerifyFailed hand-back.
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
            Ok(mut a) => {
                let errors = intake_gate(&a);
                if errors.is_empty() {
                    // The persisted declaration is canonical: sorted and
                    // deduped, so seam matching is exact.
                    a.diagnostic_seams.sort();
                    a.diagnostic_seams.dedup();
                }
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
                if errors.is_empty()
                    && let Some(seam) =
                        a.steps
                            .iter()
                            .filter_map(|st| st.seam.as_deref())
                            .find(|s| {
                                !case
                                    .intake
                                    .as_ref()
                                    .is_some_and(|i| i.diagnostic_seams.iter().any(|d| d == s))
                            })
                {
                    return (
                        Gate::Route(format!(
                            "required diagnostic seam not declared at intake: {seam}"
                        )),
                        serde_json::to_string(&a).ok(),
                    );
                }
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

/// The eval's ungated acceptor (arm C): any parseable JSON is accepted
/// and normalized through the same typed artifacts — the machine runs on,
/// the laws do not run. Unparseable text still fails (the machine cannot
/// apply what it cannot read); that is the ablation's only honesty floor.
/// Production constructors never set `ablated`, so this is unreachable
/// outside the eval.
fn accept_ungated(phase: GdlPhase, text: &str) -> (Gate, Option<String>) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text.trim()) else {
        return (
            Gate::Fail(vec!["JSON: artifact is not a JSON object".into()]),
            None,
        );
    };
    let normalized: Option<String> = match phase {
        GdlPhase::Intake => typed_json::<IntakeArtifact>(value),
        GdlPhase::Triage => typed_json::<TriageArtifact>(value),
        GdlPhase::Hypothesize => typed_json::<HypothesizeArtifact>(value),
        GdlPhase::Plan => typed_json::<PlanArtifact>(value),
        GdlPhase::Act => typed_json::<ActArtifact>(value),
        GdlPhase::Verify => typed_json::<VerifyArtifact>(value),
        GdlPhase::Handoff => typed_json::<HandoffArtifact>(value),
    };
    normalized.map_or_else(
        || {
            (
                Gate::Fail(vec!["JSON: artifact does not fit the phase shape".into()]),
                None,
            )
        },
        |json| (Gate::Pass, Some(json)),
    )
}

/// Parse then re-serialize a typed artifact (the ablation's normalizer).
fn typed_json<T: serde::Serialize + serde::de::DeserializeOwned>(
    value: serde_json::Value,
) -> Option<String> {
    serde_json::to_string(&serde_json::from_value::<T>(value).ok()?).ok()
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

/// A hypothesis's arbiter-computed status: ≥2 sources of DISTINCT
/// evidence kinds confirm it (triangulation — the preregistered
/// "different buckets" law); two strings of the same kind are one bucket
/// and never confirm (L7 — one line is a hypothesis, not a root cause).
pub(crate) fn hypothesis_status(h: &Hypothesis) -> HypothesisStatus {
    let mut kinds: Vec<&'static str> = h
        .sources
        .iter()
        .filter_map(|s| kind_source(s))
        .map(super::evidence::EvidenceKind::prefix)
        .collect();
    kinds.sort();
    kinds.dedup();
    if kinds.len() >= 2 {
        HypothesisStatus::Confirmed
    } else {
        HypothesisStatus::Hypothesis
    }
}

/// The registered eval arm C's corroboration law (prereg :39 "1 line
/// suffices"): one valid kind-prefixed source confirms. The kind law is
/// NOT relaxed — a source without a valid kind prefix never counts
/// (L7 still fires at hypothesize); only the threshold moves.
pub(crate) fn hypothesis_status_corroboration_ablated(h: &Hypothesis) -> HypothesisStatus {
    if h.sources.iter().any(|s| kind_source(s).is_some()) {
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
    plan_strip_with_law(case, false)
}

/// The strip under a selected corroboration law: `false` = the registered
/// ≥2-distinct-kinds law; `true` = the registered eval arm C's threshold
/// (≥1 valid source). No other strip byte moves.
pub(crate) fn plan_strip_with_law(case: &GdlCase, corroboration_ablated: bool) -> String {
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
        let status = if corroboration_ablated {
            hypothesis_status_corroboration_ablated(h)
        } else {
            hypothesis_status(h)
        };
        lines.push(format!(
            "H: {} [{}] — predicts: {}",
            h.statement,
            match status {
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
    phase_instruction_with_law(phase, case, retry_errors, false)
}

/// [`phase_instruction`] under a selected corroboration law (the
/// registered eval arm C selects the 1-source threshold; production
/// always passes `false`).
pub(crate) fn phase_instruction_with_law(
    phase: GdlPhase,
    case: &GdlCase,
    retry_errors: &[String],
    corroboration_ablated: bool,
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
        plan_strip_with_law(case, corroboration_ablated),
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
        reason: String,
    },
}

// ── the driver ─────────────────────────────────────────────────────────────

/// The GDL driver: one case, seven phases, every dependency injected.
/// Composes the loop-line's [`LoopDriver`] per phase (the 5-step loop IS the phase
/// engine — the GDL machine only decides what a phase accepts).
/// The unconditional human-escape control flag: a pure boolean the
/// operator side can raise at any moment. The driver observes it ONLY at
/// phase boundaries — the first exchange settle after the flag is raised
/// — where the case terminally escalates with the full bundle. No route,
/// no HTTP, no env knob: the flag is a library seam on the driver call.
#[derive(Debug, Clone, Default)]
pub(crate) struct EscapeFlag(Arc<AtomicBool>);

impl EscapeFlag {
    pub(crate) fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    pub(crate) fn request(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }

    pub(crate) fn is_requested(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Acquire)
    }
}

pub(crate) struct GdlDriver {
    pool: Pool,
    loop_driver: LoopDriver,
    proficiency: super::proficiency::Proficiency,
    /// Eval ablation: skip the arbiter (draft arm C). Production
    /// constructors never set it.
    ablated: bool,
    /// Eval ablation, registered arm C: the corroboration threshold only
    /// (≥1 valid source confirms instead of ≥2 distinct kinds). Every
    /// gate, the authority waterfall, and the contradiction settlement
    /// are untouched; the only consumption of the confirmation status is
    /// the plan-strip label. Production constructors never set it.
    corroboration_ablated: bool,
    /// The adversarial re-check posture (the loop config's default is ON;
    /// the eval ablation constructors turn it off together with the
    /// gates): before the second verification may pass, ONE scoped child
    /// with NO tools attempts to falsify the confirmed hypothesis from
    /// the captured evidence rows.
    adversarial_recheck: bool,
    /// What the single re-check delegation needs: the shared host lineage
    /// and the already-narrowed parent env + tools. The child's
    /// allowed-tool set is empty by construction — it reasons over the
    /// task text only, never executes.
    recheck: Option<RecheckDeps>,
}

/// The one-delegation seam for the adversarial re-check.
struct RecheckDeps {
    host: Arc<SqliteWorkflowHost>,
    provider: Arc<dyn LlmProvider>,
    env: ExecutionEnv,
    tools: Vec<ToolDef>,
}

/// The typed outcome of the adversarial re-check. A contradiction is the
/// named A4 gate failure; an unavailable child degrades honestly —
/// recorded on the session log, non-blocking, visible in the row.
enum AdversarialFinding {
    Contradicted(String),
    Consistent(String),
    Unavailable(String),
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
        Self::new_with_proficiency(
            pool,
            host,
            provider,
            tools,
            env,
            config,
            super::proficiency::Proficiency::L3,
        )
    }

    /// The proficiency constructor: the level narrows BOTH the execution
    /// env (capability subtraction — L1 observes, L2 acts, L3 remediates)
    /// and the phase/step authority matrix. Escalation is never narrowed:
    /// a phase above the level's authority escalates WITH the bundle.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_with_proficiency(
        pool: Pool,
        host: Arc<SqliteWorkflowHost>,
        provider: Arc<dyn LlmProvider>,
        tools: Vec<ToolDef>,
        env: ExecutionEnv,
        config: LoopConfig,
        proficiency: super::proficiency::Proficiency,
    ) -> Self {
        let env = super::proficiency::env_for(&env, proficiency);
        let adversarial_recheck = config.adversarial_recheck;
        let recheck = RecheckDeps {
            host: host.clone(),
            provider: provider.clone(),
            env: env.clone(),
            tools: tools.clone(),
        };
        let harness = Arc::new(AgentHarness::new(host.clone(), "gdl", GDL_METHOD_PROMPT));
        let mut loop_driver = LoopDriver::new(
            pool.clone(),
            host,
            harness,
            provider,
            tools,
            env,
            config,
            "",
            LoopHooks::pass_through(),
        );
        // The harness config is fixed at construction; bind it so
        // policy_identity() is valid for pre-work checkpoint admission.
        loop_driver.bind_policy("gdl", GDL_METHOD_PROMPT, vec![], vec![]);
        GdlDriver {
            pool,
            loop_driver,
            proficiency,
            ablated: false,
            corroboration_ablated: false,
            adversarial_recheck,
            recheck: Some(recheck),
        }
    }

    /// The eval ablation (test-only): the SAME machine with the phase-gate
    /// waterfall switched OFF — every parseable artifact is accepted, the
    /// authority matrix does not run. The outcome logic (verify-fail is
    /// terminal, handoff closes the run) is IDENTICAL; only the arbiter
    /// is ablated. This is the prereg's draft arm C (labeled
    /// `C_draft_all_gates` in the eval runner): kept to reproduce the
    /// historical structural result, NOT the registered ablation.
    #[cfg(test)]
    pub(crate) fn new_ablated(
        pool: Pool,
        host: Arc<SqliteWorkflowHost>,
        provider: Arc<dyn LlmProvider>,
        tools: Vec<ToolDef>,
        env: ExecutionEnv,
        config: LoopConfig,
    ) -> Self {
        let harness = Arc::new(AgentHarness::new(host.clone(), "gdl", GDL_METHOD_PROMPT));
        let mut loop_driver = LoopDriver::new(
            pool.clone(),
            host,
            harness,
            provider,
            tools,
            env,
            config,
            "",
            LoopHooks::pass_through(),
        );
        loop_driver.bind_policy("gdl", GDL_METHOD_PROMPT, vec![], vec![]);
        GdlDriver {
            pool,
            loop_driver,
            proficiency: super::proficiency::Proficiency::L3,
            ablated: true,
            corroboration_ablated: false,
            adversarial_recheck: false,
            recheck: None,
        }
    }

    /// The REGISTERED eval arm C (test-only; EVAL_GDL_VS_AUTONOMOUS :39 —
    /// "identical to B with `G_CORROBORATE` off … 1 line suffices"):
    /// byte-identical to [`GdlDriver::new`] except that a hypothesis
    /// carrying ≥1 valid (kind-prefixed) source is labeled Confirmed
    /// where the registered law requires ≥2 distinct kinds. The shape
    /// law is untouched — a malformed source still fails L7 and never
    /// counts; only the THRESHOLD moves, and the only consumer of the
    /// confirmation status is the plan-strip label. Proven differential
    /// in `eval_run1::registered_arm_c_is_b_except_one_source_confirmation_labels`.
    #[cfg(test)]
    pub(crate) fn new_corroboration_ablated(
        pool: Pool,
        host: Arc<SqliteWorkflowHost>,
        provider: Arc<dyn LlmProvider>,
        tools: Vec<ToolDef>,
        env: ExecutionEnv,
        config: LoopConfig,
    ) -> Self {
        Self::new_with_proficiency(
            pool,
            host,
            provider,
            tools,
            env,
            config,
            super::proficiency::Proficiency::L3,
        )
        .with_corroboration_ablated()
    }

    #[cfg(test)]
    fn with_corroboration_ablated(mut self) -> Self {
        self.corroboration_ablated = true;
        self
    }

    fn policy_identity(&self) -> String {
        let corr = if self.corroboration_ablated {
            ":corr_ablated=true"
        } else {
            ""
        };
        format!(
            "gdl-v1:{:?}:ablated={}{}:{}",
            self.proficiency,
            self.ablated,
            corr,
            self.loop_driver.policy_identity()
        )
    }

    /// One automation episode per run. Terminal retry is read-only with respect
    /// to provider/tool work; human edits invalidate its binding, never restart it.
    /// ONE scoped delegation (the bounds law): a tool-less child attempts
    /// to falsify the confirmed hypothesis from the captured evidence
    /// rows. The child's outcome maps onto the typed finding; a
    /// delegation fault is the honest Unavailable, never a contradiction.
    async fn adversarial_finding(
        &self,
        run_id: i64,
        case: &GdlCase,
        episode: &str,
        attempt: u32,
        owner: &str,
        cancel: &CancellationToken,
    ) -> AdversarialFinding {
        use crate::agentloop::subagents::{SubagentCaps, SubagentOutcome, SubagentSpec};
        const TRUNCATE: usize = 512;
        let Some(deps) = self.recheck.as_ref() else {
            return AdversarialFinding::Unavailable("no delegation seam".into());
        };
        let confirmed = case
            .hypotheses
            .iter()
            .map(|h| h.statement.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        let sources = case
            .hypotheses
            .iter()
            .flat_map(|h| h.sources.iter())
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        let re_run = case
            .verify_step
            .as_ref()
            .map(|v| v.re_run.as_str())
            .unwrap_or("");
        let mut task = format!(
            "Adversarial re-check: attempt to falsify the confirmed \
             hypothesis using ONLY the captured evidence below. Reply with \
             ONE JSON object {{\"contradicted\":bool,\"reason\":\"..\"}} \
             and nothing else.\nHypothesis: {confirmed}\nEvidence \
             sources: {sources}\nPlanned failing scenario: {re_run}\n\
             Ticket: {}",
            case.ticket
        );
        task.truncate(2_000);
        let spec = SubagentSpec {
            name: "adversarial-recheck".into(),
            system_prompt: ADVERSARIAL_RECHECK_PROMPT.into(),
            task,
            // Disjoint by construction: the child sees NO tools — it
            // reasons over the task text only; exec is never in reach.
            allowed_tools: vec![],
            caps: SubagentCaps {
                write: false,
                process: false,
                commands: vec![],
            },
            max_turns: 1,
            token_budget: 2_000,
        };
        let invocation_key = format!("gdl:{episode}:phase:verify:attempt:{attempt}:recheck");
        match crate::agentloop::subagents::delegate_owned(
            &self.pool,
            &deps.host,
            &deps.env,
            &deps.tools,
            deps.provider.clone(),
            run_id,
            &spec,
            &invocation_key,
            owner,
            cancel,
        )
        .await
        {
            Ok(SubagentOutcome::Completed { summary, .. }) => {
                match serde_json::from_str::<serde_json::Value>(summary.trim()) {
                    Ok(v) => {
                        let reason = v
                            .get("reason")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("")
                            .chars()
                            .take(TRUNCATE)
                            .collect::<String>();
                        if v.get("contradicted").and_then(serde_json::Value::as_bool) == Some(true)
                        {
                            AdversarialFinding::Contradicted(reason)
                        } else {
                            AdversarialFinding::Consistent(reason)
                        }
                    }
                    Err(_) => AdversarialFinding::Unavailable(
                        "recheck finding was not the agreed JSON shape".into(),
                    ),
                }
            }
            Ok(_) => AdversarialFinding::Unavailable(
                "recheck child hit its cap without a finding".into(),
            ),
            Err(e) => AdversarialFinding::Unavailable(
                e.to_string().chars().take(TRUNCATE).collect::<String>(),
            ),
        }
    }

    /// The typed soft-handoff row at the escalation path: the integer
    /// predicate evaluated on the case's evidence-derived confidence, with
    /// the latch state for this case run. The latch is DERIVED from the
    /// run's prior rows (stateless, replay-consistent): a fires:true
    /// evaluation on an already-fired run without any recorded
    /// justification is the named violation — it lands a `denied` audit
    /// row and a gap proposal through the SDK's gap rule.
    async fn record_soft_handoff_row(
        &self,
        run_id: i64,
        owner: &str,
        case: &GdlCase,
    ) -> Result<(), LoopError> {
        let confidence_pct = if case.verify.as_ref().is_some_and(|v| v.pass) {
            100
        } else {
            0
        };
        let fires = soft_handoff(confidence_pct);
        let payload = serde_json::json!({
            "confidence_pct": confidence_pct,
            "fires": fires,
            "justification": null,
        })
        .to_string();
        let key = format!("run{run_id}:control:soft_handoff:{owner}");
        let owner = owner.to_string();
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(checkpoint::persist_error)?;
            let mut tx =
                super::tx::WorkflowTx::begin(&mut conn).map_err(checkpoint::persist_error)?;
            // The prior latch state for this run, read before this row
            // lands: fired = any earlier fires:true row; justified = any
            // recorded non-empty justification on those rows.
            let prior: Vec<String> = {
                let mut stmt = tx
                    .tx()
                    .prepare(
                        "SELECT payload_json FROM agent_session_events \
                         WHERE run_id = ?1 AND kind = 'control:soft_handoff' ORDER BY seq",
                    )
                    .map_err(checkpoint::persist_error)?;
                stmt.query_map(rusqlite::params![run_id], |r| r.get(0))
                    .map(|it| it.filter_map(Result::ok).collect())
                    .map_err(checkpoint::persist_error)?
            };
            let prior_fired = prior.iter().any(|p| {
                serde_json::from_str::<serde_json::Value>(p)
                    .ok()
                    .and_then(|v| v["fires"].as_bool())
                    .unwrap_or(false)
            });
            let prior_justified = prior.iter().any(|p| {
                serde_json::from_str::<serde_json::Value>(p)
                    .ok()
                    .and_then(|v| v["justification"].as_str().map(|s| !s.trim().is_empty()))
                    .unwrap_or(false)
            });
            let now = chrono::Utc::now().timestamp();
            session_log::append(tx.tx(), run_id, "control:soft_handoff", &payload, &key, now)
                .map_err(checkpoint::persist_error)?;
            // The unjustified-revisit law: a fired evaluation on an
            // already-fired, never-justified run. The audit row is DENIED;
            // the knowledge gap rides the SDK's gap rule (no coverage ->
            // propose a new article) as a recorded proposal, never an
            // auto-publish.
            if fires && prior_fired && !prior_justified {
                super::audit_write(
                    tx.tx(),
                    run_id,
                    "soft_handoff",
                    AuditStatus::Denied,
                    SOFT_HANDOFF_VIOLATION,
                );
                let action = brain_engine_sdk::pure::qa_score::gap_decision(0);
                let gap = serde_json::json!({
                    "violation": SOFT_HANDOFF_VIOLATION,
                    "proposal": action.map(|a| format!("{a:?}")),
                })
                .to_string();
                session_log::append(
                    tx.tx(),
                    run_id,
                    "control:soft_handoff_gap",
                    &gap,
                    &format!("run{run_id}:control:soft_handoff_gap:{owner}"),
                    now,
                )
                .map_err(checkpoint::persist_error)?;
            }
            tx.commit().map_err(checkpoint::persist_error)?;
            Ok::<_, LoopError>(())
        })
        .await
        .map_err(checkpoint::persist_error)??;
        Ok(())
    }

    /// The typed SLA-arming row at the Triage pass: the pinned P-class
    /// window added to the pass's integer epoch — the clock, the row, and
    /// the envelope value all come from ONE clock read.
    async fn record_sla_armed_row(
        &self,
        run_id: i64,
        owner: &str,
        attempt: u32,
        priority: &str,
        now: i64,
        deadline: i64,
    ) -> Result<(), LoopError> {
        let payload = serde_json::json!({
            "priority": priority,
            "sla_deadline_epoch": deadline,
        })
        .to_string();
        let key = format!("run{run_id}:control:sla_armed:{owner}:{attempt}");
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(checkpoint::persist_error)?;
            let mut tx =
                super::tx::WorkflowTx::begin(&mut conn).map_err(checkpoint::persist_error)?;
            session_log::append(tx.tx(), run_id, "control:sla_armed", &payload, &key, now)
                .map_err(checkpoint::persist_error)?;
            tx.commit().map_err(checkpoint::persist_error)?;
            Ok::<_, LoopError>(())
        })
        .await
        .map_err(checkpoint::persist_error)??;
        Ok(())
    }

    /// The machine-landed handoff offer at the escalation terminal: the
    /// I-PASS packet pre-filled from the case. The offer is a DRAFT for a
    /// human — it sits in `offered` until a role-gated operator decides;
    /// the machine never calls the decision path. The typed row records
    /// the offer id and the packet's remaining gaps — the coaching
    /// surface, never an auto-publish.
    async fn record_escalation_offer(
        &self,
        run_id: i64,
        owner: &str,
        case: &GdlCase,
    ) -> Result<(), LoopError> {
        let to = escalation_target(case);
        let to = to
            .chars()
            .take(crate::workflow::relay::MAX_PRINCIPAL_LEN)
            .collect::<String>();
        let owner = owner.to_string();
        let case = case.clone();
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(checkpoint::persist_error)?;
            let mut tx =
                super::tx::WorkflowTx::begin(&mut conn).map_err(checkpoint::persist_error)?;
            let now = chrono::Utc::now().timestamp();
            let domain: String = tx
                .tx()
                .query_row(
                    "SELECT domain FROM workflow_runs WHERE id = ?1",
                    rusqlite::params![run_id],
                    |r| r.get(0),
                )
                .map_err(checkpoint::persist_error)?;
            // The armed SLA clock rides the packet; the relay's own P3 TTL
            // fallback applies when no clock was armed (a pre-R13 case).
            let armed: Option<i64> = tx
                .tx()
                .query_row(
                    "SELECT payload_json FROM agent_session_events \
                     WHERE run_id = ?1 AND kind = 'control:sla_armed' \
                     ORDER BY seq DESC LIMIT 1",
                    rusqlite::params![run_id],
                    |r| r.get::<_, String>(0),
                )
                .optional()
                .map_err(checkpoint::persist_error)?
                .and_then(|p| serde_json::from_str::<serde_json::Value>(&p).ok())
                .and_then(|v| v["sla_deadline_epoch"].as_i64());
            let deadline =
                armed.unwrap_or_else(|| now + brain_engine_sdk::policy::Priority::P3.ttl_secs());
            let draft = crate::workflow::relay::OfferDraft {
                domain: &domain,
                run_id,
                from_principal: "gdl-loop",
                to_principal: &to,
                overlap_minutes: 0,
                sla_deadline: deadline,
                now,
            };
            let (offer_id, created) = crate::workflow::relay::insert_offer(tx.tx(), &draft)
                .map_err(checkpoint::persist_error)?;
            if created {
                let facts = ipass_facts(&case, Some(deadline), now);
                let missing = crate::workflow::relay::packet_missing(&facts);
                let payload = serde_json::json!({
                    "offer_id": offer_id,
                    "to": to,
                    "missing": missing,
                })
                .to_string();
                session_log::append(
                    tx.tx(),
                    run_id,
                    "control:handoff_offer",
                    &payload,
                    &format!("run{run_id}:control:handoff_offer:{owner}"),
                    now,
                )
                .map_err(checkpoint::persist_error)?;
            }
            tx.commit().map_err(checkpoint::persist_error)?;
            Ok::<_, LoopError>(())
        })
        .await
        .map_err(checkpoint::persist_error)??;
        Ok(())
    }

    /// The typed finding row on the session log — recorded for the
    /// operator on EVERY delegation, whatever the outcome.
    async fn record_adversarial_row(
        &self,
        run_id: i64,
        exchange_id: i64,
        outcome: &str,
        detail: &str,
    ) -> Result<(), LoopError> {
        let pool = self.pool.clone();
        let payload = serde_json::json!({ "outcome": outcome, "detail": detail }).to_string();
        let key = format!("run{run_id}:control:adversarial_recheck:{exchange_id}");
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(checkpoint::persist_error)?;
            let mut tx =
                super::tx::WorkflowTx::begin(&mut conn).map_err(checkpoint::persist_error)?;
            session_log::append(
                tx.tx(),
                run_id,
                "control:adversarial_recheck",
                &payload,
                &key,
                chrono::Utc::now().timestamp(),
            )
            .map_err(checkpoint::persist_error)?;
            tx.commit().map_err(checkpoint::persist_error)?;
            Ok::<_, LoopError>(())
        })
        .await
        .map_err(checkpoint::persist_error)??;
        Ok(())
    }

    /// One case, seven phases, every dependency injected; the terminal is
    /// durable and replay-exact. The auto-close reconciliation law
    /// (docs/LOOP_AUTOCLOSE_RECONCILIATION.md) binds every terminal path
    /// here: Resolved requires a passing, law-clean verify artifact AND a
    /// present handoff capture — closure is evidence-gated, never silent.
    pub(crate) async fn run_case(
        &self,
        run_id: i64,
        ticket: &str,
        cancel: &CancellationToken,
    ) -> Result<GdlOutcome, LoopError> {
        self.run_case_with_escape(run_id, ticket, cancel, &EscapeFlag::new())
            .await
    }

    /// The full-run form with the human-escape seam: the caller keeps the
    /// [`EscapeFlag`] handle and may raise it at any moment; the case
    /// terminally escalates with the full bundle at the next boundary.
    pub(crate) async fn run_case_with_escape(
        &self,
        run_id: i64,
        ticket: &str,
        cancel: &CancellationToken,
        escape: &EscapeFlag,
    ) -> Result<GdlOutcome, LoopError> {
        self.run_case_until(run_id, ticket, cancel, None, escape)
            .await?
            .ok_or_else(|| checkpoint::persist_error("unexpected GDL pause"))
    }

    /// A bounded clean pause is an explicit caller request, NOT crash recovery.
    /// It is honored only after a fully committed exchange/checkpoint. Production
    /// run_case holds the same claim across every phase (None means no pause).
    async fn run_case_until(
        &self,
        run_id: i64,
        ticket: &str,
        cancel: &CancellationToken,
        pause_after: Option<u32>,
        escape: &EscapeFlag,
    ) -> Result<Option<GdlOutcome>, LoopError> {
        use checkpoint::{Transition, persist_error};
        let owner = uuid::Uuid::new_v4().to_string();
        let pool = self.pool.clone();
        let ticket = ticket.to_string();
        let policy = self.policy_identity();
        let owned = owner.clone();
        let mut cp = tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(persist_error)?;
            checkpoint::admit(&mut conn, run_id, &ticket, &policy, &owned)
        })
        .await
        .map_err(persist_error)??;
        let mut completed = 0;
        loop {
            if let Some(outcome) = cp.terminal {
                return Ok(Some(outcome));
            }
            let phase = cp
                .next_phase
                .ok_or_else(|| persist_error("GDL next phase absent"))?;
            let attempt = cp.attempt + 1;
            let mut change = Transition {
                phase,
                attempt,
                verdict: "route",
                errors: Vec::new(),
                artifact: None,
                exchange: None,
                terminal: None,
            };
            let mut case = cp.case.clone();
            case.phase = phase;
            if self.proficiency < super::proficiency::phase_owner(phase) {
                change.errors.push(format!(
                    "authority: phase {} is owned by a higher tier — escalating with the bundle",
                    phase.as_str()
                ));
                change.terminal = Some(GdlOutcome::Escalated {
                    at: phase,
                    bundle: case.escalation_bundle(),
                });
            } else {
                // Recheck status/revision immediately before external work, not
                // only at admission. Concurrent changes also fail at checkpoint.
                let pool = self.pool.clone();
                let check = cp.clone();
                let owned = owner.clone();
                tokio::task::spawn_blocking(move || {
                    let mut conn = pool.get().map_err(persist_error)?;
                    let mut tx = super::tx::WorkflowTx::begin(&mut conn).map_err(persist_error)?;
                    check.verify(tx.tx(), &owned)?;
                    tx.commit().map_err(persist_error)?;
                    Ok::<_, LoopError>(())
                })
                .await
                .map_err(persist_error)??;
                let instruction = phase_instruction_with_law(
                    phase,
                    &case,
                    &cp.errors,
                    self.corroboration_ablated,
                );
                let receipt = self
                    .loop_driver
                    .run_turns_owned(
                        run_id,
                        &format!(
                            "gdl:{}:phase:{}:attempt:{attempt}",
                            cp.episode,
                            phase.as_str()
                        ),
                        &instruction,
                        &owner,
                        cancel,
                    )
                    .await?;
                change.exchange = Some(receipt.exchange_id);
                change.terminal = match receipt.outcome {
                    RunOutcome::Canceled => Some(GdlOutcome::Canceled),
                    RunOutcome::TurnCapReached { .. } => Some(GdlOutcome::Capped {
                        at: phase,
                        reason: "turn_cap".into(),
                    }),
                    RunOutcome::BudgetExceeded { .. } => Some(GdlOutcome::Capped {
                        at: phase,
                        reason: "budget".into(),
                    }),
                    RunOutcome::Completed { .. } => None,
                };
                // C2: the unconditional human escape, observed at the
                // post-exchange boundary — the FIRST settle after the flag
                // was raised. Regardless of gate state (the gate never
                // runs), the case terminally escalates with the full
                // bundle; the committed terminal is replay-exact and
                // nothing un-routes it. The escalation shape is the
                // checkpoint law's receipt-backed boundary terminal; the
                // reason rides the gate record verbatim.
                if change.terminal.is_none() && escape.is_requested() {
                    change.verdict = "route";
                    change.errors = vec!["operator escape".into()];
                    change.terminal = Some(GdlOutcome::Escalated {
                        at: phase,
                        bundle: case.escalation_bundle(),
                    });
                }
                if change.terminal.is_none() {
                    let (mut gate, artifact) = if self.ablated {
                        accept_ungated(phase, &receipt.final_text)
                    } else {
                        parse_and_gate(phase, &case, &receipt.final_text)
                    };
                    if matches!(gate, Gate::Pass) && phase == GdlPhase::Act {
                        let act: ActArtifact =
                            serde_json::from_str(artifact.as_deref().unwrap_or_default())
                                .map_err(persist_error)?;
                        let errors = super::proficiency::act_authority_errors(
                            &case,
                            &act.rows,
                            self.proficiency,
                        );
                        if !errors.is_empty() {
                            gate = Gate::Fail(errors);
                        }
                    }
                    if matches!(gate, Gate::Pass) && phase == GdlPhase::Handoff {
                        // The contradiction settlement: the reducer is the
                        // only detector, and a Pass at Handoff may not close
                        // over an open pair — resolve_contradiction is the
                        // disposition seam, so the attempt fails A4 and the
                        // bounded retry/exhaustion machinery takes over.
                        let pool = self.pool.clone();
                        let open = tokio::task::spawn_blocking(move || {
                            let conn = pool.get().map_err(persist_error)?;
                            Ok::<_, LoopError>(super::evidence::open_contradictions(&conn, run_id))
                        })
                        .await
                        .map_err(persist_error)??;
                        if open > 0 {
                            gate = Gate::Fail(vec![err(
                                "A4",
                                "open contradictions must be dispositioned via \
                                 resolve_contradiction before resolution",
                            )]);
                        }
                    }
                    match gate {
                        Gate::Pass => {
                            let artifact = artifact
                                .ok_or_else(|| persist_error("gate passed without artifact"))?;
                            apply(&mut case, phase, &artifact);
                            change.verdict = "pass";
                            if phase == GdlPhase::Verify
                                && case.verify.as_ref().is_some_and(|v| !v.pass)
                            {
                                change.artifact = Some(artifact);
                                change.terminal = Some(GdlOutcome::VerifyFailed {
                                    at: phase,
                                    bundle: case.escalation_bundle(),
                                });
                            } else if phase == GdlPhase::Verify && !self.ablated {
                                // C3 first — the adversarial re-check: ONE
                                // tool-less child attempts to falsify the
                                // confirmed hypothesis from the captured
                                // evidence. A contradiction is the named A4
                                // gate failure (the bounded retry re-runs the
                                // whole phase); an unavailable child degrades
                                // honestly (recorded, non-blocking). Every
                                // delegation lands a typed row on the session
                                // log.
                                if self.adversarial_recheck {
                                    let finding = self
                                        .adversarial_finding(
                                            run_id,
                                            &case,
                                            &cp.episode,
                                            attempt,
                                            &owner,
                                            cancel,
                                        )
                                        .await;
                                    let (outcome_label, detail): (&str, &String) = match &finding {
                                        AdversarialFinding::Contradicted(d) => ("contradicted", d),
                                        AdversarialFinding::Consistent(d) => ("consistent", d),
                                        AdversarialFinding::Unavailable(d) => ("unavailable", d),
                                    };
                                    self.record_adversarial_row(
                                        run_id,
                                        receipt.exchange_id,
                                        outcome_label,
                                        detail,
                                    )
                                    .await?;
                                    if let AdversarialFinding::Contradicted(d) = finding {
                                        change.verdict = "fail";
                                        let errors = vec![err(
                                            "A4",
                                            &format!(
                                                "adversarial re-check contradicted \
                                                 the confirmed hypothesis: {d}"
                                            ),
                                        )];
                                        if attempt >= MAX_PHASE_ATTEMPTS {
                                            change.terminal = Some(GdlOutcome::Routed {
                                                at: phase,
                                                reason: format!(
                                                    "gate exhausted after {attempt} attempts: {}",
                                                    errors.join("; ")
                                                ),
                                            });
                                        }
                                        change.errors = errors;
                                    }
                                }
                                // C2 — the independent second verification: a
                                // separate provider exchange (its own
                                // `:confirm` request key and exchange id)
                                // re-asks Verify under the same L6/A6
                                // contract. Both artifacts must be law-clean
                                // AND agree (same planned re-run, both pass)
                                // before the case carries a verify artifact
                                // toward Handoff. A disagreeing or unparseable
                                // second pass is the named A6 gate failure and
                                // rides the bounded retry; an interrupted
                                // confirmation routes with the named reason —
                                // neither ever resolves. The checkpoint
                                // transition stays bound to the FIRST
                                // exchange (the receipt law): its artifact is
                                // what the persisted case keeps; the
                                // confirmation's rows live in the session log.
                                if change.terminal.is_none() && change.verdict == "pass" {
                                    let confirm_key = format!(
                                        "gdl:{}:phase:{}:attempt:{attempt}:confirm",
                                        cp.episode,
                                        phase.as_str()
                                    );
                                    let confirm = self
                                        .loop_driver
                                        .run_turns_owned(
                                            run_id,
                                            &confirm_key,
                                            &instruction,
                                            &owner,
                                            cancel,
                                        )
                                        .await?;
                                    change.terminal = match confirm.outcome {
                                        RunOutcome::Canceled => Some(GdlOutcome::Routed {
                                            at: phase,
                                            reason: "second verification interrupted: \
                                                     canceled"
                                                .into(),
                                        }),
                                        RunOutcome::TurnCapReached { .. } => {
                                            Some(GdlOutcome::Routed {
                                                at: phase,
                                                reason: "second verification \
                                                         interrupted: turn cap"
                                                    .into(),
                                            })
                                        }
                                        RunOutcome::BudgetExceeded { .. } => {
                                            Some(GdlOutcome::Routed {
                                                at: phase,
                                                reason: "second verification \
                                                         interrupted: budget"
                                                    .into(),
                                            })
                                        }
                                        RunOutcome::Completed { .. } => None,
                                    };
                                    if change.terminal.is_none() {
                                        let (gate2, artifact2) =
                                            parse_and_gate(phase, &case, &confirm.final_text);
                                        let agreed = match (gate2, artifact2) {
                                            (Gate::Pass, Some(second_json)) => {
                                                let first: VerifyArtifact =
                                                    serde_json::from_str(&artifact)
                                                        .map_err(persist_error)?;
                                                let second: VerifyArtifact =
                                                    serde_json::from_str(&second_json)
                                                        .map_err(persist_error)?;
                                                second.pass
                                                    && second.re_run.trim() == first.re_run.trim()
                                            }
                                            _ => false,
                                        };
                                        if agreed {
                                            change.artifact = Some(artifact);
                                        } else {
                                            change.verdict = "fail";
                                            let errors = vec![err(
                                                "A6",
                                                "second verification absent or \
                                             inconsistent — the same planned \
                                             failing scenario must pass twice, \
                                             in separate exchanges, before the \
                                             case may close",
                                            )];
                                            if attempt >= MAX_PHASE_ATTEMPTS {
                                                change.terminal = Some(GdlOutcome::Routed {
                                                    at: phase,
                                                    reason: format!(
                                                        "gate exhausted after {attempt} attempts: {}",
                                                        errors.join("; ")
                                                    ),
                                                });
                                            }
                                            change.errors = errors;
                                        }
                                    }
                                }
                            } else {
                                change.artifact = Some(artifact);
                                if phase == GdlPhase::Handoff {
                                    change.terminal = Some(GdlOutcome::Resolved {
                                        phases: cp.phases + 1,
                                        verify: case
                                            .verify
                                            .clone()
                                            .ok_or_else(|| persist_error("Verify absent"))?,
                                        capture: case
                                            .capture
                                            .clone()
                                            .ok_or_else(|| persist_error("Capture absent"))?,
                                    });
                                }
                            }
                        }
                        Gate::Route(reason) => {
                            change.errors.push(reason);
                            change.terminal = Some(GdlOutcome::Escalated {
                                at: phase,
                                bundle: case.escalation_bundle(),
                            });
                        }
                        Gate::Fail(errors) => {
                            change.verdict = "fail";
                            if attempt >= MAX_PHASE_ATTEMPTS {
                                change.terminal = Some(GdlOutcome::Routed {
                                    at: phase,
                                    reason: format!(
                                        "gate exhausted after {attempt} attempts: {}",
                                        errors.join("; ")
                                    ),
                                });
                            }
                            change.errors = errors;
                        }
                    }
                }
            }
            // The soft-handoff law at the escalation path: every
            // escalated/routed terminal evaluates the integer predicate and
            // lands a typed row — the operator sees whether the 80%
            // soft-handoff posture fired for this case.
            if matches!(
                change.terminal,
                Some(GdlOutcome::Escalated { .. } | GdlOutcome::Routed { .. })
            ) {
                self.record_soft_handoff_row(run_id, &owner, &case).await?;
            }
            // C3: the I-PASS packet pre-fill — an escalated case lands ONE
            // offer draft, pre-filled from the case, for a human to decide.
            if matches!(change.terminal, Some(GdlOutcome::Escalated { .. })) {
                self.record_escalation_offer(run_id, &owner, &case).await?;
            }
            let triage_passed = phase == GdlPhase::Triage && change.verdict == "pass";
            let pool = self.pool.clone();
            let owned = owner.clone();
            cp = tokio::task::spawn_blocking(move || {
                let mut conn = pool.get().map_err(persist_error)?;
                checkpoint::advance(&mut conn, &cp, &owned, change)
            })
            .await
            .map_err(persist_error)??;
            // The typed SLA-arming row: recorded only after the arming pass
            // has committed. The row IS the durable clock — append-only,
            // replay-exact — because the checkpoint state-derivation law
            // requires the committed case to re-derive exactly from the
            // prior case plus the artifact, which excludes any live-clock
            // envelope field. ONE clock read arms both the stamp and the
            // deadline, so the row's window is exact.
            if triage_passed {
                if let Some(t) = case.triage.as_ref() {
                    if let Some(window) = sla_seconds(&t.priority) {
                        let now = chrono::Utc::now().timestamp();
                        self.record_sla_armed_row(
                            run_id,
                            &owner,
                            attempt,
                            &t.priority,
                            now,
                            now + window,
                        )
                        .await?;
                    }
                }
            }
            completed += 1;
            if cp.terminal.is_none() && pause_after.is_some_and(|limit| completed >= limit) {
                let pool = self.pool.clone();
                tokio::task::spawn_blocking(move || {
                    let mut conn = pool.get().map_err(persist_error)?;
                    checkpoint::pause(&mut conn, &cp, &owner)
                })
                .await
                .map_err(persist_error)??;
                return Ok(None);
            }
        }
    }
}

// ── the QA bridge (skipped verify classifies as AGENT cause, §Verification) ─

/// Build the pure scorer's [`RunArtifacts`] view of a finished (or routed)
/// case: a skipped verify is an AGENT failure, never a system ceiling.
/// The draft's addressee: the case's declared escalation target when the
/// plan has declared one, else the operator queue (an early escalation
/// carries no dead-end yet).
pub(crate) fn escalation_target(case: &GdlCase) -> String {
    case.dead_end
        .as_ref()
        .map(|d| d.escalate_to.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "operator".into())
}

/// The I-PASS packet pre-fill (pure): the loop's case state mapped onto
/// the relay's packet facts. The machine fills only the sender-owned
/// sections — illness (the triage class), patient (the intake IS/NOT
/// table), action (the plan), situation (evidence + open question); the
/// synthesis has NO machine-side field at all. The receiver's decision is
/// the human's: the draft sits in `offered` until `decide_offer`.
pub(crate) fn ipass_facts(
    case: &GdlCase,
    sla_deadline: Option<i64>,
    now: i64,
) -> crate::workflow::relay::PacketFacts {
    crate::workflow::relay::PacketFacts {
        // The open question is the planned re-run — the thing the case
        // was trying to answer when it escalated.
        pending_question: case
            .verify_step
            .as_ref()
            .map(|v| v.re_run.clone())
            .filter(|s| !s.trim().is_empty()),
        sla_deadline,
        now,
        has_current_step: !case.plan.is_empty(),
        has_evidence: !case.test_log.is_empty(),
        // The terminal IS the honored escalation: the case reached a
        // human with its bundle (the soft-handoff posture).
        escalation_honored: true,
    }
}

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
        let mgr = crate::pool::SqliteConnectionManager::file(tmp.path());
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
    const HYPOTHESIZE_JSON: &str = r#"{"hypotheses":[{"statement":"PERC battery dead","prediction":"racadm battery state reports Failed","sources":["actual:SEL event 0x42","test:racadm get storageservices.battery"],"confidence":0.7}]}"#;
    const PLAN_JSON: &str = r#"{"steps":[{"order":1,"kind":"check","skill_gate":"L1","description":"query battery state","command":"racadm get storageservices.battery","expected":"Ready","fail_action":2,"invasiveness":0,"justification":null},{"order":2,"kind":"action","skill_gate":"L2","description":"replace battery ring 3","command":"hw replace battery","expected":"battery Ready","fail_action":null,"invasiveness":2,"justification":null}],"verify_step":{"re_run":"rebuild rate on VD 5 under the customer load","pass_condition":">10%/h"},"dead_end":{"escalate_to":"eng-storage","required_evidence":["TSR","test log"]}}"#;
    const ACT_JSON: &str = r#"{"rows":[{"order":1,"kind":"check","description":"query battery state","playbook_ref":"P-STORAGE-0104","variables":["battery state"],"expected":"Ready","actual":"Failed","verdict":"fail","evidence_ref":"TSR p.12","dtfvc":{"diagnose":"battery fault hypothesis","test":"racadm query","fix":null,"verify":"battery state readback matches Failed","capture":null},"invasiveness":0,"justification":null},{"order":2,"kind":"action","description":"replace battery ring 3","playbook_ref":"P-STORAGE-0104","variables":["battery"],"expected":"battery Ready","actual":"Ready","verdict":"pass","evidence_ref":"TSR p.13","dtfvc":{"diagnose":"battery fault confirmed by row 1","test":"racadm query post-replace","fix":"replaced battery ring 3","verify":"rebuild rate 14%/h","capture":"battery replacement row"},"invasiveness":2,"justification":null}],"complete":true}"#;
    const VERIFY_JSON: &str = r#"{"re_run":"rebuild rate on VD 5 under the customer load","pass":true,"stability_window_min":15,"negative_check":true}"#;
    const RECHECK_JSON: &str =
        r#"{"contradicted":false,"reason":"no falsifier in the captured evidence"}"#;
    const HANDOFF_JSON: &str = r#"{"capture":{"resolution":"write-through during rebuild -> dead PERC battery -> replaced ring 3 -> verified 14%/h","bundle_hash":"h0"}}"#;

    fn happy_script() -> Vec<Vec<crate::agentloop::provider::StreamEvent>> {
        vec![
            scripted_text(INTAKE_JSON),
            scripted_text(TRIAGE_JSON),
            scripted_text(HYPOTHESIZE_JSON),
            scripted_text(PLAN_JSON),
            scripted_text(ACT_JSON),
            scripted_text(VERIFY_JSON),
            // The adversarial re-check: ONE tool-less child attempts to
            // falsify the confirmed hypothesis before the confirmation.
            scripted_text(RECHECK_JSON),
            // The second verification: the gated machine re-asks Verify as a
            // separate exchange and requires the same planned failing
            // scenario to pass twice.
            scripted_text(VERIFY_JSON),
            scripted_text(HANDOFF_JSON),
        ]
    }

    /// Seed the exact exchange journal rows a receipt check reads back: the
    /// start row, one assistant turn, and the done receipt. Test-only.
    fn fixture_exchange(
        conn: &mut Connection,
        cp: &checkpoint::Checkpoint,
        phase: GdlPhase,
        attempt: u32,
        artifact: &str,
        owner: &str,
    ) -> i64 {
        let mut tx = super::super::tx::WorkflowTx::begin(conn).unwrap();
        let fingerprint = format!("v2:{}:v2:{}", "0".repeat(64), "0".repeat(64));
        let (created, id) = session_log::admit_exchange(
            tx.tx(),
            1,
            owner,
            &format!(
                "gdl:{}:phase:{}:attempt:{attempt}",
                cp.episode,
                phase.as_str()
            ),
            &fingerprint,
        )
        .unwrap();
        assert!(created);
        session_log::append(
            tx.tx(),
            1,
            "assistant",
            &serde_json::json!({"text": artifact}).to_string(),
            &format!("run1:a{id}:t1"),
            1,
        )
        .unwrap();
        session_log::append(tx.tx(), 1, "control:exchange_done", &serde_json::json!({"version":1,"outcome":{"Completed":{"turns":1,"usage":{"input_tokens":0,"output_tokens":0}}},"assistant_key":format!("run1:a{id}:t1")}).to_string(), &format!("control:exchange_done:{id}"), 1).unwrap();
        tx.commit().unwrap();
        id
    }

    fn reload(
        path: &std::path::Path,
        script: Vec<Vec<crate::agentloop::provider::StreamEvent>>,
        config: LoopConfig,
    ) -> (GdlDriver, Arc<LoopbackProvider>) {
        let pool = r2d2::Pool::builder()
            .max_size(4)
            .build(crate::pool::SqliteConnectionManager::file(path))
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
            config,
        );
        (driver, provider)
    }

    #[test]
    fn r1_reload_every_phase_never_duplicates_act() {
        let f = fixture(vec![]);
        let runtime = rt();
        let mut script = happy_script().into_iter();
        // One reload per phase settlement. Verify settles over THREE
        // exchanges (the pass, the adversarial re-check, the second
        // verification) inside one pause window.
        let mut groups: Vec<Vec<Vec<crate::agentloop::provider::StreamEvent>>> = Vec::new();
        for _ in 0..5 {
            groups.push(vec![script.next().expect("phase event")]);
        }
        groups.push(vec![
            script.next().expect("verify artifact"),
            script.next().expect("adversarial re-check verdict"),
            script.next().expect("verify confirmation"),
        ]);
        groups.push(vec![script.next().expect("handoff artifact")]);
        for (i, group) in groups.into_iter().enumerate() {
            let (driver, provider) = reload(f.tmp.path(), group, LoopConfig::default());
            let result = runtime
                .block_on(driver.run_case_until(
                    1,
                    "reload",
                    &CancellationToken::new(),
                    Some(1),
                    &EscapeFlag::new(),
                ))
                .unwrap();
            assert_eq!(
                provider.requests().len(),
                if i == 5 { 3 } else { 1 },
                "one settlement per group; Verify exchanges three times"
            );
            assert_eq!(result.is_some(), i == 6);
            if let Some(outcome) = result {
                assert!(matches!(outcome, GdlOutcome::Resolved { phases: 7, .. }));
            }
            drop(driver);
        }
        let rows = step_rows(f.tmp.path());
        assert_eq!(rows.len(), 9);
        assert_eq!(rows.iter().filter(|(_, _, key, _)| key == "act").count(), 1);
    }

    #[test]
    fn r1_failed_attempts_survive_reload_and_terminal_retry() {
        let f = fixture(vec![]);
        let runtime = rt();
        let mut terminal = None;
        for attempt in 1..=3 {
            let (driver, provider) = reload(
                f.tmp.path(),
                vec![scripted_text("not json")],
                LoopConfig::default(),
            );
            terminal = runtime
                .block_on(driver.run_case_until(
                    1,
                    "retry",
                    &CancellationToken::new(),
                    Some(1),
                    &EscapeFlag::new(),
                ))
                .unwrap();
            assert_eq!(provider.requests().len(), 1);
            let conn = Connection::open(f.tmp.path()).unwrap();
            let payload: String = conn.query_row("SELECT payload_json FROM agent_session_events WHERE kind='control:gdl' ORDER BY seq DESC LIMIT 1", [], |r| r.get(0)).unwrap();
            let value: serde_json::Value = serde_json::from_str(&payload).unwrap();
            assert_eq!(value["attempt"], attempt);
            assert!(!value["errors"].as_array().unwrap().is_empty());
            assert_eq!(terminal.is_some(), attempt == 3);
        }
        let (driver, provider) = reload(f.tmp.path(), happy_script(), LoopConfig::default());
        let retry = runtime
            .block_on(driver.run_case(1, "retry", &CancellationToken::new()))
            .unwrap();
        assert_eq!(Some(retry), terminal);
        assert!(provider.requests().is_empty());
    }

    #[test]
    fn r1_all_terminal_branches_replay_exactly_after_reload() {
        let runtime = rt();
        for branch in 0..7 {
            let mut script = happy_script();
            let mut config = LoopConfig::default();
            let cancel = CancellationToken::new();
            match branch {
                1 => script = vec![scripted_text("invalid"); 3],
                2 => {
                    script[1] = scripted_text(
                        r#"{"priority":"P3","stabilized":false,"search_hits":[],"verdict":"defer"}"#,
                    )
                }
                3 => {
                    script[5] =
                        scripted_text(&VERIFY_JSON.replace("\"pass\":true", "\"pass\":false"))
                }
                4 => cancel.cancel(),
                5 => config.max_turns = 0,
                6 => config.token_budget = Some(0),
                _ => {}
            }
            let f = fixture(vec![]);
            let (driver, _) = reload(f.tmp.path(), script, config.clone());
            let first = runtime
                .block_on(driver.run_case(1, "terminal", &cancel))
                .unwrap();
            assert!(
                match (&first, branch) {
                    (GdlOutcome::Resolved { .. }, 0)
                    | (GdlOutcome::Routed { .. }, 1)
                    | (GdlOutcome::Escalated { .. }, 2)
                    | (GdlOutcome::VerifyFailed { .. }, 3)
                    | (GdlOutcome::Canceled, 4) => true,
                    (GdlOutcome::Capped { reason, .. }, 5) => reason == "turn_cap",
                    (GdlOutcome::Capped { reason, .. }, 6) => reason == "budget",
                    _ => false,
                },
                "branch {branch}: {first:?}"
            );
            drop(driver);
            let (driver, provider) = reload(f.tmp.path(), happy_script(), config);
            let second = runtime
                .block_on(driver.run_case(1, "terminal", &CancellationToken::new()))
                .unwrap();
            assert_eq!(first, second);
            assert!(provider.requests().is_empty());
            assert!(
                runtime
                    .block_on(driver.run_case(1, "changed ticket", &CancellationToken::new()))
                    .is_err()
            );
        }
    }

    /// The registered reconciliation of "the machine never auto-closes"
    /// (docs/LOOP_AUTOCLOSE_RECONCILIATION.md): exhaustively drives every
    /// terminal path and asserts the only resolved terminal carries BOTH
    /// (a) a passing, law-clean verify artifact and (b) a present handoff
    /// capture. The illegal-closure arms enumerate the ways a case tries
    /// to close without the evidence; each lands in a named non-resolved
    /// terminal through the bounded machinery. No wildcard arms: a new
    /// GdlOutcome variant fails this test at compile time until its
    /// terminal path is reconciled here too.
    #[test]
    fn machine_never_auto_closes_resolved_requires_law_clean_verify_and_capture() {
        let runtime = rt();
        // (1) The evidence-gated closure: resolved carries both conditions.
        let f = fixture(happy_script());
        let outcome = runtime
            .block_on(f.driver.run_case(1, "close", &CancellationToken::new()))
            .unwrap();
        match outcome {
            GdlOutcome::Resolved {
                verify, capture, ..
            } => {
                assert!(verify.pass, "resolved without a passing verify artifact");
                assert!(
                    verify.re_run.contains("rebuild rate"),
                    "resolved without the planned failing scenario re-run"
                );
                assert!(
                    !capture.resolution.trim().is_empty(),
                    "resolved without a handoff capture"
                );
            }
            other => panic!("the happy script must close resolved, got {other:?}"),
        }
        // (2) Every illegal closure lands in a named non-resolved terminal.
        //     Slots: 5 = Verify, 6 = Handoff (the happy script is one
        //     artifact per phase). Routed arms retry the same artifact for
        //     the full bounded attempt budget.
        let illegal: [(&str, &str, &str, usize); 5] = [
            // A7 refused through the bounded attempts, then routed. (Slot 8
            // is the Handoff artifact; slots 6-7 are the recheck verdict and
            // the verify confirmation.)
            (
                "empty_capture_resolution",
                r#"{"capture":{"resolution":"   ","bundle_hash":"h0"}}"#,
                "routed",
                8,
            ),
            // A6 floor refused to closure.
            (
                "stability_window_under_floor",
                &VERIFY_JSON.replace("stability_window_min\":15", "stability_window_min\":5"),
                "routed",
                5,
            ),
            // A6 negative check refused to closure.
            (
                "negative_check_false",
                &VERIFY_JSON.replace("\"negative_check\":true", "\"negative_check\":false"),
                "routed",
                5,
            ),
            // L6 refused to closure: the re-run is not the planned scenario.
            (
                "re_run_not_the_planned_scenario",
                &VERIFY_JSON.replace(
                    "rebuild rate on VD 5 under the customer load",
                    "a different scenario entirely",
                ),
                "routed",
                5,
            ),
            // A failed verify is the terminal hand-back, never a closure.
            (
                "verify_pass_false",
                &VERIFY_JSON.replace("\"pass\":true", "\"pass\":false"),
                "verify_failed",
                5,
            ),
        ];
        for (name, artifact, expected, index) in illegal {
            let mut script = happy_script();
            *script.get_mut(index).expect("phase script slot") = scripted_text(artifact);
            if expected == "routed" {
                script.insert(index + 1, scripted_text(artifact));
                script.insert(index + 2, scripted_text(artifact));
            }
            let f = fixture(script);
            let outcome = runtime
                .block_on(f.driver.run_case(1, name, &CancellationToken::new()))
                .unwrap();
            assert!(
                !matches!(outcome, GdlOutcome::Resolved { .. }),
                "{name}: the machine auto-closed without the evidence"
            );
            let landed = match &outcome {
                GdlOutcome::Routed { .. } => "routed",
                GdlOutcome::Escalated { .. } => "escalated",
                GdlOutcome::VerifyFailed { .. } => "verify_failed",
                GdlOutcome::Canceled => "canceled",
                GdlOutcome::Capped { .. } => "capped",
                GdlOutcome::Resolved { .. } => "resolved",
            };
            assert_eq!(
                landed, expected,
                "{name}: terminal {landed} is not the named shape {expected}"
            );
        }
    }

    /// The consumption-limit law at the case boundary: a zero token ceiling
    /// stops the case at the FIRST exchange boundary — zero provider work,
    /// zero turns — and the durable Capped terminal is RECEIPT-BACKED (the
    /// checkpoint law): the refused exchange opened, ended BudgetExceeded,
    /// and its done receipt binds the cap. The cap is never asserted
    /// without an evidencing exchange receipt.
    #[test]
    fn budget_capped_cases_are_receipt_backed_never_silent() {
        let runtime = rt();
        let f = fixture(vec![]);
        let config = LoopConfig {
            token_budget: Some(0),
            ..LoopConfig::default()
        };
        let (driver, provider) = reload(f.tmp.path(), vec![], config);
        let outcome = runtime
            .block_on(driver.run_case(1, "admission", &CancellationToken::new()))
            .unwrap();
        match outcome {
            GdlOutcome::Capped { reason, .. } => assert_eq!(reason, "budget"),
            other => panic!("a zero budget caps the case, got {other:?}"),
        }
        assert!(
            provider.requests().is_empty(),
            "the refusal precedes all provider work"
        );
        let (opened, done_budget): (i64, i64) = Connection::open(f.tmp.path())
            .unwrap()
            .query_row(
                "SELECT
                   (SELECT COUNT(*) FROM agent_session_events
                    WHERE run_id = 1 AND kind = 'control:exchange'),
                   (SELECT COUNT(*) FROM agent_session_events
                    WHERE run_id = 1 AND kind = 'control:exchange_done'
                      AND payload_json LIKE '%BudgetExceeded%')",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(opened, 1, "the cap is receipt-backed by a real exchange");
        assert_eq!(
            done_budget, 1,
            "the exchange's done receipt records the BudgetExceeded stop"
        );
    }

    /// The second verification's own law: a second pass that DISAGREES (a
    /// different planned re-run) is the named A6 gate failure; the bounded
    /// retries re-run the whole phase (fresh pass + fresh confirmation) and
    /// the case routes at exhaustion — never resolves. Each confirmation is
    /// a real, distinct exchange (`:confirm` request key) on the session log.
    #[test]
    fn resolved_requires_a_second_agreeing_verification_exchange() {
        let runtime = rt();
        let disagree = VERIFY_JSON.replace("rebuild rate on VD 5", "a different re-run");
        let mut script = happy_script();
        script.truncate(5); // through Act
        for _ in 0..MAX_PHASE_ATTEMPTS {
            script.push(scripted_text(VERIFY_JSON)); // the passing first pass
            script.push(scripted_text(RECHECK_JSON)); // the recheck finds no falsifier
            script.push(scripted_text(&disagree)); // the disagreeing confirmation
        }
        script.push(scripted_text(HANDOFF_JSON)); // never reached
        let f = fixture(script);
        let outcome = runtime
            .block_on(f.driver.run_case(1, "disagree", &CancellationToken::new()))
            .unwrap();
        match outcome {
            GdlOutcome::Routed { at, reason } => {
                assert_eq!(at, GdlPhase::Verify);
                assert!(
                    reason.contains("gate exhausted"),
                    "the disagreement exhausts the bounded retries: {reason}"
                );
            }
            other => panic!("a disagreeing second verification must never resolve: {other:?}"),
        }
        let conn = Connection::open(f.tmp.path()).unwrap();
        let confirm_exchanges: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_session_events
                 WHERE kind = 'control:exchange' AND idempotency_key LIKE '%:confirm'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            confirm_exchanges, 3,
            "each retried Verify re-asks a distinct confirmation exchange"
        );
        let named: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_session_events
                 WHERE kind = 'gdl_gate' AND payload_json LIKE '%second verification%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            named, 3,
            "the A6 second-verification failure is named on every gate row"
        );
    }

    /// C3: a contradictory re-check verdict is the named A4 gate failure —
    /// the bounded retries re-run the whole phase and the case routes at
    /// exhaustion, never resolves. Every delegation lands a typed row.
    #[test]
    fn adversarial_contradiction_is_a_named_gate_failure() {
        let runtime = rt();
        let contradiction = r#"{"contradicted":true,"reason":"the SEL shows the battery healthy"}"#;
        let mut script = happy_script();
        script.truncate(5); // through Act
        for _ in 0..MAX_PHASE_ATTEMPTS {
            script.push(scripted_text(VERIFY_JSON)); // the passing first pass
            script.push(scripted_text(contradiction)); // the child falsifies it
        }
        script.push(scripted_text(HANDOFF_JSON)); // never reached
        let f = fixture(script);
        let outcome = runtime
            .block_on(f.driver.run_case(1, "falsified", &CancellationToken::new()))
            .unwrap();
        match outcome {
            GdlOutcome::Routed { at, reason } => {
                assert_eq!(at, GdlPhase::Verify);
                assert!(reason.contains("gate exhausted"), "{reason}");
            }
            other => panic!("a contradicted hypothesis must never resolve: {other:?}"),
        }
        let conn = Connection::open(f.tmp.path()).unwrap();
        let rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_session_events
                 WHERE kind = 'control:adversarial_recheck'
                   AND payload_json LIKE '%contradicted%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rows, 3, "one typed row per delegation");
    }

    /// C3's honest degradation: an off-contract re-check verdict (the
    /// child answered, but not with the agreed JSON shape) is typed
    /// UNAVAILABLE — recorded, non-blocking — and the case still resolves
    /// on its evidence.
    #[test]
    fn adversarial_unavailable_degrades_honestly_nonblocking() {
        let runtime = rt();
        let mut script = happy_script();
        // The child answered in prose, not the agreed JSON shape.
        script[6] = scripted_text("no contradiction found after reviewing the evidence");
        let f = fixture(script);
        let outcome = runtime
            .block_on(
                f.driver
                    .run_case(1, "no recheck child", &CancellationToken::new()),
            )
            .unwrap();
        assert!(
            matches!(outcome, GdlOutcome::Resolved { .. }),
            "an unavailable re-check never blocks the evidence-gated closure: {outcome:?}"
        );
        let conn = Connection::open(f.tmp.path()).unwrap();
        let rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_session_events
                 WHERE kind = 'control:adversarial_recheck'
                   AND payload_json LIKE '%unavailable%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rows, 1, "the degradation is visible in the row");
    }

    /// The re-check posture is caller-narrowable: with the flag off there
    /// is no delegation, no typed rows, and the confirmation follows the
    /// pass directly.
    #[test]
    fn adversarial_recheck_off_never_delegates() {
        let runtime = rt();
        let script = vec![
            scripted_text(INTAKE_JSON),
            scripted_text(TRIAGE_JSON),
            scripted_text(HYPOTHESIZE_JSON),
            scripted_text(PLAN_JSON),
            scripted_text(ACT_JSON),
            scripted_text(VERIFY_JSON),
            scripted_text(VERIFY_JSON), // the confirmation, no recheck in between
            scripted_text(HANDOFF_JSON),
        ];
        let f = fixture(vec![]);
        let config = LoopConfig {
            adversarial_recheck: false,
            ..LoopConfig::default()
        };
        let (driver, _) = reload(f.tmp.path(), script, config);
        let outcome = runtime
            .block_on(driver.run_case(1, "no recheck", &CancellationToken::new()))
            .unwrap();
        assert!(
            matches!(outcome, GdlOutcome::Resolved { .. }),
            "{outcome:?}"
        );
        let conn = Connection::open(f.tmp.path()).unwrap();
        let rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_session_events
                 WHERE kind = 'control:adversarial_recheck'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rows, 0, "the flag off means no delegation, no rows");
    }

    // ── the failure drills (the Keel set) ──────────────────────────────

    /// The soft-handoff law, pure: fires at the integer threshold, exactly
    /// once per case, and a handoff after the fire without a recorded
    /// justification is the named violation.
    #[test]
    fn soft_handoff_fires_once_and_demands_justification() {
        let mut latch = SoftHandoffLatch::default();
        assert!(!soft_handoff(79), "79 is under the integer threshold");
        assert!(soft_handoff(80), "80 is the threshold, integer-only");
        assert!(matches!(latch.evaluate(100, None), Ok(true)), "first fire");
        assert!(latch.fired);
        // A justified handoff after the fire is legal.
        assert!(
            matches!(
                latch.evaluate(100, Some("customer asked to defer; bundle attached")),
                Ok(true)
            ),
            "a justified handoff after the fire is legal"
        );
        // An unjustified handoff after the fire is the named violation.
        assert_eq!(
            latch.evaluate(100, None),
            Err("soft-handoff fired without justification".into()),
        );
        assert_eq!(
            latch.evaluate(100, Some("   ")),
            Err("soft-handoff fired without justification".into()),
            "a blank justification is no justification"
        );
    }

    /// C7a — sustained write volume: a pinned burst of case episodes keeps
    /// the audit chain green and every terminal durable.
    #[test]
    fn drill_sustained_writes_keep_the_chain_green() {
        let f = fixture(vec![]);
        let runtime = rt();
        {
            let conn = Connection::open(f.tmp.path()).unwrap();
            for run_id in 2..=4 {
                conn.execute(
                    "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
                     VALUES ('acme', 'troubleshoot', '{}', 0, 'active', 1, 1)",
                    [],
                )
                .unwrap();
                let _ = run_id;
            }
        }
        for run_id in 1..=4 {
            let (driver, provider) = reload(f.tmp.path(), happy_script(), LoopConfig::default());
            let outcome = runtime
                .block_on(driver.run_case(
                    run_id,
                    &format!("burst {run_id}"),
                    &CancellationToken::new(),
                ))
                .unwrap();
            assert!(
                matches!(outcome, GdlOutcome::Resolved { .. }),
                "burst case {run_id} resolved: {outcome:?}"
            );
            assert!(!provider.requests().is_empty());
        }
        let conn = Connection::open(f.tmp.path()).unwrap();
        assert!(
            crate::audit::verify_chain(&conn),
            "the audit chain verifies green across the burst"
        );
        let resolved: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM workflow_runs WHERE status = 'resolved'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(resolved, 4, "every burst episode settled durably");
    }

    /// C7b — DSAR across workflow data: the workflow's publication law IS
    /// the erasure posture. A resolved run's capture lands as a PENDING
    /// proposal (never knowledge), so the knowledge-side legal-hold
    /// machinery governs everything erasable and nothing erasable escapes
    /// it. The workflow session log is append-only by law — the r1
    /// checkpoint family refuses tampering — so there is no run-row
    /// erasure path to refuse: named limit, recorded.
    #[test]
    fn drill_run_capture_stays_proposal_only_under_dsar_posture() {
        let f = fixture(happy_script());
        seed_repeater_priors(f.tmp.path(), 2);
        let runtime = rt();
        let outcome = runtime
            .block_on(f.driver.run_case(1, "dsar", &CancellationToken::new()))
            .unwrap();
        assert!(matches!(outcome, GdlOutcome::Resolved { .. }));
        let conn = Connection::open(f.tmp.path()).unwrap();
        let knowledge: i64 = conn
            .query_row("SELECT COUNT(*) FROM knowledge", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            knowledge, 0,
            "a resolved run publishes nothing erasable — capture is proposal-only"
        );
        let pending: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM proposals WHERE status = 'pending'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(pending >= 1, "the capture sits as a pending proposal");
        assert!(
            crate::audit::verify_chain(&conn),
            "the capture trail stays on the green chain"
        );
    }

    /// The escalation path records the soft-handoff evaluation: an
    /// escalated case (no passing verify — confidence 0) lands the typed
    /// row with the predicate's verdict for the operator.
    #[test]
    fn escalation_records_the_soft_handoff_evaluation() {
        let runtime = rt();
        let mut script = happy_script();
        script[1] = scripted_text(
            r#"{"priority":"P3","stabilized":false,"search_hits":[],"verdict":"defer"}"#,
        );
        let f = fixture(script);
        let outcome = runtime
            .block_on(f.driver.run_case(1, "defer", &CancellationToken::new()))
            .unwrap();
        assert!(
            matches!(outcome, GdlOutcome::Escalated { .. }),
            "{outcome:?}"
        );
        let conn = Connection::open(f.tmp.path()).unwrap();
        let (rows, fires): (i64, String) = conn
            .query_row(
                "SELECT COUNT(*), COALESCE(MAX(CASE WHEN payload_json LIKE '%\"fires\":true%' THEN 'true' ELSE 'false' END), 'none')
                 FROM agent_session_events WHERE kind = 'control:soft_handoff'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(rows, 1, "one typed soft-handoff row per escalated case");
        assert_eq!(fires, "false", "no passing verify — the predicate is false");
    }

    #[test]
    fn escalated_case_lands_exactly_one_prefilled_offer_draft() {
        let runtime = rt();
        let mut script = happy_script();
        script[1] = scripted_text(
            r#"{"priority":"P3","stabilized":false,"search_hits":[],"verdict":"defer"}"#,
        );
        let f = fixture(script);
        let outcome = runtime
            .block_on(f.driver.run_case(1, "defer", &CancellationToken::new()))
            .unwrap();
        assert!(
            matches!(outcome, GdlOutcome::Escalated { .. }),
            "{outcome:?}"
        );
        let conn = Connection::open(f.tmp.path()).unwrap();
        let (offers, states): (i64, String) = conn
            .query_row(
                "SELECT COUNT(*), COALESCE(GROUP_CONCAT(state), 'none') \
                 FROM handover_offers WHERE run_id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(offers, 1, "exactly one offer draft per escalated case");
        assert_eq!(states, "offered", "the offer is a DRAFT: a human decides");
        let (to_principal, decided_at): (String, Option<i64>) = conn
            .query_row(
                "SELECT to_principal, decided_at FROM handover_offers WHERE run_id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            to_principal, "operator",
            "a Triage defer has no declared dead-end yet — the draft goes \
             to the operator queue"
        );
        assert!(
            decided_at.is_none(),
            "the machine never decides — HITL by construction"
        );
        // The typed row names the packet gaps (the coaching surface).
        let (typed,): (String,) = conn
            .query_row(
                "SELECT payload_json FROM agent_session_events \
                 WHERE kind = 'control:handoff_offer' ORDER BY seq DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?,)),
            )
            .unwrap();
        let typed: serde_json::Value = serde_json::from_str(&typed).unwrap();
        let missing: Vec<String> = typed["missing"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            !missing.is_empty(),
            "an early escalation names its packet gaps: {missing:?}"
        );
        assert!(
            missing.iter().all(|m| m.contains("situation:")
                || m.contains("action:")
                || m.contains("safety:")),
            "the missing list names the unanswered I-PASS questions: {missing:?}"
        );
        assert!(
            typed["offer_id"].as_i64().is_some(),
            "the row binds the offer id: {typed}"
        );
    }

    #[test]
    fn escape_armed_before_the_run_routes_at_the_first_boundary() {
        let f = fixture(happy_script());
        let escape = EscapeFlag::new();
        escape.request();
        let outcome = rt()
            .block_on(f.driver.run_case_with_escape(
                1,
                "escape",
                &CancellationToken::new(),
                &escape,
            ))
            .unwrap();
        // The Intake exchange settles, then the FIRST boundary routes: the
        // gate never runs, so the intake artifact was never applied and
        // the bundle is honestly incomplete.
        assert!(
            matches!(
                &outcome,
                GdlOutcome::Escalated {
                    at: GdlPhase::Intake,
                    bundle,
                    ..
                } if !bundle.complete
            ),
            "{outcome:?}"
        );
        assert_eq!(
            f.provider.requests().len(),
            1,
            "one exchange to the first settle, then the escape routes"
        );
        // The reason rides the gate record verbatim.
        let conn = Connection::open(f.tmp.path()).unwrap();
        let (gate,): (String,) = conn
            .query_row(
                "SELECT payload_json FROM agent_session_events \
                 WHERE kind = 'gdl_gate' ORDER BY seq DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?,)),
            )
            .unwrap();
        assert!(gate.contains("operator escape"), "{gate}");
        // Durable monotonicity: the committed terminal replays exactly.
        let (driver, _provider) = reload(f.tmp.path(), vec![], LoopConfig::default());
        let replayed = rt()
            .block_on(driver.run_case_with_escape(
                1,
                "escape",
                &CancellationToken::new(),
                &EscapeFlag::new(),
            ))
            .unwrap();
        assert_eq!(replayed, outcome, "the escape terminal is replay-exact");
    }

    #[test]
    fn escape_mid_act_routes_at_the_next_boundary_with_no_further_exchanges() {
        let f = fixture(vec![]);
        let runtime = rt();
        let mut script = happy_script().into_iter();
        // Intake..Act settle on the first driver; the escape is raised
        // while the case is paused; the resumed driver carries Verify's
        // turn only.
        let first: Vec<Vec<crate::agentloop::provider::StreamEvent>> =
            (0..5).map(|_| script.next().unwrap()).collect();
        let (driver, _provider) = reload(f.tmp.path(), first, LoopConfig::default());
        let paused = runtime
            .block_on(driver.run_case_until(
                1,
                "mid-act",
                &CancellationToken::new(),
                Some(5),
                &EscapeFlag::new(),
            ))
            .unwrap();
        assert!(paused.is_none(), "clean pause after Act");
        drop(driver);
        let escape = EscapeFlag::new();
        escape.request();
        let (driver2, provider2) = reload(
            f.tmp.path(),
            vec![script.next().unwrap()],
            LoopConfig::default(),
        );
        let outcome = runtime
            .block_on(driver2.run_case_with_escape(
                1,
                "mid-act",
                &CancellationToken::new(),
                &escape,
            ))
            .unwrap();
        // The Verify exchange settles; the escape preempts the gate — the
        // adversarial re-check and the second verification never run.
        assert!(
            matches!(
                &outcome,
                GdlOutcome::Escalated {
                    at: GdlPhase::Verify,
                    bundle,
                    ..
                } if bundle.hypotheses.len() == 1 && bundle.test_log_rows == 2
            ),
            "{outcome:?}"
        );
        assert_eq!(
            provider2.requests().len(),
            1,
            "the escape consumes no exchanges beyond the settling phase"
        );
        // The escape rides the existing escalation law: the soft-handoff
        // evaluation is recorded at the terminal.
        let conn = Connection::open(f.tmp.path()).unwrap();
        let (rows,): (i64,) = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_session_events \
                 WHERE kind = 'control:soft_handoff'",
                [],
                |r| Ok((r.get(0)?,)),
            )
            .unwrap();
        assert_eq!(rows, 1, "the escalated escape records its soft-handoff row");
    }

    #[test]
    fn unjustified_revisit_lands_denied_audit_and_gap_proposal() {
        let f = fixture(vec![]);
        let runtime = rt();
        let mut case = GdlCase::fresh("t");
        // A passing verify maps to confidence 100 — the predicate fires.
        case.verify = Some(serde_json::from_str(VERIFY_JSON).unwrap());
        // First evaluation: the latch fires, no violation (nothing prior).
        runtime
            .block_on(f.driver.record_soft_handoff_row(1, "first", &case))
            .unwrap();
        // The revisit: fired again, still no recorded justification — the
        // named violation lands a DENIED audit row and a gap proposal.
        runtime
            .block_on(f.driver.record_soft_handoff_row(1, "second", &case))
            .unwrap();
        let conn = Connection::open(f.tmp.path()).unwrap();
        let (rows,): (i64,) = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_session_events \
                 WHERE kind = 'control:soft_handoff'",
                [],
                |r| Ok((r.get(0)?,)),
            )
            .unwrap();
        assert_eq!(rows, 2, "two fired evaluations recorded");
        let (denials,): (i64,) = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events \
                 WHERE actor = 'workflow' AND status = 'denied' AND detail_hash = ?1",
                [crate::audit::hash(SOFT_HANDOFF_VIOLATION)],
                |r| Ok((r.get(0)?,)),
            )
            .unwrap();
        assert_eq!(
            denials, 1,
            "exactly the revisit is denied — the first fire was lawful"
        );
        let (gap,): (String,) = conn
            .query_row(
                "SELECT payload_json FROM agent_session_events \
                 WHERE kind = 'control:soft_handoff_gap'",
                [],
                |r| Ok((r.get(0)?,)),
            )
            .unwrap();
        assert!(
            gap.contains("soft-handoff fired without justification"),
            "{gap}"
        );
        assert!(
            gap.contains("ProposeNew"),
            "no coverage proposes a new article: {gap}"
        );
    }

    #[test]
    fn ipass_facts_prefill_maps_case_state_and_never_writes_synthesis() {
        let now = 1_000;
        let mut case = GdlCase::fresh("t");
        // Nothing yet: the packet names the gaps honestly.
        let facts = ipass_facts(&case, None, now);
        assert_eq!(facts.pending_question, None);
        assert_eq!(facts.sla_deadline, None);
        assert!(!facts.has_current_step);
        assert!(!facts.has_evidence);
        assert!(facts.escalation_honored);
        let missing = crate::workflow::relay::packet_missing(&facts);
        assert_eq!(missing.len(), 4, "the coaching surface names every gap");
        // After Plan+Act: the sender sections are filled from the case.
        case.plan = vec![PlanStep {
            order: 1,
            kind: "check".into(),
            skill_gate: "L1".into(),
            description: "d".into(),
            command: "c".into(),
            expected: "e".into(),
            fail_action: None,
            seam: None,
            invasiveness: 0,
            justification: None,
        }];
        case.test_log = vec![TestLogRow::default()];
        case.verify_step = Some(VerifyStepSpec {
            re_run: "rebuild rate under load".into(),
            pass_condition: ">10%/h".into(),
        });
        let facts = ipass_facts(&case, Some(2_000), now);
        assert_eq!(
            facts.pending_question.as_deref(),
            Some("rebuild rate under load"),
            "the open question is the planned re-run"
        );
        assert_eq!(facts.sla_deadline, Some(2_000));
        assert!(facts.has_current_step);
        assert!(facts.has_evidence);
        assert_eq!(
            crate::workflow::relay::packet_missing(&facts).len(),
            0,
            "a planned, evidenced, SLA-armed case offers complete"
        );
        // The address: the plan's declared target, else the operator queue.
        assert_eq!(escalation_target(&case), "operator");
        case.dead_end = Some(DeadEnd {
            escalate_to: "eng-storage".into(),
            required_evidence: vec!["TSR".into()],
        });
        assert_eq!(escalation_target(&case), "eng-storage");
    }

    #[test]
    fn sla_clock_table_pins_the_p_class_literals() {
        assert_eq!(sla_seconds("P1"), Some(3_600), "P1 → 1 hour");
        assert_eq!(sla_seconds("P2"), Some(14_400), "P2 → 4 hours");
        assert_eq!(sla_seconds("P3"), Some(86_400), "P3 → 24 hours");
        assert_eq!(sla_seconds("P4"), Some(604_800), "P4 → 168 hours");
        assert_eq!(sla_seconds("P5"), None, "the gate admits only P1..P4");
        assert_eq!(sla_seconds(""), None);
        assert_eq!(sla_seconds("p1"), None, "exact literals only");
    }

    #[test]
    fn sla_clock_arms_at_triage_on_a_typed_row() {
        let runtime = rt();
        let mut script = happy_script();
        script[1] = scripted_text(
            r#"{"priority":"P1","stabilized":true,"search_hits":["P-STORAGE-0104"],"verdict":"accept"}"#,
        );
        let f = fixture(script);
        let outcome = runtime
            .block_on(f.driver.run_case(1, "P1 clock", &CancellationToken::new()))
            .unwrap();
        assert!(
            matches!(outcome, GdlOutcome::Resolved { .. }),
            "a P1 triage pass resolves; {outcome:?}"
        );
        let conn = Connection::open(f.tmp.path()).unwrap();
        let (rows, payload): (i64, String) = conn
            .query_row(
                "SELECT COUNT(*), COALESCE(MAX(payload_json), 'none') \
                 FROM agent_session_events WHERE kind = 'control:sla_armed'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(rows, 1, "the Triage pass arms the clock: one typed row");
        assert!(payload.contains("\"priority\":\"P1\""), "{payload}");
        let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
        let deadline = payload["sla_deadline_epoch"].as_i64().unwrap();
        let (armed_at,): (i64,) = conn
            .query_row(
                "SELECT created_at FROM agent_session_events WHERE kind = 'control:sla_armed'",
                [],
                |r| Ok((r.get(0)?,)),
            )
            .unwrap();
        let window = deadline - armed_at;
        assert_eq!(
            window, 3_600,
            "P1 window is exactly the pinned hour — \
            the row's stamp and deadline come from ONE clock read"
        );
    }

    #[test]
    fn r1_terminal_retry_refuses_changed_loop_policy() {
        let f = fixture(happy_script());
        let runtime = rt();
        runtime
            .block_on(f.driver.run_case(1, "policy", &CancellationToken::new()))
            .unwrap();
        let changed = LoopConfig {
            max_turns: 1,
            ..LoopConfig::default()
        };
        let (driver, provider) = reload(f.tmp.path(), happy_script(), changed);
        assert!(
            runtime
                .block_on(driver.run_case(1, "policy", &CancellationToken::new()))
                .is_err()
        );
        assert!(provider.requests().is_empty());
        let (mut driver, provider) = reload(f.tmp.path(), happy_script(), LoopConfig::default());
        driver
            .loop_driver
            .set_retry_spec("changed provider implementation".into());
        assert!(
            runtime
                .block_on(driver.run_case(1, "policy", &CancellationToken::new()))
                .is_err()
        );
        assert!(provider.requests().is_empty());
    }

    #[test]
    fn r1_checkpoint_receipt_and_gate_corruption_refuses() {
        let runtime = rt();
        for corruption in [
            "outcome",
            "exchange",
            "start_kind",
            "done_kind",
            "gate_exchange",
            "assistant",
            "gate_artifact",
        ] {
            let f = fixture(happy_script());
            let cancel = CancellationToken::new();
            if corruption == "outcome" {
                cancel.cancel();
            }
            runtime
                .block_on(f.driver.run_case(1, "receipt", &cancel))
                .unwrap();
            let conn = Connection::open(f.tmp.path()).unwrap();
            let (key, payload): (String, String) = conn.query_row("SELECT idempotency_key, payload_json FROM agent_session_events WHERE kind='control:gdl' ORDER BY seq DESC LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
            let mut cp: serde_json::Value = serde_json::from_str(&payload).unwrap();
            let exchange = cp["exchange"].as_i64().unwrap();
            match corruption {
                "outcome" => {
                    cp["terminal"] =
                        serde_json::json!({"Capped":{"at":"Intake","reason":"turn_cap"}});
                    conn.execute(
                        "UPDATE agent_session_events SET payload_json=?1 WHERE idempotency_key=?2",
                        rusqlite::params![cp.to_string(), key],
                    )
                    .unwrap();
                }
                "exchange" => {
                    cp["exchange"] = serde_json::json!(999999);
                    conn.execute(
                        "UPDATE agent_session_events SET payload_json=?1 WHERE idempotency_key=?2",
                        rusqlite::params![cp.to_string(), key],
                    )
                    .unwrap();
                }
                "start_kind" => {
                    conn.execute(
                        "UPDATE agent_session_events SET kind='user' WHERE run_id=1 AND seq=?1",
                        [exchange],
                    )
                    .unwrap();
                }
                "done_kind" => {
                    conn.execute(
                        "UPDATE agent_session_events SET kind='user' WHERE idempotency_key=?1",
                        [format!("control:exchange_done:{exchange}")],
                    )
                    .unwrap();
                }
                "assistant" => {
                    conn.execute("UPDATE agent_session_events SET payload_json='{}' WHERE idempotency_key=?1", [format!("run1:a{exchange}:t1")]).unwrap();
                }
                _ => {
                    let gate_key = format!("{key}:gate");
                    let payload: String = conn.query_row("SELECT payload_json FROM agent_session_events WHERE idempotency_key=?1", [&gate_key], |r| r.get(0)).unwrap();
                    let mut gate: serde_json::Value = serde_json::from_str(&payload).unwrap();
                    if corruption == "gate_exchange" {
                        gate["exchange"] = serde_json::json!(999999);
                    } else {
                        gate["artifact"] = serde_json::json!("{}");
                    }
                    conn.execute(
                        "UPDATE agent_session_events SET payload_json=?1 WHERE idempotency_key=?2",
                        rusqlite::params![gate.to_string(), gate_key],
                    )
                    .unwrap();
                }
            }
            let (driver, provider) = reload(f.tmp.path(), happy_script(), LoopConfig::default());
            assert!(
                runtime
                    .block_on(driver.run_case(1, "receipt", &CancellationToken::new()))
                    .is_err(),
                "must refuse {corruption}"
            );
            assert!(provider.requests().is_empty());
        }
    }

    #[test]
    fn r1_checked_seal_audit_failure_rolls_back_checkpoint() {
        let f = fixture(happy_script());
        let runtime = rt();
        runtime
            .block_on(f.driver.run_case_until(
                1,
                "audit",
                &CancellationToken::new(),
                Some(2),
                &EscapeFlag::new(),
            ))
            .unwrap();
        let conn = Connection::open(f.tmp.path()).unwrap();
        let before = super::super::state::read_state_and_revision(&conn, 1).unwrap();
        let rows = step_rows(f.tmp.path());
        conn.execute_batch("CREATE TRIGGER refuse_audit BEFORE INSERT ON audit_events WHEN NEW.actor='workflow' AND NEW.status='ok' AND EXISTS(SELECT 1 FROM workflow_steps WHERE phase='hypothesize') BEGIN SELECT RAISE(ABORT, 'audit test'); END;").unwrap();
        let (driver, provider) = reload(
            f.tmp.path(),
            vec![scripted_text(HYPOTHESIZE_JSON)],
            LoopConfig::default(),
        );
        assert!(
            runtime
                .block_on(driver.run_case(1, "audit", &CancellationToken::new()))
                .is_err()
        );
        assert_eq!(provider.requests().len(), 1);
        assert_eq!(
            super::super::state::read_state_and_revision(&conn, 1).unwrap(),
            before
        );
        assert_eq!(step_rows(f.tmp.path()), rows);
        let (gates, findings, denials, denied_audits): (i64, i64, i64, i64) = conn.query_row("SELECT (SELECT COUNT(*) FROM agent_session_events WHERE kind='gdl_gate'), (SELECT COUNT(*) FROM findings), (SELECT COUNT(*) FROM agent_session_events WHERE kind='control:gdl_denied'), (SELECT COUNT(*) FROM audit_events WHERE actor='workflow' AND status='denied')", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).unwrap();
        assert_eq!((gates, findings, denials, denied_audits), (2, 0, 1, 1));
    }

    #[test]
    fn r1_denial_audit_failure_preserves_original_error() {
        let f = fixture(vec![]);
        let mut conn = Connection::open(f.tmp.path()).unwrap();
        let cp = checkpoint::admit(&mut conn, 1, "t", "test", "owner").unwrap();
        let exchange = fixture_exchange(&mut conn, &cp, GdlPhase::Intake, 1, INTAKE_JSON, "owner");
        conn.execute("UPDATE workflow_runs SET status='cancelled' WHERE id=1", [])
            .unwrap();
        conn.execute_batch("CREATE TRIGGER refuse_all_audit BEFORE INSERT ON audit_events BEGIN SELECT RAISE(ABORT, 'audit test'); END;").unwrap();
        let err = checkpoint::advance(
            &mut conn,
            &cp,
            "owner",
            checkpoint::Transition {
                phase: GdlPhase::Intake,
                attempt: 1,
                verdict: "pass",
                errors: vec![],
                artifact: Some(INTAKE_JSON.into()),
                exchange: Some(exchange),
                terminal: None,
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, LoopError::Persist(ref message) if message.contains("binding changed")),
            "original refusal must survive: {err:?}"
        );
        let denials: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_session_events WHERE kind='control:gdl_denied'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            denials, 0,
            "failed checked denial audit rolls back its journal row"
        );
    }

    #[test]
    fn r1_terminal_binding_refuses_human_changes() {
        let runtime = rt();
        for update in [
            "status='cancelled'",
            "state_revision=state_revision+1",
            "state_json='{}'",
            "domain='other'",
            "kind='interview'",
        ] {
            let f = fixture(happy_script());
            runtime
                .block_on(f.driver.run_case(1, "t", &CancellationToken::new()))
                .unwrap();
            let conn = Connection::open(f.tmp.path()).unwrap();
            conn.execute(&format!("UPDATE workflow_runs SET {update} WHERE id=1"), [])
                .unwrap();
            let (driver, provider) = reload(f.tmp.path(), happy_script(), LoopConfig::default());
            assert!(
                runtime
                    .block_on(driver.run_case(1, "t", &CancellationToken::new()))
                    .is_err()
            );
            assert!(provider.requests().is_empty());
        }
    }

    #[test]
    fn r1_malformed_checkpoint_refuses_without_fallback() {
        let runtime = rt();
        for corrupt in ["null", "{}", "[]", "{", "version", "terminal"] {
            let f = fixture(happy_script());
            runtime
                .block_on(f.driver.run_case(1, "t", &CancellationToken::new()))
                .unwrap();
            let conn = Connection::open(f.tmp.path()).unwrap();
            let payload: String = conn.query_row("SELECT payload_json FROM agent_session_events WHERE kind='control:gdl' ORDER BY seq DESC LIMIT 1", [], |r| r.get(0)).unwrap();
            let replacement = if matches!(corrupt, "version" | "terminal") {
                let mut value: serde_json::Value = serde_json::from_str(&payload).unwrap();
                if corrupt == "version" {
                    value["version"] = serde_json::json!(99);
                } else {
                    value["terminal"] = serde_json::json!("Canceled");
                }
                value.to_string()
            } else {
                corrupt.into()
            };
            conn.execute("UPDATE agent_session_events SET payload_json=?1 WHERE seq=(SELECT MAX(seq) FROM agent_session_events WHERE kind='control:gdl')", [replacement]).unwrap();
            let (driver, provider) = reload(f.tmp.path(), happy_script(), LoopConfig::default());
            assert!(
                runtime
                    .block_on(driver.run_case(1, "t", &CancellationToken::new()))
                    .is_err()
            );
            assert!(provider.requests().is_empty());
        }
    }

    #[test]
    fn r1_checkpoint_trigger_failure_rolls_back_and_retains_claim() {
        let f = fixture(happy_script());
        let runtime = rt();
        runtime
            .block_on(f.driver.run_case_until(
                1,
                "t",
                &CancellationToken::new(),
                Some(2),
                &EscapeFlag::new(),
            ))
            .unwrap();
        let conn = Connection::open(f.tmp.path()).unwrap();
        let before = super::super::state::read_state_and_revision(&conn, 1).unwrap();
        let rows = step_rows(f.tmp.path());
        let gates: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_session_events WHERE kind='gdl_gate'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        conn.execute_batch("CREATE TRIGGER refuse_gdl BEFORE INSERT ON agent_session_events WHEN NEW.kind='control:gdl' BEGIN SELECT RAISE(ABORT, 'checkpoint test'); END;").unwrap();
        let (driver, provider) = reload(
            f.tmp.path(),
            vec![scripted_text(HYPOTHESIZE_JSON)],
            LoopConfig::default(),
        );
        assert!(
            runtime
                .block_on(driver.run_case(1, "t", &CancellationToken::new()))
                .is_err()
        );
        assert_eq!(provider.requests().len(), 1);
        assert_eq!(
            super::super::state::read_state_and_revision(&conn, 1).unwrap(),
            before
        );
        assert_eq!(step_rows(f.tmp.path()), rows);
        let (after, findings, denials): (i64, i64, i64) = conn.query_row("SELECT (SELECT COUNT(*) FROM agent_session_events WHERE kind='gdl_gate'), (SELECT COUNT(*) FROM findings), (SELECT COUNT(*) FROM agent_session_events WHERE kind='control:gdl_denied')", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
        assert_eq!((after, findings, denials), (gates, 0, 1));
        conn.execute_batch("DROP TRIGGER refuse_gdl;").unwrap();
        let (retry, provider) = reload(f.tmp.path(), happy_script(), LoopConfig::default());
        assert!(
            runtime
                .block_on(retry.run_case(1, "t", &CancellationToken::new()))
                .is_err()
        );
        assert!(provider.requests().is_empty());
        assert!(verify_chain(&conn));
    }

    #[test]
    fn r1_nonexpiring_claim_excludes_independent_driver() {
        let f = fixture(vec![]);
        let mut conn = Connection::open(f.tmp.path()).unwrap();
        let cp =
            checkpoint::admit(&mut conn, 1, "t", "gdl-v1:L3:ablated=false", "old-owner").unwrap();
        let (driver, provider) = reload(f.tmp.path(), happy_script(), LoopConfig::default());
        assert!(
            rt().block_on(driver.run_case(1, "t", &CancellationToken::new()))
                .is_err()
        );
        assert!(provider.requests().is_empty());
        let mut tx = super::super::tx::WorkflowTx::begin(&mut conn).unwrap();
        cp.verify(tx.tx(), "old-owner").unwrap();
    }

    #[test]
    fn r1_terminal_retry_is_exact_without_provider_work() {
        let f = fixture(happy_script());
        let runtime = rt();
        let first = runtime
            .block_on(f.driver.run_case(1, "retry", &CancellationToken::new()))
            .unwrap();
        let calls = f.provider.requests().len();
        let second = runtime
            .block_on(f.driver.run_case(1, "retry", &CancellationToken::new()))
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(f.provider.requests().len(), calls);
    }

    #[test]
    fn r1_empty_state_with_history_refuses() {
        for steps_only in [false, true] {
            let f = fixture(happy_script());
            let conn = Connection::open(f.tmp.path()).unwrap();
            if steps_only {
                conn.execute("INSERT INTO workflow_steps(run_id, phase, step_key, state_json) VALUES (1, 'act', 'old', '{}')", []).unwrap();
            } else {
                session_log::append(&conn, 1, "user", "{}", "old", 1).unwrap();
            }
            assert!(
                rt().block_on(f.driver.run_case(1, "retry", &CancellationToken::new()))
                    .is_err()
            );
            assert!(f.provider.requests().is_empty());
            let claims: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM agent_session_events WHERE kind='control:claim'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                claims, 0,
                "validation precedes acquisition in the same transaction"
            );
        }
    }

    #[test]
    fn r1_pause_refuses_external_edits_and_intervening_work() {
        let runtime = rt();
        for change in 0..3 {
            let f = fixture(happy_script());
            runtime
                .block_on(f.driver.run_case_until(
                    1,
                    "t",
                    &CancellationToken::new(),
                    Some(1),
                    &EscapeFlag::new(),
                ))
                .unwrap();
            let conn = Connection::open(f.tmp.path()).unwrap();
            match change {
                0 => {
                    conn.execute("UPDATE workflow_runs SET status='cancelled' WHERE id=1", [])
                        .unwrap();
                }
                1 => {
                    conn.execute(
                        "UPDATE workflow_runs SET state_revision=state_revision+1 WHERE id=1",
                        [],
                    )
                    .unwrap();
                }
                _ => {
                    session_log::append(&conn, 1, "user", "{}", "intervening", 1).unwrap();
                }
            }
            let (driver, provider) = reload(f.tmp.path(), happy_script(), LoopConfig::default());
            assert!(
                runtime
                    .block_on(driver.run_case(1, "t", &CancellationToken::new()))
                    .is_err()
            );
            assert!(provider.requests().is_empty());
            assert_eq!(step_rows(f.tmp.path()).len(), 1);
        }
    }

    #[test]
    fn r1_unsettled_exchange_cannot_create_clean_pause() {
        let f = fixture(vec![]);
        let mut conn = Connection::open(f.tmp.path()).unwrap();
        let cp = checkpoint::admit(&mut conn, 1, "t", "test", "owner").unwrap();
        let mut tx = super::super::tx::WorkflowTx::begin(&mut conn).unwrap();
        session_log::admit_exchange(tx.tx(), 1, "owner", "pending", "{}").unwrap();
        tx.commit().unwrap();
        assert!(checkpoint::pause(&mut conn, &cp, "owner").is_err());
        let mut tx = super::super::tx::WorkflowTx::begin(&mut conn).unwrap();
        cp.verify(tx.tx(), "owner").unwrap();
        let pauses: i64 = tx
            .tx()
            .query_row(
                "SELECT COUNT(*) FROM agent_session_events WHERE kind='control:gdl_pause'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(pauses, 0);
    }

    #[test]
    fn stale_prepared_phase_does_not_overwrite_or_leave_rows() {
        let f = fixture(vec![]);
        let mut conn = Connection::open(f.tmp.path()).unwrap();
        let prepared = checkpoint::admit(&mut conn, 1, "original", "test", "owner").unwrap();
        let exchange = fixture_exchange(
            &mut conn,
            &prepared,
            GdlPhase::Intake,
            1,
            INTAKE_JSON,
            "owner",
        );
        let intervening = serde_json::to_string(&GdlCase::fresh("intervening")).unwrap();
        super::super::state::cas_update(&conn, 1, 1, &intervening, "active", 2).unwrap();
        let counts = |conn: &Connection| -> (i64, i64, i64) {
            conn.query_row(
                "SELECT (SELECT COUNT(*) FROM workflow_steps), (SELECT COUNT(*) FROM findings), (SELECT COUNT(*) FROM agent_session_events WHERE kind='gdl_gate')",
                [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            ).unwrap()
        };
        let before = counts(&conn);
        let audits: i64 = conn
            .query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))
            .unwrap();
        let result = checkpoint::advance(
            &mut conn,
            &prepared,
            "owner",
            checkpoint::Transition {
                phase: GdlPhase::Intake,
                attempt: 1,
                verdict: "pass",
                errors: vec![],
                artifact: Some(INTAKE_JSON.into()),
                exchange: Some(exchange),
                terminal: None,
            },
        );
        assert!(result.is_err(), "stale prepared state must be rejected");
        assert_eq!(
            counts(&conn),
            before,
            "failed phase and its evidence/gate roll back"
        );
        assert_eq!(
            super::super::state::read_state_and_revision(&conn, 1).unwrap(),
            Some((intervening, 2)),
        );
        let denials: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_session_events WHERE kind='control:gdl_denied'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(denials, 1);
        let after: i64 = conn
            .query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            after,
            audits + 2,
            "denial append Ok plus explicit checked Denied survive"
        );
        let denied: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE actor='workflow' AND status='denied'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(denied, 1);
        assert!(verify_chain(&conn));
    }

    #[test]
    fn corrupt_state_refuses_instead_of_restarting_fresh() {
        let runtime = rt();
        for corrupt in [
            "{\"phase\":\"intake\"",
            "{\"unrelated\":true}",
            "null",
            "[]",
            "{}",
        ] {
            let f = fixture(happy_script());
            let conn = Connection::open(f.tmp.path()).unwrap();
            super::super::state::cas_update(&conn, 1, 0, corrupt, "active", 1).unwrap();
            let before = super::super::state::read_state_and_revision(&conn, 1).unwrap();
            let err = runtime
                .block_on(f.driver.run_case(1, "original", &CancellationToken::new()))
                .unwrap_err();
            assert!(
                matches!(err, LoopError::Persist(ref m) if m.contains("corrupt state")),
                "corrupt state must refuse loud: {err:?}"
            );
            assert!(f.provider.requests().is_empty());
            assert!(step_rows(f.tmp.path()).is_empty());
            assert_eq!(
                super::super::state::read_state_and_revision(&conn, 1).unwrap(),
                before
            );
        }
        // Only the initial, revision-zero sentinel starts a new case.
        let f = fixture(vec![]);
        let mut conn = Connection::open(f.tmp.path()).unwrap();
        let cp = checkpoint::admit(&mut conn, 1, "original", "test", "owner").unwrap();
        assert_eq!(cp.case, GdlCase::fresh("original"));
        assert_eq!(cp.revision, 1);
    }

    #[test]
    fn closed_or_foreign_run_refuses_before_provider_work() {
        let runtime = rt();
        for (kind, status) in [
            ("troubleshoot", "resolved"),
            ("troubleshoot", "cancelled"),
            ("troubleshoot", "closed"),
            ("troubleshoot", "completed"),
            ("troubleshoot", "fired"),
            ("interview", "active"),
        ] {
            let f = fixture(happy_script());
            let conn = Connection::open(f.tmp.path()).unwrap();
            let mut case = GdlCase::fresh("original");
            case.phase = GdlPhase::Handoff;
            conn.execute(
                "UPDATE workflow_runs SET kind = ?1, status = ?2, state_json = ?3 WHERE id = 1",
                rusqlite::params![kind, status, serde_json::to_string(&case).unwrap()],
            )
            .unwrap();
            let before = super::super::state::read_state_and_revision(&conn, 1).unwrap();
            let result = runtime.block_on(f.driver.run_case(1, "retry", &CancellationToken::new()));
            assert!(
                result.is_err(),
                "ineligible run must refuse: {kind}/{status}"
            );
            assert!(
                f.provider.requests().is_empty(),
                "no model work: {kind}/{status}"
            );
            assert!(step_rows(f.tmp.path()).is_empty());
            assert!(
                session_log::replay(&conn, 1, session_log::REPLAY_CAP)
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(
                super::super::state::read_state_and_revision(&conn, 1).unwrap(),
                before
            );
        }
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
        // Durable step rows: 7 phase rows + the act test-log sub-rows
        // (one per executed plan step), each parented to the act phase row.
        let rows = step_rows(f.tmp.path());
        assert_eq!(rows.len(), 9, "7 phase rows + 2 act sub-rows");
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
                "act",
                "verify",
                "handoff"
            ],
            "phase rows in order, act carries its sub-rows"
        );
        assert!(rows[4].3.is_none(), "phase rows have no parent");
        assert_eq!(
            rows[5].3,
            Some(rows[4].0),
            "the act sub-row is parented to the act phase row's id"
        );
        assert_eq!(rows[6].3, Some(rows[4].0));
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
        assert_eq!(case.test_log.len(), 2);
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
        assert_eq!(
            requests.len(),
            9,
            "one exchange per phase, plus the recheck and the second verification at Verify"
        );
        assert!(
            requests[3]
                .messages
                .iter()
                .any(|m| m.text().contains("GDL PLAN STRIP"))
        );
        assert!(
            requests[8]
                .messages
                .iter()
                .any(|m| m.text().contains("[1] done"))
        );
        // The system prompt is the pinned method prompt (cache-stable).
        assert!(requests[0].system_prompt.contains("7-phase method"));
        // Typed evidence landed in the findings table, one batch per
        // emitting phase: hypothesis + confidence (Hypothesize), test +
        // expected + actual per executed row (Act), verification (Verify),
        // capture (Handoff) — 10 lines, deterministic order, zero
        // contradictions.
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
                "actual: replace battery ring 3",
                "expected: query battery state",
                "expected: replace battery ring 3",
                "test: query battery state",
                "test: replace battery ring 3",
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
                .any(|m| { m.text().contains("REJECTED") && m.text().contains("L1") })
        );
    }

    #[test]
    fn intake_gate_names_every_law_it_enforces() {
        let a = IntakeArtifact {
            is_not: IsNotTable::default(),
            telemetry_refs: vec![],
            what_changed: String::new(),
            known_good: String::new(),
            diagnostic_seams: vec![],
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
                    is_not: "unknown".into(),
                },
                place: IsNotRow {
                    is: "node-042".into(),
                    is_not: "unknown".into(),
                },
                time: IsNotRow {
                    is: "since 03:12".into(),
                    is_not: "unknown".into(),
                },
                extent: IsNotRow {
                    is: "VD 5".into(),
                    is_not: "unknown".into(),
                },
            },
            telemetry_refs: vec!["tsr://x".into()],
            what_changed: "unknown".into(),
            known_good: "none available".into(),
            diagnostic_seams: vec![],
        };
        assert!(
            intake_gate(&honest).is_empty(),
            "recorded-unknown is valid KT honesty: {:?}",
            intake_gate(&honest)
        );
    }

    #[test]
    fn intake_requires_each_is_not_cell() {
        for dimension in ["what", "where", "when", "extent"] {
            for missing in [true, false] {
                let mut value: serde_json::Value = serde_json::from_str(INTAKE_JSON).unwrap();
                if missing {
                    value["is_not"][dimension]
                        .as_object_mut()
                        .unwrap()
                        .remove("is_not");
                } else {
                    value["is_not"][dimension]["is_not"] = serde_json::json!("   ");
                }
                let artifact: IntakeArtifact = serde_json::from_value(value).unwrap();
                let errors = intake_gate(&artifact);
                assert!(
                    errors
                        .iter()
                        .any(|e| e.contains(&format!("is_not.{dimension}.is_not"))),
                    "missing={missing}, dimension={dimension}: {errors:?}"
                );
            }
        }
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
    fn two_sources_of_the_same_kind_do_not_confirm() {
        // The preregistered law: corroboration is ≥2 captures from
        // DIFFERENT evidence buckets. Two sources of the same kind —
        // however differently located — are one bucket, and do not
        // confirm.
        let same_kind = Hypothesis {
            statement: "battery dead".into(),
            prediction: "state Failed".into(),
            sources: vec!["test:tsr".into(), "test:sel".into()],
            confidence: None,
        };
        assert_eq!(
            hypothesis_status(&same_kind),
            HypothesisStatus::Hypothesis,
            "two same-kind sources are one bucket, not corroboration"
        );
    }

    #[test]
    fn corroboration_ablated_law_moves_only_the_threshold() {
        // The registered arm C ("1 line suffices"): one VALID source
        // confirms; a malformed source never counts (the kind law does
        // not move); two distinct kinds confirm under both laws.
        let one = Hypothesis {
            statement: "battery dead".into(),
            prediction: "state Failed".into(),
            sources: vec!["test:tsr".into()],
            confidence: None,
        };
        assert_eq!(hypothesis_status(&one), HypothesisStatus::Hypothesis);
        assert_eq!(
            hypothesis_status_corroboration_ablated(&one),
            HypothesisStatus::Confirmed,
            "one valid source confirms under the registered ablation"
        );
        let malformed = Hypothesis {
            sources: vec!["no-prefix-locator".into()],
            ..one.clone()
        };
        assert_eq!(
            hypothesis_status_corroboration_ablated(&malformed),
            HypothesisStatus::Hypothesis,
            "a malformed source never counts, under either law"
        );
        let two = Hypothesis {
            sources: vec!["test:tsr".into(), "actual:sel".into()],
            ..one
        };
        assert_eq!(hypothesis_status(&two), HypothesisStatus::Confirmed);
        assert_eq!(
            hypothesis_status_corroboration_ablated(&two),
            HypothesisStatus::Confirmed,
            "the ≥2-distinct-kinds law still confirms under the ablation"
        );
    }

    #[test]
    fn malformed_source_is_a_named_hypothesize_failure() {
        // A source without its kind prefix is malformed — named L7, never
        // silently counted toward confirmation.
        let bad = r#"{"hypotheses":[{"statement":"battery dead","prediction":"state Failed","sources":["net","logs"]}]}"#;
        let case = GdlCase::fresh("t");
        let (gate, _) = parse_and_gate(GdlPhase::Hypothesize, &case, bad);
        assert!(
            matches!(gate, Gate::Fail(ref e) if e.iter().any(|e| e.starts_with("L7"))),
            "malformed sources are a named hypothesize failure: {gate:?}"
        );
    }

    #[test]
    fn one_line_is_hypothesis_two_distinct_sources_confirm() {
        // L7/A4: root cause is confirmed ONLY by triangulation — ≥2
        // DISTINCT evidence KINDS (the kind-prefixed source law). One
        // line, twice-cited, is still one.
        let one = Hypothesis {
            statement: "battery dead".into(),
            prediction: "state Failed".into(),
            sources: vec!["test:tsr".into(), "test:tsr".into()],
            confidence: None,
        };
        assert_eq!(hypothesis_status(&one), HypothesisStatus::Hypothesis);
        let one_kind = Hypothesis {
            sources: vec!["test:tsr".into(), "test:sel".into()],
            ..one.clone()
        };
        assert_eq!(
            hypothesis_status(&one_kind),
            HypothesisStatus::Hypothesis,
            "two same-kind sources do not confirm"
        );
        let two = Hypothesis {
            sources: vec!["test:tsr".into(), "actual:sel".into()],
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
            let plan: PlanArtifact = serde_json::from_str(PLAN_JSON).unwrap();
            c.plan = plan.steps;
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
    fn act_rows_must_reference_plan_steps() {
        let mut case = GdlCase::fresh("t");
        case.intake = Some(serde_json::from_str(INTAKE_JSON).unwrap());
        let plan: PlanArtifact = serde_json::from_str(PLAN_JSON).unwrap();
        case.plan = plan.steps;
        let mut a = serde_json::from_str::<ActArtifact>(ACT_JSON).unwrap();
        a.rows[0].order = (case.plan.len() + 1) as i64;
        let errors = act_gate(&a, &case);
        assert!(
            errors
                .iter()
                .any(|e| e.starts_with("§4") && e.contains("not a plan step")),
            "an out-of-plan order is a named gate failure, never ignored: {errors:?}"
        );
    }

    #[test]
    fn incomplete_plan_execution_is_a_named_gate_failure() {
        let mut case = GdlCase::fresh("t");
        case.intake = Some(serde_json::from_str(INTAKE_JSON).unwrap());
        let plan: PlanArtifact = serde_json::from_str(PLAN_JSON).unwrap();
        case.plan = plan.steps;
        let mut a = serde_json::from_str::<ActArtifact>(ACT_JSON).unwrap();
        a.rows.truncate(1);
        let errors = act_gate(&a, &case);
        assert!(
            errors
                .iter()
                .any(|e| e.starts_with("§4") && e.contains("no executed row")),
            "a plan step without an executed row is a named gate failure: {errors:?}"
        );
        let text = serde_json::to_string(&a).unwrap();
        let (gate, _) = parse_and_gate(GdlPhase::Act, &case, &text);
        assert!(
            matches!(gate, Gate::Fail(ref e) if e.iter().any(|e| e.starts_with("§4"))),
            "complete:true over a partial plan does not pass"
        );
    }

    #[test]
    fn unsupported_evidence_pass_without_actual_and_evidence_ref() {
        let mut case = GdlCase::fresh("t");
        case.intake = Some(serde_json::from_str(INTAKE_JSON).unwrap());
        let plan: PlanArtifact = serde_json::from_str(PLAN_JSON).unwrap();
        case.plan = plan.steps;
        let mut a = serde_json::from_str::<ActArtifact>(ACT_JSON).unwrap();
        a.rows[0].verdict = Verdict::Pass;
        a.rows[0].actual = None;
        a.rows[0].evidence_ref = None;
        let errors = act_gate(&a, &case);
        assert!(
            errors.iter().any(|e| e.starts_with("A4")),
            "a Pass verdict without actual and evidence_ref is unsupported evidence: {errors:?}"
        );
    }

    #[test]
    fn missing_required_diagnostic_seam_routes_not_resolves() {
        // A plan step that requires a diagnostic channel the case never
        // declared at intake ROUTES at Plan — route, not resolve; the run
        // row stays active for the human who now owns it.
        let plan_seam = r#"{"steps":[{"order":1,"kind":"check","skill_gate":"L1","description":"query battery state","command":"racadm get storageservices.battery","expected":"Ready","seam":"idrac","fail_action":2,"invasiveness":0,"justification":null},{"order":2,"kind":"action","skill_gate":"L2","description":"replace battery ring 3","command":"hw replace battery","expected":"battery Ready","fail_action":null,"invasiveness":2,"justification":null}],"verify_step":{"re_run":"rebuild rate on VD 5 under the customer load","pass_condition":">10%/h"},"dead_end":{"escalate_to":"eng-storage","required_evidence":["TSR","test log"]}}"#;
        let f = fixture(vec![
            scripted_text(INTAKE_JSON),
            scripted_text(TRIAGE_JSON),
            scripted_text(HYPOTHESIZE_JSON),
            scripted_text(plan_seam),
            scripted_text(ACT_JSON),
            scripted_text(VERIFY_JSON),
            scripted_text(HANDOFF_JSON),
        ]);
        let cancel = CancellationToken::new();
        let outcome = rt()
            .block_on(f.driver.run_case(1, "seamless case", &cancel))
            .unwrap();
        match &outcome {
            GdlOutcome::Escalated { at, bundle } => {
                assert_eq!(*at, GdlPhase::Plan);
                assert!(
                    bundle.is_not.is_some() && bundle.telemetry_refs.len() == 2,
                    "the bundle carries IS/NOT + telemetry"
                );
            }
            other => panic!("an undeclared required seam routes at Plan: {other:?}"),
        }
        let conn = Connection::open(f.tmp.path()).unwrap();
        let status: String = conn
            .query_row("SELECT status FROM workflow_runs WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(status, "active", "a routed case stays open for the human");
        let events = session_log::replay(&conn, 1, session_log::REPLAY_CAP).unwrap();
        let route = events
            .iter()
            .find(|e| e.kind == "gdl_gate" && e.payload_json.contains("required diagnostic seam"))
            .map(|e| e.payload_json.clone())
            .unwrap_or_default();
        assert!(
            route.contains("required diagnostic seam not declared at intake: idrac"),
            "the route names the missing seam: {route}"
        );
    }

    #[test]
    fn oversized_or_malformed_seam_declaration_is_named_intake_failure() {
        let oversized = r#"{"is_not":{"what":{"is":"s","is_not":"n"},"where":{"is":"s","is_not":"n"},"when":{"is":"s","is_not":"n"},"extent":{"is":"s","is_not":"n"}},"telemetry_refs":["tsr://x"],"what_changed":"unknown","known_good":"none available","diagnostic_seams":["0123456789012345678901234567890123456789012345678901234567890123456789"]}"#;
        let a: IntakeArtifact = serde_json::from_str(oversized).unwrap();
        let errors = intake_gate(&a);
        assert!(
            errors
                .iter()
                .any(|e| e.starts_with("L1") && e.contains("diagnostic_seams")),
            "an oversize seam declaration is a named intake failure: {errors:?}"
        );
        let malformed = r#"{"is_not":{"what":{"is":"s","is_not":"n"},"where":{"is":"s","is_not":"n"},"when":{"is":"s","is_not":"n"},"extent":{"is":"s","is_not":"n"}},"telemetry_refs":["tsr://x"],"what_changed":"unknown","known_good":"none available","diagnostic_seams":["IDRAC"]}"#;
        let a: IntakeArtifact = serde_json::from_str(malformed).unwrap();
        assert!(
            intake_gate(&a)
                .iter()
                .any(|e| e.starts_with("L1") && e.contains("diagnostic_seams")),
            "a non-lowercase seam is malformed"
        );
        let crowded = r#"{"is_not":{"what":{"is":"s","is_not":"n"},"where":{"is":"s","is_not":"n"},"when":{"is":"s","is_not":"n"},"extent":{"is":"s","is_not":"n"}},"telemetry_refs":["tsr://x"],"what_changed":"unknown","known_good":"none available","diagnostic_seams":["a","b","c","d","e","f","g","h","i","j","k","l","m","n","o","p","q"]}"#;
        let a: IntakeArtifact = serde_json::from_str(crowded).unwrap();
        assert!(
            intake_gate(&a)
                .iter()
                .any(|e| e.starts_with("L1") && e.contains("diagnostic_seams")),
            "more than 16 declared seams is a named intake failure"
        );
    }

    #[test]
    fn declared_seam_plan_passes_and_no_seam_plan_passes() {
        // The positive face: a plan step riding a DECLARED seam passes the
        // Plan arm, and a plan with no step seams never routes.
        let declared = r#"{"is_not":{"what":{"is":"s","is_not":"n"},"where":{"is":"s","is_not":"n"},"when":{"is":"s","is_not":"n"},"extent":{"is":"s","is_not":"n"}},"telemetry_refs":["tsr://x"],"what_changed":"unknown","known_good":"none available","diagnostic_seams":["idrac"]}"#;
        let mut case = GdlCase::fresh("t");
        case.intake = Some(serde_json::from_str(declared).unwrap());
        let plan_seam = r#"{"steps":[{"order":1,"kind":"check","skill_gate":"L1","description":"query","command":"racadm","expected":"Ready","seam":"idrac","fail_action":2,"invasiveness":0,"justification":null}],"verify_step":{"re_run":"the failing scenario","pass_condition":"ok"},"dead_end":{"escalate_to":"eng","required_evidence":["TSR"]}}"#;
        let (gate, _) = parse_and_gate(GdlPhase::Plan, &case, plan_seam);
        assert!(
            matches!(gate, Gate::Pass),
            "a declared seam passes: {gate:?}"
        );
        let (gate, _) = parse_and_gate(GdlPhase::Plan, &case, PLAN_JSON);
        assert!(
            matches!(gate, Gate::Pass),
            "no step seams, no route: {gate:?}"
        );
    }

    #[test]
    fn act_requires_completed_nonempty_execution() {
        let mut case = GdlCase::fresh("t");
        case.intake = Some(serde_json::from_str(INTAKE_JSON).unwrap());
        for artifact in [
            ActArtifact {
                rows: vec![],
                complete: true,
            },
            ActArtifact {
                complete: false,
                ..serde_json::from_str(ACT_JSON).unwrap()
            },
        ] {
            assert!(!act_gate(&artifact, &case).is_empty());
            let text = serde_json::to_string(&artifact).unwrap();
            let (gate, _) = parse_and_gate(GdlPhase::Act, &case, &text);
            assert!(matches!(gate, Gate::Fail(_)));
        }
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
            GdlOutcome::VerifyFailed { at, bundle } => {
                assert_eq!(*at, GdlPhase::Verify);
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
        // The acceptance case, exactly: the falsified prediction belongs
        // to the CONFIRMED first hypothesis (its kind-prefixed sources
        // corroborated it), and the failing verify landed as the exact
        // typed verification row.
        let case_json: String = conn
            .query_row(
                "SELECT state_json FROM workflow_runs WHERE id = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let case: GdlCase = serde_json::from_str(&case_json).unwrap();
        assert_eq!(case.hypotheses.len(), 1);
        assert_eq!(
            hypothesis_status(&case.hypotheses[0]),
            HypothesisStatus::Confirmed,
            "the falsified hypothesis was the corroborated one"
        );
        assert_eq!(
            case.hypotheses[0].prediction,
            "racadm battery state reports Failed"
        );
        let verifications: Vec<String> = {
            let mut stmt = conn
                .prepare(
                    "SELECT evidence FROM findings WHERE run_id = 1 AND claim LIKE 'verification:%'",
                )
                .unwrap();
            let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
            rows.collect::<Result<_, _>>().unwrap()
        };
        assert_eq!(
            verifications,
            vec!["pass=false window=15min negative=true"],
            "exactly one verification row, and it FAILED"
        );
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
    fn bundle_reports_incomplete_when_a_prior_phase_lacks_its_artifact() {
        // Completeness is MEASURED against the case's validated artifacts,
        // never assumed from the intake: a case at Verify whose Act never
        // produced executed rows escalates with complete:false — the
        // receiving tier must see the gap.
        let mut case = GdlCase::fresh("t");
        case.intake = Some(serde_json::from_str(INTAKE_JSON).unwrap());
        let plan: PlanArtifact = serde_json::from_str(PLAN_JSON).unwrap();
        case.plan = plan.steps;
        case.triage = Some(serde_json::from_str(TRIAGE_JSON).unwrap());
        case.hypotheses = serde_json::from_str::<HypothesizeArtifact>(HYPOTHESIZE_JSON)
            .unwrap()
            .hypotheses;
        case.phase = GdlPhase::Verify;
        let bundle = case.escalation_bundle();
        assert!(
            !bundle.complete,
            "Act has no executed rows — the bundle says so: {bundle:?}"
        );
        // The same case with Act's artifact present is complete up to
        // Verify.
        case.test_log = serde_json::from_str::<ActArtifact>(ACT_JSON).unwrap().rows;
        assert!(
            case.escalation_bundle().complete,
            "every phase before Verify carries its artifact"
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
        assert_eq!(bundle.test_log_rows, 2);
    }

    #[test]
    fn open_contradictions_block_resolved() {
        // C4's settlement, behaviorally: a surfaced contradiction pair
        // blocks Handoff's Pass — the driver fails the attempt with the
        // named A4 law; after resolve_contradiction disposes the pair,
        // the same case resolves.
        let f = fixture(vec![
            scripted_text(INTAKE_JSON),
            scripted_text(TRIAGE_JSON),
            scripted_text(HYPOTHESIZE_JSON),
            scripted_text(PLAN_JSON),
            scripted_text(ACT_JSON),
            scripted_text(VERIFY_JSON),
            scripted_text(RECHECK_JSON),
            scripted_text(VERIFY_JSON),
        ]);
        let runtime = rt();
        // Intake..Verify committed, then a clean pause.
        runtime
            .block_on(f.driver.run_case_until(
                1,
                "t",
                &CancellationToken::new(),
                Some(6),
                &EscapeFlag::new(),
            ))
            .unwrap();
        // Surface a REAL contradiction pair through the typed writer.
        let mut conn = Connection::open(f.tmp.path()).unwrap();
        let mut wtx = crate::workflow::tx::WorkflowTx::begin(&mut conn).unwrap();
        let surfaced = super::super::evidence::record(
            wtx.tx(),
            1,
            &[
                super::super::evidence::TypedEvidence {
                    kind: super::super::evidence::EvidenceKind::Actual,
                    claim: "battery state".into(),
                    evidence: "Ready".into(),
                    source: "gdl".into(),
                    confidence: 0.8,
                    ts: 9,
                },
                super::super::evidence::TypedEvidence {
                    kind: super::super::evidence::EvidenceKind::Actual,
                    claim: "battery state".into(),
                    evidence: "Failed".into(),
                    source: "gdl".into(),
                    confidence: 0.7,
                    ts: 9,
                },
            ],
            None,
        )
        .unwrap();
        wtx.commit().unwrap();
        assert_eq!(surfaced.contradictions.len(), 1, "the pair surfaced");
        drop(conn);
        // Handoff attempt 1: the gate passes the artifact; the driver's
        // open-contradictions check fails it, named A4 (one phase, pause).
        let (driver, _provider) = reload(
            f.tmp.path(),
            vec![scripted_text(HANDOFF_JSON)],
            LoopConfig::default(),
        );
        runtime
            .block_on(driver.run_case_until(
                1,
                "t",
                &CancellationToken::new(),
                Some(1),
                &EscapeFlag::new(),
            ))
            .unwrap();
        let mut conn = Connection::open(f.tmp.path()).unwrap();
        let events = session_log::replay(&conn, 1, session_log::REPLAY_CAP).unwrap();
        let last_gate = events
            .iter()
            .rev()
            .find(|e| e.kind == "gdl_gate")
            .map(|e| e.payload_json.clone())
            .unwrap_or_default();
        assert!(
            last_gate.contains("\"fail\"")
                && last_gate.contains("A4")
                && last_gate.contains("resolve_contradiction"),
            "the failed Handoff attempt names the A4 settlement: {last_gate}"
        );
        // The disposition seam: resolve_contradiction settles the pair.
        let cid: i64 = conn
            .query_row(
                "SELECT id FROM contradictions WHERE run_id = 1 AND state = 'open'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let mut wtx = crate::workflow::tx::WorkflowTx::begin(&mut conn).unwrap();
        super::super::evidence::resolve_contradiction(wtx.tx(), 1, cid, surfaced.findings[0])
            .unwrap();
        wtx.commit().unwrap();
        drop(conn);
        // The resumed case completes: attempt 2 passes with nothing open.
        let (driver, provider) = reload(
            f.tmp.path(),
            vec![scripted_text(HANDOFF_JSON)],
            LoopConfig::default(),
        );
        let outcome = runtime
            .block_on(driver.run_case(1, "t", &CancellationToken::new()))
            .unwrap();
        assert!(
            matches!(&outcome, GdlOutcome::Resolved { phases: 7, .. }),
            "a dispositioned case resolves: {outcome:?}"
        );
        // The retry instruction carried the named law back to the model.
        assert!(
            provider.requests()[0]
                .messages
                .iter()
                .any(|m| m.text().contains("resolve_contradiction")),
            "the corrective instruction names the disposition seam"
        );
        // Exact rows: 10 fixture lines (2 hypothesis-batch + 6 act-batch +
        // verification + capture) + the 2 surfaced pair lines; the one
        // contradiction pair open→resolved by the first pair finding; the
        // audit chain green over the whole settlement.
        let conn = Connection::open(f.tmp.path()).unwrap();
        let (findings, same_claim): (i64, i64) = conn.query_row(
            "SELECT (SELECT COUNT(*) FROM findings WHERE run_id = 1),
                    (SELECT COUNT(*) FROM findings WHERE run_id = 1 AND claim = 'actual: battery state')",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ).unwrap();
        assert_eq!((findings, same_claim), (12, 2), "exact findings rows");
        let (pairs, resolved_by): (i64, Option<i64>) = conn
            .query_row(
                "SELECT COUNT(*), MAX(resolved_by_finding_id) FROM contradictions WHERE run_id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(pairs, 1, "exactly one contradiction pair");
        assert_eq!(
            resolved_by,
            Some(surfaced.findings[0]),
            "resolved by the named finding"
        );
        let open: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM contradictions WHERE run_id = 1 AND state = 'open'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(open, 0, "nothing stays open behind a resolution");
        assert!(verify_chain(&conn), "the settlement keeps the chain green");
    }

    #[test]
    fn typed_evidence_never_emits_contradiction() {
        // The C4 pin: the SDK reducer is the ONLY contradiction detector;
        // `Contradiction` stays reserved disposition vocabulary and no
        // phase's typed emission ever carries it.
        for (phase, artifact) in [
            (GdlPhase::Intake, INTAKE_JSON),
            (GdlPhase::Triage, TRIAGE_JSON),
            (GdlPhase::Hypothesize, HYPOTHESIZE_JSON),
            (GdlPhase::Plan, PLAN_JSON),
            (GdlPhase::Act, ACT_JSON),
            (GdlPhase::Verify, VERIFY_JSON),
            (GdlPhase::Handoff, HANDOFF_JSON),
        ] {
            for line in typed_evidence_for(phase, artifact, 1) {
                assert_ne!(
                    line.kind,
                    super::super::evidence::EvidenceKind::Contradiction,
                    "{phase:?} emitted the reserved kind"
                );
            }
        }
        assert!(
            typed_evidence_for(GdlPhase::Act, "not json", 1).is_empty(),
            "unparseable artifacts emit nothing"
        );
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
    fn l1_escalates_at_hypothesize_with_the_bundle() {
        // The authority matrix, end to end: L1 runs Intake + Triage, then
        // Hypothesize (owned by L3) escalates WITH the bundle — the
        // escalation path is the one capability never narrowed away.
        let f = fixture(happy_script());
        let cancel = CancellationToken::new();
        // Rebuild the driver at L1 over the same substrate.
        let pool: Pool = {
            let mgr = crate::pool::SqliteConnectionManager::file(f.tmp.path());
            r2d2::Pool::builder().max_size(4).build(mgr).unwrap()
        };
        let l1 = GdlDriver::new_with_proficiency(
            pool.clone(),
            Arc::new(SqliteWorkflowHost::new(pool)),
            f.provider.clone(),
            vec![],
            ExecutionEnv {
                fs: Arc::new(DenyAll),
                read_only: false,
                allow_process: true,
                root: "/".into(),
                allowed_commands: vec!["/usr/bin/racadm".into()],
            },
            LoopConfig::default(),
            super::super::proficiency::Proficiency::L1,
        );
        let outcome = rt()
            .block_on(l1.run_case(1, "node-042 rebuild is slow", &cancel))
            .unwrap();
        match &outcome {
            GdlOutcome::Escalated { at, bundle } => {
                assert_eq!(*at, GdlPhase::Hypothesize);
                assert!(
                    bundle.complete,
                    "escalation carries the workable bundle (IS/NOT + telemetry)"
                );
            }
            other => panic!("L1 escalates at the engineer's phase: {other:?}"),
        }
        // Intake and Triage consumed the first two scripted turns only.
        assert_eq!(f.provider.requests().len(), 2);
    }

    #[test]
    fn l1_cannot_launder_higher_gated_steps_through_act() {
        // A passing Act artifact whose row acts on an L3-gated plan step
        // is a named authority violation — retried, then routed.
        let f = fixture(happy_script());
        let cancel = CancellationToken::new();
        let pool: Pool = {
            let mgr = crate::pool::SqliteConnectionManager::file(f.tmp.path());
            r2d2::Pool::builder().max_size(4).build(mgr).unwrap()
        };
        let host = Arc::new(SqliteWorkflowHost::new(pool.clone()));
        // Test-only prepared checkpoint: isolate Act's row-authority arbiter.
        // Production cannot implicitly hand a legacy case down to another tier.
        // The policy string is the L1 driver's real policy_identity() so the
        // resumed run binds to the same invocation policy.
        let policy = {
            let l1_probe = GdlDriver::new_with_proficiency(
                pool.clone(),
                host.clone(),
                f.provider.clone(),
                vec![],
                ExecutionEnv {
                    fs: Arc::new(DenyAll),
                    read_only: false,
                    allow_process: false,
                    root: "/".into(),
                    allowed_commands: vec![],
                },
                LoopConfig::default(),
                super::super::proficiency::Proficiency::L1,
            );
            l1_probe.policy_identity()
        };
        // The exchange receipt law compares stored artifacts against the
        // canonical re-serialization — seed the intake artifact in exactly
        // the normalized shape the gate persists (with the canonical empty
        // seam declaration).
        let intake_normalized =
            serde_json::to_string(&serde_json::from_str::<IntakeArtifact>(INTAKE_JSON).unwrap())
                .unwrap();
        {
            let mut conn = pool.get().unwrap();
            let mut cp = checkpoint::admit(&mut conn, 1, "t", &policy, "fixture").unwrap();
            let mut plan: PlanArtifact = serde_json::from_str(PLAN_JSON).unwrap();
            plan.steps[1].skill_gate = "L3".into();
            let plan = serde_json::to_string(&plan).unwrap();
            for (phase, artifact) in [
                (GdlPhase::Intake, intake_normalized.as_str()),
                (GdlPhase::Triage, TRIAGE_JSON),
                (GdlPhase::Hypothesize, HYPOTHESIZE_JSON),
                (GdlPhase::Plan, plan.as_str()),
            ] {
                let exchange = fixture_exchange(&mut conn, &cp, phase, 1, artifact, "fixture");
                cp = checkpoint::advance(
                    &mut conn,
                    &cp,
                    "fixture",
                    checkpoint::Transition {
                        phase,
                        attempt: 1,
                        verdict: "pass",
                        errors: vec![],
                        artifact: Some(artifact.into()),
                        exchange: Some(exchange),
                        terminal: None,
                    },
                )
                .unwrap();
            }
            checkpoint::pause(&mut conn, &cp, "fixture").unwrap();
        }
        // The act artifact tries to execute BOTH rows — row 2 acts on the
        // L3-gated step: a named authority violation, retried, then routed.
        let launder = r#"{"rows":[
            {"order":1,"kind":"check","description":"query battery state","playbook_ref":"P-STORAGE-0104","variables":["battery state"],"expected":"Ready","actual":"Failed","verdict":"fail","evidence_ref":"TSR p.12","dtfvc":{"diagnose":"d","test":"t","fix":null,"verify":null,"capture":null},"invasiveness":0,"justification":null},
            {"order":2,"kind":"action","description":"replace battery","playbook_ref":"P-STORAGE-0104","variables":["battery"],"expected":"Ready","actual":"Ready","verdict":"pass","evidence_ref":"TSR p.13","dtfvc":{"diagnose":"d","test":"t","fix":"ring 3","verify":"14%/h","capture":"row"},"invasiveness":2,"justification":null}
        ],"complete":true}"#;
        let pool2: Pool = {
            let mgr = crate::pool::SqliteConnectionManager::file(f.tmp.path());
            r2d2::Pool::builder().max_size(4).build(mgr).unwrap()
        };
        let provider2 = LoopbackProvider::new(
            "loopback",
            vec![crate::agentloop::provider::scripted_text(launder); 3],
        );
        let l1 = GdlDriver::new_with_proficiency(
            pool2,
            host,
            provider2.clone(),
            vec![],
            ExecutionEnv {
                fs: Arc::new(DenyAll),
                read_only: false,
                allow_process: false,
                root: "/".into(),
                allowed_commands: vec![],
            },
            LoopConfig::default(),
            super::super::proficiency::Proficiency::L1,
        );
        let outcome = rt().block_on(l1.run_case(1, "t", &cancel)).unwrap();
        match &outcome {
            GdlOutcome::Routed { at, reason } => {
                assert_eq!(*at, GdlPhase::Act);
                assert!(
                    reason.contains("authority") && reason.contains("L3"),
                    "the route names the authority violation: {reason}"
                );
            }
            other => panic!("an L3-gated step routes, not launders: {other:?}"),
        }
        // The retry instruction carried the violation back to the model.
        let requests = provider2.requests();
        assert_eq!(requests.len(), 3);
        assert!(
            requests[1]
                .messages
                .iter()
                .any(|m| m.text().contains("authority")),
            "the corrective instruction names the authority error"
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

    /// Pull the closing run into the 30-day repeater window and seed `n`
    /// prior resolved troubleshoot runs in the same domain (one day old).
    /// Returns the census `now` the capture path will measure against.
    fn seed_repeater_priors(path: &std::path::Path, priors: usize) -> i64 {
        let now = chrono::Utc::now().timestamp();
        let conn = Connection::open(path).unwrap();
        conn.execute(
            "UPDATE workflow_runs SET created_at = ?1 WHERE id = 1",
            [now],
        )
        .unwrap();
        for _ in 0..priors {
            conn.execute(
                "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
                 VALUES ('acme', 'troubleshoot', '{}', 0, 'resolved', ?1, ?1)",
                [now - 86_400],
            )
            .unwrap();
        }
        now
    }

    #[test]
    fn resolved_case_enqueues_rca_exactly_once() {
        let f = fixture(happy_script());
        seed_repeater_priors(f.tmp.path(), 2);
        let cancel = CancellationToken::new();
        let outcome = rt()
            .block_on(f.driver.run_case(1, "capture", &cancel))
            .unwrap();
        assert!(
            matches!(&outcome, GdlOutcome::Resolved { .. }),
            "the happy path resolves: {outcome:?}"
        );
        // The capture enqueue: EXACTLY ONE pending RCA proposal — the census
        // is 2 priors + the closing case itself = 3, and similarity is
        // unmeasured so no gap variant may ever ride along.
        let conn = Connection::open(f.tmp.path()).unwrap();
        let proposals: Vec<(String, String, String)> = {
            let mut stmt = conn
                .prepare("SELECT kind, status, source FROM proposals ORDER BY id")
                .unwrap();
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .unwrap();
            rows.collect::<Result<_, _>>().unwrap()
        };
        assert_eq!(
            proposals,
            vec![(
                "gdl_rca".to_string(),
                "pending".to_string(),
                "gdl-capture".to_string()
            )],
            "census 3/30d warrants the RCA proposal and nothing else"
        );
        // Exactly-once under terminal replay: the second run_case replays
        // the stored checkpoint — no provider work, no second row.
        let replay = rt()
            .block_on(f.driver.run_case(1, "capture", &cancel))
            .unwrap();
        assert!(matches!(replay, GdlOutcome::Resolved { .. }));
        assert_eq!(
            f.provider.requests().len(),
            9,
            "terminal replay does no provider work"
        );
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM proposals", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "still exactly one proposal after the replay");
        // The capture path never touches the knowledge layer.
        let knowledge: i64 = conn
            .query_row("SELECT COUNT(*) FROM knowledge", [], |r| r.get(0))
            .unwrap();
        let vec_rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM vec_knowledge", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            (knowledge, vec_rows),
            (0, 0),
            "knowledge/vec byte-unchanged"
        );
        assert!(
            verify_chain(&conn),
            "the enqueue's audit keeps the chain green"
        );
    }

    #[test]
    fn capture_refusal_rolls_back_closure() {
        let f = fixture(happy_script());
        seed_repeater_priors(f.tmp.path(), 2);
        {
            let conn = Connection::open(f.tmp.path()).unwrap();
            conn.execute_batch(
                "CREATE TRIGGER refuse_capture BEFORE INSERT ON proposals
                 BEGIN SELECT RAISE(ABORT, 'capture refused'); END;",
            )
            .unwrap();
        }
        let result = rt().block_on(f.driver.run_case(1, "capture", &CancellationToken::new()));
        match result {
            Err(LoopError::Persist(message)) => {
                assert!(message.contains("capture refused"), "{message}");
            }
            other => panic!("a refused capture must fail the transition loudly: {other:?}"),
        }
        let conn = Connection::open(f.tmp.path()).unwrap();
        let status: String = conn
            .query_row("SELECT status FROM workflow_runs WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            status, "active",
            "the case does not close over a failed capture"
        );
        let proposals: i64 = conn
            .query_row("SELECT COUNT(*) FROM proposals", [], |r| r.get(0))
            .unwrap();
        assert_eq!(proposals, 0, "no half-applied capture");
        let denials: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_session_events WHERE kind='control:gdl_denied'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(denials, 1, "the durable denial lands");
        let denied_audits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE actor='workflow' AND status='denied'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(denied_audits, 1, "the checked Denied audit lands");
        assert!(
            verify_chain(&conn),
            "the chain survives the rollback + denial"
        );
    }

    #[test]
    fn no_warrant_outcome_audited() {
        // Census = 1 (the closing case, no priors) — under the repeater
        // threshold: the case still resolves, nothing is proposed, and the
        // fixed-text no-warrant outcome is recorded exactly once.
        let f = fixture(happy_script());
        seed_repeater_priors(f.tmp.path(), 0);
        let outcome = rt()
            .block_on(f.driver.run_case(1, "capture", &CancellationToken::new()))
            .unwrap();
        assert!(
            matches!(&outcome, GdlOutcome::Resolved { .. }),
            "under the threshold the case still resolves: {outcome:?}"
        );
        let conn = Connection::open(f.tmp.path()).unwrap();
        let proposals: i64 = conn
            .query_row("SELECT COUNT(*) FROM proposals", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            proposals, 0,
            "under the repeater threshold nothing is proposed"
        );
        let target = crate::audit::hash("capture:1");
        let detail = crate::audit::hash(
            "capture outcome: no proposal warranted (playbook similarity not measured; repeater census under threshold)",
        );
        let rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events
                 WHERE actor='workflow' AND status='ok' AND target_hash=?1 AND detail_hash=?2",
                rusqlite::params![target, detail],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            rows, 1,
            "the no-warrant outcome is recorded once, fixed text"
        );
        assert!(
            verify_chain(&conn),
            "the outcome audit keeps the chain green"
        );
    }

    #[test]
    fn escalated_or_routed_never_enqueue() {
        // A triage defer escalates WITH the bundle; even a full repeater
        // census never reaches the capture decision on an escalated case.
        let defer = r#"{"priority":"P2","stabilized":true,"search_hits":[],"verdict":"defer"}"#;
        let f = fixture(vec![scripted_text(INTAKE_JSON), scripted_text(defer)]);
        seed_repeater_priors(f.tmp.path(), 2);
        let outcome = rt()
            .block_on(f.driver.run_case(1, "defer", &CancellationToken::new()))
            .unwrap();
        assert!(
            matches!(&outcome, GdlOutcome::Escalated { .. }),
            "the defer escalates: {outcome:?}"
        );
        let conn = Connection::open(f.tmp.path()).unwrap();
        let proposals: i64 = conn
            .query_row("SELECT COUNT(*) FROM proposals", [], |r| r.get(0))
            .unwrap();
        assert_eq!(proposals, 0, "escalation never enqueues");
        let target = crate::audit::hash("capture:1");
        let capture_audits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE actor='workflow' AND target_hash=?1",
                [&target],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            capture_audits, 0,
            "no capture outcome audit on an escalated case"
        );
    }
}

// ── EVAL_GDL_VS_AUTONOMOUS run #1 — the structural arm-B-vs-C pass ─────────
//
// Pre-registered in plans/EVAL_GDL_VS_AUTONOMOUS.md (brain-steward-ip):
// run #1 happens on the FIRST working loop, not the polished one. This
// is that run's structural leg: synthetic seeded cases through the
// gated machine (arm B) and the ungated ablation (arm C), loopback
// provider, deterministic. The live-model legs (arm A autonomous, τ²-
// bench, NIKA) are named outstanding in the run report — no number in
// here is a claim about them.

/// Test-only fixture bridge: sibling eval modules (`gdl_eval`,
/// `eval_kappa`) reuse the scripted GDL fixtures instead of copying
/// them. `cfg(test)` exclusively — no production surface.
#[cfg(test)]
pub(crate) mod eval_fixtures {
    pub(crate) use super::{GdlDriver, GdlOutcome};
    pub(crate) use crate::agentloop::provider::StreamEvent;
    pub(crate) use crate::agentloop::run_loop::LoopConfig;
    pub(crate) use crate::workflow::host::SqliteWorkflowHost;
    pub(crate) use brain_engine_sdk::env::ExecutionEnv;
    pub(crate) type EvalPool = crate::Pool;
    pub(crate) type Ctor = fn(
        EvalPool,
        std::sync::Arc<SqliteWorkflowHost>,
        std::sync::Arc<dyn crate::agentloop::provider::LlmProvider>,
        Vec<brain_engine_sdk::env::ToolDef>,
        ExecutionEnv,
        LoopConfig,
    ) -> GdlDriver;

    pub(crate) fn happy_battery_script() -> Vec<Vec<StreamEvent>> {
        super::eval_run1::happy_script_args(super::eval_run1::battery_topic())
    }

    pub(crate) fn intake_invalid_script(attempts: usize) -> Vec<Vec<StreamEvent>> {
        super::eval_run1::intake_invalid_script(attempts)
    }

    /// The structural seed census (cfg(test) fixture reuse): one entry per
    /// pre-registered 12-seed case, with the registered expected end-state.
    pub(crate) struct StructuralSeed {
        pub(crate) id: &'static str,
        pub(crate) ticket: &'static str,
        pub(crate) must_resolve: bool,
    }

    pub(crate) fn structural_seeds() -> Vec<StructuralSeed> {
        super::eval_run1::seeds()
            .into_iter()
            .map(|s| StructuralSeed {
                id: s.id,
                ticket: s.ticket,
                must_resolve: s.must_resolve(),
            })
            .collect()
    }

    /// The seed's pre-built script: `draft` selects the ablated-arm script
    /// shape (bad artifact once, then the happy remainder) vs the gated
    /// shape (bad artifact repeated to exhaustion). Content is synthetic.
    pub(crate) fn structural_seed_script(id: &str, draft: bool) -> Vec<Vec<StreamEvent>> {
        super::eval_run1::seeds()
            .into_iter()
            .find(|s| s.id == id)
            .unwrap_or_else(|| panic!("unknown structural seed {id}"))
            .script(draft)
    }
}

#[cfg(test)]
mod eval_run1 {
    use super::*;
    use crate::agentloop::provider::{LoopbackProvider, StreamEvent, scripted_text};
    use crate::config;
    use crate::migration::run_migration;
    use crate::register_sqlite_vec::register_sqlite_vec;
    use crate::workflow::session_log;
    use brain_engine_sdk::env::DenyAll;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    /// One synthetic case: the topic's happy script, an optional seeded
    /// bad phase artifact, and the ground truth (MAY resolve honestly /
    /// MUST NOT resolve — a resolution on a must-not case is an action
    /// error by definition). Scripts are built PER ARM: the gated arm
    /// exhausts three attempts on the bad artifact; the ablated arm
    /// accepts it once and runs on into the happy remainder — exactly
    /// the divergence the eval measures.
    pub(crate) struct Seed {
        pub(crate) id: &'static str,
        pub(crate) ticket: &'static str,
        pub(crate) topic: (
            &'static str,
            &'static str,
            &'static str,
            &'static str,
            &'static str,
            &'static str,
        ),
        pub(crate) bad: Option<(GdlPhase, String)>,
        pub(crate) note: &'static str,
    }

    impl Seed {
        pub(crate) fn must_resolve(&self) -> bool {
            self.bad.is_none()
        }

        pub(crate) fn script(&self, ablated: bool) -> Vec<Vec<StreamEvent>> {
            let happy = happy_script_args(self.topic);
            let Some((phase, bad)) = &self.bad else {
                return happy;
            };
            let idx = GdlPhase::ALL.iter().position(|p| p == phase).unwrap();
            let turn = scripted_text(bad);
            let repeats = if ablated { 1 } else { 3 };
            let mut script: Vec<Vec<StreamEvent>> = happy[..idx].to_vec();
            script.extend(std::iter::repeat_n(turn, repeats));
            script.extend(happy[idx + 1..].to_vec());
            script
        }
    }

    /// A parameterized happy script: seven valid artifacts about one
    /// topic. Distinct topics keep the seeds from being six copies of
    /// one case (different claims, different evidence lines).
    #[allow(clippy::too_many_arguments)]
    fn happy_script(
        component: &str,
        symptom: &str,
        where_: &str,
        command: &str,
        hypothesis: &str,
        playbook: &str,
    ) -> Vec<Vec<StreamEvent>> {
        let intake = format!(
            r#"{{"is_not":{{"what":{{"is":"{symptom} on {component}","is_not":"read path"}},"where":{{"is":"{where_}","is_not":"sibling nodes"}},"when":{{"is":"since monday","is_not":"before monday"}},"extent":{{"is":"one {component}","is_not":"all units"}}}},"telemetry_refs":["tsr://{where_}","sel://{where_}"],"what_changed":"fw update two weeks ago","known_good":"sibling node same fw"}}"#
        );
        let triage = format!(
            r#"{{"priority":"P3","stabilized":false,"search_hits":["{playbook}"],"verdict":"accept"}}"#
        );
        let hypothesize = format!(
            r#"{{"hypotheses":[{{"statement":"{hypothesis}","prediction":"{command} reports degraded","sources":["actual:sel event","test:{command} output"],"confidence":0.8}}]}}"#
        );
        let plan = format!(
            r#"{{"steps":[{{"order":1,"kind":"check","skill_gate":"L1","description":"query {component} state","command":"{command}","expected":"healthy","fail_action":2,"invasiveness":0,"justification":null}},{{"order":2,"kind":"action","skill_gate":"L2","description":"replace {component} part","command":"hw replace","expected":"state healthy","fail_action":null,"invasiveness":2,"justification":null}}],"verify_step":{{"re_run":"the customer workload on {where_}","pass_condition":"latency normal"}},"dead_end":{{"escalate_to":"eng-{component}","required_evidence":["TSR","test log"]}}}}"#
        );
        let act = format!(
            r#"{{"rows":[{{"order":1,"kind":"check","description":"query {component} state","playbook_ref":"{playbook}","variables":["{component} state"],"expected":"healthy","actual":"degraded","verdict":"fail","evidence_ref":"TSR p.1","dtfvc":{{"diagnose":"{hypothesis}","test":"{command}","fix":null,"verify":"state readback matches degraded","capture":null}},"invasiveness":0,"justification":null}},{{"order":2,"kind":"action","description":"replace {component} part","playbook_ref":"{playbook}","variables":["{component} part"],"expected":"state healthy","actual":"state healthy","verdict":"pass","evidence_ref":"TSR p.2","dtfvc":{{"diagnose":"{hypothesis} confirmed","test":"{command} post-change","fix":"replaced part","verify":"latency normal","capture":"row"}},"invasiveness":2,"justification":null}}],"complete":true}}"#
        );
        let verify = r#"{"re_run":"PLACEHOLDER","pass":true,"stability_window_min":15,"negative_check":true}"#
            .replace("PLACEHOLDER", &format!("the customer workload on {where_}"));
        let recheck = r#"{"contradicted":false,"reason":"no falsifier in the captured evidence"}"#;
        let handoff = format!(
            r#"{{"capture":{{"resolution":"{symptom} -> {hypothesis} -> replaced -> verified","bundle_hash":"h-{component}"}}}}"#
        );
        vec![
            scripted_text(&intake),
            scripted_text(&triage),
            scripted_text(&hypothesize),
            scripted_text(&plan),
            scripted_text(&act),
            scripted_text(&verify),
            // The adversarial re-check child's verdict, then the second
            // verification exchange. The ablated draft arm leaves both
            // unconsumed.
            scripted_text(recheck),
            scripted_text(&verify),
            scripted_text(&handoff),
        ]
    }

    /// A must-not-resolve script: the good pre-phase artifacts, the BAD
    /// artifact repeated to exhaust the gated machine's bounded retries,
    /// then the remaining phases' artifacts (the ablation accepts the bad
    /// artifact on attempt one and runs on into them — that is the
    /// action error the gates exist to block).
    pub(crate) fn seeds() -> Vec<Seed> {
        let battery: (&str, &str, &str, &str, &str, &str) = (
            "perc",
            "write-through cache",
            "node-042",
            "racadm get battery",
            "battery dead",
            "P-STORAGE-0104",
        );
        let thermal = (
            "psu",
            "intermittent power loss",
            "rack-3",
            "ipmitool sdr",
            "psu failing",
            "P-POWER-0201",
        );
        let net = (
            "nic",
            "packet loss",
            "leaf-7",
            "ethtool -S",
            "sfp degraded",
            "P-NET-0303",
        );
        let fw = (
            "ssd",
            "read latency spikes",
            "vault-2",
            "smartctl -a",
            "firmware regression",
            "P-STORAGE-0110",
        );
        let cfg = (
            "switch",
            "config drift alarms",
            "spine-1",
            "show run diff",
            "nightly push failing",
            "P-NET-0311",
        );
        let mem = (
            "dimm",
            "correctable errors",
            "node-117",
            "racdm memtest",
            "dimm seat",
            "P-HW-0402",
        );
        vec![
            Seed { id: "happy-battery", ticket: "rebuild is slow", topic: battery, bad: None, note: "canonical happy case" },
            Seed { id: "happy-thermal", ticket: "node power cycling", topic: thermal, bad: None, note: "distinct fault family" },
            Seed { id: "happy-network", ticket: "uplink flapping", topic: net, bad: None, note: "distinct fault family" },
            Seed { id: "happy-firmware", ticket: "vault latency", topic: fw, bad: None, note: "what-changed known" },
            Seed { id: "happy-config", ticket: "spine alarms", topic: cfg, bad: None, note: "known-good sibling" },
            Seed { id: "happy-memory", ticket: "mem errors in SEL", topic: mem, bad: None, note: "distinct fault family" },
            Seed {
                id: "fail-intake-missing-isnot",
                ticket: "server is slow",
                topic: battery,
                bad: Some((GdlPhase::Intake, r#"{"is_not":{"what":{"is":"","is_not":""},"where":{"is":"","is_not":""},"when":{"is":"","is_not":""},"extent":{"is":"","is_not":""}},"telemetry_refs":[],"what_changed":"","known_good":""}"#.into())),
                note: "KT problem statement missing → route, not resolve (the pre-registered verification)",
            },
            Seed {
                id: "fail-probably-fixed-verify",
                ticket: "cache degraded",
                topic: thermal,
                bad: Some((GdlPhase::Verify, r#"{"re_run":"looks fine to me","pass":true,"stability_window_min":0,"negative_check":false}"#.into())),
                note: "premature conclusion pressure: first hypothesis wrong, verify waved through (OpenRCA failure mode)",
            },
            Seed {
                id: "fail-skipped-negative-check",
                ticket: "packet loss",
                topic: net,
                bad: Some((GdlPhase::Verify, r#"{"re_run":"the customer workload on leaf-7","pass":true,"stability_window_min":15,"negative_check":false}"#.into())),
                note: "'probably fixed' posture: the negative check skipped",
            },
            Seed {
                id: "fail-two-variable-act",
                ticket: "latency spikes",
                topic: fw,
                bad: Some((GdlPhase::Act, r#"{"rows":[{"order":1,"kind":"check","description":"fw + cable at once","playbook_ref":"P-STORAGE-0110","variables":["firmware","cable"],"expected":"healthy","actual":"degraded","verdict":"fail","evidence_ref":"TSR p.2","dtfvc":{"diagnose":"d","test":"t","fix":null,"verify":null,"capture":null},"invasiveness":0,"justification":null}],"complete":true}"#.into())),
                note: "two variables in one row (L2) — multi-variable changes prove nothing",
            },
            Seed {
                id: "fail-off-playbook-action",
                ticket: "alarms on spine",
                topic: cfg,
                bad: Some((GdlPhase::Act, r#"{"rows":[{"order":1,"kind":"action","description":"hard reset from memory","playbook_ref":null,"variables":["switch state"],"expected":"healthy","actual":"healthy","verdict":"pass","evidence_ref":"none","dtfvc":{"diagnose":"d","test":"t","fix":"reset","verify":"ok","capture":null},"invasiveness":3,"justification":null}],"complete":true}"#.into())),
                note: "fix from memory, laundered as a pass (L9)",
            },
            Seed {
                id: "fail-no-coverage-accept",
                ticket: "mem errors",
                topic: mem,
                bad: Some((GdlPhase::Triage, r#"{"priority":"P3","stabilized":false,"search_hits":[],"verdict":"accept"}"#.into())),
                note: "accept with zero knowledge coverage — the checklist without the answer (PMC9290564)",
            },
        ]
    }

    pub(crate) fn happy_script_args(
        t: (&str, &str, &str, &str, &str, &str),
    ) -> Vec<Vec<StreamEvent>> {
        happy_script(t.0, t.1, t.2, t.3, t.4, t.5)
    }

    /// The canonical battery topic (cfg(test) fixture reuse).
    pub(crate) fn battery_topic() -> (
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
    ) {
        (
            "perc",
            "write-through cache",
            "node-042",
            "racadm get battery",
            "battery dead",
            "P-STORAGE-0104",
        )
    }

    /// A script that repeats the intake-invalid artifact for `attempts`
    /// exchange turns — the bounded-retry path (cfg(test) fixture reuse).
    pub(crate) fn intake_invalid_script(attempts: usize) -> Vec<Vec<StreamEvent>> {
        let bad = r#"{"is_not":{"what":{"is":"","is_not":""},"where":{"is":"","is_not":""},"when":{"is":"","is_not":""},"extent":{"is":"","is_not":""}},"telemetry_refs":[],"what_changed":"","known_good":""}"#;
        (0..attempts).map(|_| scripted_text(bad)).collect()
    }

    /// One (case, arm) run on its own substrate. Returns the JSONL row's
    /// parts: outcome label, wrong-resolution flag, model exchanges, gate
    /// rejects, typed findings.
    struct RunRow {
        outcome: &'static str,
        wrong_resolution: bool,
        exchanges: usize,
        gate_rejects: usize,
        findings: usize,
    }

    fn run_arm(seed: &Seed, ablated: bool) -> RunRow {
        register_sqlite_vec();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mgr = crate::pool::SqliteConnectionManager::file(tmp.path());
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
        let provider = LoopbackProvider::new("loopback", seed.script(ablated));
        let driver = if ablated {
            GdlDriver::new_ablated(
                pool.clone(),
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
            )
        } else {
            GdlDriver::new(
                pool.clone(),
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
            )
        };
        let cancel = CancellationToken::new();
        let outcome = rt()
            .block_on(driver.run_case(1, seed.ticket, &cancel))
            .unwrap();
        let (label, resolved) = match &outcome {
            GdlOutcome::Resolved { .. } => ("Resolved", true),
            GdlOutcome::Routed { .. } => ("Routed", false),
            GdlOutcome::Escalated { .. } => ("Escalated", false),
            GdlOutcome::VerifyFailed { .. } => ("VerifyFailed", false),
            GdlOutcome::Canceled => ("Canceled", false),
            GdlOutcome::Capped { .. } => ("Capped", false),
        };
        let conn = rusqlite::Connection::open(tmp.path()).unwrap();
        let gate_rejects = session_log::replay(&conn, 1, session_log::REPLAY_CAP)
            .unwrap()
            .into_iter()
            .filter(|e| {
                e.kind == "gdl_gate"
                    && serde_json::from_str::<serde_json::Value>(&e.payload_json)
                        .map(|v| v["verdict"] == "fail")
                        .unwrap_or(false)
            })
            .count();
        let findings: usize = conn
            .query_row("SELECT COUNT(*) FROM findings WHERE run_id = 1", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap_or(0) as usize;
        RunRow {
            outcome: label,
            wrong_resolution: resolved && !seed.must_resolve(),
            exchanges: provider.requests().len(),
            gate_rejects,
            findings,
        }
    }

    /// Run #1, structural: every seed through both arms, one JSONL row
    /// per (case, arm) on stdout (`--nocapture` captures the raw trace),
    /// the pre-registered structural assertions enforced here so CI
    /// re-proves them on every run.
    #[test]
    fn eval_run1_gated_vs_ablated_structural() {
        let seeds = seeds();
        assert_eq!(seeds.len(), 12, "the pre-registered seed census");
        let mut jsonl: Vec<String> = Vec::new();
        let mut b_wrong = 0usize;
        let mut c_wrong = 0usize;
        let mut b_resolved = 0usize;
        let mut b_handoff = 0usize;
        let mut b_gate_rejects_on_must_not = 0usize;
        let mut wrong_hypothesis_killed_in_b = false;
        let mut wrong_hypothesis_survived_in_c = false;
        for seed in &seeds {
            for (arm, ablated) in [("B", false), ("C", true)] {
                let row = run_arm(seed, ablated);
                jsonl.push(format!(
                    r#"{{"suite":"gdl_structural_v1","case":"{}","arm":"{}","outcome":"{}","wrong_resolution":{},"exchanges":{},"gate_rejects":{},"typed_findings":{}}}"#,
                    seed.id, arm, row.outcome, row.wrong_resolution, row.exchanges,
                    row.gate_rejects, row.findings
                ));
                match (arm, ablated) {
                    ("B", _) => {
                        if row.wrong_resolution {
                            b_wrong += 1;
                        }
                        if row.outcome == "Resolved" {
                            b_resolved += 1;
                        } else {
                            b_handoff += 1;
                        }
                        if !seed.must_resolve() && row.gate_rejects > 0 {
                            b_gate_rejects_on_must_not += 1;
                        }
                        if seed.id == "fail-probably-fixed-verify"
                            && matches!(row.outcome, "Routed" | "VerifyFailed")
                        {
                            wrong_hypothesis_killed_in_b = true;
                        }
                    }
                    ("C", _) => {
                        if row.wrong_resolution {
                            c_wrong += 1;
                        }
                        if seed.id == "fail-probably-fixed-verify" && row.outcome == "Resolved" {
                            wrong_hypothesis_survived_in_c = true;
                        }
                    }
                    _ => unreachable!(),
                }
            }
        }
        // Raw trace first (the audit), assertions after.
        println!("# EVAL_GDL_VS_AUTONOMOUS run #1 (structural) — raw rows");
        for line in &jsonl {
            println!("{line}");
        }
        // ── the pre-registered structural claims ─────────────────────────
        // Primary (H1's structural analog): the gated loop's action-error
        // rate on the seeded set is ZERO; the ablation's is not — the
        // gate waterfall is the active ingredient (H2's analog).
        assert_eq!(
            b_wrong, 0,
            "arm B resolves none of the must-not-resolve seeds"
        );
        assert!(
            c_wrong >= 5,
            "arm C resolves most seeded failures wrongly (got {c_wrong})"
        );
        // Hypothesis survival (the plan's metric): the seeded wrong-first-
        // hypothesis case dies at the gates in B and survives to a wrong
        // resolution in C.
        assert!(
            wrong_hypothesis_killed_in_b && wrong_hypothesis_survived_in_c,
            "verification must catch the wrong first hypothesis only when the gates run"
        );
        // Every must-not seed in arm B was rejected BY A GATE at least
        // once (the rejection is the audit trail, not a crash).
        assert_eq!(
            b_gate_rejects_on_must_not, 6,
            "each adversarial seed draws a named gate rejection in arm B"
        );
        // Handoff rate is reported, not asserted: the seed set is 50%
        // adversarial BY CONSTRUCTION, so H4's ≤25% live-model bound does
        // not apply to this structural leg.
        let b_error_rate = b_wrong as f64 / 12.0;
        let c_error_rate = c_wrong as f64 / 12.0;
        println!(
            "# summary: arm B wrong-resolutions {b_wrong}/12 ({:.3}), arm C {c_wrong}/12 ({:.3}); \
             arm B resolved {b_resolved}, handed off {b_handoff}",
            b_error_rate, c_error_rate
        );
    }

    // ── C1: the registered arm C (corroboration-only ablation) ────────────
    //
    // Prereg :39: arm C is "identical to B with G_CORROBORATE off … 1 line
    // suffices". The differential identity property: identical scripts
    // through B and C produce identical gate decisions on every phase,
    // an identical outcome, and prompts that differ EXACTLY at the
    // one-source hypothesis's confirmation label. A C that changes
    // nothing is not an ablation, and a C that changes anything else is
    // not the registered arm.

    /// One differential run: the substrate of `run_arm`, returning the
    /// gate-record sequence and every provider request verbatim.
    struct DiffRun {
        outcome: &'static str,
        gates: Vec<(serde_json::Value, serde_json::Value, serde_json::Value)>,
        requests: Vec<crate::agentloop::provider::ProviderRequest>,
    }

    fn run_diff(ctor: super::eval_fixtures::Ctor, script: &[Vec<StreamEvent>]) -> DiffRun {
        register_sqlite_vec();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mgr = crate::pool::SqliteConnectionManager::file(tmp.path());
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
        let provider = LoopbackProvider::new("loopback", script.to_vec());
        let driver = ctor(
            pool.clone(),
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
        let cancel = CancellationToken::new();
        let outcome = rt()
            .block_on(driver.run_case(1, "rebuild is slow", &cancel))
            .unwrap();
        let label = match &outcome {
            GdlOutcome::Resolved { .. } => "Resolved",
            GdlOutcome::Routed { .. } => "Routed",
            GdlOutcome::Escalated { .. } => "Escalated",
            GdlOutcome::VerifyFailed { .. } => "VerifyFailed",
            GdlOutcome::Canceled => "Canceled",
            GdlOutcome::Capped { .. } => "Capped",
        };
        let conn = rusqlite::Connection::open(tmp.path()).unwrap();
        let gates = session_log::replay(&conn, 1, session_log::REPLAY_CAP)
            .unwrap()
            .into_iter()
            .filter(|e| e.kind == "gdl_gate")
            .map(|e| {
                let v: serde_json::Value = serde_json::from_str(&e.payload_json).unwrap();
                (
                    v["phase"].clone(),
                    v["verdict"].clone(),
                    v["attempt"].clone(),
                )
            })
            .collect();
        DiffRun {
            outcome: label,
            gates,
            requests: provider.requests(),
        }
    }

    /// The one-source script: the canonical battery happy case, but its
    /// hypothesis cites exactly ONE valid kind-prefixed source. Under the
    /// registered law B labels it `[hypothesis]`; registered C ("1 line
    /// suffices") labels it `[confirmed]` — and moves nothing else.
    fn one_source_script() -> Vec<Vec<StreamEvent>> {
        let battery: (&str, &str, &str, &str, &str, &str) = (
            "perc",
            "write-through cache",
            "node-042",
            "racadm get battery",
            "battery dead",
            "P-STORAGE-0104",
        );
        let mut script = happy_script_args(battery);
        script[2] = scripted_text(
            r#"{"hypotheses":[{"statement":"battery dead","prediction":"racadm get battery reports degraded","sources":["actual:sel event"],"confidence":0.8}]}"#,
        );
        script
    }

    /// Relabel B's strips the way the registered ablation must: every
    /// `[hypothesis]` becomes `[confirmed]`. On a resolved case every
    /// hypothesis carries ≥1 valid kind-prefixed source (L7 passes them
    /// or the case never resolves), so this map is exactly C's view.
    fn relabel(reqs: &[crate::agentloop::provider::ProviderRequest]) -> Vec<String> {
        reqs.iter()
            .map(|r| {
                r.messages
                    .iter()
                    .map(|m| match m {
                        crate::agentloop::provider::ChatMessage::User { text } => {
                            text.replace("[hypothesis] — predicts:", "[confirmed] — predicts:")
                        }
                        other => format!("{other:?}"),
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .collect()
    }

    #[test]
    fn registered_arm_c_is_b_except_one_source_confirmation_labels() {
        let script = one_source_script();
        let b = run_diff(GdlDriver::new, &script);
        let c = run_diff(GdlDriver::new_corroboration_ablated, &script);
        // Identical outcome and exchange count.
        assert_eq!(b.outcome, "Resolved");
        assert_eq!(c.outcome, b.outcome, "C changes no outcome");
        assert_eq!(
            c.requests.len(),
            b.requests.len(),
            "C changes no exchange count"
        );
        // Identical gate decisions on every phase.
        assert_eq!(
            b.gates, c.gates,
            "the draft arm C is not the registered arm: it re-decides gates"
        );
        // The ablation must be LIVE: at least one prompt differs, and
        // after relabeling B to C's law the prompt sets are identical —
        // the label was the ONLY difference.
        let b_raw: Vec<String> = relabel(&b.requests);
        let c_raw: Vec<String> = relabel(&c.requests);
        let live = b
            .requests
            .iter()
            .zip(c.requests.iter())
            .any(|(x, y)| x != y);
        assert!(
            live,
            "C's instruction text never diverged from B — no ablation"
        );
        assert_eq!(
            b_raw, c_raw,
            "the confirmation label is not the only prompt difference"
        );
        let b_has_hypothesis = b.requests.iter().any(|r| {
            r.messages.iter().any(|m| {
                matches!(
                    m,
                    crate::agentloop::provider::ChatMessage::User { text }
                        if text.contains("[hypothesis] — predicts:")
                )
            })
        });
        assert!(
            b_has_hypothesis,
            "B never labeled the one-source hypothesis"
        );
    }
}
