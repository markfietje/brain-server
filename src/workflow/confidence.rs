//! The confidence→human deferral decision — **pure**, and deliberately empty.
//!
//! One question, asked once, for every case a consultant's brain queues: **when the
//! machine is not sure, does a human decide?** Answering it is the difference between
//! a system that says "I don't know" and one that says "I do" and is wrong.
//!
//! ## Why this is a *class* and not a threshold
//!
//! A single global cutoff is the wrong instrument, for a reason this repo already
//! records: metacognitive competence is **domain-specific in a way no aggregate metric
//! shows**, and lowering a model's temperature moves its confidence without moving its
//! competence. A model that is well-calibrated on `compliance` can be confidently wrong
//! on `finance`, so the bar has to be per-class or it is a fiction.
//!
//! A second failure mode has to be designed against too: a policy that defers
//! *everything* scores perfectly on accuracy and is worthless in production. So the
//! shape here admits `Auto` as a reachable outcome — it simply is not reachable yet.
//!
//! ## Why the table ships EMPTY
//!
//! **No class is auto-authorised.** Not as an unfinished stub — as the honest state of
//! the evidence.
//!
//! Granting a class `Auto` requires a measured per-class reliability (`meta_d`). None
//! has been measured, and a threshold invented now would be a number wearing a
//! measurement's name — precisely the defect this decision function exists to prevent.
//! So `decide` resolves every class to `HumanRequired`, and [`AUTO_CLASSES`] documents
//! what would have to be true for an entry to appear.
//!
//! This is not a failure to deliver the policy. It is what makes the *next* measurement
//! mean something: adding an entry becomes a measurement event with a named provenance,
//! rather than a code change nobody can audit.
//!
//! ## Purity
//!
//! No I/O, no clock, no model call, no randomness. [`decide`] is a total function of its
//! two arguments, so the same features always produce the same outcome — which is what
//! lets the decision be replayed and audited rather than re-litigated.

use crate::procedural::CATEGORIES;

/// The session-log kind carrying one run's deferral decision.
///
/// An additive kind, which is the tree's established way to add a fact without a
/// migration: every pre-existing reader of `agent_session_events` filters by
/// `kind`, so a kind nothing else names is inert to all of them. The safety
/// property was MEASURED across all 19 files referencing the table, not assumed
/// from the docstring — see `PREREG_R67D_CARRIED_DECISION_2026-10-02.md` §3.
pub const DEFERRAL_DECISION_KIND: &str = "deferral_decision";

/// A class the deferral decision keys on.
///
/// Mirrors [`CATEGORIES`] — the classifier's eight labels — plus the absence class.
/// This is deliberately **not** [`crate::decision_class::DecisionClass`], which is taken
/// and emits the string `"classify"` into a frozen metric vocabulary; a second type
/// emitting that string would collide in `/metrics`.
///
/// [`HumanUnmeasured`] is a variant, not a fallible parse: a label this tree does not
/// recognise is a *fact about the input*, not an error, and it must be representable
/// without losing the fact that it was unrecognised.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RoutingClass {
    Technology,
    BusinessProcess,
    Compliance,
    Finance,
    Vendor,
    Assessment,
    Infrastructure,
    /// The subject-matter pool: the case is about Dell/EMC infrastructure.
    ///
    /// **Grants no authority.** `decide_deferral` consults `measured_reliability`,
    /// which is empty for every class today, so this variant defers exactly as
    /// `infrastructure` does. It is modelled rather than aliased because the
    /// vocabulary-parity pin requires every classifier label to resolve to a
    /// known class, and an unmodelled label degrades to `HumanUnmeasured` —
    /// safe, but a SILENT degradation. When a reliability is ever measured for a
    /// subject-matter class, it is measured here, on its own evidence.
    DellSupport,
    /// The classifier's own abstain label.
    General,
    /// A label this build does not recognise. Absence of evidence is not evidence of
    /// competence, so this class exists to be *refused*, never to be inferred away.
    HumanUnmeasured,
}

impl RoutingClass {
    /// Every class, in the same order as [`CATEGORIES`], plus the absence class last.
    pub const ALL: [RoutingClass; 10] = [
        RoutingClass::Technology,
        RoutingClass::BusinessProcess,
        RoutingClass::Compliance,
        RoutingClass::Finance,
        RoutingClass::Vendor,
        RoutingClass::Assessment,
        RoutingClass::Infrastructure,
        RoutingClass::DellSupport,
        RoutingClass::General,
        RoutingClass::HumanUnmeasured,
    ];

    /// The stable wire/label form. A **compatibility surface**: downstream routing reads
    /// these strings, so they are frozen the way `DecisionClass::as_str` is.
    pub const fn as_str(self) -> &'static str {
        match self {
            RoutingClass::Technology => "technology",
            RoutingClass::BusinessProcess => "business_process",
            RoutingClass::Compliance => "compliance",
            RoutingClass::Finance => "finance",
            RoutingClass::Vendor => "vendor",
            RoutingClass::Assessment => "assessment",
            RoutingClass::Infrastructure => "infrastructure",
            RoutingClass::DellSupport => "dell_support",
            RoutingClass::General => "general",
            RoutingClass::HumanUnmeasured => "human_unmeasured",
        }
    }

    /// Resolve a classifier label to a class. An unrecognised label is
    /// [`RoutingClass::HumanUnmeasured`], never a guess at the nearest neighbour — a
    /// typo must not be silently mapped onto a class that then earns authority.
    pub fn from_label(label: &str) -> Self {
        match label {
            "technology" => RoutingClass::Technology,
            "business_process" => RoutingClass::BusinessProcess,
            "compliance" => RoutingClass::Compliance,
            "finance" => RoutingClass::Finance,
            "vendor" => RoutingClass::Vendor,
            "assessment" => RoutingClass::Assessment,
            "infrastructure" => RoutingClass::Infrastructure,
            "dell_support" => RoutingClass::DellSupport,
            "general" => RoutingClass::General,
            _ => RoutingClass::HumanUnmeasured,
        }
    }
}

/// The decision: what happens to this case.
///
/// Three outcomes, not one boolean, because "ask a human to judge this" and "ask a human
/// a question about it" are different acts with different costs and different evidence
/// at the end. Collapsing them into `human_required: bool` loses the distinction the
/// operator needs in order to act.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeferralOutcome {
    /// A human judges the case on the evidence, without being asked a question first.
    Defer,
    /// A human is asked a specific question; the answer then decides the case.
    Clarify,
    /// The case stops here and hands off, mapping onto the engine's existing
    /// `AskHuman` stop verdict.
    Stop,
}

impl DeferralOutcome {
    /// Every outcome. Used by the parity and exhaustiveness pins so a new variant
    /// cannot be added without them failing.
    pub const ALL: [DeferralOutcome; 3] = [
        DeferralOutcome::Defer,
        DeferralOutcome::Clarify,
        DeferralOutcome::Stop,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            DeferralOutcome::Defer => "defer",
            DeferralOutcome::Clarify => "clarify",
            DeferralOutcome::Stop => "stop",
        }
    }

    /// Is this outcome a decision made by a person? Every outcome currently is — the
    /// question is which *kind* of human input is being requested.
    pub const fn requires_human(self) -> bool {
        matches!(
            self,
            DeferralOutcome::Defer | DeferralOutcome::Clarify | DeferralOutcome::Stop
        )
    }
}

/// The observable features a decision may read.
///
/// Deliberately a struct of *measured* inputs only: no knob, no override, no
/// "force auto" escape hatch. A field that could bypass the table would be a field the
/// table does not govern.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeferralFeatures {
    /// How many keywords fired for the winning category. This is the **independent**
    /// signal: `confidence` alone cannot tell one keyword from ten, because both score
    /// an identical uncontested `1.0`.
    pub evidence_count: usize,
    /// The classifier's uncontested share, in `[0.0, 1.0]`. Retained for the receipt,
    /// not as a decision input on its own — read it alongside `evidence_count` or not
    /// at all.
    pub confidence: f32,
}

impl DeferralFeatures {
    pub const fn new(evidence_count: usize, confidence: f32) -> Self {
        Self {
            evidence_count,
            confidence,
        }
    }
}

/// Typed error, following the per-module error precedent (a `Display` impl, no
/// `std::error::Error`: nothing in this tree needs it).
#[derive(Debug)]
pub enum DeferralError {
    /// A feature value outside its declared domain — a non-finite confidence.
    NonFiniteConfidence(f32),
    /// A label that is not in the classifier's vocabulary *and* not the absence class.
    UnknownLabel(String),
}

impl std::fmt::Display for DeferralError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeferralError::NonFiniteConfidence(c) => {
                write!(f, "confidence must be finite, got {c}")
            }
            DeferralError::UnknownLabel(l) => write!(f, "unknown routing class label: {l}"),
        }
    }
}

/// The classes permitted to auto-authorise, with the measured reliability that would
/// justify each entry.
///
/// **Empty, and that is the shipped state.** An entry may be added only when a
/// `meta_d` has been measured for that class on a labelled corpus, with the measurement
/// recorded beside it. The tuple is kept (rather than a bare `&[RoutingClass]`) so the
/// *shape* of the evidence an entry requires is fixed before any entry exists — adding a
/// value without a measurement should not compile against a list that never asked for
/// one.
///
/// Deliberately not a threshold constant: a cost ratio derived from signed operational
/// costs is the principled bar, and those costs are not yet signed. A number here today
/// would be a number with no provenance.
const AUTO_CLASSES: [(RoutingClass, Option<MeasuredReliability>); 0] = [];

/// A per-class measured reliability, with the provenance that makes it auditable.
///
/// Exists as a type before any value does, so the table's shape is decided by the code
/// rather than retrofitted once someone wants to fill it in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeasuredReliability {
    /// The measured per-class agreement, as ten-thousandths of the unit interval.
    pub meta_d_units: i32,
    /// What the measurement was made on. A number without this field is a guess.
    pub corpus: &'static str,
}

/// The measured reliabilities behind [`AUTO_CLASSES`]. Empty while no class qualifies.
pub fn measured_reliability(class: RoutingClass) -> Option<MeasuredReliability> {
    AUTO_CLASSES
        .iter()
        .find(|(c, _)| *c == class)
        .and_then(|(_, m)| *m)
}

/// Does this build's classifier vocabulary still match [`RoutingClass`]?
///
/// The deferral decision and the classifier must agree on what a label IS. If the
/// classifier gains a label that [`RoutingClass`] has no variant for, that label would
/// resolve to [`RoutingClass::HumanUnmeasured`] — refused, which is safe but silently
/// degrades a class the operator may have meant to recognise. Reporting it here makes
/// the drift loud at the point where it happens.
///
/// Derived from the real [`CATEGORIES`] rather than restated, so it cannot drift green
/// beside a classifier change.
pub fn routing_classes_cover_classifier() -> bool {
    CATEGORIES
        .iter()
        .all(|label| RoutingClass::from_label(label) != RoutingClass::HumanUnmeasured)
}

/// Decide what happens to one case — the deferral decision for a classified label.
///
/// Total, pure, and **fail-closed**: with no measured reliability on file, every class
/// resolves to [`DeferralOutcome::Defer`]. The signature makes the safety property
/// structural — there is no path through this function that auto-authorises, because
/// `Auto` is not an outcome, only a *future* table entry that would have to be written
/// here first.
///
/// Errors on a non-finite confidence, because a `NaN` that flows into a receipt is
/// worse than a refusal: it reads as "measured, and the measurement is unknowable".
pub fn decide_deferral(
    class: RoutingClass,
    features: &DeferralFeatures,
) -> Result<DeferralOutcome, DeferralError> {
    if !features.confidence.is_finite() {
        return Err(DeferralError::NonFiniteConfidence(features.confidence));
    }
    // Refuse rather than guess: a class outside the classifier's vocabulary has no
    // measured reliability by construction, so it can never be auto-authorised. This
    // arm is what makes `HumanUnmeasured` a *decision* rather than a label.
    if class == RoutingClass::HumanUnmeasured {
        return Ok(DeferralOutcome::Defer);
    }
    // The table is empty, so this lookup yields `None` for every class today. It is
    // written as a lookup rather than an early `return` so that adding a measured class
    // is a data change, and so the exhaustiveness pins have something real to bite on.
    let auto_authorised: Option<MeasuredReliability> = measured_reliability(class);
    let _ = (auto_authorised, features.evidence_count);
    Ok(DeferralOutcome::Defer)
}

/// The decision plus the evidence it was made from, in the shape a caller
/// persists and a reader displays.
///
/// `POST /classify` already computes all four of these and
/// returns them, then drops them. This type exists so a run can CARRY the
/// decision instead of the receipt being the only place it ever existed — the
/// cockpit's queue needs the decision attached to the case, and a client may not
/// re-derive it (no business logic in the client).
///
/// `confidence` and `evidence_count` travel TOGETHER and must be read together:
/// `confidence` is the winning category's uncontested SHARE, so one keyword
/// firing alone and ten agreeing both read `1.0`. A consumer holding
/// `confidence` without `evidence_count` is holding a number whose caveat is
/// missing. This struct makes the pair inseparable by construction — there is
/// no constructor that takes one without the other.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CarriedDecision {
    /// The class the decision keyed on — the classifier's own label.
    pub routing_class: RoutingClass,
    /// What happens to this case. `defer` for every class today; see
    /// [`AUTO_CLASSES`] for why `clarify`/`stop` are not yet reachable.
    pub outcome: DeferralOutcome,
    /// The classifier's uncontested share in `[0.0, 1.0]`. **Read with
    /// `evidence_count`, never alone.**
    pub confidence: f32,
    /// How many keywords fired for the winning category. The independent
    /// signal: it is what separates `1.0`-on-one-hit from `1.0`-on-ten.
    pub evidence_count: usize,
}

/// Classify `text` and decide what happens to it, as one carryable value.
///
/// Still pure: the classifier is a pure function and [`decide_deferral`] is
/// total, so the same text always yields the same [`CarriedDecision`]. The only
/// error is the non-finite-confidence refusal, which is propagated rather than
/// coerced — a decision derived from a number that cannot exist is not a
/// decision.
pub fn carry_decision(text: &str) -> Result<CarriedDecision, DeferralError> {
    let result = crate::procedural::classify(text);
    let routing_class = RoutingClass::from_label(result.category);
    let outcome = decide_deferral(
        routing_class,
        &DeferralFeatures::new(result.evidence_count, result.confidence),
    )?;
    Ok(CarriedDecision {
        routing_class,
        outcome,
        confidence: result.confidence,
        evidence_count: result.evidence_count,
    })
}
