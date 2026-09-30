//! The admission-gate suite: **is a thing with a known correct answer fit to
//! ship?**
//!
//! SWE-Proof's contribution is domain-generic and is transplanted here whole:
//! a suite of checks that decides admissibility, in two kinds — *mechanical*
//! (decided by running something) and *adversarial* (an attack whose verdict is
//! adjudicated by running it) — plus the design rule this round exists to
//! install:
//!
//! > "A negative outcome does not produce a patch to the bundle; it sends the
//! > instance back for revision at whichever stage owns the defect. … A
//! > specification that admits a wrong implementation goes back to **stage one,
//! > not to stage three**."
//!
//! A gate suite that can only reject is a **filter**. One that routes a failure
//! to the stage that caused it is a **control loop**. That is the difference
//! [`patch_at_gate`] makes unrepresentable rather than merely forbidden.
//!
//! **Inapplicable is a third state and is never a pass.** Three checks
//! ([`CheckId::ContentDisposition`], [`CheckId::ReadSeam`],
//! [`CheckId::AuditCalibration`]) are unreachable from this crate for a
//! measured reason, and each records that reason. A case with an *undeclared*
//! inapplicability is refused; a case with a *declared* one is admitted **with
//! the gap named in its receipt**, so an admitted case never looks fully green.
//!
//! **This is a fitness-to-ship gate over a frozen test corpus. It is not a
//! truth oracle, it must not gate production traffic, and no route may read
//! it.**

use std::convert::Infallible;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Stages
// ---------------------------------------------------------------------------

/// The five stages of the loop. A negative check names the stage that **owns**
/// the defect; it never patches in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Stage {
    Create,
    Solve,
    Evolve,
    Deflect,
    Operate,
}

impl Stage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Stage::Create => "S_CREATE",
            Stage::Solve => "S_SOLVE",
            Stage::Evolve => "S_EVOLVE",
            Stage::Deflect => "S_DEFLECT",
            Stage::Operate => "S_OPERATE",
        }
    }
}

// ---------------------------------------------------------------------------
// Checks
// ---------------------------------------------------------------------------

/// The twelve admission checks. [`CheckId::Faithfulness`] is **number 3 and
/// executed first**: the assessment's own calibration makes it the dominant
/// failure (42.5%), which is a property of the *population*, not a reason to
/// check it last. Numbering is the plan's taxonomy; execution order is
/// [`EXECUTION_ORDER`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CheckId {
    Faithfulness,
    Resolution,
    Discrimination,
    Provenance,
    Soundness,
    OwnerStamp,
    ContentDisposition,
    ReadSeam,
    GateAgreement,
    BudgetCompliance,
    AttackerPanel,
    AuditCalibration,
}

/// How a check decides. This is the plan's mechanical/adversarial axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckKind {
    Mechanical,
    Adversarial,
}

/// One check's static description. Every check resolves to exactly one
/// [`CheckKind`], and carries either `None` or a measured reason for being
/// unreachable from this crate.
#[derive(Debug, Clone, Copy)]
pub struct CheckSpec {
    pub id: CheckId,
    /// The plan's number for this check.
    pub number: u8,
    pub kind: CheckKind,
    /// What a positive outcome establishes.
    pub establishes: &'static str,
    /// `None` when the check is reachable here. `Some(reason)` when it is not —
    /// and then the check is recorded **inapplicable with that reason**, never
    /// as a pass and never as a stub.
    pub unreachable_reason: Option<&'static str>,
}

/// The write-time screen seam lives in the **server** crate, and reaching it
/// from this workspace node needs exactly the edge a landed pin forbids.
pub const CONTENT_DISPOSITION_UNREACHABLE: &str = "the write-time screen seam is screen_content at src/workflow/channel.rs:122 in the SERVER crate; crates/gold-sets is a separate workspace node, and linking it adds the workspace path edge tests/r57_census_pins.rs::the_census_reaches_the_corpus_without_a_new_dependency_edge forbids by name. REOPEN: when the suite is driven server-side, or when that pin is amended in the open.";
/// The read seam is likewise server-side, and it is the same edge.
pub const READ_SEAM_UNREACHABLE: &str = "the read seam is sanitize_read at src/gate.rs:928 in the SERVER crate; same workspace boundary and same forbidding pin as CONTENT_DISPOSITION_UNREACHABLE. REOPEN: with ContentDisposition.";
/// No out-of-loop artifact exists to calibrate the panel against.
pub const AUDIT_CALIBRATION_UNREACHABLE: &str = "check 12 requires artifacts the gate's own construction loop did not shape; every artifact in this pack was authored by the round that wrote the gate, so there is nothing out-of-loop to calibrate against. Authoring a calibration set is a different round's call, not this one's. REOPEN: when an independently frozen calibration set lands.";

/// The closed check registry, in **execution order**: faithfulness first, then
/// the plan's numbering ascending. The panel runs after every mechanical check
/// it attacks, and calibration adjudicates the panel's own reliability.
pub const EXECUTION_ORDER: [CheckId; 12] = [
    CheckId::Faithfulness,
    CheckId::Resolution,
    CheckId::Discrimination,
    CheckId::Provenance,
    CheckId::Soundness,
    CheckId::OwnerStamp,
    CheckId::ContentDisposition,
    CheckId::ReadSeam,
    CheckId::GateAgreement,
    CheckId::BudgetCompliance,
    CheckId::AttackerPanel,
    CheckId::AuditCalibration,
];

/// The registry, keyed by the plan's number.
pub fn spec(id: CheckId) -> CheckSpec {
    let (number, kind, establishes, unreachable_reason) = match id {
        CheckId::Resolution => (
            1,
            CheckKind::Mechanical,
            "the case reaches the resolution the corpus claims",
            None,
        ),
        CheckId::Discrimination => (
            2,
            CheckKind::Mechanical,
            "the pre-fix twin fails — the case is not vacuous",
            None,
        ),
        CheckId::Faithfulness => (
            3,
            CheckKind::Adversarial,
            "the case covers the whole required surface, not a slice",
            None,
        ),
        CheckId::Provenance => (
            4,
            CheckKind::Mechanical,
            "every evidence reference resolves to the bytes it cites",
            None,
        ),
        CheckId::Soundness => (
            5,
            CheckKind::Mechanical,
            "no superseded or withdrawn knowledge is relied on",
            None,
        ),
        CheckId::OwnerStamp => (6, CheckKind::Mechanical, "the claim carries a writer", None),
        CheckId::ContentDisposition => (
            7,
            CheckKind::Mechanical,
            "screened at write time; the screen is not vacuous",
            Some(CONTENT_DISPOSITION_UNREACHABLE),
        ),
        CheckId::ReadSeam => (
            8,
            CheckKind::Mechanical,
            "every emitted field passes sanitize_read",
            Some(READ_SEAM_UNREACHABLE),
        ),
        CheckId::GateAgreement => (
            9,
            CheckKind::Mechanical,
            "no gate rejected a step that the case claims succeeded",
            None,
        ),
        CheckId::BudgetCompliance => (
            10,
            CheckKind::Mechanical,
            "the case stayed inside its pinned budgets",
            None,
        ),
        CheckId::AttackerPanel => (
            11,
            CheckKind::Adversarial,
            "blind attack finds no defect",
            None,
        ),
        CheckId::AuditCalibration => (
            12,
            CheckKind::Mechanical,
            "the panel is calibrated on out-of-loop artifacts",
            Some(AUDIT_CALIBRATION_UNREACHABLE),
        ),
    };
    CheckSpec {
        id,
        number,
        kind,
        establishes,
        unreachable_reason,
    }
}

// ---------------------------------------------------------------------------
// Outcomes
// ---------------------------------------------------------------------------

/// The result axis. `Inapplicable` is deliberately **not** a `Pass` variant:
/// the distinction is in the type, so it cannot be lost in a refactor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckOutcome {
    Pass,
    Inapplicable { reason: String },
    Negative { stage: Stage, reason: String },
}

impl CheckOutcome {
    pub fn negative(stage: Stage, reason: impl Into<String>) -> Self {
        CheckOutcome::Negative {
            stage,
            reason: reason.into(),
        }
    }
    pub fn inapplicable(reason: impl Into<String>) -> Self {
        CheckOutcome::Inapplicable {
            reason: reason.into(),
        }
    }
    pub fn is_pass(&self) -> bool {
        matches!(self, CheckOutcome::Pass)
    }
    pub fn is_negative(&self) -> bool {
        matches!(self, CheckOutcome::Negative { .. })
    }
    pub fn is_inapplicable(&self) -> bool {
        matches!(self, CheckOutcome::Inapplicable { .. })
    }
}

// ---------------------------------------------------------------------------
// The admission-shaped case
// ---------------------------------------------------------------------------

/// How a step was adjudicated by the run's gates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GateVerdict {
    Pass,
    Reject,
}

/// The terminal resolution a case claims to reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Resolution {
    Resolved,
    Unresolved,
    Escalated,
}

impl Resolution {
    pub fn as_str(&self) -> &'static str {
        match self {
            Resolution::Resolved => "R_RESOLVED",
            Resolution::Unresolved => "R_UNRESOLVED",
            Resolution::Escalated => "R_ESCALATED",
        }
    }
}

/// One evidence reference: a locator plus the content digest of the bytes it
/// cites. Check 4 verifies the reference **addresses** bytes — see the ceiling
/// note on [`check_provenance`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub kind: String,
    pub locator: String,
    pub digest: String,
}

/// One step of the run, with the gate verdict it received.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatedStep {
    pub expected: String,
    pub actual: String,
    pub gate_verdict: GateVerdict,
}

/// The pinned budgets a case must stay inside (check 10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budget {
    pub steps: u32,
    pub evidence_refs: u32,
}

/// The mutation a pre-fix twin applies to its case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TwinMutation {
    /// The step was never performed: its outcome does not match its intent.
    UndoneStep,
    /// The step's gate rejected it.
    GateRejectedStep,
}

/// A pre-fix twin: the same case with one step's outcome broken. If breaking it
/// does not move the recomputed resolution, the case's assertion is satisfied
/// by construction and is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreFixTwin {
    pub step_index: usize,
    pub mutation: TwinMutation,
}

/// An inapplicability the author has **accepted**, with the reason. Accepted is
/// not passing: it is a named gap carried into the admission receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptedGap {
    pub check: CheckId,
    pub reason: String,
}

/// One case put forward for admission.
///
/// Deliberately **not** [`crate::GoldCase`]: the frozen pack's seven cases are
/// R53's contested surface and are not touched by this round. This shape is
/// additive and loads through its own accessor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdmissionCase {
    pub id: String,
    pub family: String,
    pub scorer_version: String,
    pub kappa_units: i32,
    /// The corpus's human verdict. **No check arbitrates this against
    /// `resolution`** — see [`Mutant::HumanVerdictContradiction`], which
    /// survives the panel by design.
    pub human_pass: bool,
    /// The resolution the case claims.
    pub resolution: Resolution,
    pub verified: bool,
    pub handoff_complete: bool,
    pub escalation_honored: bool,
    /// The surface the case is **required** to cover (check 3).
    pub required_surface: Vec<String>,
    /// The writer stamped on the claim (check 6).
    pub owner: String,
    pub evidence_refs: Vec<EvidenceRef>,
    /// Locators the corpus knows are superseded (check 5).
    pub superseded_refs: Vec<String>,
    pub withdrawn: bool,
    pub steps: Vec<GatedStep>,
    pub budget: Budget,
    pub pre_fix_twin: Option<PreFixTwin>,
    pub accepted_gaps: Vec<AcceptedGap>,
}

impl AdmissionCase {
    /// The closed provenance vocabulary (check 4).
    pub fn is_known_evidence_kind(kind: &str) -> bool {
        matches!(
            kind,
            "run" | "audit" | "handoff" | "finding" | "contradiction" | "contact-history"
        )
    }

    fn accepts(&self, check: CheckId) -> Option<&str> {
        self.accepted_gaps
            .iter()
            .find(|g| g.check == check)
            .map(|g| g.reason.as_str())
    }
}

// ---------------------------------------------------------------------------
// The recomputation — the spine the suite shares
// ---------------------------------------------------------------------------

/// The preregistered recomputation: **a case's resolution is what its own steps
/// say, not what its header claims.** Check 1 compares the claim to this, and
/// check 2 asks whether the claim survives its own pre-fix twin.
pub fn recomputed_resolution(c: &AdmissionCase) -> Resolution {
    if c.steps
        .iter()
        .any(|s| s.gate_verdict == GateVerdict::Reject)
    {
        return Resolution::Escalated;
    }
    if c.steps.iter().any(|s| s.actual != s.expected) {
        return Resolution::Unresolved;
    }
    if c.verified && c.handoff_complete {
        return Resolution::Resolved;
    }
    Resolution::Unresolved
}

fn prefix(check: CheckId, detail: impl std::fmt::Display) -> String {
    format!("{} {detail}", check.as_str())
}

impl CheckId {
    pub fn as_str(&self) -> &'static str {
        match self {
            CheckId::Faithfulness => "C_FAITHFULNESS",
            CheckId::Resolution => "C_RESOLUTION",
            CheckId::Discrimination => "C_DISCRIMINATION",
            CheckId::Provenance => "C_PROVENANCE",
            CheckId::Soundness => "C_SOUNDNESS",
            CheckId::OwnerStamp => "C_OWNER_STAMP",
            CheckId::ContentDisposition => "C_CONTENT_DISPOSITION",
            CheckId::ReadSeam => "C_READ_SEAM",
            CheckId::GateAgreement => "C_GATE_AGREEMENT",
            CheckId::BudgetCompliance => "C_BUDGET_COMPLIANCE",
            CheckId::AttackerPanel => "C_ATTACKER_PANEL",
            CheckId::AuditCalibration => "C_AUDIT_CALIBRATION",
        }
    }
}

// ---------------------------------------------------------------------------
// The eleven core checks
// ---------------------------------------------------------------------------

/// Check 3, executed first. **Adversarial**, and the weakest link in the list
/// by construction: this is a declared-surface **coverage** check, which is a
/// *proxy* for faithfulness and not faithfulness itself. The assessment's own
/// figure is that faithfulness fails 42.5% against ≤13.1% for every other
/// property — and that figure is Opus 4.8 on SWE-bench, not a target.
pub fn check_faithfulness(c: &AdmissionCase) -> CheckOutcome {
    if c.required_surface.is_empty() {
        return CheckOutcome::negative(
            Stage::Create,
            prefix(
                CheckId::Faithfulness,
                "declares no required surface, so it cannot be unfaithful by construction — a case \
                 that covers nothing covers everything",
            ),
        );
    }
    let uncovered: Vec<&str> = c
        .required_surface
        .iter()
        .filter(|el| {
            let needle = el.to_lowercase();
            !c.steps
                .iter()
                .any(|s| s.expected.to_lowercase().contains(&needle))
        })
        .map(String::as_str)
        .collect();
    if uncovered.is_empty() {
        CheckOutcome::Pass
    } else {
        CheckOutcome::negative(
            Stage::Create,
            prefix(
                CheckId::Faithfulness,
                format!(
                    "required surface not covered by any step: {}",
                    uncovered.join(", ")
                ),
            ),
        )
    }
}

/// Check 1. The claimed resolution must equal the recomputed one.
pub fn check_resolution(c: &AdmissionCase) -> CheckOutcome {
    let recomputed = recomputed_resolution(c);
    if recomputed == c.resolution {
        CheckOutcome::Pass
    } else {
        CheckOutcome::negative(
            Stage::Solve,
            prefix(
                CheckId::Resolution,
                format!(
                    "claims {} but its own steps recompute to {}",
                    c.resolution.as_str(),
                    recomputed.as_str()
                ),
            ),
        )
    }
}

/// Check 2. The pre-fix twin must **move** the recomputed resolution. If
/// breaking a step leaves the verdict standing, the assertion is satisfied by
/// construction — the case is refused.
pub fn check_discrimination(c: &AdmissionCase) -> CheckOutcome {
    let Some(twin) = c.pre_fix_twin else {
        return CheckOutcome::negative(
            Stage::Create,
            prefix(
                CheckId::Discrimination,
                "declares no pre-fix twin, so nothing discriminates this case from a case that \
                 asserts nothing",
            ),
        );
    };
    if twin.step_index >= c.steps.len() {
        return CheckOutcome::negative(
            Stage::Create,
            prefix(
                CheckId::Discrimination,
                format!(
                    "pre-fix twin points at step {} of {} — past the last step",
                    twin.step_index,
                    c.steps.len()
                ),
            ),
        );
    }
    let mut broken = c.clone();
    // The index was bounds-checked immediately above, and this crate denies
    // `expect_used`, so the step is fetched with the SAME check rather than a
    // second one plus a panic path: an `else` that cannot be reached, and
    // returns the same Negative if it ever were.
    let Some(step) = broken.steps.get_mut(twin.step_index) else {
        return CheckOutcome::negative(
            Stage::Create,
            prefix(
                CheckId::Discrimination,
                format!(
                    "pre-fix twin points at step {} — no such step",
                    twin.step_index
                ),
            ),
        );
    };
    match twin.mutation {
        TwinMutation::UndoneStep => {
            step.actual = format!("{} (not performed)", step.expected);
        }
        TwinMutation::GateRejectedStep => {
            step.gate_verdict = GateVerdict::Reject;
        }
    }
    let after = recomputed_resolution(&broken);
    if after == recomputed_resolution(c) {
        CheckOutcome::negative(
            Stage::Create,
            prefix(
                CheckId::Discrimination,
                format!(
                    "the pre-fix twin leaves the resolution at {} — the assertion does not depend \
                     on the evidence it cites",
                    after.as_str()
                ),
            ),
        )
    } else {
        CheckOutcome::Pass
    }
}

/// Check 4. Every reference must name a kind in the closed vocabulary, a
/// locator, and a 64-character lowercase hex digest of the bytes it cites.
///
/// **Ceiling, stated because the check's name overstates it:** this verifies the
/// reference *addresses* bytes with a content digest. It does **not** re-read
/// those bytes, because the corpus does not ship them.
pub fn check_provenance(c: &AdmissionCase) -> CheckOutcome {
    if c.evidence_refs.is_empty() {
        return CheckOutcome::negative(
            Stage::Operate,
            prefix(CheckId::Provenance, "freezes no evidence reference"),
        );
    }
    for r in &c.evidence_refs {
        if !AdmissionCase::is_known_evidence_kind(&r.kind) {
            return CheckOutcome::negative(
                Stage::Operate,
                prefix(
                    CheckId::Provenance,
                    format!(
                        "evidence kind {:?} is outside the closed vocabulary",
                        r.kind
                    ),
                ),
            );
        }
        if r.locator.trim().is_empty() {
            return CheckOutcome::negative(
                Stage::Operate,
                prefix(
                    CheckId::Provenance,
                    format!("evidence of kind {} has an empty locator", r.kind),
                ),
            );
        }
        let digest_ok = r.digest.len() == 64
            && r.digest
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c));
        if !digest_ok {
            return CheckOutcome::negative(
                Stage::Operate,
                prefix(
                    CheckId::Provenance,
                    format!(
                        "evidence locator {} does not carry a 64-char lowercase hex digest",
                        r.locator
                    ),
                ),
            );
        }
    }
    CheckOutcome::Pass
}

/// Check 5. No withdrawn case, and no reliance on a superseded locator.
pub fn check_soundness(c: &AdmissionCase) -> CheckOutcome {
    if c.withdrawn {
        return CheckOutcome::negative(
            Stage::Evolve,
            prefix(
                CheckId::Soundness,
                "the case is withdrawn; a withdrawn case cannot be admitted",
            ),
        );
    }
    for r in &c.evidence_refs {
        if c.superseded_refs.iter().any(|s| s == &r.locator) {
            return CheckOutcome::negative(
                Stage::Evolve,
                prefix(
                    CheckId::Soundness,
                    format!("relies on superseded locator {}", r.locator),
                ),
            );
        }
    }
    CheckOutcome::Pass
}

/// Check 6. The claim carries a writer. `loopback` is a **valid** owner — it is
/// the house stamp for the opaque superuser — so it is not special-cased here.
pub fn check_owner_stamp(c: &AdmissionCase) -> CheckOutcome {
    let owner = c.owner.trim();
    if owner.is_empty() {
        return CheckOutcome::negative(
            Stage::Create,
            prefix(CheckId::OwnerStamp, "the claim carries no writer"),
        );
    }
    if matches!(
        owner.to_ascii_lowercase().as_str(),
        "unknown" | "undefined" | "null" | "none"
    ) {
        return CheckOutcome::negative(
            Stage::Create,
            prefix(
                CheckId::OwnerStamp,
                format!("the owner stamp {owner:?} names the absence of a writer"),
            ),
        );
    }
    CheckOutcome::Pass
}

/// Check 7. Unreachable from this crate; see [`CheckSpec::unreachable_reason`].
pub fn check_content_disposition(_c: &AdmissionCase) -> CheckOutcome {
    CheckOutcome::inapplicable(CONTENT_DISPOSITION_UNREACHABLE)
}

/// Check 8. Unreachable from this crate; see [`CheckSpec::unreachable_reason`].
pub fn check_read_seam(_c: &AdmissionCase) -> CheckOutcome {
    CheckOutcome::inapplicable(READ_SEAM_UNREACHABLE)
}

/// Check 9. A gate that rejected a step must not coexist with a case claiming
/// the run succeeded. The gate is the harder evidence; it wins.
pub fn check_gate_agreement(c: &AdmissionCase) -> CheckOutcome {
    let rejected = c
        .steps
        .iter()
        .filter(|s| s.gate_verdict == GateVerdict::Reject)
        .count();
    if rejected > 0 && c.resolution == Resolution::Resolved {
        return CheckOutcome::negative(
            Stage::Solve,
            prefix(
                CheckId::GateAgreement,
                format!(
                    "{rejected} step(s) were gate-rejected but the case claims {}",
                    Resolution::Resolved.as_str()
                ),
            ),
        );
    }
    if rejected > 0 && c.verified {
        return CheckOutcome::negative(
            Stage::Solve,
            prefix(
                CheckId::GateAgreement,
                format!("{rejected} step(s) were gate-rejected but the case claims verified"),
            ),
        );
    }
    CheckOutcome::Pass
}

/// Check 10. The case stayed inside its pinned budgets.
pub fn check_budget_compliance(c: &AdmissionCase) -> CheckOutcome {
    if c.steps.len() as u32 > c.budget.steps {
        return CheckOutcome::negative(
            Stage::Evolve,
            prefix(
                CheckId::BudgetCompliance,
                format!(
                    "{} steps against a pinned budget of {}",
                    c.steps.len(),
                    c.budget.steps
                ),
            ),
        );
    }
    if c.evidence_refs.len() as u32 > c.budget.evidence_refs {
        return CheckOutcome::negative(
            Stage::Evolve,
            prefix(
                CheckId::BudgetCompliance,
                format!(
                    "{} evidence references against a pinned budget of {}",
                    c.evidence_refs.len(),
                    c.budget.evidence_refs
                ),
            ),
        );
    }
    CheckOutcome::Pass
}

// ---------------------------------------------------------------------------
// The attacker panel (check 11)
// ---------------------------------------------------------------------------

/// A defect injected into a case. The panel's job is to notice; a mutant that
/// survives is a **measured miss**, and the miss rate is reported rather than
/// asserted away.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mutant {
    /// A required-surface element the case's steps do **not** cover. This is the
    /// defect check 3 exists to catch, so the mutant must ADD the gap, not
    /// remove the requirement: a mutant that merely pops an element from
    /// `required_surface` can never be caught by a coverage check, because
    /// removing a requirement can only make coverage look better.
    UncoveredSurface,
    CorruptDigest,
    FlipGateVerdict,
    OverdrawBudget,
    StripOwner,
    CiteSuperseded,
    /// **Unwatched by design.** The corpus's human verdict is a human boundary,
    /// and no mechanical check arbitrates it against the recomputed resolution.
    /// This mutant survives, and that non-zero miss rate is what makes
    /// [`CheckId::AuditCalibration`] load-bearing rather than decorative.
    HumanVerdictContradiction,
}

impl Mutant {
    pub fn all() -> Vec<Mutant> {
        vec![
            Mutant::UncoveredSurface,
            Mutant::CorruptDigest,
            Mutant::FlipGateVerdict,
            Mutant::OverdrawBudget,
            Mutant::StripOwner,
            Mutant::CiteSuperseded,
            Mutant::HumanVerdictContradiction,
        ]
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Mutant::UncoveredSurface => "MUT_UNCOVERED_SURFACE",
            Mutant::CorruptDigest => "MUT_CORRUPT_DIGEST",
            Mutant::FlipGateVerdict => "MUT_FLIP_GATE_VERDICT",
            Mutant::OverdrawBudget => "MUT_OVERDRAW_BUDGET",
            Mutant::StripOwner => "MUT_STRIP_OWNER",
            Mutant::CiteSuperseded => "MUT_CITE_SUPERSEDED",
            Mutant::HumanVerdictContradiction => "MUT_HUMAN_VERDICT_CONTRADICTION",
        }
    }
    /// The check that **must** catch this mutant, and the stage it must route
    /// to. `None` for [`Mutant::HumanVerdictContradiction`]: it is a declared
    /// survivor, and naming a watcher for it would make the panel total and
    /// therefore untestable.
    pub fn watcher(&self) -> Option<(CheckId, Stage)> {
        match self {
            Mutant::UncoveredSurface => Some((CheckId::Faithfulness, Stage::Create)),
            Mutant::CorruptDigest => Some((CheckId::Provenance, Stage::Operate)),
            Mutant::FlipGateVerdict => Some((CheckId::Resolution, Stage::Solve)),
            Mutant::OverdrawBudget => Some((CheckId::BudgetCompliance, Stage::Evolve)),
            Mutant::StripOwner => Some((CheckId::OwnerStamp, Stage::Create)),
            Mutant::CiteSuperseded => Some((CheckId::Soundness, Stage::Evolve)),
            Mutant::HumanVerdictContradiction => None,
        }
    }
    /// Apply the defect. Every mutant except the declared survivor is
    /// applicable to any case with the minimum shape (one surface element, one
    /// evidence ref, one step).
    pub fn apply(&self, c: &AdmissionCase) -> AdmissionCase {
        let mut m = c.clone();
        match self {
            Mutant::UncoveredSurface => {
                // Widen the CLAIM, leaving the evidence alone: the case now
                // asserts coverage it does not have.
                m.required_surface
                    .push("an element no step in this case ever addresses".into());
            }
            Mutant::CorruptDigest => {
                if let Some(r) = m.evidence_refs.first_mut() {
                    r.digest = "z".repeat(64);
                }
            }
            Mutant::FlipGateVerdict => {
                if let Some(s) = m.steps.first_mut() {
                    s.gate_verdict = GateVerdict::Reject;
                }
            }
            Mutant::OverdrawBudget => {
                m.budget.steps = 0;
                m.budget.evidence_refs = 0;
            }
            Mutant::StripOwner => {
                m.owner = String::new();
            }
            Mutant::CiteSuperseded => {
                if let Some(r) = m.evidence_refs.first() {
                    let locator = r.locator.clone();
                    m.superseded_refs.push(locator);
                } else {
                    m.withdrawn = true;
                }
            }
            Mutant::HumanVerdictContradiction => {
                m.human_pass = !m.human_pass;
            }
        }
        m
    }
}

/// Check 11. Runs every mechanical check against every mutant.
///
/// **The verdict is a measured miss rate, not a proof of absence.** A watched
/// mutant that nothing catches is a survivor and lands on the list. The
/// declared survivor is *verified*, not assumed: if a core check has started
/// reading the field it was declared safe on, the declaration is stale and the
/// panel reports that as its own negative rather than quietly going total.
pub fn check_attacker_panel(c: &AdmissionCase) -> CheckOutcome {
    let mut survivors: Vec<&'static str> = Vec::new();
    for mutant in Mutant::all() {
        let attacked = mutant.apply(c);
        let outcomes = run_core_checks(&attacked);
        let caught_by_name: Vec<&str> = outcomes
            .iter()
            .filter(|(_, o)| o.is_negative())
            .map(|(id, _)| id.as_str())
            .collect();
        match mutant.watcher() {
            Some((watched, stage)) => {
                let caught = outcomes.iter().any(|(id, outcome)| {
                    *id == watched
                        && matches!(
                            outcome,
                            CheckOutcome::Negative { stage: s, .. } if *s == stage
                        )
                });
                if !caught {
                    survivors.push(mutant.as_str());
                }
            }
            None => {
                // The declared survivor must ACTUALLY survive. If any core check
                // went Negative, the panel is total and its silence is the
                // flattering kind of lie — so that is a negative in its own
                // right, not a survivor.
                if caught_by_name.is_empty() {
                    survivors.push(mutant.as_str());
                } else {
                    return CheckOutcome::negative(
                        Stage::Evolve,
                        prefix(
                            CheckId::AttackerPanel,
                            format!(
                                "{} is declared unwatched but {} went negative against it — the \
                                 declaration is stale and the panel would be total",
                                mutant.as_str(),
                                caught_by_name.join(", ")
                            ),
                        ),
                    );
                }
            }
        }
    }
    if survivors.is_empty() {
        CheckOutcome::negative(
            Stage::Evolve,
            prefix(
                CheckId::AttackerPanel,
                "the panel missed every mutant — a panel that cannot miss cannot adjudicate, \
                 and its silence is not a verdict",
            ),
        )
    } else {
        CheckOutcome::Pass
    }
}

/// Check 12. Unreachable from this crate; see [`CheckSpec::unreachable_reason`].
pub fn check_audit_calibration(_c: &AdmissionCase) -> CheckOutcome {
    CheckOutcome::inapplicable(AUDIT_CALIBRATION_UNREACHABLE)
}

// ---------------------------------------------------------------------------
// The suite
// ---------------------------------------------------------------------------

/// Every check except the panel, in [`EXECUTION_ORDER`] minus check 11. The
/// panel recurses through this, so the panel itself must not be in it.
pub fn run_core_checks(c: &AdmissionCase) -> Vec<(CheckId, CheckOutcome)> {
    EXECUTION_ORDER
        .iter()
        .filter(|id| **id != CheckId::AttackerPanel)
        .map(|id| (*id, run_one(id, c)))
        .collect()
}

fn run_one(id: &CheckId, c: &AdmissionCase) -> CheckOutcome {
    match id {
        CheckId::Faithfulness => check_faithfulness(c),
        CheckId::Resolution => check_resolution(c),
        CheckId::Discrimination => check_discrimination(c),
        CheckId::Provenance => check_provenance(c),
        CheckId::Soundness => check_soundness(c),
        CheckId::OwnerStamp => check_owner_stamp(c),
        CheckId::ContentDisposition => check_content_disposition(c),
        CheckId::ReadSeam => check_read_seam(c),
        CheckId::GateAgreement => check_gate_agreement(c),
        CheckId::BudgetCompliance => check_budget_compliance(c),
        CheckId::AttackerPanel => check_attacker_panel(c),
        CheckId::AuditCalibration => check_audit_calibration(c),
    }
}

/// The suite's verdict on one case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionReport {
    pub case_id: String,
    pub outcomes: Vec<(CheckId, CheckOutcome)>,
}

impl AdmissionReport {
    /// The owning stages this case must be sent back to, in execution order.
    pub fn routes(&self) -> Vec<Stage> {
        self.outcomes
            .iter()
            .filter_map(|(_, o)| match o {
                CheckOutcome::Negative { stage, .. } => Some(*stage),
                _ => None,
            })
            .collect()
    }

    /// Inapplicabilities the author did **not** declare. Any of these refuses
    /// admission: a case cannot be admitted on inapplicability alone.
    pub fn undeclared_gaps(&self, c: &AdmissionCase) -> Vec<CheckId> {
        let mut gaps: Vec<CheckId> = Vec::new();
        for (id, outcome) in self.outcomes.iter() {
            if outcome.is_inapplicable() && c.accepts(*id).is_none() {
                gaps.push(*id);
            }
        }
        gaps
    }

    /// The preregistered admission rule. **Reads outcomes, not `is_pass()`**:
    /// rule (a) is "no check returns `Negative`", which is a different predicate
    /// from "every check returns `Pass`". Writing it as `all(is_pass)` makes
    /// admission unreachable while any check is inapplicable — which is always,
    /// because three are — and a rule that can never fire is not a rule.
    pub fn admitted(&self, c: &AdmissionCase) -> bool {
        !self.outcomes.iter().any(|(_, o)| o.is_negative())
            && self.undeclared_gaps(c).is_empty()
            && self.declared_gap_reasons_ok(c)
    }

    /// Every declared gap must carry a non-empty reason. A bare acceptance is
    /// an undeclared gap wearing a name.
    fn declared_gap_reasons_ok(&self, c: &AdmissionCase) -> bool {
        let mut ok = true;
        for (id, outcome) in self.outcomes.iter() {
            if outcome.is_inapplicable() {
                let declared = c.accepts(*id).map(str::trim).unwrap_or_default();
                if declared.is_empty() {
                    ok = false;
                }
            }
        }
        ok
    }

    /// The receipt: a case is only admitted *with its gaps named*, so an
    /// admitted case never looks fully green.
    pub fn receipt(&self) -> String {
        let passes = self.outcomes.iter().filter(|(_, o)| o.is_pass()).count();
        let gaps: Vec<&str> = self
            .outcomes
            .iter()
            .filter(|(_, o)| o.is_inapplicable())
            .map(|(id, _)| id.as_str())
            .collect();
        let negatives: Vec<&str> = self
            .outcomes
            .iter()
            .filter_map(|(id, o)| match o {
                CheckOutcome::Negative { .. } => Some(id.as_str()),
                _ => None,
            })
            .collect();
        format!(
            "{}: {passes} pass / {} inapplicable [{}] / {} negative [{}]",
            self.case_id,
            gaps.len(),
            gaps.join(", "),
            negatives.len(),
            negatives.join(", ")
        )
    }
}

/// Run the whole suite. Execution order is [`EXECUTION_ORDER`].
pub fn admit(c: &AdmissionCase) -> AdmissionReport {
    AdmissionReport {
        case_id: c.id.clone(),
        outcomes: EXECUTION_ORDER
            .iter()
            .map(|id| (*id, run_one(id, c)))
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// The refused action
// ---------------------------------------------------------------------------

/// Why the gate will not repair a case. There is no success variant, and
/// [`patch_at_gate`] returns [`Infallible`] in the `Ok` position — so "patched
/// at the gate" is **not a state this type can represent**, not a branch a
/// future edit could take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchRefusal {
    /// Only the owning stage may revise the case.
    StageOwnsDefect { stage: Stage, check: CheckId },
}

impl PatchRefusal {
    pub fn as_str(&self) -> &'static str {
        "AD_GATE_PATCH_REFUSED"
    }
}

impl std::fmt::Display for PatchRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PatchRefusal::StageOwnsDefect { stage, check } => write!(
                f,
                "{}: {} owns this defect; it goes back to {}, not to the gate",
                self.as_str(),
                check.as_str(),
                stage.as_str()
            ),
        }
    }
}

impl std::error::Error for PatchRefusal {}

/// The refused action, made unrepresentable rather than merely forbidden.
///
/// Total, and always an `Err`: the `Ok` type is [`Infallible`], so no caller
/// can obtain a patched case from the gate. A negative check's remedy is
/// revision **at the owning stage** — the SWE-Proof rule, mechanical.
pub fn patch_at_gate(stage: Stage, check: CheckId) -> Result<Infallible, PatchRefusal> {
    Err(PatchRefusal::StageOwnsDefect { stage, check })
}

// ---------------------------------------------------------------------------
// The suite's own tests
//
// Every test here asserts on a real function's real return value. None scans
// this file's source for a string: a pin that greps its own subject passes when
// the subject is reflowed and fails when it is renamed, which is a comment
// about the prose rather than about the gate.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    // -- helpers -------------------------------------------------------------

    /// One case out of the shipped admission pack, by id.
    fn case(id: &str) -> AdmissionCase {
        crate::admission_cases()
            .unwrap()
            .into_iter()
            .find(|c| c.id == id)
            .unwrap()
    }

    /// The pack's one well-formed case: the control every negative path is
    /// measured against.
    fn clean_case() -> AdmissionCase {
        case("care_inquiry_clean")
    }

    /// The reason an outcome carries. `Pass` has none, so it reads as empty —
    /// callers assert on a state before they assert on a reason.
    fn reason_of(outcome: &CheckOutcome) -> &str {
        match outcome {
            CheckOutcome::Pass => "",
            CheckOutcome::Inapplicable { reason } => reason,
            CheckOutcome::Negative { reason, .. } => reason,
        }
    }

    /// The routing predicate under test everywhere below: a negative names the
    /// stage that **owns** the defect, and owning the wrong stage is a wrong
    /// answer even though the verdict is right.
    fn is_negative_at(outcome: &CheckOutcome, stage: Stage) -> bool {
        matches!(outcome, CheckOutcome::Negative { stage: s, .. } if *s == stage)
    }

    fn ids(of: &[(CheckId, CheckOutcome)]) -> Vec<CheckId> {
        of.iter().map(|(id, _)| *id).collect()
    }

    /// The type-level half of the patch refusal: if `patch_at_gate`'s return
    /// type ever grew a success variant, this coercion stops compiling.
    fn assert_infallible_ok_is_impossible<T>(r: Result<T, PatchRefusal>) -> bool {
        r.is_err()
    }

    // -- A. the registry is closed and honestly typed -------------------------

    #[test]
    fn the_check_registry_is_twelve_and_closed() {
        assert_eq!(EXECUTION_ORDER.len(), 12);
        let registered: BTreeSet<CheckId> = EXECUTION_ORDER.iter().copied().collect();
        assert_eq!(
            registered.len(),
            12,
            "a check is registered twice or not at all"
        );

        // The plan's taxonomy: numbers 1..12, one per check, no gaps.
        let plan: [(CheckId, u8); 12] = [
            (CheckId::Resolution, 1),
            (CheckId::Discrimination, 2),
            (CheckId::Faithfulness, 3),
            (CheckId::Provenance, 4),
            (CheckId::Soundness, 5),
            (CheckId::OwnerStamp, 6),
            (CheckId::ContentDisposition, 7),
            (CheckId::ReadSeam, 8),
            (CheckId::GateAgreement, 9),
            (CheckId::BudgetCompliance, 10),
            (CheckId::AttackerPanel, 11),
            (CheckId::AuditCalibration, 12),
        ];
        for (id, number) in plan {
            let got = spec(id);
            assert_eq!(got.number, number, "{} is not check {number}", id.as_str());
            assert_eq!(got.id, id, "spec answered about a different check");
        }

        // Execution order is deliberately NOT the numbering order: faithfulness
        // is the dominant failure in the population, so it runs first even
        // though the plan numbered it third.
        assert_eq!(EXECUTION_ORDER[0], CheckId::Faithfulness);
        assert_eq!(spec(CheckId::Faithfulness).number, 3);
    }

    #[test]
    fn exactly_three_checks_are_unreachable_and_each_carries_a_reason() {
        let unreachable: BTreeSet<CheckId> = EXECUTION_ORDER
            .iter()
            .filter(|id| spec(**id).unreachable_reason.is_some())
            .copied()
            .collect();
        let expected: BTreeSet<CheckId> = [
            CheckId::ContentDisposition,
            CheckId::ReadSeam,
            CheckId::AuditCalibration,
        ]
        .into_iter()
        .collect();
        assert_eq!(unreachable, expected);

        for id in unreachable {
            let name = id.as_str();
            let reason = spec(id).unreachable_reason.unwrap();
            assert!(
                !reason.trim().is_empty(),
                "{name} is unreachable with no reason at all"
            );
            assert!(
                reason.contains("REOPEN"),
                "{name} must name the re-measurement path, not only its absence: {reason}"
            );
        }
    }

    #[test]
    fn every_check_id_as_str_is_distinct_and_prefixed() {
        let names: BTreeSet<&'static str> = EXECUTION_ORDER.iter().map(|id| id.as_str()).collect();
        assert_eq!(names.len(), 12, "two checks share an id string");
        for id in EXECUTION_ORDER {
            let name = id.as_str();
            assert!(
                name.starts_with("C_"),
                "{name} is not a C_-prefixed check id"
            );
        }
    }

    #[test]
    fn every_check_id_is_reachable_from_run_one() {
        let c = clean_case();
        for id in EXECUTION_ORDER {
            let outcome = run_one(&id, &c);
            assert!(
                matches!(
                    outcome,
                    CheckOutcome::Pass
                        | CheckOutcome::Inapplicable { .. }
                        | CheckOutcome::Negative { .. }
                ),
                "{} answered with a state outside the three-variant enum",
                id.as_str()
            );
        }
        // The registry and the dispatcher agree: the suite runs exactly what the
        // registry says it runs, and the unreachable three answer INAPPLICABLE
        // through the dispatch seam rather than being stubbed to Pass.
        assert_eq!(admit(&c).outcomes.len(), EXECUTION_ORDER.len());
        for id in [
            CheckId::ContentDisposition,
            CheckId::ReadSeam,
            CheckId::AuditCalibration,
        ] {
            let name = id.as_str();
            assert!(
                run_one(&id, &c).is_inapplicable(),
                "{name} dispatched as a pass"
            );
        }
    }

    // -- B. the pack loads and the frozen seven are untouched -----------------

    #[test]
    fn the_frozen_seven_are_unchanged_by_this_round() {
        let frozen = crate::all().unwrap();
        assert_eq!(
            frozen.len(),
            7,
            "this round added a case to the frozen accessor"
        );
        for c in &frozen {
            let family = c.family.as_str();
            assert!(
                family == "qc_report" || family == "gdl_cases",
                "frozen case {} carries family {family}",
                c.id
            );
            assert!(
                family != "admission",
                "frozen case {} is an admission case",
                c.id
            );
        }
    }

    #[test]
    fn the_admission_pack_is_five_cases_with_distinct_ids() {
        let pack = crate::admission_cases().unwrap();
        assert_eq!(pack.len(), 5);
        let distinct: BTreeSet<&str> = pack.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(distinct.len(), 5, "two admission cases share an id");
        for c in &pack {
            let id = c.id.as_str();
            assert_eq!(c.family, "admission", "{id} is not in the admission family");
            assert!(
                c.kappa_units >= crate::KAPPA_GATE_UNITS,
                "{id} is labeled at kappa {} below the {} floor",
                c.kappa_units,
                crate::KAPPA_GATE_UNITS
            );
        }
    }

    #[test]
    fn the_admission_pack_does_not_alias_the_frozen_pack() {
        let frozen: BTreeSet<String> = crate::all().unwrap().into_iter().map(|c| c.id).collect();
        let admission: BTreeSet<String> = crate::admission_cases()
            .unwrap()
            .into_iter()
            .map(|c| c.id)
            .collect();
        let shared: Vec<&String> = frozen.intersection(&admission).collect();
        assert!(
            shared.is_empty(),
            "an id is served by both accessors: {shared:?}"
        );
    }

    // -- C. the ADMITTED case ------------------------------------------------

    #[test]
    fn a_well_formed_case_is_admitted_with_its_gaps_named() {
        let c = clean_case();
        let report = admit(&c);
        assert!(
            report.admitted(&c),
            "the clean case must be admitted: {}",
            report.receipt()
        );
        assert!(
            report.routes().is_empty(),
            "a clean case routes nowhere: {}",
            report.receipt()
        );
        assert!(report.undeclared_gaps(&c).is_empty());

        assert_eq!(report.outcomes.len(), 12);

        // Admitted is NOT green. The receipt names every gap the admission
        // carried, so a reader of the receipt cannot mistake it for a sweep.
        let receipt = report.receipt();
        for id in [
            CheckId::ContentDisposition,
            CheckId::ReadSeam,
            CheckId::AuditCalibration,
        ] {
            let name = id.as_str();
            assert!(
                receipt.contains(name),
                "the receipt omits gap {name}: {receipt}"
            );
        }
        let passes = report.outcomes.iter().filter(|(_, o)| o.is_pass()).count();
        let gaps = report
            .outcomes
            .iter()
            .filter(|(_, o)| o.is_inapplicable())
            .count();
        assert_eq!(
            (passes, gaps),
            (9, 3),
            "an admitted case must still carry its three gaps"
        );
    }

    #[test]
    fn the_clean_case_routes_nowhere_and_no_check_goes_negative() {
        let c = clean_case();
        let report = admit(&c);
        assert!(report.routes().is_empty());
        for (id, outcome) in &report.outcomes {
            let name = id.as_str();
            assert!(
                !outcome.is_negative(),
                "{name} went negative: {}",
                reason_of(outcome)
            );
        }
    }

    // -- D. RED-PROOF: a negative routes to the stage that owns the defect ----

    #[test]
    fn a_create_stage_defect_routes_to_create() {
        let c = case("create_defect_surface");
        assert!(
            c.pre_fix_twin.is_none(),
            "the fixture's whole defect is the absent twin"
        );

        let discrimination = check_discrimination(&c);
        assert!(
            is_negative_at(&discrimination, Stage::Create),
            "expected Create, got {discrimination:?}"
        );
        let report = admit(&c);
        assert!(!report.admitted(&c), "a non-discriminating case is refused");
        assert!(
            report.routes().contains(&Stage::Create),
            "routes: {:?}",
            report.routes()
        );
    }

    #[test]
    fn a_solve_stage_defect_routes_to_solve() {
        let c = case("solve_defect_resolution");
        let resolution = check_resolution(&c);
        assert!(
            is_negative_at(&resolution, Stage::Solve),
            "expected Solve, got {resolution:?}"
        );
        assert!(
            reason_of(&resolution).contains("recompute"),
            "{resolution:?}"
        );

        let report = admit(&c);
        assert!(!report.admitted(&c));
        let routes = report.routes();
        assert!(routes.contains(&Stage::Solve), "routes: {routes:?}");
        assert!(
            !routes.contains(&Stage::Create),
            "a solve-stage defect must not be re-filed at create: {routes:?}"
        );
    }

    #[test]
    fn an_unfaithful_surface_routes_to_create() {
        let c = case("unfaithful_slice");
        let faithfulness = check_faithfulness(&c);
        assert!(
            is_negative_at(&faithfulness, Stage::Create),
            "expected Create, got {faithfulness:?}"
        );
        assert!(
            reason_of(&faithfulness).contains("required surface not covered"),
            "{faithfulness:?}"
        );

        let report = admit(&c);
        assert!(!report.admitted(&c), "a slice is not a case");
        assert!(
            report.routes().contains(&Stage::Create),
            "routes: {:?}",
            report.routes()
        );
    }

    #[test]
    fn patching_at_the_gate_is_refused_for_every_stage_and_check() {
        let stages = [
            Stage::Create,
            Stage::Solve,
            Stage::Evolve,
            Stage::Deflect,
            Stage::Operate,
        ];
        for stage in stages {
            for id in EXECUTION_ORDER {
                let refusal = patch_at_gate(stage, id).expect_err("the gate has no Ok to hand out");
                assert_eq!(refusal.as_str(), "AD_GATE_PATCH_REFUSED");
            }
        }
    }

    #[test]
    fn the_patch_refusal_names_the_owning_stage_not_the_gate() {
        let refusal = PatchRefusal::StageOwnsDefect {
            stage: Stage::Create,
            check: CheckId::Faithfulness,
        };
        let rendered = refusal.to_string();
        assert!(rendered.contains("AD_GATE_PATCH_REFUSED"), "{rendered}");
        assert!(rendered.contains("C_FAITHFULNESS"), "{rendered}");
        assert!(rendered.contains("S_CREATE"), "{rendered}");
    }

    #[test]
    fn patch_at_gate_cannot_return_ok_because_its_ok_type_is_infallible() {
        // Type-level, not a runtime branch: if the signature ever grew a
        // success variant this coercion stops compiling.
        let ok_type_is_infallible: fn(Stage, CheckId) -> Result<Infallible, PatchRefusal> =
            patch_at_gate;

        assert!(assert_infallible_ok_is_impossible(ok_type_is_infallible(
            Stage::Solve,
            CheckId::Resolution
        )));

        // And the Ok arm is uninhabited: matching it is the empty arm, so a
        // future Ok variant makes this match NON-exhaustive rather than being
        // silently handled.
        match ok_type_is_infallible(Stage::Operate, CheckId::Provenance) {
            Err(PatchRefusal::StageOwnsDefect { stage, check }) => {
                assert_eq!(stage, Stage::Operate);
                assert_eq!(check, CheckId::Provenance);
            }
            Ok(never) => match never {},
        }
    }

    // -- E. RED-PROOF: inapplicable is not passing ----------------------------

    #[test]
    fn an_undeclared_inapplicability_refuses_admission() {
        let declared = clean_case();
        let mut c = declared.clone();
        c.accepted_gaps.clear();

        let report = admit(&c);
        let undeclared: BTreeSet<CheckId> = report.undeclared_gaps(&c).into_iter().collect();
        let expected: BTreeSet<CheckId> = [
            CheckId::ContentDisposition,
            CheckId::ReadSeam,
            CheckId::AuditCalibration,
        ]
        .into_iter()
        .collect();
        assert_eq!(undeclared, expected);
        assert!(
            !report.admitted(&c),
            "a case cannot be admitted on inapplicability alone: {}",
            report.receipt()
        );

        // The refusal came from the DECLARATION, not from a defect: not one
        // check went negative, which is what makes this the anti-conflation pin.
        for (id, outcome) in &report.outcomes {
            let name = id.as_str();
            assert!(
                !outcome.is_negative(),
                "{name} went negative: {}",
                reason_of(outcome)
            );
        }
        // Control: the identical case, with the gaps declared, is admitted.
        assert!(admit(&declared).admitted(&declared));
    }

    #[test]
    fn a_declared_gap_with_a_blank_reason_is_refused() {
        let mut c = clean_case();
        let target = c.accepted_gaps[0].check;
        c.accepted_gaps[0].reason = "   ".into();

        let report = admit(&c);
        // It is still DECLARED, so it is not undeclared — the refusal is the
        // blank reason. A bare acceptance is an undeclared gap wearing a name.
        assert!(!report.undeclared_gaps(&c).contains(&target));
        assert!(!report.admitted(&c), "a blank reason is not a reason");
        let name = target.as_str();
        assert!(
            report
                .outcomes
                .iter()
                .any(|(id, o)| *id == target && o.is_inapplicable()),
            "{name} should still read inapplicable"
        );
    }

    #[test]
    fn an_inapplicable_outcome_is_never_a_pass() {
        let c = clean_case();
        let report = admit(&c);
        let gaps: Vec<CheckId> = report
            .outcomes
            .iter()
            .filter(|(_, o)| o.is_inapplicable())
            .map(|(id, _)| *id)
            .collect();
        assert_eq!(
            gaps.len(),
            3,
            "expected three inapplicable outcomes, got {:?}",
            ids(&report.outcomes)
        );
        for id in gaps {
            let name = id.as_str();
            let outcome = run_one(&id, &c);
            assert!(
                outcome.is_inapplicable(),
                "{name} did not answer inapplicable"
            );
            assert!(
                !outcome.is_pass(),
                "{name} conflated inapplicable with a pass"
            );
            assert!(
                !outcome.is_negative(),
                "{name} answered negative, which is a different claim"
            );
        }
    }

    #[test]
    fn inapplicable_is_a_distinct_variant_not_a_pass_with_a_reason() {
        let gap = CheckOutcome::inapplicable("x");
        assert_ne!(gap, CheckOutcome::Pass);
        assert_ne!(
            CheckOutcome::inapplicable("x"),
            CheckOutcome::inapplicable("y")
        );
        assert!(gap.is_inapplicable() && !gap.is_pass() && !gap.is_negative());

        let neg = CheckOutcome::negative(Stage::Solve, "x");
        assert_ne!(neg, CheckOutcome::Pass);
        assert_ne!(neg, gap);
        assert!(neg.is_negative() && !neg.is_pass() && !neg.is_inapplicable());

        let green = CheckOutcome::Pass;
        assert!(green.is_pass() && !green.is_inapplicable() && !green.is_negative());

        // The reason survives the variant: an inapplicable outcome is never a
        // bare marker, and a negative always says which stage owns it.
        assert_eq!(reason_of(&gap), "x");
        assert_eq!(reason_of(&neg), "x");
    }

    // -- F. RED-PROOF: the vacuity refusal ------------------------------------

    #[test]
    fn a_case_whose_twin_does_not_move_the_verdict_is_refused() {
        let c = case("vacuous_twin");
        // Why it is vacuous: it was already unresolved on its own evidence, so
        // breaking a step cannot move the verdict.
        assert_eq!(c.resolution, Resolution::Unresolved);
        assert_eq!(recomputed_resolution(&c), Resolution::Unresolved);

        let discrimination = check_discrimination(&c);
        assert!(
            is_negative_at(&discrimination, Stage::Create),
            "expected Create, got {discrimination:?}"
        );
        assert!(
            reason_of(&discrimination).contains("does not depend"),
            "the refusal must say the assertion is not load-bearing: {discrimination:?}"
        );

        let report = admit(&c);
        assert!(
            !report.admitted(&c),
            "a vacuous case is refused: {}",
            report.receipt()
        );
        assert!(report.routes().contains(&Stage::Create));
    }

    #[test]
    fn a_case_with_no_twin_is_refused_as_non_discriminating() {
        let c = case("create_defect_surface");
        assert!(c.pre_fix_twin.is_none());
        let discrimination = check_discrimination(&c);
        assert!(
            is_negative_at(&discrimination, Stage::Create),
            "expected Create, got {discrimination:?}"
        );
        assert!(
            reason_of(&discrimination).contains("no pre-fix twin"),
            "{discrimination:?}"
        );
    }

    #[test]
    fn an_empty_required_surface_is_refused_as_vacuously_faithful() {
        let mut c = clean_case();
        c.required_surface.clear();
        let faithfulness = check_faithfulness(&c);
        assert!(
            is_negative_at(&faithfulness, Stage::Create),
            "expected Create, got {faithfulness:?}"
        );
        assert!(
            reason_of(&faithfulness).contains("cannot be unfaithful by construction"),
            "{faithfulness:?}"
        );
        // Control: with the surface declared, the same case is faithful.
        assert!(check_faithfulness(&clean_case()).is_pass());
    }

    // -- G. the panel is an instrument, not a rubber stamp --------------------

    #[test]
    fn every_watched_mutant_is_caught_by_its_named_watcher() {
        let c = clean_case();
        let mut watched = 0usize;
        for mutant in Mutant::all() {
            let Some((watcher, stage)) = mutant.watcher() else {
                continue;
            };
            watched += 1;
            let attacked = mutant.apply(&c);
            let outcomes = run_core_checks(&attacked);
            let caught = outcomes
                .iter()
                .any(|(id, o)| *id == watcher && is_negative_at(o, stage));
            assert!(
                caught,
                "{} declared {} as its watcher at {}, and nothing negative came back: {outcomes:?}",
                mutant.as_str(),
                watcher.as_str(),
                stage.as_str()
            );
        }
        assert_eq!(watched, 6, "six of the seven mutants declare a watcher");
    }

    #[test]
    fn the_panel_passes_by_measuring_survivors_not_by_absence() {
        let c = clean_case();
        let panel = check_attacker_panel(&c);
        assert!(panel.is_pass(), "the clean case's panel verdict: {panel:?}");

        // The panel is NOT total. Its single DECLARED survivor genuinely
        // survives every mechanical check, which is precisely why the panel's
        // Pass is a measurement of survivors and not a proof of absence — and
        // why check 12 (calibration on out-of-loop artifacts) is load-bearing.
        let flipped = Mutant::HumanVerdictContradiction.apply(&c);
        let outcomes = run_core_checks(&flipped);
        let negatives: Vec<&str> = outcomes
            .iter()
            .filter(|(_, o)| o.is_negative())
            .map(|(id, _)| id.as_str())
            .collect();
        assert!(
            negatives.is_empty(),
            "the declared survivor MUT_HUMAN_VERDICT_CONTRADICTION was expected to survive the \
             core checks and did not: {negatives:?}"
        );

        // And the only reason it survives is that no mechanical check reads the
        // human verdict — pinned by flipping it in BOTH directions and getting
        // the same silence, so this cannot pass by accident.
        let flipped_back = Mutant::HumanVerdictContradiction.apply(&flipped);
        assert_eq!(
            flipped_back.human_pass, c.human_pass,
            "the survivor mutant is its own inverse, so both directions leave the verdict unchanged"
        );
        assert!(
            run_core_checks(&flipped_back)
                .iter()
                .all(|(_, o)| !o.is_negative()),
            "flipping the human verdict twice must be just as silent as flipping it once"
        );
    }

    #[test]
    fn a_panel_that_missed_everything_would_be_negative() {
        // The panel is Negative only when NOTHING survives, so the survivor list
        // has to be non-empty for a healthy case to read Pass. The declared
        // survivor is what keeps it so.
        let survivor = Mutant::HumanVerdictContradiction;
        assert!(
            survivor.watcher().is_none(),
            "a watcher here would make the panel total, and therefore untestable"
        );

        let c = clean_case();
        let attacked = survivor.apply(&c);
        let outcomes = run_core_checks(&attacked);
        assert!(
            outcomes.iter().all(|(_, o)| !o.is_negative()),
            "the declared survivor must actually survive, or the declaration is a lie: {outcomes:?}"
        );

        // It survives in BOTH directions: flipping the human verdict is a real
        // edit and no core check reads the human verdict at all.
        let twice = survivor.apply(&attacked);
        assert_ne!(attacked.human_pass, twice.human_pass);
        assert_eq!(
            twice.human_pass, c.human_pass,
            "two flips return to the original verdict"
        );
        assert!(
            run_core_checks(&twice)
                .iter()
                .all(|(_, o)| !o.is_negative())
        );
    }

    #[test]
    fn the_miss_rate_is_reported_not_asserted_away() {
        let all = Mutant::all();
        assert_eq!(all.len(), 7);
        let unwatched: Vec<&str> = all
            .iter()
            .filter(|m| m.watcher().is_none())
            .map(|m| m.as_str())
            .collect();
        assert_eq!(unwatched.len(), 1, "the panel reports one miss, not zero");
        assert_eq!(unwatched[0], "MUT_HUMAN_VERDICT_CONTRADICTION");

        // Every declared watcher names a check the registry can actually run,
        // at a stage the loop actually has. A watcher for an unreachable check
        // would be a decorator.
        let stages = [
            Stage::Create,
            Stage::Solve,
            Stage::Evolve,
            Stage::Deflect,
            Stage::Operate,
        ];
        for m in &all {
            let Some((id, stage)) = m.watcher() else {
                continue;
            };
            let name = m.as_str();
            assert!(
                EXECUTION_ORDER.contains(&id),
                "{name} watches an unregistered check"
            );
            assert!(
                spec(id).unreachable_reason.is_none(),
                "{name} watches a check that is unreachable from this crate"
            );
            assert!(
                stages.contains(&stage),
                "{name} names a stage outside the five-stage loop"
            );
        }
    }

    // -- H. each remaining check is real (red-proofed by mutation) -----------

    #[test]
    fn each_mechanical_check_fires_on_its_own_pathology() {
        type Edit = fn(&mut AdmissionCase);
        struct Path {
            label: &'static str,
            edit: Edit,
            check: CheckId,
            stage: Stage,
        }

        let paths: [Path; 10] = [
            Path {
                label: "provenance / non-hex digest",
                edit: |c: &mut AdmissionCase| c.evidence_refs[0].digest = "z".repeat(64),
                check: CheckId::Provenance,
                stage: Stage::Operate,
            },
            Path {
                label: "provenance / kind outside the vocabulary",
                edit: |c: &mut AdmissionCase| c.evidence_refs[0].kind = "telepathy".into(),
                check: CheckId::Provenance,
                stage: Stage::Operate,
            },
            Path {
                label: "provenance / no evidence at all",
                edit: |c: &mut AdmissionCase| c.evidence_refs.clear(),
                check: CheckId::Provenance,
                stage: Stage::Operate,
            },
            Path {
                label: "soundness / withdrawn",
                edit: |c: &mut AdmissionCase| c.withdrawn = true,
                check: CheckId::Soundness,
                stage: Stage::Evolve,
            },
            Path {
                label: "soundness / relies on a superseded locator",
                edit: |c: &mut AdmissionCase| {
                    let locator = c.evidence_refs[0].locator.clone();
                    c.superseded_refs.push(locator);
                },
                check: CheckId::Soundness,
                stage: Stage::Evolve,
            },
            Path {
                label: "owner stamp / blank",
                edit: |c: &mut AdmissionCase| c.owner = String::new(),
                check: CheckId::OwnerStamp,
                stage: Stage::Create,
            },
            Path {
                label: "owner stamp / names the absence",
                edit: |c: &mut AdmissionCase| c.owner = "unknown".into(),
                check: CheckId::OwnerStamp,
                stage: Stage::Create,
            },
            Path {
                label: "gate agreement / reject under a Resolved claim",
                edit: |c: &mut AdmissionCase| c.steps[0].gate_verdict = GateVerdict::Reject,
                check: CheckId::GateAgreement,
                stage: Stage::Solve,
            },
            Path {
                label: "budget / steps",
                edit: |c: &mut AdmissionCase| c.budget.steps = 0,
                check: CheckId::BudgetCompliance,
                stage: Stage::Evolve,
            },
            Path {
                label: "budget / evidence refs",
                edit: |c: &mut AdmissionCase| c.budget.evidence_refs = 0,
                check: CheckId::BudgetCompliance,
                stage: Stage::Evolve,
            },
        ];

        for p in &paths {
            let mut c = clean_case();
            // The control: the named check is green before the edit, so the
            // negative below is attributable to the pathology and not to the
            // fixture already being broken.
            let before = run_one(&p.check, &c);
            assert!(
                before.is_pass(),
                "{}: control must start green, got {before:?}",
                p.label
            );

            (p.edit)(&mut c);
            let outcome = run_one(&p.check, &c);
            assert!(
                is_negative_at(&outcome, p.stage),
                "{}: {} did not go negative at {} — got {outcome:?}",
                p.label,
                p.check.as_str(),
                p.stage.as_str()
            );
        }

        // Control: `loopback` is a real owner, so the same seam must NOT fire.
        let mut loopback = clean_case();
        loopback.owner = "loopback".into();
        assert!(
            check_owner_stamp(&loopback).is_pass(),
            "loopback is the house stamp for the opaque superuser"
        );
    }

    #[test]
    fn a_valid_hex_digest_passes_and_an_uppercase_one_does_not() {
        let c = clean_case();
        assert!(
            check_provenance(&c).is_pass(),
            "the control case's digests are already valid"
        );

        let mut upper = c.clone();
        let original = upper.evidence_refs[0].digest.clone();
        let mut mutated = original.clone();
        mutated.replace_range(0..1, "A");
        upper.evidence_refs[0].digest = mutated;

        // Same length, one character different: the check is on the ALPHABET, not
        // on the length.
        assert_eq!(upper.evidence_refs[0].digest.len(), original.len());
        assert_ne!(upper.evidence_refs[0].digest, original);

        let outcome = check_provenance(&upper);
        assert!(
            is_negative_at(&outcome, Stage::Operate),
            "an uppercase hex digest is not the lowercase hex the check pins: {outcome:?}"
        );
        assert!(
            reason_of(&outcome).contains("64-char lowercase hex"),
            "{outcome:?}"
        );
    }

    #[test]
    fn the_evidence_kind_vocabulary_is_closed() {
        for kind in [
            "run",
            "audit",
            "handoff",
            "finding",
            "contradiction",
            "contact-history",
        ] {
            assert!(
                AdmissionCase::is_known_evidence_kind(kind),
                "{kind} is in the vocabulary"
            );
        }
        // Closed means closed: no case folding, no trimming, no near misses.
        for kind in ["telepathy", "Run", "run ", "", "contact_history", "RUn"] {
            assert!(
                !AdmissionCase::is_known_evidence_kind(kind),
                "{kind:?} is outside the closed vocabulary"
            );
        }
        // And every kind the shipped pack cites is inside it.
        for c in crate::admission_cases().unwrap() {
            for r in &c.evidence_refs {
                assert!(
                    AdmissionCase::is_known_evidence_kind(&r.kind),
                    "{} cites kind {}",
                    c.id,
                    r.kind
                );
            }
        }
    }

    // -- I. the recomputation spine -------------------------------------------

    #[test]
    fn recomputed_resolution_follows_the_steps_not_the_claim() {
        let clean = clean_case();
        assert_eq!(recomputed_resolution(&clean), Resolution::Resolved);
        assert_eq!(clean.resolution, Resolution::Resolved);

        let solve = case("solve_defect_resolution");
        assert_eq!(recomputed_resolution(&solve), Resolution::Unresolved);
        assert_eq!(
            solve.resolution,
            Resolution::Resolved,
            "the case CLAIMS resolved; its own steps recompute otherwise"
        );

        let mut rejected = clean.clone();
        rejected.steps[0].gate_verdict = GateVerdict::Reject;
        assert_eq!(
            recomputed_resolution(&rejected),
            Resolution::Escalated,
            "a gate-rejected step escalates whatever the header says"
        );

        // Every step performed, handoff unverified: still not resolved.
        let mut unverified = clean.clone();
        unverified.verified = false;
        unverified.handoff_complete = false;
        assert_eq!(recomputed_resolution(&unverified), Resolution::Unresolved);
    }

    #[test]
    fn loopback_is_a_valid_owner_because_it_is_the_house_stamp() {
        let stamped = |owner: &str| {
            let mut c = clean_case();
            c.owner = owner.to_string();
            check_owner_stamp(&c)
        };
        assert!(stamped("loopback").is_pass());
        assert!(stamped("operator@care").is_pass());
        assert_eq!(stamped("loopback"), CheckOutcome::Pass);

        // What is refused is the ABSENCE of a writer, and only the tokens that
        // literally name that absence.
        for absent in ["", "   ", "unknown", "UNDEFINED", "null", "None"] {
            assert!(
                is_negative_at(&stamped(absent), Stage::Create),
                "{absent:?} names the absence of a writer"
            );
        }
    }
}
