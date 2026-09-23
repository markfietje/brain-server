//! The DecisionModel seam — the governed decision harness's typed boundary
//! (v1.32.10). One trait, plain types, zero dependencies: a model declares
//! its identity and vocabulary, consumes evidence BY PROVENANCE REFERENCE
//! (ids + trust tiers — raw text is unrepresentable here), and returns
//! either a typed output over the shipped choice/score/noul vocabulary or
//! an HONEST REFUSAL. Always-on (no feature gate): the seam's consumers
//! implement it without kernel features, exactly like [`crate::pure`],
//! [`crate::policy`], and [`crate::host`].
//!
//! Authority law (monotonic-narrow): **a DecisionModel proposes; only the
//! gate disposes.** The trait's receivers are `&self`, the context is
//! `&DecisionContext` (read-only — a model cannot widen its own role
//! scope), and every seam type is plain data: an output value alone cannot
//! reach durable state. Promotion, persistence, and side effects are the
//! pipeline's job, never the model's.
//!
//! The evaluation is PURE: no I/O, no process spawn, no dynamic loading,
//! no clock (time enters via [`DecisionContext::created_at`]), no unsafe
//! (the crate forbids it), no panics on recoverable paths — a model that
//! cannot decide returns [`DecisionError::Refused`] with a reason from a
//! CLOSED vocabulary. Refusals are data the pipeline escalates on.

/// A governed decision model: identity + vocabulary, then pure evaluation.
///
/// The signature is the authoritative architecture's §4 verbatim, typed
/// with `Send + Sync` so a model can be shared as `Arc<dyn DecisionModel>`
/// or `Box<dyn DecisionModel>` (the [`crate::host`]-side object-safety
/// precedent).
pub trait DecisionModel: Send + Sync {
    /// Who this model is: id, version, kind, the vocabulary it emits, and
    /// — for learned models — the weights digest (the supply-chain law:
    /// digests or it didn't happen).
    fn metadata(&self) -> ModelMetadata;

    /// Evaluate one decision request against one context. Returns a typed
    /// output over the shipped vocabulary, or an honest refusal — never a
    /// guess, never a panic, never a side effect.
    fn evaluate(
        &self,
        input: &DecisionInput,
        ctx: &DecisionContext,
    ) -> Result<DecisionOutput, DecisionError>;
}

/// Compile-time pin: the trait stays object-safe and thread-safe. Breaking
/// either breaks this line, not a test run.
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Box<dyn DecisionModel>>();
};

/// Who a model is, at the supply-chain level (ASI04/LLM04): identity is
/// id + version + kind, and a LEARNED model cannot exist without its
/// weights digest — [`ModelKind::Learned`] carries the digest structurally,
/// so an un-digestable learned model is unrepresentable. Construct through
/// [`ModelMetadata::deterministic`] / [`ModelMetadata::learned`]; the
/// fields are private so the kind/digest pairing cannot be built
/// inconsistently by hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelMetadata {
    id: String,
    version: String,
    kind: ModelKind,
    output_vocabulary: Vec<OutputKind>,
    calibration_ref: Option<String>,
}

/// The model kind, with the learned model's weights digest structural (the
/// type-level law: `kind = learned` REQUIRES `weights_digest`; a
/// deterministic model has no weights to digest).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelKind {
    /// Hand-declared rules/policy — no weights, therefore no weights
    /// digest; `weights_digest()` is `None` by construction.
    Deterministic,
    /// Trained weights — cannot be constructed without their digest.
    Learned { weights_digest: String },
}

impl ModelMetadata {
    /// Metadata for a deterministic model: `weights_digest()` is `None`.
    pub fn deterministic(
        id: String,
        version: String,
        output_vocabulary: Vec<OutputKind>,
        calibration_ref: Option<String>,
    ) -> Self {
        Self {
            id,
            version,
            kind: ModelKind::Deterministic,
            output_vocabulary,
            calibration_ref,
        }
    }

    /// Metadata for a learned model: the weights digest is REQUIRED (a
    /// `String` in [`ModelKind::Learned`]) — there is no way to call this
    /// without one.
    pub fn learned(
        id: String,
        version: String,
        weights_digest: String,
        output_vocabulary: Vec<OutputKind>,
        calibration_ref: Option<String>,
    ) -> Self {
        Self {
            id,
            version,
            kind: ModelKind::Learned { weights_digest },
            output_vocabulary,
            calibration_ref,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn kind(&self) -> &ModelKind {
        &self.kind
    }

    /// The weights digest — `Some` exactly when `kind` is
    /// [`ModelKind::Learned`] (the pairing is enforced at construction).
    pub fn weights_digest(&self) -> Option<&str> {
        match &self.kind {
            ModelKind::Deterministic => None,
            ModelKind::Learned { weights_digest } => Some(weights_digest),
        }
    }

    /// The closed output vocabulary this model declares it can emit.
    pub fn output_vocabulary(&self) -> &[OutputKind] {
        &self.output_vocabulary
    }

    pub fn calibration_ref(&self) -> Option<&str> {
        self.calibration_ref.as_deref()
    }
}

/// The shipped output vocabulary (the decide line's three closed question
/// types, mirrored 0/1/2 — never extended quietly).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutputKind {
    Choice = 0,
    Score = 1,
    Noul = 2,
}

impl OutputKind {
    /// The decide line's type codes, mirrored exactly.
    pub const fn as_u8(self) -> u8 {
        match self {
            OutputKind::Choice => 0,
            OutputKind::Score => 1,
            OutputKind::Noul => 2,
        }
    }
}

/// The question kind being asked (the same closed three-type mirror as
/// [`OutputKind`] — kept a distinct type because a question asked and an
/// answer emitted are different facts).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QuestionKind {
    Choice = 0,
    Score = 1,
    Noul = 2,
}

impl QuestionKind {
    /// The decide line's type codes, mirrored exactly.
    pub const fn as_u8(self) -> u8 {
        match self {
            QuestionKind::Choice => 0,
            QuestionKind::Score => 1,
            QuestionKind::Noul => 2,
        }
    }
}

/// One decision request. Evidence arrives BY REFERENCE — ids + trust
/// tiers, never raw text (the poisoning pre-wiring: a model sees where
/// evidence came from and how much to trust it, not the bytes).
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionInput {
    /// Caller-assigned id of this ask (provenance for the request itself).
    pub request_id: String,
    /// The specific question asked, if one is.
    pub question: Option<QuestionRef>,
    /// The question-id set (the decide workflow matcher's input shape).
    pub question_ids: Vec<String>,
    /// The evidence provenance refs consumed.
    pub evidence: Vec<EvidenceRef>,
}

/// The asked question: its id and its closed kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionRef {
    pub id: String,
    pub kind: QuestionKind,
}

/// One piece of evidence, by reference only: its id and its trust tier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceRef {
    pub evidence_id: String,
    pub tier: TrustTier,
}

/// The closed trust tiers (ordered [`TrustTier::at_least`]): governed
/// provenance is digest-bound in-kernel, vetted provenance is a declared
/// external source, untrusted is user/agent-supplied. Filtering by tier is
/// the pipeline's job; the tier itself travels with the ref.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TrustTier {
    /// User/agent-supplied — trusted least.
    Untrusted = 0,
    /// A declared, operator-configured external source.
    Vetted = 1,
    /// In-kernel, digest-bound, audited — trusted most.
    Governed = 2,
}

impl TrustTier {
    /// Whether this tier is at least as trustworthy as `floor`.
    pub const fn at_least(self, floor: TrustTier) -> bool {
        (self as u8) >= (floor as u8)
    }
}

/// The evaluation context — read-only by the trait's `&` receiver (ASI03:
/// a model executes inside the caller's already-authorized context and
/// cannot escalate its own scope).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionContext {
    /// The governing run (workflow_runs.id in the kernel's store).
    pub run_id: i64,
    /// The run mode, recorded per run (recording is the pipeline's job —
    /// this seam only carries it).
    pub mode: RunMode,
    /// The config digests in force: name → digest pairs, so an evaluation
    /// is bound to the exact configuration it ran under.
    pub config_digests: Vec<ConfigRef>,
    /// The caller's role scope, carried read-only.
    pub role_scope: Vec<String>,
    /// When the context was created (unix seconds; time enters as an
    /// argument — evaluation never reads a clock).
    pub created_at: i64,
}

/// One config binding: a config key and its content digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigRef {
    pub key: String,
    pub digest: String,
}

/// The run mode, recorded per decision run from day one: `Deterministic`
/// runs are the governed lane; `Exploratory` runs are recorded as such so
/// nothing exploratory can later masquerade as governed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RunMode {
    Deterministic,
    Exploratory,
}

impl RunMode {
    /// The closed string vocabulary (the run-record representation).
    pub const fn as_str(self) -> &'static str {
        match self {
            RunMode::Deterministic => "deterministic",
            RunMode::Exploratory => "exploratory",
        }
    }
}

/// What a model decided, over the shipped vocabulary only.
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionOutput {
    /// Which model produced this (mirrors the metadata at evaluation time).
    pub model_id: String,
    pub model_version: String,
    /// When: the model's copy of the context's `created_at` — evaluation
    /// never reads a clock, so the same input + context reproduce this.
    pub produced_at: i64,
    /// The typed decision value (the shipped three-type vocabulary).
    pub value: DecisionValue,
    /// Which evidence refs were consumed (provenance travels with the
    /// output — ids + tiers, never text).
    pub evidence_refs: Vec<EvidenceRef>,
}

/// The shipped decision vocabulary (the decide line's choice/score/noul,
/// mirrored exactly — a second type system is never invented here).
#[derive(Debug, Clone, PartialEq)]
pub enum DecisionValue {
    /// A choice among declared options; probabilities/confidence are
    /// `None` unless the model can honestly produce them.
    Choice {
        label: String,
        probabilities: Option<Vec<(String, f64)>>,
        confidence: Option<f64>,
    },
    /// A score in a declared inclusive range.
    Score { value: i64, range: (i64, i64) },
    /// The binary pair.
    Noul { value: bool },
}

/// Why a model declined to decide. The honest-refusal envelope: a refusal
/// is DATA the pipeline escalates on — never a guess dressed as an output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionError {
    /// The model declines, for a reason from the CLOSED vocabulary.
    Refused { reason: RefusalReason },
    /// The model is disabled by configuration or policy.
    Disabled,
    /// The wiring is wrong (e.g. the context's config digest does not bind
    /// the model's declared configuration).
    Misconfigured,
}

/// The closed refusal reasons. Exhaustive everywhere: adding a variant is
/// a breaking change to this module's vocabulary, by the compiler's own
/// hand (the `Display` match below does not have a wildcard arm).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RefusalReason {
    /// The asked question (or label) is outside the declared vocabulary.
    OutOfVocabulary,
    /// No evidence at all arrived, so there is nothing to decide on.
    InsufficientEvidence,
    /// Evidence arrived, but not enough of it at the required trust tier.
    InsufficientEvidenceTier,
}

impl core::fmt::Display for DecisionError {
    /// Honest by construction: the exhaustive match states what happened,
    /// never invents an outcome, and cannot forget a variant.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            DecisionError::Refused { reason } => match reason {
                RefusalReason::OutOfVocabulary => {
                    write!(
                        f,
                        "decision refused: the ask is outside the model's declared vocabulary"
                    )
                }
                RefusalReason::InsufficientEvidence => {
                    write!(f, "decision refused: no evidence arrived to decide on")
                }
                RefusalReason::InsufficientEvidenceTier => {
                    write!(
                        f,
                        "decision refused: the evidence does not meet the required trust tier"
                    )
                }
            },
            DecisionError::Disabled => {
                write!(f, "decision unavailable: the model is disabled")
            }
            DecisionError::Misconfigured => {
                write!(
                    f,
                    "decision unavailable: the context does not bind the model's configuration"
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(mode: RunMode) -> DecisionContext {
        DecisionContext {
            run_id: 7,
            mode,
            config_digests: vec![ConfigRef {
                key: "k".into(),
                digest: "d".into(),
            }],
            role_scope: vec!["operator".into()],
            created_at: 1_800_000_000,
        }
    }

    fn input() -> DecisionInput {
        DecisionInput {
            request_id: "req-1".into(),
            question: Some(QuestionRef {
                id: "q".into(),
                kind: QuestionKind::Choice,
            }),
            question_ids: vec!["q".into()],
            evidence: vec![EvidenceRef {
                evidence_id: "e-1".into(),
                tier: TrustTier::Governed,
            }],
        }
    }

    struct YesModel;

    impl DecisionModel for YesModel {
        fn metadata(&self) -> ModelMetadata {
            ModelMetadata::deterministic(
                "yes".into(),
                "1.0.0".into(),
                vec![OutputKind::Choice],
                None,
            )
        }

        fn evaluate(
            &self,
            _input: &DecisionInput,
            ctx: &DecisionContext,
        ) -> Result<DecisionOutput, DecisionError> {
            Ok(DecisionOutput {
                model_id: "yes".into(),
                model_version: "1.0.0".into(),
                produced_at: ctx.created_at,
                value: DecisionValue::Choice {
                    label: "ok".into(),
                    probabilities: None,
                    confidence: None,
                },
                evidence_refs: Vec::new(),
            })
        }
    }

    /// The seam is used as `dyn` across threads (the Embedder pattern):
    /// object-safety + Send + Sync, exercised through an actual
    /// `Box<dyn DecisionModel>` moved into a scoped thread.
    #[test]
    fn decision_model_trait_is_dyn_compatible_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Box<dyn DecisionModel>>();

        let model: Box<dyn DecisionModel> = Box::new(YesModel);
        let seen = std::thread::scope(|s| {
            s.spawn(|| model.metadata().id().to_string())
                .join()
                .unwrap_or_default()
        });
        assert_eq!(seen, "yes");

        let out = model
            .evaluate(&input(), &ctx(RunMode::Deterministic))
            .expect("dyn evaluate must succeed");
        assert_eq!(out.produced_at, 1_800_000_000);
    }

    /// The refusal vocabulary is CLOSED (no wildcard arm can render a new
    /// reason) and DISPLAY-HONEST: every message states what happened and
    /// none invents a decision.
    #[test]
    fn decision_error_vocabulary_is_closed_and_display_honest() {
        let refused = [
            RefusalReason::OutOfVocabulary,
            RefusalReason::InsufficientEvidence,
            RefusalReason::InsufficientEvidenceTier,
        ];
        for reason in refused {
            let msg = DecisionError::Refused { reason }.to_string();
            assert!(
                msg.starts_with("decision refused:"),
                "refusals name themselves: {msg}"
            );
        }
        assert_eq!(
            DecisionError::Disabled.to_string(),
            "decision unavailable: the model is disabled"
        );
        assert_eq!(
            DecisionError::Misconfigured.to_string(),
            "decision unavailable: the context does not bind the model's configuration"
        );
        // Honesty: an error's display never carries an outcome-bearing
        // word — it refuses or is unavailable, it never decides.
        let all = refused
            .iter()
            .map(|r| DecisionError::Refused { reason: *r }.to_string())
            .chain([
                DecisionError::Disabled.to_string(),
                DecisionError::Misconfigured.to_string(),
            ])
            .collect::<Vec<_>>();
        for msg in all {
            for invented in ["decided", "chosen", "approved", "value:"] {
                assert!(!msg.contains(invented), "honest display: {msg}");
            }
        }
    }

    /// RunMode is a closed two-value vocabulary with a stable string form
    /// (the run-record representation).
    #[test]
    fn run_mode_is_a_closed_two_value_vocabulary() {
        assert_eq!(RunMode::Deterministic.as_str(), "deterministic");
        assert_eq!(RunMode::Exploratory.as_str(), "exploratory");
        // The closed pair: both modes round-trip through the only two
        // names, and the codes stay distinct.
        let modes = [RunMode::Deterministic, RunMode::Exploratory];
        assert_eq!(modes.len(), 2);
        assert_ne!(modes[0].as_str(), modes[1].as_str());
    }

    /// The ASI04/LLM04 law at the type level: a learned model cannot be
    /// declared without its weights digest, and a deterministic model has
    /// none — the inconsistent pairing is unconstructible.
    #[test]
    fn metadata_learned_kind_requires_the_digest_at_the_type_level() {
        let learned = ModelMetadata::learned(
            "laya".into(),
            "0.1.0".into(),
            "sha256:abc".into(),
            vec![OutputKind::Choice, OutputKind::Score, OutputKind::Noul],
            Some("gold-2026-09".into()),
        );
        assert_eq!(
            learned.kind(),
            &ModelKind::Learned {
                weights_digest: "sha256:abc".into()
            }
        );
        assert_eq!(learned.weights_digest(), Some("sha256:abc"));

        let deterministic = ModelMetadata::deterministic(
            "rules".into(),
            "1.0.0".into(),
            vec![OutputKind::Noul],
            None,
        );
        assert_eq!(deterministic.kind(), &ModelKind::Deterministic);
        assert_eq!(deterministic.weights_digest(), None);
    }
}
