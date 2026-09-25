//! The delivery loop's decision core — pure, total, and I/O-free.
//!
//! One question decides a release: may this exact artifact, under this exact
//! evidence, be promoted? [`promote`] answers it as a pure function of its
//! arguments — no clock, no store, no network, no provider — so the whole law
//! book is unit-testable without a running host, and deny always wins.
//!
//! The shape is the governed case machine's: a closed vocabulary, a
//! deterministic phase machine, a pure arbiter, and a lane for the model that
//! proposes. What is deliberately absent is the part that cannot be pure:
//! **this crate does not sign, does not verify signatures, and does not
//! attest to authorship.** [`Attestation::record_digest`] is a deterministic
//! self-digest that makes a chain's parent links checkable; a digest is not a
//! signature, and an unsigned or foreign-signer case is a refusal the host
//! must make, never a degraded mark from here.
//!
//! The other honest ceiling: autonomy never widens. A tier may only narrow what
//! a principal may do, [`promote`] reads the **tier** and never the recorded
//! trace mode, and the budget ledger's arithmetic saturates so an exhaustion is
//! visible rather than wrapped. One dimension is carried but **unenforced** —
//! [`BudgetKind::BlastRadius`] has no ceiling semantics yet, so it is recorded
//! and never consulted for a decision.

#![deny(unsafe_code)]

use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Lowercase hex, self-contained.
///
/// The prereg fixes this crate's dependency set at exactly `serde`,
/// `serde_json`, and `sha2`, and a need for anything else is a
/// stop-and-preregister-addendum event. Hex encoding is eight lines, so it lives
/// here rather than arriving as a fourth dependency.
fn hex_encode(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    out
}

/// The predicate type this crate builds, as specified in-repo. A resolvable
/// in-repo identifier; no external interoperability is claimed for it.
pub const PREDICATE_TYPE: &str = "urn:brain:attest:delivery:v1";

/// Domain separator for the predicate digest. A digest taken over bare
/// canonical bytes could collide with a digest taken over any other document;
/// the separator binds the hash to this predicate type.
const PREDICATE_DIGEST_DOMAIN: &[u8] = b"urn:brain:attest:delivery:v1\0";

/// Domain separator for an attestation record's self-digest. Distinct from the
/// predicate domain: a record digest and a predicate digest are different
/// questions about the same release.
const RECORD_DIGEST_DOMAIN: &[u8] = b"urn:brain:attest:delivery:record:v1\0";

/// How much autonomy a run's operator granted it. Closed vocabulary — the
/// grant narrows with depth and never widens, and anything outside this set is
/// a parse error rather than a default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutonomyTier {
    /// Watches. May propose, never promotes.
    Observe,
    /// Proposes. May propose, never promotes.
    Propose,
    /// Acts inside a budget, promotes only through the gate.
    BoundedAuto,
    /// Acts on a standing authorization, promotes through the gate once a human
    /// has granted that authorization.
    Delegated,
}

impl AutonomyTier {
    /// Every tier, narrowest first — the order the tier law reads.
    pub const ALL: [AutonomyTier; 4] = [
        AutonomyTier::Observe,
        AutonomyTier::Propose,
        AutonomyTier::BoundedAuto,
        AutonomyTier::Delegated,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            AutonomyTier::Observe => "observe",
            AutonomyTier::Propose => "propose",
            AutonomyTier::BoundedAuto => "bounded_auto",
            AutonomyTier::Delegated => "delegated",
        }
    }

    /// Exact wire match; an unknown tier is an error, never a guess.
    pub fn parse(s: &str) -> Result<Self, UnknownVocabulary> {
        match s {
            "observe" => Ok(AutonomyTier::Observe),
            "propose" => Ok(AutonomyTier::Propose),
            "bounded_auto" => Ok(AutonomyTier::BoundedAuto),
            "delegated" => Ok(AutonomyTier::Delegated),
            other => Err(UnknownVocabulary(other.to_string())),
        }
    }

    /// Whether this tier may be promoted through the gate at all. The two
    /// narrowest tiers propose and never promote.
    pub fn is_promoting(self) -> bool {
        matches!(self, AutonomyTier::BoundedAuto | AutonomyTier::Delegated)
    }
}

/// The closed trace-mode vocabulary. Deliberately NOT overloaded with autonomy:
/// it records how a trace was produced, and the gate never infers authority
/// from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceMode {
    Deterministic,
    Exploratory,
}

impl TraceMode {
    pub const ALL: [TraceMode; 2] = [TraceMode::Deterministic, TraceMode::Exploratory];

    pub fn as_str(self) -> &'static str {
        match self {
            TraceMode::Deterministic => "deterministic",
            TraceMode::Exploratory => "exploratory",
        }
    }

    /// Exact wire match; an unknown mode is an error, never a guess.
    pub fn parse(s: &str) -> Result<Self, UnknownVocabulary> {
        match s {
            "deterministic" => Ok(TraceMode::Deterministic),
            "exploratory" => Ok(TraceMode::Exploratory),
            other => Err(UnknownVocabulary(other.to_string())),
        }
    }
}

/// The single place the tier-to-trace-mode mapping exists. The two narrow
/// tiers produce exploratory traces; the two promoting tiers produce
/// deterministic ones. An exploratory run proposes and never promotes, so the
/// existing decision-run law holds here without widening the mode vocabulary.
pub fn trace_mode_for_tier(tier: AutonomyTier) -> TraceMode {
    match tier {
        AutonomyTier::Observe | AutonomyTier::Propose => TraceMode::Exploratory,
        AutonomyTier::BoundedAuto | AutonomyTier::Delegated => TraceMode::Deterministic,
    }
}

/// Where a run is in the delivery lifecycle. Opaque engine-owned state: the
/// run's public status vocabulary and its four routing keys are untouched by
/// this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Scope,
    Design,
    Build,
    Release,
    Operate,
    Done,
}

impl Phase {
    /// Every phase in lifecycle order. The machine is forward-only.
    pub const ALL: [Phase; 6] = [
        Phase::Scope,
        Phase::Design,
        Phase::Build,
        Phase::Release,
        Phase::Operate,
        Phase::Done,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Scope => "scope",
            Phase::Design => "design",
            Phase::Build => "build",
            Phase::Release => "release",
            Phase::Operate => "operate",
            Phase::Done => "done",
        }
    }

    /// Exact wire match; an unknown phase is an error, never a guess.
    pub fn parse(s: &str) -> Result<Self, UnknownVocabulary> {
        match s {
            "scope" => Ok(Phase::Scope),
            "design" => Ok(Phase::Design),
            "build" => Ok(Phase::Build),
            "release" => Ok(Phase::Release),
            "operate" => Ok(Phase::Operate),
            "done" => Ok(Phase::Done),
            other => Err(UnknownVocabulary(other.to_string())),
        }
    }

    /// The phase in which the release evidence is produced. A promotion is a
    /// release-phase fact and nothing else.
    pub fn release(self) -> bool {
        matches!(self, Phase::Release)
    }
}

/// A value outside a closed vocabulary, carried verbatim so a caller can name
/// it in a refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownVocabulary(pub String);

impl std::fmt::Display for UnknownVocabulary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown vocabulary value: {}", self.0)
    }
}

impl std::error::Error for UnknownVocabulary {}

/// Whether the machine may move from one phase to the next.
///
/// Total and forward-only: exactly the five adjacent moves are legal. A
/// machine that never skips and never rewinds is the same posture the
/// troubleshooting case machine holds — a run that cannot reach the next phase
/// stops and asks, it does not jump. `done` is terminal, so the only route into
/// it is the operate phase handing the run over.
pub fn is_legal_phase_transition(from: Phase, to: Phase) -> bool {
    matches!(
        (from, to),
        (Phase::Scope, Phase::Design)
            | (Phase::Design, Phase::Build)
            | (Phase::Build, Phase::Release)
            | (Phase::Release, Phase::Operate)
            | (Phase::Operate, Phase::Done)
    )
}

/// The only phase a run finishes through.
pub const fn terminal_phase() -> Phase {
    Phase::Done
}

/// One budget dimension. The `kind` CHECK list is closed, and one member is
/// carried without semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetKind {
    Tokens,
    ToolCalls,
    Files,
    Minutes,
    /// Carried, never enforced. No ceiling semantics are defined for it yet,
    /// so it records what happened and takes part in no decision — a budget
    /// that cannot be judged must not silently judge.
    BlastRadius,
}

impl BudgetKind {
    pub const ALL: [BudgetKind; 5] = [
        BudgetKind::Tokens,
        BudgetKind::ToolCalls,
        BudgetKind::Files,
        BudgetKind::Minutes,
        BudgetKind::BlastRadius,
    ];

    /// The count of closed kinds; the ledger's storage is exactly this wide.
    pub const COUNT: usize = 5;

    pub fn as_str(self) -> &'static str {
        match self {
            BudgetKind::Tokens => "tokens",
            BudgetKind::ToolCalls => "tool_calls",
            BudgetKind::Files => "files",
            BudgetKind::Minutes => "minutes",
            BudgetKind::BlastRadius => "blast_radius",
        }
    }

    /// Exact wire match; an unknown kind is an error, never a guess.
    pub fn parse(s: &str) -> Result<Self, UnknownVocabulary> {
        match s {
            "tokens" => Ok(BudgetKind::Tokens),
            "tool_calls" => Ok(BudgetKind::ToolCalls),
            "files" => Ok(BudgetKind::Files),
            "minutes" => Ok(BudgetKind::Minutes),
            "blast_radius" => Ok(BudgetKind::BlastRadius),
            other => Err(UnknownVocabulary(other.to_string())),
        }
    }

    /// Stable storage index; `as_str` order and index order agree, which is
    /// what lets the canonical predicate form order a spend list by kind.
    pub const fn index(self) -> usize {
        match self {
            BudgetKind::Tokens => 0,
            BudgetKind::ToolCalls => 1,
            BudgetKind::Files => 2,
            BudgetKind::Minutes => 3,
            BudgetKind::BlastRadius => 4,
        }
    }

    /// Whether a decision may be refused on this dimension's exhaustion.
    /// Everything except the undefined dimension.
    pub const fn is_enforced(self) -> bool {
        !matches!(self, BudgetKind::BlastRadius)
    }
}

/// A ceiling and what has been drawn against it, for one kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub kind: BudgetKind,
    pub ceiling: u64,
    pub spent: u64,
}

impl Budget {
    pub const fn new(kind: BudgetKind, ceiling: u64) -> Self {
        Self {
            kind,
            ceiling,
            spent: 0,
        }
    }

    /// Draw `amount` against the ceiling. Saturating: a draw that would wrap a
    /// counter pins it at the maximum instead, so an exhausted budget is
    /// visible as exhaustion and never as headroom.
    pub fn spend(self, amount: u64) -> Self {
        Self {
            kind: self.kind,
            ceiling: self.ceiling,
            spent: self.spent.saturating_add(amount),
        }
    }

    /// Exhausted at the boundary: a budget with nothing left and a budget
    /// overdrawn are the same fact, and neither is headroom.
    pub const fn is_exhausted(self) -> bool {
        self.spent >= self.ceiling
    }

    /// Whether any further draw is permitted, judged the same fail-closed way.
    pub const fn has_headroom(self) -> bool {
        !self.is_exhausted()
    }
}

/// Every ceiling a run was granted, one per closed kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Ceilings {
    pub tokens: u64,
    pub tool_calls: u64,
    pub files: u64,
    pub minutes: u64,
    pub blast_radius: u64,
}

/// The per-run budget ledger: one [`Budget`] per kind, drawn with saturating
/// arithmetic.
///
/// The default ledger grants nothing, so every enforced kind is exhausted and a
/// promotion is refused. Ceilings are supplied explicitly; a ledger that
/// forgot one is the refusing case, not the permissive one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetLedger {
    entries: [Budget; BudgetKind::COUNT],
}

impl BudgetLedger {
    pub fn new(ceilings: Ceilings) -> Self {
        Self {
            entries: [
                Budget::new(BudgetKind::Tokens, ceilings.tokens),
                Budget::new(BudgetKind::ToolCalls, ceilings.tool_calls),
                Budget::new(BudgetKind::Files, ceilings.files),
                Budget::new(BudgetKind::Minutes, ceilings.minutes),
                Budget::new(BudgetKind::BlastRadius, ceilings.blast_radius),
            ],
        }
    }

    pub fn budget(&self, kind: BudgetKind) -> &Budget {
        &self.entries[kind.index()]
    }

    /// Draw against one kind and return the new ledger. Pure: the caller
    /// decides whether to persist the result, inside its own transaction.
    pub fn spend(&self, kind: BudgetKind, amount: u64) -> Self {
        let mut next = *self;
        next.entries[kind.index()] = next.entries[kind.index()].spend(amount);
        next
    }

    /// The enforced kinds with nothing left, in vocabulary order. The
    /// unenforced dimension is absent by construction.
    pub fn exhausted(&self) -> Vec<BudgetKind> {
        BudgetKind::ALL
            .into_iter()
            .filter(|kind| kind.is_enforced() && self.budget(*kind).is_exhausted())
            .collect()
    }

    /// Whether every enforced kind still has headroom.
    pub fn has_headroom(&self) -> bool {
        self.exhausted().is_empty()
    }
}

impl Default for BudgetLedger {
    fn default() -> Self {
        Self::new(Ceilings::default())
    }
}

/// One link in a release's attestation chain.
///
/// The subject is the exact artifact the release is about; the predicate digest
/// is the digest of the typed predicate that describes what was done; the
/// parent digest is the previous link's [`Attestation::record_digest`], and is
/// empty at the root.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attestation {
    pub subject_name: String,
    pub subject_digest: String,
    pub predicate_digest: String,
    pub predicate_type: String,
    /// The policy the link was made under. Carried here because the gate is
    /// handed the policy it expects and must be able to compare.
    pub policy_digest: String,
    pub signer_did: String,
    pub parent_digest: String,
}

impl Attestation {
    /// A root link: nothing to point back at.
    pub fn root(
        subject_name: impl Into<String>,
        subject_digest: impl Into<String>,
        predicate_digest: impl Into<String>,
        policy_digest: impl Into<String>,
        signer_did: impl Into<String>,
    ) -> Self {
        Self {
            subject_name: subject_name.into(),
            subject_digest: subject_digest.into(),
            predicate_digest: predicate_digest.into(),
            predicate_type: PREDICATE_TYPE.to_string(),
            policy_digest: policy_digest.into(),
            signer_did: signer_did.into(),
            parent_digest: String::new(),
        }
    }

    /// A child link pointing at `parent`.
    pub fn child_of(
        parent: &Attestation,
        subject_name: impl Into<String>,
        predicate_digest: impl Into<String>,
        signer_did: impl Into<String>,
    ) -> Self {
        Self {
            subject_name: subject_name.into(),
            subject_digest: parent.subject_digest.clone(),
            predicate_digest: predicate_digest.into(),
            predicate_type: parent.predicate_type.clone(),
            policy_digest: parent.policy_digest.clone(),
            signer_did: signer_did.into(),
            parent_digest: parent.record_digest(),
        }
    }

    /// The record's deterministic self-digest, domain-separated.
    ///
    /// This is what makes a parent link checkable. It is a digest, not a
    /// signature: it says nothing about who wrote the record, and no
    /// verification decision in this crate may be read as authorship.
    pub fn record_digest(&self) -> String {
        let mut message = Vec::new();
        message.extend_from_slice(RECORD_DIGEST_DOMAIN);
        message.push(b'|');
        message.extend_from_slice(self.subject_name.as_bytes());
        message.push(b'|');
        message.extend_from_slice(self.subject_digest.as_bytes());
        message.push(b'|');
        message.extend_from_slice(self.predicate_digest.as_bytes());
        message.push(b'|');
        message.extend_from_slice(self.predicate_type.as_bytes());
        message.push(b'|');
        message.extend_from_slice(self.policy_digest.as_bytes());
        message.push(b'|');
        message.extend_from_slice(self.signer_did.as_bytes());
        message.push(b'|');
        message.extend_from_slice(self.parent_digest.as_bytes());
        hex_encode(&Sha256::digest(&message))
    }

    /// Every field a link must carry to be usable as evidence. An empty field
    /// is a missing fact, and a missing fact is a refusal.
    fn is_complete(&self) -> bool {
        !self.subject_name.is_empty()
            && !self.subject_digest.is_empty()
            && !self.predicate_digest.is_empty()
            && !self.predicate_type.is_empty()
            && !self.policy_digest.is_empty()
            && !self.signer_did.is_empty()
    }
}

/// A human's authorization to promote one exact artifact.
///
/// The binding is the tuple `(subject_digest, principal, scope, expiry)`. There
/// is no signature field here on purpose: this crate cannot verify one, and
/// carrying a string it will never check would read as a guarantee it does not
/// give. Signature verification belongs to the host's integrity stack.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Approval {
    pub subject_digest: String,
    pub principal_did: String,
    pub scope: String,
    /// Expiry as a caller-supplied epoch second. A crate with no clock cannot
    /// read one, so the caller passes the instant it is deciding at.
    pub expires_at_epoch: u64,
}

impl Approval {
    /// Whether the approval names this exact artifact. A change to the subject
    /// digest voids it by construction: there is no field that could survive
    /// the change.
    pub fn binds(&self, subject_digest: &str) -> bool {
        !self.subject_digest.is_empty() && self.subject_digest == subject_digest
    }

    /// Whether the approval binds the subject and has not expired at
    /// `now_epoch`. The boundary is fail-closed: an approval whose expiry
    /// equals the deciding instant has expired.
    pub fn is_current(&self, subject_digest: &str, now_epoch: u64) -> bool {
        self.binds(subject_digest) && now_epoch < self.expires_at_epoch
    }
}

/// Why a promotion was refused. Closed vocabulary, and the declaration order
/// is the reporting precedence: the first reason that applies is the one
/// reported, so a refusal always names the same thing for the same evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DenyReason {
    /// The chain is empty, incomplete, broken at a link, or does not describe
    /// the artifact being promoted. Checked before any allowance.
    AttestationChainBroken,
    /// The chain was made under a different policy than the one in force.
    PolicyDigestMismatch,
    /// No approval was presented.
    ApprovalMissing,
    /// An approval was presented and has expired at the deciding instant.
    ApprovalExpired,
    /// The approval names a different artifact than the one being promoted.
    ApprovalSubjectMismatch,
    /// At least one enforced budget dimension has nothing left.
    BudgetExhausted,
    /// The run's autonomy tier may propose and never promote.
    TierNotPermitted,
}

impl DenyReason {
    /// Every reason, in reporting precedence order.
    pub const ALL: [DenyReason; 7] = [
        DenyReason::AttestationChainBroken,
        DenyReason::PolicyDigestMismatch,
        DenyReason::ApprovalMissing,
        DenyReason::ApprovalExpired,
        DenyReason::ApprovalSubjectMismatch,
        DenyReason::BudgetExhausted,
        DenyReason::TierNotPermitted,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            DenyReason::AttestationChainBroken => "attestation_chain_broken",
            DenyReason::PolicyDigestMismatch => "policy_digest_mismatch",
            DenyReason::ApprovalMissing => "approval_missing",
            DenyReason::ApprovalExpired => "approval_expired",
            DenyReason::ApprovalSubjectMismatch => "approval_subject_mismatch",
            DenyReason::BudgetExhausted => "budget_exhausted",
            DenyReason::TierNotPermitted => "tier_not_permitted",
        }
    }
}

/// What the gate decided. Three-valued and total: a caller never has to
/// interpret a missing answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Every requirement is satisfied.
    Allow,
    /// Every requirement is satisfied, but a human must still grant the
    /// standing authorization this tier depends on.
    Prompt,
    /// A requirement failed. Carries the reason; denial outranks everything.
    Deny(DenyReason),
}

impl Decision {
    /// Whether the gate refused.
    pub const fn is_denied(self) -> bool {
        matches!(self, Decision::Deny(_))
    }
}

/// Everything the gate reads. One struct because the gate is total: it decides
/// for every combination, so its inputs are named rather than positional.
pub struct PromotionRequest<'a> {
    /// The complete chain for the release, root first.
    pub chain: &'a [Attestation],
    /// The policy in force right now.
    pub policy_digest: &'a str,
    /// The artifact's digest as it exists right now — the one a promotion
    /// would move. Re-derived at the promotion instant, not remembered from
    /// the approval.
    pub live_subject_digest: &'a str,
    /// The approval presented, if any.
    pub approval: Option<&'a Approval>,
    pub budgets: &'a BudgetLedger,
    /// The run's autonomy tier, read from the evidence that carries it.
    pub tier: AutonomyTier,
    /// The trace's recorded mode, carried so that the gate's refusal to read
    /// it is observable. No branch below consults it: authority comes from
    /// the tier, never from how a trace was produced.
    pub trace_mode: TraceMode,
    /// The instant the decision is being made at, as an epoch second.
    pub now_epoch: u64,
}

/// The promotion gate: the one pure function that decides whether an artifact
/// may be promoted.
///
/// Deny-wins and total. Every requirement is evaluated, and the first failure
/// in [`DenyReason::ALL`] order is what the refusal names; nothing short-
/// circuits to `Allow`, and no input combination has no answer. The chain is
/// checked before any allowance.
///
/// The recorded [`TraceMode`] is deliberately unread. A run's autonomy comes
/// from its tier; a trace that claims to be deterministic does not buy
/// authority it was not granted.
pub fn promote(request: &PromotionRequest<'_>) -> Decision {
    if let Some(reason) = deny_reasons(request).first() {
        return Decision::Deny(*reason);
    }
    // The widest tier promotes only once a human has granted the standing
    // authorization it depends on, so the gate asks rather than allows.
    if request.tier == AutonomyTier::Delegated {
        return Decision::Prompt;
    }
    Decision::Allow
}

/// Every requirement the gate enforces, in reporting precedence. A caller can
/// surface the whole list; the gate reports the first.
fn deny_reasons(request: &PromotionRequest<'_>) -> Vec<DenyReason> {
    let mut reasons = Vec::new();
    if chain_defect(request).is_some() {
        reasons.push(DenyReason::AttestationChainBroken);
    }
    if policy_defect(request) {
        reasons.push(DenyReason::PolicyDigestMismatch);
    }
    match request.approval {
        None => reasons.push(DenyReason::ApprovalMissing),
        Some(approval) => {
            if !approval.binds(request.live_subject_digest) {
                reasons.push(DenyReason::ApprovalSubjectMismatch);
            } else if !approval.is_current(request.live_subject_digest, request.now_epoch) {
                reasons.push(DenyReason::ApprovalExpired);
            }
        }
    }
    if !request.budgets.has_headroom() {
        reasons.push(DenyReason::BudgetExhausted);
    }
    if !request.tier.is_promoting() {
        reasons.push(DenyReason::TierNotPermitted);
    }
    reasons
}

/// How a chain fails to describe the artifact being promoted. Typed, so a
/// refusal can say what was wrong with the evidence rather than only that
/// something was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainDefect {
    /// No chain at all.
    Empty,
    /// The chain starts mid-way: its first link names a parent, so the chain
    /// presented is a fragment of a longer one.
    RootHasParent,
    /// A link carries no artifact digest.
    SubjectMissing,
    /// The chain describes an artifact that is not the one being promoted now.
    SubjectMoved,
    /// A link is missing a field, and a missing fact is a refusal.
    IncompleteLink,
    /// Two links describe different artifacts.
    SubjectDisagrees,
    /// Two links use different predicate types.
    PredicateTypeDisagrees,
    /// A link's parent digest is not the previous link's record digest.
    BrokenLink,
}

impl ChainDefect {
    pub fn as_str(self) -> &'static str {
        match self {
            ChainDefect::Empty => "empty",
            ChainDefect::RootHasParent => "root_has_parent",
            ChainDefect::SubjectMissing => "subject_missing",
            ChainDefect::SubjectMoved => "subject_moved",
            ChainDefect::IncompleteLink => "incomplete_link",
            ChainDefect::SubjectDisagrees => "subject_disagrees",
            ChainDefect::PredicateTypeDisagrees => "predicate_type_disagrees",
            ChainDefect::BrokenLink => "broken_link",
        }
    }
}

/// The chain's structural defect, if any.
///
/// The gate receives the complete chain for the release, so a chain that starts
/// mid-way is a fragment and is refused rather than accepted. Every check here
/// is structural: this crate does not verify signatures, so a chain that passes
/// is well-formed and digest-bound, not authenticated.
pub fn chain_defect(request: &PromotionRequest<'_>) -> Option<ChainDefect> {
    let chain = request.chain;
    let Some(root) = chain.first() else {
        return Some(ChainDefect::Empty);
    };
    if !root.parent_digest.is_empty() {
        return Some(ChainDefect::RootHasParent);
    }
    if root.subject_digest.is_empty() {
        return Some(ChainDefect::SubjectMissing);
    }
    if root.subject_digest != request.live_subject_digest {
        return Some(ChainDefect::SubjectMoved);
    }
    for link in chain {
        if !link.is_complete() {
            return Some(ChainDefect::IncompleteLink);
        }
        if link.subject_digest != root.subject_digest {
            return Some(ChainDefect::SubjectDisagrees);
        }
        if link.predicate_type != root.predicate_type {
            return Some(ChainDefect::PredicateTypeDisagrees);
        }
    }
    for pair in chain.windows(2) {
        if pair[1].parent_digest != pair[0].record_digest() {
            return Some(ChainDefect::BrokenLink);
        }
    }
    None
}

/// Whether the chain was made under a different policy than the one in force.
fn policy_defect(request: &PromotionRequest<'_>) -> bool {
    if request.policy_digest.is_empty() {
        return true;
    }
    request
        .chain
        .iter()
        .any(|link| link.policy_digest != request.policy_digest)
}

/// The typed predicate describing what an agent did to produce a release.
///
/// Structure and serialization only: this is the subject a signature would
/// later cover, not a signature. Every field is a digest, a ref, a count, or a
/// closed vocabulary value — never a secret, a provider body, or a bearer
/// value.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttestationPredicate {
    pub run_ref: String,
    pub subject_name: String,
    pub subject_digest: String,
    /// The registry row of the model that acted, by reference and by digest.
    pub model_ref: String,
    pub model_digest: String,
    pub plan_digest: String,
    pub policy_digest: String,
    /// Tool calls in execution order. Order is evidence, so it is preserved
    /// exactly as given and never sorted.
    pub tool_call_digests: Vec<String>,
    /// Gate verdicts as `(gate, verdict)`, normalized into name order.
    pub gate_verdicts: Vec<(String, String)>,
    pub approval_ref: String,
    /// Authority observations as `(target_kind, receipt digest)`, normalized
    /// into kind order.
    pub authority_receipts: Vec<(String, String)>,
    /// Spend per budget kind, normalized into vocabulary order.
    pub budget_spend: Vec<(BudgetKind, u64)>,
    pub tier: AutonomyTier,
}

impl AttestationPredicate {
    /// A predicate with the three fields that identify what is being released
    /// and under whose authority. The rest is filled in by the phase that
    /// produced it.
    pub fn new(
        run_ref: impl Into<String>,
        subject_name: impl Into<String>,
        subject_digest: impl Into<String>,
        tier: AutonomyTier,
    ) -> Self {
        Self {
            run_ref: run_ref.into(),
            subject_name: subject_name.into(),
            subject_digest: subject_digest.into(),
            model_ref: String::new(),
            model_digest: String::new(),
            plan_digest: String::new(),
            policy_digest: String::new(),
            tool_call_digests: Vec::new(),
            gate_verdicts: Vec::new(),
            approval_ref: String::new(),
            authority_receipts: Vec::new(),
            budget_spend: Vec::new(),
            tier,
        }
    }

    /// Canonical ordering. Insertion order is not evidence, so every
    /// unordered collection is sorted here; the one ordered collection — the
    /// tool-call sequence — is left exactly as the caller recorded it.
    pub fn normalized(mut self) -> Self {
        self.gate_verdicts.sort();
        self.authority_receipts.sort();
        self.budget_spend
            .sort_by_key(|(kind, amount)| (kind.index(), *amount));
        self
    }
}

/// The canonical byte form of a predicate: sorted keys, no whitespace, no
/// floating point, explicit escaping, integers only.
///
/// This crate has exactly one serializer for this type and it is this one, so
/// a second, divergent encoder cannot drift away from the digest that covers
/// it. Normalization happens here rather than at the call site, so a caller
/// who forgets still gets the same bytes.
pub fn canonical_predicate_bytes(predicate: &AttestationPredicate) -> Vec<u8> {
    let predicate = predicate.clone().normalized();
    let mut out = Vec::new();
    CanonicalJson::Obj(vec![
        ("approval_ref", CanonicalJson::Str(predicate.approval_ref)),
        (
            "authority_receipts",
            CanonicalJson::List(
                predicate
                    .authority_receipts
                    .iter()
                    .map(|(kind, receipt)| {
                        CanonicalJson::List(vec![
                            CanonicalJson::Str(kind.clone()),
                            CanonicalJson::Str(receipt.clone()),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "budget_spend",
            CanonicalJson::List(
                predicate
                    .budget_spend
                    .iter()
                    .map(|(kind, amount)| {
                        CanonicalJson::List(vec![
                            CanonicalJson::Str(kind.as_str().to_string()),
                            CanonicalJson::Uint(*amount),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "gate_verdicts",
            CanonicalJson::List(
                predicate
                    .gate_verdicts
                    .iter()
                    .map(|(gate, verdict)| {
                        CanonicalJson::List(vec![
                            CanonicalJson::Str(gate.clone()),
                            CanonicalJson::Str(verdict.clone()),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("model_digest", CanonicalJson::Str(predicate.model_digest)),
        ("model_ref", CanonicalJson::Str(predicate.model_ref)),
        ("plan_digest", CanonicalJson::Str(predicate.plan_digest)),
        ("policy_digest", CanonicalJson::Str(predicate.policy_digest)),
        ("run_ref", CanonicalJson::Str(predicate.run_ref)),
        (
            "subject_digest",
            CanonicalJson::Str(predicate.subject_digest),
        ),
        ("subject_name", CanonicalJson::Str(predicate.subject_name)),
        (
            "tier",
            CanonicalJson::Str(predicate.tier.as_str().to_string()),
        ),
        (
            "tool_call_digests",
            CanonicalJson::List(
                predicate
                    .tool_call_digests
                    .iter()
                    .map(|digest| CanonicalJson::Str(digest.clone()))
                    .collect(),
            ),
        ),
    ])
    .write(&mut out);
    out
}

/// The predicate's digest, domain-separated so it cannot collide with a digest
/// taken over any other document.
pub fn predicate_digest(predicate: &AttestationPredicate) -> String {
    let mut message = Vec::from(PREDICATE_DIGEST_DOMAIN);
    message.extend_from_slice(&canonical_predicate_bytes(predicate));
    hex_encode(&Sha256::digest(&message))
}

/// Decode a canonical predicate.
///
/// Refuses anything that is not already canonical: the decoded value is
/// re-encoded and compared byte for byte, so a non-canonical encoding cannot
/// pass as one. An unknown field is an error, never a dropped fact.
pub fn parse_canonical_predicate(bytes: &[u8]) -> Result<AttestationPredicate, String> {
    let predicate: AttestationPredicate =
        serde_json::from_slice(bytes).map_err(|e| format!("delivery_predicate_malformed: {e}"))?;
    if canonical_predicate_bytes(&predicate) != bytes {
        return Err("delivery_predicate_not_canonical".to_string());
    }
    Ok(predicate)
}

/// The minimal canonical JSON value this crate emits. No floats by
/// construction: a floating point number has no portable canonical form, so
/// the value space is strings, integers, lists, and objects.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CanonicalJson {
    Str(String),
    Uint(u64),
    List(Vec<CanonicalJson>),
    Obj(Vec<(&'static str, CanonicalJson)>),
}

impl CanonicalJson {
    fn write(&self, out: &mut Vec<u8>) {
        match self {
            CanonicalJson::Str(s) => write_json_string(out, s),
            CanonicalJson::Uint(n) => out.extend_from_slice(n.to_string().as_bytes()),
            CanonicalJson::List(items) => {
                out.push(b'[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(b',');
                    }
                    item.write(out);
                }
                out.push(b']');
            }
            CanonicalJson::Obj(entries) => {
                // Object keys are ordered by UTF-16 code unit, which is the
                // ordering rule a canonical JSON form is defined by — and it
                // is applied here rather than trusted from the literal above.
                let mut sorted: Vec<&(&'static str, CanonicalJson)> = entries.iter().collect();
                sorted.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
                out.push(b'{');
                for (i, (key, value)) in sorted.into_iter().enumerate() {
                    if i > 0 {
                        out.push(b',');
                    }
                    write_json_string(out, key);
                    out.push(b':');
                    value.write(out);
                }
                out.push(b'}');
            }
        }
    }
}

/// Quote and escape one string. Only the characters JSON requires are escaped:
/// the quote, the backslash, and the control characters. Everything else —
/// including non-ASCII — is emitted as UTF-8.
fn write_json_string(out: &mut Vec<u8>, s: &str) {
    out.push(b'"');
    for ch in s.chars() {
        match ch {
            '"' => out.extend_from_slice(b"\\\""),
            '\\' => out.extend_from_slice(b"\\\\"),
            '\u{08}' => out.extend_from_slice(b"\\b"),
            '\t' => out.extend_from_slice(b"\\t"),
            '\n' => out.extend_from_slice(b"\\n"),
            '\u{0C}' => out.extend_from_slice(b"\\f"),
            '\r' => out.extend_from_slice(b"\\r"),
            c if (c as u32) < 0x20 => {
                out.extend_from_slice(format!("\\u{:04x}", c as u32).as_bytes());
            }
            c => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
    out.push(b'"');
}

/// One stage's recorded digests, as recorded by the phase that ran it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageDigest {
    pub stage: String,
    pub input_digest: String,
    pub output_digest: String,
}

impl StageDigest {
    pub fn new(
        stage: impl Into<String>,
        input: impl Into<String>,
        output: impl Into<String>,
    ) -> Self {
        Self {
            stage: stage.into(),
            input_digest: input.into(),
            output_digest: output.into(),
        }
    }
}

/// How one stage differs between what was recorded and what was re-derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageMismatch {
    /// The re-derived run has a stage the recorded run does not.
    MissingRecorded,
    /// The recorded run has a stage the re-derived run does not.
    MissingRederived,
    /// The same stage, different input.
    StageDiffers,
    InputDigestDiffers,
    OutputDigestDiffers,
}

/// One stage's difference, carrying both sides so the report is evidence
/// rather than a verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageDiff {
    pub stage: String,
    pub recorded: Option<StageDigest>,
    pub rederived: Option<StageDigest>,
    pub mismatch: StageMismatch,
}

/// The result of a replay comparison. Data, not a state: it is not a run
/// status, not an approval, not a promotion, and not a trace mutation. A
/// caller that wants a status has to decide what one is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayDiff {
    /// Positions compared across the two sequences.
    pub compared: usize,
    /// Positions where all three recorded fields were byte-identical.
    pub matched: usize,
    /// Positions that differed.
    pub mismatched: usize,
    pub diffs: Vec<StageDiff>,
}

impl ReplayDiff {
    /// Whether every compared position was byte-identical.
    pub fn all_digests_match(&self) -> bool {
        self.mismatched == 0
    }
}

/// Replay-verify: compare recorded stage digests against digests re-derived
/// from the same run's evidence, position by position.
///
/// A byte comparison and nothing else. Models are never re-run — their outputs
/// were recorded and digest-bound, and this checks those bytes rather than
/// trusting a recollection — so the function has no model path to take, by
/// construction and by dependency: this crate cannot reach one.
pub fn compare_replay(recorded: &[StageDigest], rederived: &[StageDigest]) -> ReplayDiff {
    let compared = recorded.len().max(rederived.len());
    let mut matched = 0usize;
    let mut diffs = Vec::new();
    for index in 0..compared {
        let left = recorded.get(index);
        let right = rederived.get(index);
        let (stage, mismatch) = match (left, right) {
            (None, Some(right)) => (right.stage.clone(), StageMismatch::MissingRecorded),
            (Some(left), None) => (left.stage.clone(), StageMismatch::MissingRederived),
            // Cannot occur: positions run to the longer of the two lengths,
            // so one side is always present. Handled rather than assumed.
            (None, None) => continue,
            (Some(left), Some(right)) => {
                let mismatch = if left.stage != right.stage {
                    StageMismatch::StageDiffers
                } else if left.input_digest != right.input_digest {
                    StageMismatch::InputDigestDiffers
                } else if left.output_digest != right.output_digest {
                    StageMismatch::OutputDigestDiffers
                } else {
                    matched += 1;
                    continue;
                };
                (left.stage.clone(), mismatch)
            }
        };
        diffs.push(StageDiff {
            stage,
            recorded: left.cloned(),
            rederived: right.cloned(),
            mismatch,
        });
    }
    ReplayDiff {
        compared,
        matched,
        mismatched: diffs.len(),
        diffs,
    }
}

/// A release's position in its lifecycle. Closed vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseStatus {
    Proposed,
    Approved,
    Building,
    Attested,
    Staged,
    Promoted,
    Verified,
    RolledBack,
    Failed,
}

impl ReleaseStatus {
    /// Every status, in lifecycle order.
    pub const ALL: [ReleaseStatus; 9] = [
        ReleaseStatus::Proposed,
        ReleaseStatus::Approved,
        ReleaseStatus::Building,
        ReleaseStatus::Attested,
        ReleaseStatus::Staged,
        ReleaseStatus::Promoted,
        ReleaseStatus::Verified,
        ReleaseStatus::RolledBack,
        ReleaseStatus::Failed,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ReleaseStatus::Proposed => "proposed",
            ReleaseStatus::Approved => "approved",
            ReleaseStatus::Building => "building",
            ReleaseStatus::Attested => "attested",
            ReleaseStatus::Staged => "staged",
            ReleaseStatus::Promoted => "promoted",
            ReleaseStatus::Verified => "verified",
            ReleaseStatus::RolledBack => "rolled_back",
            ReleaseStatus::Failed => "failed",
        }
    }

    /// Exact wire match; an unknown status is an error, never a guess.
    pub fn parse(s: &str) -> Result<Self, UnknownVocabulary> {
        match s {
            "proposed" => Ok(ReleaseStatus::Proposed),
            "approved" => Ok(ReleaseStatus::Approved),
            "building" => Ok(ReleaseStatus::Building),
            "attested" => Ok(ReleaseStatus::Attested),
            "staged" => Ok(ReleaseStatus::Staged),
            "promoted" => Ok(ReleaseStatus::Promoted),
            "verified" => Ok(ReleaseStatus::Verified),
            "rolled_back" => Ok(ReleaseStatus::RolledBack),
            "failed" => Ok(ReleaseStatus::Failed),
            other => Err(UnknownVocabulary(other.to_string())),
        }
    }

    /// A status with no legal move out of it. A rolled-back or failed release
    /// is finished; a verified one is not, because a verified release can
    /// still be rolled back.
    pub const fn is_terminal(self) -> bool {
        matches!(self, ReleaseStatus::RolledBack | ReleaseStatus::Failed)
    }
}

/// Whether a release may move from one status to the next.
///
/// Total, and it is the law rather than the vocabulary: the closed set of names
/// says what may exist, this says what may change. The happy path advances one
/// step at a time; `failed` is reachable from every status before promotion;
/// after promotion the only escape is a rollback.
pub fn is_legal_release_transition(from: ReleaseStatus, to: ReleaseStatus) -> bool {
    match from {
        ReleaseStatus::Proposed => matches!(to, ReleaseStatus::Approved | ReleaseStatus::Failed),
        ReleaseStatus::Approved => matches!(to, ReleaseStatus::Building | ReleaseStatus::Failed),
        ReleaseStatus::Building => matches!(to, ReleaseStatus::Attested | ReleaseStatus::Failed),
        ReleaseStatus::Attested => {
            matches!(to, ReleaseStatus::Staged | ReleaseStatus::Failed)
        }
        ReleaseStatus::Staged => matches!(to, ReleaseStatus::Promoted | ReleaseStatus::Failed),
        ReleaseStatus::Promoted => {
            matches!(to, ReleaseStatus::Verified | ReleaseStatus::RolledBack)
        }
        ReleaseStatus::Verified => matches!(to, ReleaseStatus::RolledBack),
        ReleaseStatus::RolledBack | ReleaseStatus::Failed => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUBJECT: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
    const OTHER_SUBJECT: &str =
        "sha256:2222222222222222222222222222222222222222222222222222222222222222";
    const POLICY: &str = "policy:delivery:1";
    const OPERATOR: &str = "did:key:zOperator";
    const AGENT: &str = "did:key:zAgent";
    const NOW: u64 = 1_000;
    const EXPIRES: u64 = 2_000;

    fn root_link() -> Attestation {
        Attestation::root(
            "artifact.tar.gz",
            SUBJECT,
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            POLICY,
            OPERATOR,
        )
    }

    fn two_link_chain() -> Vec<Attestation> {
        let root = root_link();
        let child = Attestation::child_of(
            &root,
            "artifact.tar.gz",
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            AGENT,
        );
        vec![root, child]
    }

    fn approval_for(subject_digest: &str) -> Approval {
        Approval {
            subject_digest: subject_digest.to_string(),
            principal_did: OPERATOR.to_string(),
            scope: "promote".to_string(),
            expires_at_epoch: EXPIRES,
        }
    }

    fn ledger() -> BudgetLedger {
        BudgetLedger::new(Ceilings {
            tokens: 1_000,
            tool_calls: 100,
            files: 50,
            minutes: 30,
            blast_radius: 0,
        })
    }

    /// A request whose every input is satisfiable, so each test can spoil
    /// exactly one thing and read the gate's answer.
    fn good_request<'a>(
        chain: &'a [Attestation],
        approval: &'a Approval,
        budgets: &'a BudgetLedger,
    ) -> PromotionRequest<'a> {
        PromotionRequest {
            chain,
            policy_digest: POLICY,
            live_subject_digest: SUBJECT,
            approval: Some(approval),
            budgets,
            tier: AutonomyTier::BoundedAuto,
            trace_mode: TraceMode::Deterministic,
            now_epoch: NOW,
        }
    }

    /// Build a needle from two halves so this file never contains the literal
    /// it searches for — a source scan that matched its own needles would pass
    /// vacuously.
    fn needle(head: &str, tail: &str) -> String {
        format!("{head}{tail}")
    }

    fn declared_dependency_names() -> Vec<String> {
        let manifest = include_str!("../Cargo.toml");
        let after_table = manifest
            .split_once("[dependencies]")
            .expect("the manifest declares a dependencies table")
            .1;
        // The table runs to the end of the file when it is the last section, so
        // an absent following header keeps the whole tail — truncating there
        // would scan nothing and report an empty dependency set.
        let dependencies = after_table
            .split_once("\n[")
            .map_or(after_table, |(body, _)| body);
        dependencies
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .filter_map(|line| {
                line.split_once('=')
                    .map(|(name, _)| name.trim().to_string())
            })
            .collect()
    }

    /// The tier vocabulary and its trace-mode mapping are one pinned table,
    /// declared once and total over the closed set.
    #[test]
    fn tier_mode_mapping_is_pinned_and_total() {
        let expected = [
            (AutonomyTier::Observe, TraceMode::Exploratory),
            (AutonomyTier::Propose, TraceMode::Exploratory),
            (AutonomyTier::BoundedAuto, TraceMode::Deterministic),
            (AutonomyTier::Delegated, TraceMode::Deterministic),
        ];
        // Total: every tier in the closed set is mapped, and the set is closed.
        assert_eq!(AutonomyTier::ALL.len(), expected.len());
        for (tier, mode) in expected {
            assert_eq!(trace_mode_for_tier(tier), mode, "tier {}", tier.as_str());
        }
        // The two narrowest tiers propose and never promote; the two widest
        // are the only ones a promotion may ride.
        assert!(!AutonomyTier::Observe.is_promoting());
        assert!(!AutonomyTier::Propose.is_promoting());
        assert!(AutonomyTier::BoundedAuto.is_promoting());
        assert!(AutonomyTier::Delegated.is_promoting());
        // The mode vocabulary is not overloaded: it stays two words, and an
        // unknown value is an error rather than a guessed default.
        assert_eq!(TraceMode::ALL.len(), 2);
        assert!(TraceMode::parse("deterministic").is_ok());
        assert!(TraceMode::parse("exploratory").is_ok());
        assert!(TraceMode::parse("Deterministic").is_err());
        assert!(TraceMode::parse("auto").is_err());
        for tier in AutonomyTier::ALL {
            assert_eq!(AutonomyTier::parse(tier.as_str()), Ok(tier));
        }
        assert!(AutonomyTier::parse("supervised").is_err());
    }

    /// The phase machine is forward-only, total, and finishes through `done`.
    #[test]
    fn phase_transitions_are_total_and_terminal_through_done() {
        assert_eq!(terminal_phase(), Phase::Done);
        assert!(Phase::ALL.last() == Some(&Phase::Done));
        // The only legal move out of a phase is the next one.
        for (index, from) in Phase::ALL.iter().enumerate() {
            let next = Phase::ALL.get(index + 1);
            assert_eq!(
                is_legal_phase_transition(*from, Phase::Done),
                *from == Phase::Operate,
                "only the operate phase may hand the run to done"
            );
            for to in Phase::ALL {
                let expected = Some(to) == next.copied();
                assert_eq!(
                    is_legal_phase_transition(*from, to),
                    expected,
                    "{} -> {} must be {}",
                    from.as_str(),
                    to.as_str(),
                    expected
                );
            }
        }
        // Terminal means terminal: nothing leaves `done`.
        for to in Phase::ALL {
            assert!(!is_legal_phase_transition(Phase::Done, to));
        }
        // A machine that never skips and never rewinds.
        assert!(!is_legal_phase_transition(Phase::Scope, Phase::Release));
        assert!(!is_legal_phase_transition(Phase::Build, Phase::Design));
        assert!(!is_legal_phase_transition(Phase::Operate, Phase::Design));
        for phase in Phase::ALL {
            assert!(!is_legal_phase_transition(phase, phase));
        }
        // Total over the closed set, and the set is closed.
        assert_eq!(Phase::ALL.len(), 6);
        for phase in Phase::ALL {
            assert_eq!(Phase::parse(phase.as_str()), Ok(phase));
        }
        assert!(Phase::parse("handoff").is_err());
        assert!(Phase::Release.release());
        assert!(!Phase::Operate.release());
    }

    /// Authority comes from the tier. A trace that claims to be deterministic
    /// buys nothing it was not granted, and the gate never reads the mode.
    #[test]
    fn promotion_requires_promoting_tier_and_never_reads_mode() {
        let chain = two_link_chain();
        let approval = approval_for(SUBJECT);
        let budgets = ledger();

        for tier in AutonomyTier::ALL {
            // The mode is varied under a fixed tier: the decision cannot move.
            for mode in TraceMode::ALL {
                let request = PromotionRequest {
                    tier,
                    trace_mode: mode,
                    ..good_request(&chain, &approval, &budgets)
                };
                let expected = match tier {
                    AutonomyTier::Observe | AutonomyTier::Propose => {
                        Decision::Deny(DenyReason::TierNotPermitted)
                    }
                    // The widest tier still needs a human to grant the standing
                    // authorization it rides on.
                    AutonomyTier::Delegated => Decision::Prompt,
                    AutonomyTier::BoundedAuto => Decision::Allow,
                };
                assert_eq!(
                    promote(&request),
                    expected,
                    "tier {} with mode {} must decide {:?}",
                    tier.as_str(),
                    mode.as_str(),
                    expected
                );
            }
        }

        // The narrowest tier holding a deterministic mode is still refused:
        // a recorded mode is a fact about how a trace was produced, not a
        // grant of authority.
        let forged = PromotionRequest {
            tier: AutonomyTier::Observe,
            trace_mode: TraceMode::Deterministic,
            ..good_request(&chain, &approval, &budgets)
        };
        assert_eq!(
            promote(&forged),
            Decision::Deny(DenyReason::TierNotPermitted)
        );

        // An exploratory mode is never itself a denial reason: the vocabulary
        // is about the tier, and the gate has no exploratory-mode branch.
        assert!(
            !DenyReason::ALL
                .iter()
                .any(|reason| reason.as_str().contains("exploratory"))
        );
        // The mapping the storage side reads is the same table the gate refuses
        // to read.
        assert_eq!(
            trace_mode_for_tier(AutonomyTier::Observe),
            TraceMode::Exploratory
        );
    }

    /// Deny wins over every input, and a refusal always names the same thing
    /// for the same evidence.
    #[test]
    fn promotion_is_deny_wins_over_all_inputs() {
        let chain = two_link_chain();
        let approval = approval_for(SUBJECT);
        let budgets = ledger();
        let clean = good_request(&chain, &approval, &budgets);
        assert_eq!(promote(&clean), Decision::Allow);

        // Every defect at once reports the highest-precedence one.
        let empty_budgets = BudgetLedger::default();
        let mut broken = chain.clone();
        broken[1].parent_digest = "sha256:tampered".to_string();
        let spoiled = PromotionRequest {
            chain: &broken,
            policy_digest: "policy:other",
            approval: None,
            budgets: &empty_budgets,
            tier: AutonomyTier::Observe,
            ..good_request(&chain, &approval, &budgets)
        };
        assert_eq!(
            promote(&spoiled),
            Decision::Deny(DenyReason::AttestationChainBroken)
        );

        // One defect at a time: each refuses on its own reason, and the
        // precedence order is the declared one. The subject-mismatch case moves
        // the APPROVAL off the artifact rather than the live digest, because
        // moving the live digest also moves the chain off it — two defects at
        // once, and the chain check would win and name the wrong fact.
        let foreign_approval = approval_for(OTHER_SUBJECT);
        let cases: [(DenyReason, PromotionRequest<'_>); 5] = [
            (
                DenyReason::AttestationChainBroken,
                PromotionRequest {
                    chain: &[],
                    ..good_request(&chain, &approval, &budgets)
                },
            ),
            (
                DenyReason::PolicyDigestMismatch,
                PromotionRequest {
                    policy_digest: "policy:other",
                    ..good_request(&chain, &approval, &budgets)
                },
            ),
            (
                DenyReason::ApprovalMissing,
                PromotionRequest {
                    approval: None,
                    ..good_request(&chain, &approval, &budgets)
                },
            ),
            (
                DenyReason::ApprovalSubjectMismatch,
                PromotionRequest {
                    approval: Some(&foreign_approval),
                    ..good_request(&chain, &approval, &budgets)
                },
            ),
            (
                DenyReason::BudgetExhausted,
                PromotionRequest {
                    budgets: &empty_budgets,
                    ..good_request(&chain, &approval, &budgets)
                },
            ),
        ];
        for (expected, request) in cases {
            let decision = promote(&request);
            assert_eq!(decision, Decision::Deny(expected));
            assert!(decision.is_denied());
        }

        // The declared precedence is the reporting order, chain first.
        let declared = [
            DenyReason::AttestationChainBroken,
            DenyReason::PolicyDigestMismatch,
            DenyReason::ApprovalMissing,
            DenyReason::ApprovalExpired,
            DenyReason::ApprovalSubjectMismatch,
            DenyReason::BudgetExhausted,
            DenyReason::TierNotPermitted,
        ];
        assert_eq!(DenyReason::ALL, declared);
        // An expiry is judged after a subject mismatch, so an approval that is
        // both wrong and stale reports the wrong subject first.
        let stale_and_wrong = Approval {
            subject_digest: OTHER_SUBJECT.to_string(),
            expires_at_epoch: 0,
            ..approval_for(SUBJECT)
        };
        let both = PromotionRequest {
            approval: Some(&stale_and_wrong),
            ..good_request(&chain, &approval, &budgets)
        };
        assert_eq!(
            promote(&both),
            Decision::Deny(DenyReason::ApprovalSubjectMismatch)
        );

        // The gate is total: no input combination has no answer.
        for tier in AutonomyTier::ALL {
            for mode in TraceMode::ALL {
                for approval_slot in [None, Some(&approval)] {
                    for now in [0, NOW, EXPIRES, u64::MAX] {
                        let request = PromotionRequest {
                            tier,
                            trace_mode: mode,
                            approval: approval_slot,
                            now_epoch: now,
                            ..good_request(&chain, &approval, &budgets)
                        };
                        // Total, and never a silent pass.
                        let decision = promote(&request);
                        assert!(matches!(
                            decision,
                            Decision::Allow | Decision::Prompt | Decision::Deny(_)
                        ));
                        assert!(!decision.is_denied() || decision != Decision::Allow);
                    }
                }
            }
        }
    }

    /// An approval names one artifact. Change the artifact and there is no
    /// field left that could survive the change.
    #[test]
    fn approval_void_on_subject_digest_change() {
        let chain = two_link_chain();
        let approval = approval_for(SUBJECT);
        let budgets = ledger();
        assert!(approval.binds(SUBJECT));
        assert!(!approval.binds(OTHER_SUBJECT));
        assert!(approval.is_current(SUBJECT, NOW));
        // The boundary is fail-closed: expiry equal to the deciding instant
        // has expired.
        assert!(!approval.is_current(SUBJECT, EXPIRES));
        assert!(!approval.is_current(SUBJECT, EXPIRES + 1));

        // The same artifact the approval names is allowed...
        let same = good_request(&chain, &approval, &budgets);
        assert_eq!(promote(&same), Decision::Allow);

        // ...and one byte later in the artifact is not. The approval is what
        // goes stale, so the isolated case moves the APPROVAL off the artifact
        // and leaves the chain describing what is being promoted: the refusal
        // then names the approval, which is the fact that actually failed.
        let foreign = approval_for(OTHER_SUBJECT);
        let mismatched = PromotionRequest {
            approval: Some(&foreign),
            ..good_request(&chain, &approval, &budgets)
        };
        assert_eq!(
            promote(&mismatched),
            Decision::Deny(DenyReason::ApprovalSubjectMismatch)
        );

        // Moving the live digest instead also moves the chain off that
        // artifact, so two facts fail at once and the chain check fires first
        // and names the refusal. The declared precedence, observed.
        let moved = PromotionRequest {
            live_subject_digest: OTHER_SUBJECT,
            ..good_request(&chain, &approval, &budgets)
        };
        assert_eq!(
            promote(&moved),
            Decision::Deny(DenyReason::AttestationChainBroken)
        );
        // The chain check fires first, because the chain does not describe the
        // artifact either. Rebasing the chain onto the new artifact leaves every
        // fact consistent, and the same approval is good again.
        let moved_with_moved_chain = vec![Attestation {
            subject_digest: OTHER_SUBJECT.to_string(),
            ..root_link()
        }];
        let rebased = PromotionRequest {
            chain: &moved_with_moved_chain,
            live_subject_digest: OTHER_SUBJECT,
            approval: Some(&Approval {
                subject_digest: OTHER_SUBJECT.to_string(),
                ..approval.clone()
            }),
            ..good_request(&chain, &approval, &budgets)
        };
        assert_eq!(promote(&rebased), Decision::Allow);

        // An expired approval on the right artifact is an expiry refusal.
        let expired = Approval {
            expires_at_epoch: NOW,
            ..approval.clone()
        };
        let at_expiry = PromotionRequest {
            approval: Some(&expired),
            ..good_request(&chain, &approval, &budgets)
        };
        assert_eq!(
            promote(&at_expiry),
            Decision::Deny(DenyReason::ApprovalExpired)
        );

        // An approval with no subject named binds nothing.
        let unbound = Approval {
            subject_digest: String::new(),
            ..approval.clone()
        };
        assert!(!unbound.binds(SUBJECT));
        assert!(!unbound.is_current(SUBJECT, NOW));
        let with_unbound = PromotionRequest {
            approval: Some(&unbound),
            ..good_request(&chain, &approval, &budgets)
        };
        assert_eq!(
            promote(&with_unbound),
            Decision::Deny(DenyReason::ApprovalSubjectMismatch)
        );
    }

    /// One serializer, one byte form: stable across calls, independent of
    /// insertion order, sorted by key, and the only form that parses back.
    #[test]
    fn attestation_predicate_serializes_canonically_and_stably() {
        let mut predicate = AttestationPredicate::new(
            "run-7",
            "artifact.tar.gz",
            SUBJECT,
            AutonomyTier::BoundedAuto,
        );
        predicate.model_ref = "registry:model-3".to_string();
        predicate.model_digest = "sha256:model".to_string();
        predicate.plan_digest = "sha256:plan".to_string();
        predicate.policy_digest = POLICY.to_string();
        predicate.approval_ref = "approval-2".to_string();
        predicate.tool_call_digests = vec![
            "sha256:tool-1".to_string(),
            "sha256:tool-2".to_string(),
            "sha256:tool-0".to_string(),
        ];
        predicate.gate_verdicts = vec![
            ("verify".to_string(), "pass".to_string()),
            ("build".to_string(), "pass".to_string()),
        ];
        predicate.authority_receipts = vec![
            ("vcs".to_string(), "sha256:receipt-vcs".to_string()),
            ("ci".to_string(), "sha256:receipt-ci".to_string()),
        ];
        predicate.budget_spend = vec![(BudgetKind::Minutes, 12), (BudgetKind::Tokens, 900)];

        let bytes = canonical_predicate_bytes(&predicate);
        let text = String::from_utf8(bytes.clone()).expect("canonical bytes are utf-8");

        // Stable: the same predicate serializes to the same bytes every time,
        // and its digest with it.
        assert_eq!(canonical_predicate_bytes(&predicate), bytes);
        assert_eq!(predicate_digest(&predicate), predicate_digest(&predicate));

        // Sorted keys, no whitespace, integers only.
        assert!(text.starts_with("{\"approval_ref\":"));
        let key_positions: Vec<usize> = [
            "approval_ref",
            "authority_receipts",
            "budget_spend",
            "gate_verdicts",
            "model_digest",
            "model_ref",
            "plan_digest",
            "policy_digest",
            "run_ref",
            "subject_digest",
            "subject_name",
            "tier",
            "tool_call_digests",
        ]
        .iter()
        .map(|key| {
            text.find(&format!("\"{key}\""))
                .unwrap_or_else(|| panic!("key {key} missing from the canonical form"))
        })
        .collect();
        assert!(
            key_positions.windows(2).all(|pair| pair[0] < pair[1]),
            "canonical keys must be in sorted order: {key_positions:?}"
        );
        assert!(!text.contains('\n'));
        assert!(!text.contains(": "));
        assert!(!text.contains(", "));

        // Insertion order is not evidence: the same predicate assembled in the
        // opposite order produces the same bytes.
        let mut shuffled = predicate.clone();
        shuffled.gate_verdicts.reverse();
        shuffled.authority_receipts.reverse();
        shuffled.budget_spend.reverse();
        assert_eq!(canonical_predicate_bytes(&shuffled), bytes);

        // Execution order is evidence: the tool-call sequence is preserved
        // exactly, and never sorted.
        assert!(text.contains("[\"sha256:tool-1\",\"sha256:tool-2\",\"sha256:tool-0\"]"));
        let mut reordered_calls = predicate.clone();
        reordered_calls.tool_call_digests.reverse();
        assert_ne!(
            canonical_predicate_bytes(&reordered_calls),
            bytes,
            "tool-call order is evidence and must change the canonical form"
        );

        // The digest is domain-separated, so it is not the digest of the bare
        // document, and any change to any field moves it.
        let bare = hex_encode(&Sha256::digest(&bytes));
        assert_ne!(predicate_digest(&predicate), bare);
        let mut changed = predicate.clone();
        changed.plan_digest = "sha256:other-plan".to_string();
        assert_ne!(predicate_digest(&changed), predicate_digest(&predicate));

        // The round trip: the canonical form decodes to the same predicate, and
        // a re-encode is byte-identical.
        let decoded = parse_canonical_predicate(&bytes).expect("canonical form decodes");
        assert_eq!(decoded, predicate.normalized());
        assert_eq!(canonical_predicate_bytes(&decoded), bytes);

        // Escaping is explicit and lossless.
        let mut quoted = AttestationPredicate::new(
            "run-7",
            "artifact \"quoted\"\\slash\ttab\nnewline",
            SUBJECT,
            AutonomyTier::Observe,
        );
        quoted.approval_ref = "café".to_string();
        let quoted_bytes = canonical_predicate_bytes(&quoted);
        let quoted_text = String::from_utf8(quoted_bytes.clone()).expect("utf-8");
        assert!(
            quoted_text.contains(r#""subject_name":"artifact \"quoted\"\\slash\ttab\nnewline""#)
        );
        let quoted_back = parse_canonical_predicate(&quoted_bytes).expect("escaped form decodes");
        assert_eq!(quoted_back, quoted.normalized());

        // Anything that is not already canonical is refused: unknown fields,
        // and the same object written in a different key order.
        assert!(parse_canonical_predicate(br#"{"surprise":1}"#).is_err());
        let reordered = text.replace(
            "{\"approval_ref\":\"approval-2\",\"authority_receipts\"",
            "{\"authority_receipts\":[",
        );
        assert!(parse_canonical_predicate(reordered.as_bytes()).is_err());
    }

    /// Arithmetic saturates, exhaustion is judged fail-closed, and the
    /// dimension with no defined ceiling takes part in no decision.
    #[test]
    fn budget_exhaustion_is_saturating_and_fail_closed() {
        let budget = Budget::new(BudgetKind::Tokens, 10);
        assert_eq!(budget.spend(4).spent, 4);
        // A draw that would wrap pins at the maximum instead.
        let saturated = budget.spend(u64::MAX);
        assert_eq!(saturated.spent, u64::MAX);
        assert_eq!(saturated.spend(7).spent, u64::MAX);
        assert_eq!(Budget::new(BudgetKind::Tokens, 0).spend(1).spent, 1);

        // Exhausted at the boundary, in both directions: drawn exactly to the
        // ceiling, and overdrawn past it. A budget with nothing left and an
        // overdrawn one are the same fact.
        assert!(Budget::new(BudgetKind::Files, 5).spend(5).is_exhausted());
        assert!(Budget::new(BudgetKind::Files, 5).spend(6).is_exhausted());
        // Below the boundary there is headroom left, and an undrawn ceiling is
        // all headroom.
        assert!(Budget::new(BudgetKind::Files, 5).spend(4).has_headroom());
        assert!(Budget::new(BudgetKind::Files, 5).has_headroom());
        assert!(Budget::new(BudgetKind::Files, u64::MAX).has_headroom());

        // A ledger that was never granted anything refuses: the default grants
        // nothing, and nothing left is the refusing case.
        let default_ledger = BudgetLedger::default();
        assert!(!default_ledger.has_headroom());
        assert_eq!(
            default_ledger.exhausted(),
            vec![
                BudgetKind::Tokens,
                BudgetKind::ToolCalls,
                BudgetKind::Files,
                BudgetKind::Minutes
            ]
        );

        // The undefined dimension is carried and never judged.
        assert!(!BudgetKind::BlastRadius.is_enforced());
        let no_tokens = BudgetLedger::new(Ceilings {
            tokens: 10,
            tool_calls: 10,
            files: 10,
            minutes: 10,
            blast_radius: 0,
        });
        assert!(no_tokens.has_headroom());
        assert!(no_tokens.budget(BudgetKind::BlastRadius).is_exhausted());
        assert!(!no_tokens.exhausted().contains(&BudgetKind::BlastRadius));
        // Drawing against it still records what happened.
        let drawn = no_tokens.spend(BudgetKind::BlastRadius, 3);
        assert_eq!(drawn.budget(BudgetKind::BlastRadius).spent, 3);
        assert!(drawn.has_headroom());
        // Every kind has exactly one entry, and the vocabulary is closed.
        assert_eq!(BudgetKind::ALL.len(), BudgetKind::COUNT);
        for kind in BudgetKind::ALL {
            assert_eq!(no_tokens.budget(kind).kind, kind);
            assert_eq!(BudgetKind::parse(kind.as_str()), Ok(kind));
        }
        assert!(BudgetKind::parse("spend").is_err());

        // An exhausted budget refuses the promotion and is named.
        let chain = two_link_chain();
        let approval = approval_for(SUBJECT);
        let exhausted = default_ledger.spend(BudgetKind::Tokens, 1);
        let request = good_request(&chain, &approval, &exhausted);
        assert_eq!(
            promote(&request),
            Decision::Deny(DenyReason::BudgetExhausted)
        );
        // One token of headroom is enough.
        let granted = BudgetLedger::new(Ceilings {
            tokens: 1,
            tool_calls: 1,
            files: 1,
            minutes: 1,
            blast_radius: 0,
        });
        let ok = good_request(&chain, &approval, &granted);
        assert_eq!(promote(&ok), Decision::Allow);
        assert_eq!(
            promote(&good_request(
                &chain,
                &approval,
                &granted.spend(BudgetKind::ToolCalls, 1)
            )),
            Decision::Deny(DenyReason::BudgetExhausted)
        );
    }

    /// The status list says what may exist; the transition function says what
    /// may change, and it answers for every pair.
    #[test]
    fn release_status_transitions_are_total() {
        let legal = [
            (ReleaseStatus::Proposed, ReleaseStatus::Approved),
            (ReleaseStatus::Proposed, ReleaseStatus::Failed),
            (ReleaseStatus::Approved, ReleaseStatus::Building),
            (ReleaseStatus::Approved, ReleaseStatus::Failed),
            (ReleaseStatus::Building, ReleaseStatus::Attested),
            (ReleaseStatus::Building, ReleaseStatus::Failed),
            (ReleaseStatus::Attested, ReleaseStatus::Staged),
            (ReleaseStatus::Attested, ReleaseStatus::Failed),
            (ReleaseStatus::Staged, ReleaseStatus::Promoted),
            (ReleaseStatus::Staged, ReleaseStatus::Failed),
            (ReleaseStatus::Promoted, ReleaseStatus::Verified),
            (ReleaseStatus::Promoted, ReleaseStatus::RolledBack),
            (ReleaseStatus::Verified, ReleaseStatus::RolledBack),
        ];
        // Total: every pair in the closed square has an answer.
        let mut answered = 0usize;
        for from in ReleaseStatus::ALL {
            for to in ReleaseStatus::ALL {
                assert_eq!(
                    is_legal_release_transition(from, to),
                    legal.contains(&(from, to)),
                    "{} -> {}",
                    from.as_str(),
                    to.as_str()
                );
                answered += 1;
            }
        }
        assert_eq!(
            answered,
            ReleaseStatus::ALL.len() * ReleaseStatus::ALL.len()
        );

        // A release never skips a step, and never rewinds.
        for (from, to) in legal {
            assert!(ReleaseStatus::ALL.contains(&from) && ReleaseStatus::ALL.contains(&to));
            if to != ReleaseStatus::Failed && to != ReleaseStatus::RolledBack {
                let from_index = ReleaseStatus::ALL
                    .iter()
                    .position(|status| *status == from)
                    .expect("from is in the closed set");
                let to_index = ReleaseStatus::ALL
                    .iter()
                    .position(|status| *status == to)
                    .expect("to is in the closed set");
                assert_eq!(
                    to_index,
                    from_index + 1,
                    "{} -> {}",
                    from.as_str(),
                    to.as_str()
                );
            }
        }
        // Every pre-promotion status can fail; after promotion the only escape
        // is a rollback, and a finished release is finished.
        for status in ReleaseStatus::ALL {
            if status == ReleaseStatus::Failed || status == ReleaseStatus::RolledBack {
                assert!(status.is_terminal());
            } else {
                assert!(!status.is_terminal());
                if status == ReleaseStatus::Promoted || status == ReleaseStatus::Verified {
                    assert!(is_legal_release_transition(
                        status,
                        ReleaseStatus::RolledBack
                    ));
                    assert!(!is_legal_release_transition(status, ReleaseStatus::Failed));
                } else {
                    assert!(is_legal_release_transition(status, ReleaseStatus::Failed));
                }
            }
            assert!(
                !is_legal_release_transition(status, status),
                "a status never re-enters itself"
            );
        }

        // The closed vocabulary, parseable and not guessable.
        assert_eq!(ReleaseStatus::ALL.len(), 9);
        for status in ReleaseStatus::ALL {
            assert_eq!(ReleaseStatus::parse(status.as_str()), Ok(status));
        }
        assert!(ReleaseStatus::parse("rolledback").is_err());
        assert!(ReleaseStatus::parse("shipped").is_err());
    }

    /// Replay-verify compares bytes, and the crate has no path to a model.
    #[test]
    fn replay_diff_is_byte_comparison_and_zero_model_calls() {
        let recorded = vec![
            StageDigest::new("build", "sha256:in-1", "sha256:out-1"),
            StageDigest::new("verify", "sha256:in-2", "sha256:out-2"),
        ];
        let rederived = recorded.clone();

        // Byte-identical: every position matched, nothing to report.
        let clean = compare_replay(&recorded, &rederived);
        assert_eq!(clean.compared, 2);
        assert_eq!(clean.matched, 2);
        assert_eq!(clean.mismatched, 0);
        assert!(clean.diffs.is_empty());
        assert!(clean.all_digests_match());
        // The report is data: it carries no status, no approval, and no verdict
        // it could be promoted from.
        assert!(!format!("{clean:?}").contains("Allow"));
        assert!(!format!("{clean:?}").contains("Deny"));

        // One flipped character in one digest is the whole difference.
        let mut flipped = recorded.clone();
        flipped[1].output_digest = "sha256:out-3".to_string();
        let diff = compare_replay(&recorded, &flipped);
        assert_eq!(diff.compared, 2);
        assert_eq!(diff.matched, 1);
        assert_eq!(diff.mismatched, 1);
        assert_eq!(diff.diffs[0].stage, "verify");
        assert_eq!(diff.diffs[0].mismatch, StageMismatch::OutputDigestDiffers);
        assert!(!diff.all_digests_match());
        // Both sides travel with the report, so it is evidence and not a
        // verdict.
        assert_eq!(diff.diffs[0].recorded, Some(recorded[1].clone()));
        assert_eq!(diff.diffs[0].rederived, Some(flipped[1].clone()));

        // Input, stage, and length differences are named distinctly.
        let mut wrong_input = recorded.clone();
        wrong_input[0].input_digest = "sha256:in-9".to_string();
        assert_eq!(
            compare_replay(&recorded, &wrong_input).diffs[0].mismatch,
            StageMismatch::InputDigestDiffers
        );
        let mut renamed = recorded.clone();
        renamed[0].stage = "compile".to_string();
        assert_eq!(
            compare_replay(&recorded, &renamed).diffs[0].mismatch,
            StageMismatch::StageDiffers
        );
        let shorter = vec![recorded[0].clone()];
        let short_diff = compare_replay(&recorded, &shorter);
        assert_eq!(short_diff.compared, 2);
        assert_eq!(
            short_diff.diffs[0].mismatch,
            StageMismatch::MissingRederived
        );
        let long_diff = compare_replay(&shorter, &recorded);
        assert_eq!(long_diff.diffs[0].mismatch, StageMismatch::MissingRecorded);
        // Nothing recorded is nothing compared.
        let none = compare_replay(&[], &[]);
        assert_eq!(none.compared, 0);
        assert!(none.all_digests_match());

        // Zero model calls by construction: the crate declares no provider,
        // no async runtime, and no network or process edge, so there is no
        // path from a comparison to a model.
        assert_eq!(
            declared_dependency_names(),
            vec!["serde", "serde_json", "sha2"]
        );
        let source = include_str!("lib.rs");
        for (head, tail) in [
            ("req", "west"),
            ("hy", "per"),
            ("ax", "um"),
            ("to", "kio"),
            ("async", " fn"),
            ("std::", "net"),
            ("std::", "fs"),
            ("std::", "env"),
            ("std::", "process"),
            ("std::", "thread"),
            ("System", "Time"),
            ("Instant", "::"),
            ("unsafe", " fn"),
            ("unsafe", " impl"),
            ("unsafe", " {"),
        ] {
            assert!(
                !source.contains(&needle(head, tail)),
                "the crate must carry no {} edge",
                needle(head, tail)
            );
        }
    }

    /// The digest is pinned to an answer computed OUTSIDE this crate.
    ///
    /// Every other pin in this file compares a digest only against another
    /// digest this file produced, so an encoder that emitted uppercase, swapped
    /// the nibbles, or dropped a leading zero would satisfy all of them while
    /// producing a digest no host could reproduce. The expected value was
    /// computed with an independent SHA-256 implementation over the message the
    /// crate builds by hand, and it pins three things at once: the hex
    /// encoding, the exact bytes of the record domain separator, and the field
    /// order with its `|` framing.
    #[test]
    fn digest_encoding_is_a_known_answer_not_only_self_consistent() {
        // The encoder itself: empty, a leading zero byte, a high nibble, and a
        // byte whose nibbles differ (0x10 -> "10", never "01").
        assert_eq!(hex_encode(b""), "");
        assert_eq!(hex_encode(&[0x00, 0x0f, 0x10, 0xff]), "000f10ff");
        assert_eq!(hex_encode(&[0xab, 0xcd]), "abcd");
        // And the framed, domain-separated record digest of the root fixture.
        assert_eq!(
            root_link().record_digest(),
            "9fbcf60dabc26c8905a2eb8820100e6709df27a0a409d49f04eab59d85a60037"
        );
        // A one-byte change to any framed field moves the digest, so the
        // separator is load-bearing rather than decorative.
        let mut moved = root_link();
        moved.signer_did = "did:key:zOperato".to_string();
        assert_ne!(moved.record_digest(), root_link().record_digest());
    }
}
