//! The deterministic stage runner. Each stage is a PURE function of
//! `(StageInput, &DecisionPipelineConfig)` over the run's injected
//! environment (the retrieval seam, the query text, the model) — no side
//! effects, no I/O, no clock (timing is an injected argument), no unsafe,
//! no panics on recoverable paths. The runner is total: a stage refusal
//! folds into the run outcome as honest escalation data — never a
//! synthesized output, never a guess. A DecisionModel proposes; only the
//! gate disposes.
//!
//! Every emitted [`StageRecord`] digests its stage input and output over
//! the compact serde serialization (the canonical-form law), so a trace
//! binds the exact data each stage saw and produced. Raw query text and
//! raw evidence text never enter a stage's recorded shape: the query rides
//! only as its digest inside the normalized ask, and evidence rides only
//! as provenance refs (ids + trust tiers).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::config::{
    DecisionPipelineConfig, LoadedPipelineConfig, RetrievalParams, StageName, ThresholdFallback,
    sha256_of_json,
};
use super::model::{
    ConfigRef, DecisionContext, DecisionError, DecisionInput, DecisionModel, DecisionOutput,
    DecisionValue, EvidenceRef, ModelMetadata, QuestionKind, QuestionRef, RefusalReason, RunMode,
    TrustTier,
};
use crate::search::SearchResult;
use crate::search::rrf_fuse;

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

// ── the serde face of the SDK seam ─────────────────────────────────────

/// The serde face of the SDK's [`TrustTier`] (the SDK type carries no
/// serde by design; the lowercase string form is the recorded
/// representation). Variant order IS the trust order — `Ord` follows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SerdeTier {
    Untrusted,
    Vetted,
    Governed,
}

impl SerdeTier {
    pub(crate) fn of(tier: TrustTier) -> Self {
        match tier {
            TrustTier::Untrusted => SerdeTier::Untrusted,
            TrustTier::Vetted => SerdeTier::Vetted,
            TrustTier::Governed => SerdeTier::Governed,
        }
    }

    pub(crate) fn trust(self) -> TrustTier {
        match self {
            SerdeTier::Untrusted => TrustTier::Untrusted,
            SerdeTier::Vetted => TrustTier::Vetted,
            SerdeTier::Governed => TrustTier::Governed,
        }
    }
}

/// The serde face of [`QuestionKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AskKind {
    Choice,
    Score,
    Noul,
}

impl AskKind {
    pub(crate) fn of(kind: QuestionKind) -> Self {
        match kind {
            QuestionKind::Choice => AskKind::Choice,
            QuestionKind::Score => AskKind::Score,
            QuestionKind::Noul => AskKind::Noul,
        }
    }

    fn question_kind(self) -> QuestionKind {
        match self {
            AskKind::Choice => QuestionKind::Choice,
            AskKind::Score => QuestionKind::Score,
            AskKind::Noul => QuestionKind::Noul,
        }
    }
}

/// The serde face of the SDK's [`EvidenceRef`]: one evidence provenance
/// ref — id + tier, text unrepresentable by construction.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct EvidenceCandidate {
    pub(crate) evidence_id: String,
    pub(crate) tier: SerdeTier,
}

impl EvidenceCandidate {
    fn of(r: &EvidenceRef) -> Self {
        EvidenceCandidate {
            evidence_id: r.evidence_id.clone(),
            tier: SerdeTier::of(r.tier),
        }
    }

    fn evidence_ref(&self) -> EvidenceRef {
        EvidenceRef {
            evidence_id: self.evidence_id.clone(),
            tier: self.tier.trust(),
        }
    }
}

/// The serde face of [`DecisionValue`].
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SerdeDecisionValue {
    Choice {
        label: String,
        probabilities: Option<Vec<(String, f64)>>,
        confidence: Option<f64>,
    },
    Score {
        value: i64,
        range: (i64, i64),
    },
    Noul {
        value: bool,
    },
}

impl SerdeDecisionValue {
    fn of(v: &DecisionValue) -> Self {
        match v {
            DecisionValue::Choice {
                label,
                probabilities,
                confidence,
            } => SerdeDecisionValue::Choice {
                label: label.clone(),
                probabilities: probabilities.clone(),
                confidence: *confidence,
            },
            DecisionValue::Score { value, range } => SerdeDecisionValue::Score {
                value: *value,
                range: *range,
            },
            DecisionValue::Noul { value } => SerdeDecisionValue::Noul { value: *value },
        }
    }
}

/// The serde face of [`DecisionOutput`]: who decided, when (the context's
/// own created_at — never a clock), the typed value, and the consumed
/// evidence refs.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub(crate) struct SerdeDecisionOutput {
    pub(crate) model_id: String,
    pub(crate) model_version: String,
    pub(crate) produced_at: i64,
    pub(crate) value: SerdeDecisionValue,
    pub(crate) evidence_refs: Vec<EvidenceCandidate>,
}

impl SerdeDecisionOutput {
    fn of(o: &DecisionOutput) -> Self {
        SerdeDecisionOutput {
            model_id: o.model_id.clone(),
            model_version: o.model_version.clone(),
            produced_at: o.produced_at,
            value: SerdeDecisionValue::of(&o.value),
            evidence_refs: o.evidence_refs.iter().map(EvidenceCandidate::of).collect(),
        }
    }
}

// ── the retrieval seam ─────────────────────────────────────────────────

/// One retrieved context item, by REFERENCE only: the evidence id, the
/// content digest, the trust tier, the per-leg ranks and fused score the
/// fusion recorded, and the taint labels. Text never crosses the seam.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub(crate) struct ContextHit {
    pub(crate) evidence_id: String,
    pub(crate) content_digest: String,
    pub(crate) tier: SerdeTier,
    pub(crate) vector_rank: Option<usize>,
    pub(crate) fts_rank: Option<usize>,
    pub(crate) graph_rank: Option<usize>,
    pub(crate) fused_score: Option<f32>,
    pub(crate) flagged: bool,
    pub(crate) untrusted: bool,
}

/// Why the seam declined to retrieve. Closed: the callers escalate as data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RetrievalRefusal {
    Unavailable,
}

/// The context-retrieval seam: object-safe and thread-safe (the
/// `DecisionModel`/`Embedder` precedent), so a production implementation
/// can own the database-bound search internals while the stage stays pure
/// over the seam. The query text is an owned argument of the CALL — it
/// flows into the retriever and never back out into the recorded shape.
pub(crate) trait ContextRetriever: Send + Sync {
    fn retrieve(
        &self,
        query: &str,
        params: &RetrievalParams,
    ) -> Result<Vec<ContextHit>, RetrievalRefusal>;

    /// The algorithm label the trace records for this retriever — the seam
    /// names what actually ran.
    fn algorithm(&self) -> &str;
}

// Compile-time pin: the seam stays object-safe and thread-safe.
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Box<dyn ContextRetriever>>();
};

/// One declared retrieval hit: a knowledge rowid, its content digest, its
/// trust tier, and its taint labels — the deterministic in-tree
/// retriever's input rows.
#[derive(Debug, Clone)]
pub(crate) struct DeclaredHit {
    pub(crate) id: i64,
    pub(crate) digest: String,
    pub(crate) tier: TrustTier,
    pub(crate) untrusted: bool,
    pub(crate) flagged: bool,
}

/// The deterministic in-tree retriever: three declared ranked lists, fused
/// by the shipped pure [`rrf_fuse`] primitive under the config's recorded
/// parameters. The fused `SearchResult`s are built with EMPTY content —
/// text never exists on this path — and mapped to reference-only
/// [`ContextHit`]s. The DB-bound production retriever wires where a caller
/// with state exists.
pub(crate) struct DeclaredListRetriever {
    pub(crate) vector: Vec<DeclaredHit>,
    pub(crate) fts: Vec<DeclaredHit>,
    pub(crate) graph: Vec<DeclaredHit>,
}

impl DeclaredListRetriever {
    fn to_results(hits: &[DeclaredHit]) -> Vec<SearchResult> {
        hits.iter()
            .map(|h| SearchResult {
                id: h.id,
                flagged: h.flagged,
                untrusted: h.untrusted,
                ..Default::default()
            })
            .collect()
    }

    fn hit_map(&self) -> std::collections::HashMap<i64, &DeclaredHit> {
        self.vector
            .iter()
            .chain(&self.fts)
            .chain(&self.graph)
            .map(|h| (h.id, h))
            .collect()
    }
}

impl ContextRetriever for DeclaredListRetriever {
    fn retrieve(
        &self,
        _query: &str,
        params: &RetrievalParams,
    ) -> Result<Vec<ContextHit>, RetrievalRefusal> {
        let by_id = self.hit_map();
        let fused = rrf_fuse(
            &Self::to_results(&self.vector),
            &Self::to_results(&self.fts),
            &Self::to_results(&self.graph),
            params.rrf_k as usize,
            params.limit as usize,
            params.leg.leg_filter(),
        );
        let mut hits = Vec::with_capacity(fused.len());
        for r in fused {
            let Some(declared) = by_id.get(&r.id) else {
                // Every fused id came from the declared lists; a miss would
                // mean a seam contract break — skip it, never guess.
                continue;
            };
            hits.push(ContextHit {
                evidence_id: r.id.to_string(),
                content_digest: declared.digest.clone(),
                tier: SerdeTier::of(declared.tier),
                vector_rank: r.provenance.vector_rank,
                fts_rank: r.provenance.fts_rank,
                graph_rank: r.provenance.graph_rank,
                fused_score: r.provenance.fused_score,
                flagged: r.flagged,
                untrusted: r.untrusted,
            });
        }
        Ok(hits)
    }

    fn algorithm(&self) -> &str {
        "search.rrf_fuse/declared-lists"
    }
}

// ── the closed escalation vocabulary ───────────────────────────────────

/// Why a run escalated. Closed and exhaustive everywhere: adding a variant
/// is a breaking change to this module's vocabulary, by the compiler's own
/// hand (the `Display` match has no wildcard arm). An escalation is DATA
/// the caller turns into a human path — the harness never disposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EscalationReason {
    OutOfVocabulary,
    InsufficientEvidence,
    InsufficientEvidenceTier,
    InvalidInput,
    RetrievalUnavailable,
    Misconfigured,
    ModelDisabled,
    PolicyUntrustedOnly,
    MissingDecision,
    FallbackEscalate,
}

impl From<RefusalReason> for EscalationReason {
    fn from(r: RefusalReason) -> Self {
        match r {
            RefusalReason::OutOfVocabulary => EscalationReason::OutOfVocabulary,
            RefusalReason::InsufficientEvidence => EscalationReason::InsufficientEvidence,
            RefusalReason::InsufficientEvidenceTier => EscalationReason::InsufficientEvidenceTier,
        }
    }
}

impl EscalationReason {
    fn from_decision_error(e: &DecisionError) -> Self {
        match e {
            DecisionError::Refused { reason } => EscalationReason::from(*reason),
            DecisionError::Disabled => EscalationReason::ModelDisabled,
            DecisionError::Misconfigured => EscalationReason::Misconfigured,
        }
    }

    /// The static, honest sentence recorded with the escalation — machine
    /// text only, never raw content.
    fn detail(self) -> &'static str {
        match self {
            EscalationReason::OutOfVocabulary => "the ask is outside the declared vocabulary",
            EscalationReason::InsufficientEvidence => "no evidence arrived to decide on",
            EscalationReason::InsufficientEvidenceTier => {
                "the evidence does not meet the required trust tier"
            }
            EscalationReason::InvalidInput => "the run input failed validation",
            EscalationReason::RetrievalUnavailable => {
                "the retrieval seam is unavailable; nothing was decided"
            }
            EscalationReason::Misconfigured => {
                "the context does not bind the model's configuration"
            }
            EscalationReason::ModelDisabled => "the model is disabled by configuration or policy",
            EscalationReason::PolicyUntrustedOnly => {
                "policy law: every evidence tier is untrusted, so the run escalates to a human"
            }
            EscalationReason::MissingDecision => "the stage's required upstream output is absent",
            EscalationReason::FallbackEscalate => {
                "the threshold fallback declares ambiguity a human decision"
            }
        }
    }
}

impl core::fmt::Display for EscalationReason {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.detail())
    }
}

/// The typed escalation record: which stage declined, why (the closed
/// vocabulary), and its static detail. The harness writes no durable state
/// — this record is the data the caller's gate consumes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct EscalationRecord {
    pub(crate) stage: StageName,
    pub(crate) reason: EscalationReason,
    pub(crate) detail: String,
}

impl EscalationRecord {
    fn at(stage: StageName, reason: EscalationReason) -> Self {
        EscalationRecord {
            stage,
            reason,
            detail: reason.detail().to_string(),
        }
    }
}

/// Why a stage refused. Propagates as honest escalation data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StageRefusal {
    pub(crate) stage: StageName,
    pub(crate) reason: EscalationReason,
    pub(crate) detail: String,
}

impl StageRefusal {
    fn named(stage: StageName, reason: EscalationReason) -> Self {
        StageRefusal {
            stage,
            reason,
            detail: reason.detail().to_string(),
        }
    }
}

/// The policy stage's outcome: clear, or escalate with the closed reason.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PolicyOutcome {
    Clear,
    Escalate { reason: EscalationReason },
}

/// The threshold's verdict, with the escalation provenance when it
/// escalates: which stage demanded the human and why.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ActionVerdict {
    Act,
    Approve,
    Reject,
    Escalate {
        origin: StageName,
        reason: EscalationReason,
    },
}

/// The closed action labels — the threshold vocabulary {act, approve,
/// reject, escalate}. Approve and escalate are the human paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ActionLabel {
    Act,
    Approve,
    Reject,
    Escalate,
}

/// The action/escalation stage's typed output: what the run proposes and,
/// when it escalates, the typed escalation record. No durable state.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct ActionRecord {
    pub(crate) action: ActionLabel,
    pub(crate) escalation: Option<EscalationRecord>,
}

// ── stage input / output / records ─────────────────────────────────────

/// The question as the recorded shape carries it: id + closed kind.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct AskQuestion {
    pub(crate) id: String,
    pub(crate) kind: AskKind,
}

/// The normalized, validated ask — and the trace's input-commitment
/// pre-image. The query text rides ONLY as `query_digest`: query-adjacent
/// free text is hashed, never stored raw.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct NormalizedAsk {
    pub(crate) request_id: String,
    pub(crate) question: Option<AskQuestion>,
    pub(crate) question_ids: Vec<String>,
    pub(crate) query_digest: String,
}

/// The accumulated state threaded through the stages. Everything is
/// recorded-shape (refs + digests + typed outputs) — serializable, so each
/// stage's input digest is the canonical hash of exactly what it saw.
#[derive(Debug, Clone, PartialEq, Default, Deserialize, Serialize)]
pub(crate) struct StageInput {
    pub(crate) ask: Option<NormalizedAsk>,
    pub(crate) hits: Vec<ContextHit>,
    pub(crate) candidates: Vec<EvidenceCandidate>,
    pub(crate) decision: Option<SerdeDecisionOutput>,
    pub(crate) policy: Option<PolicyOutcome>,
    pub(crate) verdict: Option<ActionVerdict>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub(crate) enum StageOutput {
    Normalized(NormalizedAsk),
    Retrieved(Vec<ContextHit>),
    Candidates(Vec<EvidenceCandidate>),
    Decision(SerdeDecisionOutput),
    Policy(PolicyOutcome),
    Reordered(Vec<EvidenceCandidate>),
    Verdict(ActionVerdict),
    Action(ActionRecord),
}

/// The provenance-bearing record every stage emits — the trace's stage
/// shape, verbatim. Timing comes from the INJECTED clock argument, never
/// a clock read.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub(crate) struct StageRecord {
    pub(crate) stage: StageName,
    pub(crate) algorithm: String,
    pub(crate) config_hash: String,
    pub(crate) inputs_digest: String,
    pub(crate) outputs_digest: String,
    pub(crate) outputs: StageOutput,
    pub(crate) trust_tiers_seen: Vec<SerdeTier>,
    pub(crate) started_at: i64,
    pub(crate) duration_ns: u64,
}

/// The run's request — everything the engine needs that is NOT config.
/// `query` is the one raw-text field: it goes to the retrieval seam's
/// argument and nowhere else (the trace carries its digest).
#[derive(Debug, Clone)]
pub(crate) struct DecisionRunRequest<'a> {
    pub(crate) run_id: i64,
    pub(crate) mode: RunMode,
    pub(crate) role_scope: Vec<String>,
    /// The context's creation instant (unix seconds; time as argument).
    pub(crate) created_at: i64,
    pub(crate) request_id: String,
    pub(crate) question: Option<AskQuestion>,
    pub(crate) question_ids: Vec<String>,
    pub(crate) query: &'a str,
}

/// The registry identity attached to a trace when the binding resolved
/// through the governed model registry. The optional field keeps older
/// stored trace documents readable without a migration.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct RegistryRef {
    pub(crate) registry_id: String,
    pub(crate) registry_version: String,
}

/// The model identity the trace records (the seam's own metadata).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct ModelRefRecord {
    pub(crate) id: String,
    pub(crate) version: String,
    pub(crate) weights_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) registry_ref: Option<RegistryRef>,
}

impl ModelRefRecord {
    fn of(meta: &ModelMetadata) -> Self {
        ModelRefRecord {
            id: meta.id().to_string(),
            version: meta.version().to_string(),
            weights_digest: meta.weights_digest().map(str::to_string),
            registry_ref: None,
        }
    }
}

/// The run's total outcome: the action label, the typed escalation (Some
/// exactly when the action is escalate), the proposed decision output
/// (Some whenever the model decided — even on escalate: the trace shows
/// what was proposed AND that it was escalated), the per-stage records,
/// the input commitment, and the retrieved context refs.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DecisionRunResult {
    pub(crate) run_id: i64,
    pub(crate) input_digest: String,
    pub(crate) context_hits: Vec<ContextHit>,
    pub(crate) action: ActionLabel,
    pub(crate) escalation: Option<EscalationRecord>,
    pub(crate) output: Option<SerdeDecisionOutput>,
    pub(crate) model: ModelRefRecord,
    pub(crate) records: Vec<StageRecord>,
}

// ── the pure stages ────────────────────────────────────────────────────

fn normalize_stage(input: &StageInput) -> Result<StageOutput, StageRefusal> {
    let Some(ask) = &input.ask else {
        return Err(StageRefusal::named(
            StageName::Normalize,
            EscalationReason::InvalidInput,
        ));
    };
    if ask.request_id.trim().is_empty() {
        return Err(StageRefusal::named(
            StageName::Normalize,
            EscalationReason::InvalidInput,
        ));
    }
    if ask.question_ids.is_empty() || ask.question_ids.iter().any(|q| q.trim().is_empty()) {
        return Err(StageRefusal::named(
            StageName::Normalize,
            EscalationReason::InvalidInput,
        ));
    }
    if let Some(q) = &ask.question
        && (q.id.trim().is_empty() || !ask.question_ids.contains(&q.id))
    {
        return Err(StageRefusal::named(
            StageName::Normalize,
            EscalationReason::InvalidInput,
        ));
    }
    Ok(StageOutput::Normalized(ask.clone()))
}

fn retrieve_stage(
    _input: &StageInput,
    cfg: &DecisionPipelineConfig,
    retriever: &dyn ContextRetriever,
    query: &str,
) -> Result<StageOutput, StageRefusal> {
    match retriever.retrieve(query, &cfg.retrieval) {
        Ok(mut hits) => {
            // The seam's own clamp re-asserted: the fused set never exceeds
            // the configured limit.
            hits.truncate(cfg.retrieval.limit as usize);
            Ok(StageOutput::Retrieved(hits))
        }
        Err(_) => Err(StageRefusal::named(
            StageName::RetrieveContext,
            EscalationReason::RetrievalUnavailable,
        )),
    }
}

fn candidates_stage(input: &StageInput) -> Result<StageOutput, StageRefusal> {
    if input.hits.is_empty() {
        return Err(StageRefusal::named(
            StageName::CandidateGeneration,
            EscalationReason::InsufficientEvidence,
        ));
    }
    // Screen-tripped (flagged) hits never become evidence; everything else
    // carries its tier as-is — the policy stage judges the tiers.
    let candidates: Vec<EvidenceCandidate> = input
        .hits
        .iter()
        .filter(|h| !h.flagged)
        .map(|h| EvidenceCandidate {
            evidence_id: h.evidence_id.clone(),
            tier: h.tier,
        })
        .collect();
    if candidates.is_empty() {
        return Err(StageRefusal::named(
            StageName::CandidateGeneration,
            EscalationReason::InsufficientEvidence,
        ));
    }
    Ok(StageOutput::Candidates(candidates))
}

fn model_stage(
    input: &StageInput,
    cfg: &DecisionPipelineConfig,
    model: &dyn DecisionModel,
    req: &DecisionRunRequest,
) -> Result<StageOutput, StageRefusal> {
    let Some(ask) = &input.ask else {
        return Err(StageRefusal::named(
            StageName::DecisionModel,
            EscalationReason::MissingDecision,
        ));
    };
    let decision_input = DecisionInput {
        request_id: ask.request_id.clone(),
        question: ask.question.as_ref().map(|q| QuestionRef {
            id: q.id.clone(),
            kind: q.kind.question_kind(),
        }),
        question_ids: ask.question_ids.clone(),
        evidence: input
            .candidates
            .iter()
            .map(|c| c.evidence_ref())
            .collect::<Vec<EvidenceRef>>(),
    };
    let ctx = DecisionContext {
        run_id: req.run_id,
        mode: req.mode,
        config_digests: vec![ConfigRef {
            key: cfg.model.key.clone(),
            digest: cfg.model.digest.clone(),
        }],
        role_scope: req.role_scope.clone(),
        created_at: req.created_at,
    };
    model
        .evaluate(&decision_input, &ctx)
        .map(|out| StageOutput::Decision(SerdeDecisionOutput::of(&out)))
        .map_err(|e| StageRefusal {
            stage: StageName::DecisionModel,
            reason: EscalationReason::from_decision_error(&e),
            detail: e.to_string(),
        })
}

fn policy_stage(input: &StageInput) -> Result<StageOutput, StageRefusal> {
    let Some(decision) = &input.decision else {
        return Err(StageRefusal::named(
            StageName::RulesPolicy,
            EscalationReason::MissingDecision,
        ));
    };
    // Policy law v0: evidence whose tiers are ALL untrusted escalates to a
    // human. The rule table's own tier law ran one stage earlier (the
    // model's evaluation); this is the pipeline-level fence.
    let all_untrusted = !decision.evidence_refs.is_empty()
        && decision
            .evidence_refs
            .iter()
            .all(|r| r.tier == SerdeTier::Untrusted);
    let outcome = if all_untrusted {
        PolicyOutcome::Escalate {
            reason: EscalationReason::PolicyUntrustedOnly,
        }
    } else {
        PolicyOutcome::Clear
    };
    Ok(StageOutput::Policy(outcome))
}

fn rerank_stage(input: &StageInput) -> Result<StageOutput, StageRefusal> {
    // The deterministic re-rank leg: a stable re-order of the candidate
    // list, most-trusted first, original order preserved within a tier. No
    // learned reranker exists on this path.
    let mut reordered = input.candidates.clone();
    reordered.sort_by_key(|c| std::cmp::Reverse(c.tier));
    Ok(StageOutput::Reordered(reordered))
}

fn threshold_stage(
    input: &StageInput,
    cfg: &DecisionPipelineConfig,
) -> Result<StageOutput, StageRefusal> {
    let Some(decision) = &input.decision else {
        return Err(StageRefusal::named(
            StageName::Threshold,
            EscalationReason::MissingDecision,
        ));
    };
    let Some(policy) = &input.policy else {
        return Err(StageRefusal::named(
            StageName::Threshold,
            EscalationReason::MissingDecision,
        ));
    };
    // A policy escalation escalates — the threshold can never downgrade
    // policy. The escalation provenance names the stage that demanded the
    // human and the closed reason.
    if let PolicyOutcome::Escalate { reason } = policy {
        return Ok(StageOutput::Verdict(ActionVerdict::Escalate {
            origin: StageName::RulesPolicy,
            reason: *reason,
        }));
    }
    let t = &cfg.thresholds;
    let fallback_verdict = || match t.fallback {
        ThresholdFallback::Approve => ActionVerdict::Approve,
        ThresholdFallback::Escalate => ActionVerdict::Escalate {
            origin: StageName::Threshold,
            reason: EscalationReason::FallbackEscalate,
        },
    };
    let verdict = match &decision.value {
        SerdeDecisionValue::Choice { label, .. } => {
            if t.act_labels.iter().any(|l| l == label) {
                ActionVerdict::Act
            } else if t.reject_labels.iter().any(|l| l == label) {
                ActionVerdict::Reject
            } else {
                fallback_verdict()
            }
        }
        SerdeDecisionValue::Score { value, .. } => {
            if *value >= t.score_act_at_or_above {
                ActionVerdict::Act
            } else if *value <= t.score_reject_at_or_below {
                ActionVerdict::Reject
            } else {
                fallback_verdict()
            }
        }
        SerdeDecisionValue::Noul { value } => {
            if *value == t.noul_act_when {
                ActionVerdict::Act
            } else {
                ActionVerdict::Reject
            }
        }
    };
    Ok(StageOutput::Verdict(verdict))
}

fn action_stage(input: &StageInput) -> Result<StageOutput, StageRefusal> {
    let record = match &input.verdict {
        Some(ActionVerdict::Escalate { origin, reason }) => ActionRecord {
            action: ActionLabel::Escalate,
            escalation: Some(EscalationRecord {
                stage: *origin,
                reason: *reason,
                detail: reason.detail().to_string(),
            }),
        },
        Some(ActionVerdict::Act) => ActionRecord {
            action: ActionLabel::Act,
            escalation: None,
        },
        Some(ActionVerdict::Approve) => ActionRecord {
            action: ActionLabel::Approve,
            escalation: None,
        },
        Some(ActionVerdict::Reject) => ActionRecord {
            action: ActionLabel::Reject,
            escalation: None,
        },
        // Unreachable while the config validator holds (action_escalation is
        // declared after threshold); honest escalation, never a panic.
        None => ActionRecord {
            action: ActionLabel::Escalate,
            escalation: Some(EscalationRecord::at(
                StageName::Threshold,
                EscalationReason::MissingDecision,
            )),
        },
    };
    Ok(StageOutput::Action(record))
}

// ── the runner ─────────────────────────────────────────────────────────

fn tiers_seen(input: &StageInput) -> Vec<SerdeTier> {
    let mut tiers = std::collections::BTreeSet::new();
    for h in &input.hits {
        tiers.insert(h.tier);
    }
    for c in &input.candidates {
        tiers.insert(c.tier);
    }
    if let Some(d) = &input.decision {
        for r in &d.evidence_refs {
            tiers.insert(r.tier);
        }
    }
    tiers.into_iter().collect()
}

fn algorithm_label(
    stage: StageName,
    retriever: &dyn ContextRetriever,
    model_meta: &ModelMetadata,
) -> String {
    match stage {
        StageName::Normalize => "harness.normalize/v1".to_string(),
        StageName::RetrieveContext => retriever.algorithm().to_string(),
        StageName::CandidateGeneration => "harness.candidates/v1".to_string(),
        StageName::DecisionModel => format!("model:{}:{}", model_meta.id(), model_meta.version()),
        StageName::RulesPolicy => "policy.untrusted_only/v0".to_string(),
        StageName::Rerank => "harness.tier_stable_reorder/v1".to_string(),
        StageName::Threshold => "threshold/v0".to_string(),
        StageName::ActionEscalation => "action/v0".to_string(),
    }
}

/// Run one decision pipeline: the config's declared stages, in order, each
/// pure, each digested. Total — stage refusals fold into the returned
/// outcome as honest escalation data. `clock` is the INJECTED timing
/// source: nanosecond readings, monotone non-decreasing, caller's choice
/// of epoch (tests pin a fixed clock for full determinism).
pub(crate) fn run_decision_pipeline(
    loaded: &LoadedPipelineConfig,
    req: &DecisionRunRequest,
    model: &dyn DecisionModel,
    retriever: &dyn ContextRetriever,
    clock: &dyn Fn() -> i64,
) -> DecisionRunResult {
    let model_meta = model.metadata();

    // The input commitment: computed before any stage, so a run that
    // refuses at normalize still binds its input. The raw query text rides
    // only as its digest.
    let commitment = NormalizedAsk {
        request_id: req.request_id.clone(),
        question: req.question.clone(),
        question_ids: req.question_ids.clone(),
        query_digest: sha256_hex(req.query.as_bytes()),
    };
    let input_digest = sha256_of_json(&commitment);

    let mut state = StageInput {
        ask: Some(commitment),
        ..StageInput::default()
    };
    let mut records: Vec<StageRecord> = Vec::new();
    let mut context_hits: Vec<ContextHit> = Vec::new();
    let mut decision_out: Option<SerdeDecisionOutput> = None;
    let mut action_out: Option<ActionRecord> = None;
    let mut refusal: Option<StageRefusal> = None;

    for stage in loaded.config.stages.iter().copied() {
        let started_at = clock();
        let inputs_digest = sha256_of_json(&state);
        let tiers = tiers_seen(&state);
        let outcome = match stage {
            StageName::Normalize => normalize_stage(&state),
            StageName::RetrieveContext => {
                retrieve_stage(&state, &loaded.config, retriever, req.query)
            }
            StageName::CandidateGeneration => candidates_stage(&state),
            StageName::DecisionModel => model_stage(&state, &loaded.config, model, req),
            StageName::RulesPolicy => policy_stage(&state),
            StageName::Rerank => rerank_stage(&state),
            StageName::Threshold => threshold_stage(&state, &loaded.config),
            StageName::ActionEscalation => action_stage(&state),
        };
        let duration_ns = (clock() - started_at).max(0) as u64;
        match outcome {
            Ok(output) => {
                match &output {
                    StageOutput::Normalized(ask) => state.ask = Some(ask.clone()),
                    StageOutput::Retrieved(hits) => {
                        context_hits = hits.clone();
                        state.hits = hits.clone();
                    }
                    StageOutput::Candidates(c) => state.candidates = c.clone(),
                    StageOutput::Decision(d) => {
                        decision_out = Some(d.clone());
                        state.decision = Some(d.clone());
                    }
                    StageOutput::Policy(p) => state.policy = Some(p.clone()),
                    StageOutput::Reordered(c) => state.candidates = c.clone(),
                    StageOutput::Verdict(v) => state.verdict = Some(v.clone()),
                    StageOutput::Action(a) => action_out = Some(a.clone()),
                }
                records.push(StageRecord {
                    stage,
                    algorithm: algorithm_label(stage, retriever, &model_meta),
                    config_hash: loaded.config_hash.clone(),
                    inputs_digest,
                    outputs_digest: sha256_of_json(&output),
                    outputs: output,
                    trust_tiers_seen: tiers,
                    started_at,
                    duration_ns,
                });
            }
            Err(r) => {
                refusal = Some(r);
                break;
            }
        }
    }

    if let Some(r) = refusal {
        return DecisionRunResult {
            run_id: req.run_id,
            input_digest,
            context_hits,
            action: ActionLabel::Escalate,
            escalation: Some(EscalationRecord {
                stage: r.stage,
                reason: r.reason,
                detail: r.detail,
            }),
            output: decision_out,
            model: ModelRefRecord::of(&model_meta),
            records,
        };
    }

    // Every declared stage ran; the validator guarantees the last was the
    // action stage, so its record exists. The None arm is honest escalation
    // data, never a panic.
    let (action, escalation) = action_out.map_or_else(
        || {
            (
                ActionLabel::Escalate,
                Some(EscalationRecord::at(
                    StageName::ActionEscalation,
                    EscalationReason::MissingDecision,
                )),
            )
        },
        |ActionRecord { action, escalation }| (action, escalation),
    );
    DecisionRunResult {
        run_id: req.run_id,
        input_digest,
        context_hits,
        action,
        escalation,
        output: decision_out,
        model: ModelRefRecord::of(&model_meta),
        records,
    }
}

#[cfg(test)]
mod tests {
    use super::super::config::RetrievalParams;
    use super::*;
    use crate::workflow::harness::models::rules::RulesModel;

    /// The reference rules model, loaded once per call (the R26 model,
    /// consumed as shipped — never rebuilt).
    struct ReferenceModel {
        model: RulesModel,
    }

    impl ReferenceModel {
        fn from_table(raw: &str) -> Self {
            ReferenceModel {
                model: RulesModel::from_canonical_json(raw).expect("the reference table loads"),
            }
        }

        fn digest(&self) -> &str {
            self.model.digest()
        }

        fn as_model(&self) -> &dyn DecisionModel {
            &self.model
        }
    }

    /// A fixed stepping clock: nanosecond readings advancing by a constant
    /// step — the injected-clock law, so a run's timing is deterministic
    /// and test-pinned (no stage reads a real clock).
    fn stepping_clock() -> impl Fn() -> i64 {
        let cell = std::cell::Cell::new(1_000);
        move || {
            let now = cell.get();
            cell.set(now + 250);
            now
        }
    }

    const DIGEST_A: &str = "aa0000000000000000000000000000000000000000000000000000000000000a";
    const DIGEST_B: &str = "bb00000000000000000000000000000000000000000000000000000000000000b";
    const QUERY: &str = "find the governing policy for this ask";

    const TABLE_JSON: &str = r#"{
      "model_id": "rules-reference",
      "model_version": "1.0.0",
      "rules": [
        { "question_id": "needs_human", "min_evidence": 2, "min_tier": "vetted",
          "output": { "Choice": { "options": ["act", "reject"], "label": "act" } } },
        { "question_id": "risk", "min_evidence": 1, "min_tier": "governed",
          "output": { "Score": { "range": [0, 100], "value": 42 } } },
        { "question_id": "untrusted_ok", "min_evidence": 1, "min_tier": "untrusted",
          "output": { "Choice": { "options": ["act", "reject"], "label": "act" } } }
      ]
    }"#;

    fn config_json(model_digest: &str) -> String {
        format!(
            r#"{{
              "config_schema": "harness.pipeline/v1",
              "pipeline_id": "reference-line",
              "stages": ["normalize","retrieve_context","candidate_generation","decision_model","rules_policy","threshold","action_escalation"],
              "retrieval": {{"rrf_k": 60, "limit": 20, "leg": "both"}},
              "model": {{"key": "rules:rules-reference", "digest": "{model_digest}"}},
              "thresholds": {{
                "act_labels": ["act"],
                "reject_labels": ["reject"],
                "score_act_at_or_above": 50,
                "score_reject_at_or_below": 10,
                "noul_act_when": true,
                "fallback": "approve"
              }}
            }}"#
        )
    }

    fn loaded_config(model_digest: &str) -> LoadedPipelineConfig {
        DecisionPipelineConfig::load(&config_json(model_digest))
            .expect("the reference config loads")
    }

    fn hit(id: i64, digest: &str, tier: TrustTier) -> DeclaredHit {
        DeclaredHit {
            id,
            digest: digest.to_string(),
            tier,
            untrusted: tier == TrustTier::Untrusted,
            flagged: false,
        }
    }

    fn governed_hit(id: i64, digest: &str) -> DeclaredHit {
        hit(id, digest, TrustTier::Governed)
    }

    fn flagged_hit(id: i64, digest: &str) -> DeclaredHit {
        let mut h = governed_hit(id, digest);
        h.flagged = true;
        h
    }

    fn retriever(hits: Vec<DeclaredHit>) -> DeclaredListRetriever {
        DeclaredListRetriever {
            vector: hits,
            fts: Vec::new(),
            graph: Vec::new(),
        }
    }

    fn request() -> DecisionRunRequest<'static> {
        DecisionRunRequest {
            run_id: 77,
            mode: RunMode::Deterministic,
            role_scope: vec!["operator".into()],
            created_at: 1_800_000_123,
            request_id: "req-9".into(),
            question: Some(AskQuestion {
                id: "needs_human".into(),
                kind: AskKind::Choice,
            }),
            question_ids: vec!["needs_human".into()],
            query: QUERY,
        }
    }

    fn candidates_of(result: &DecisionRunResult) -> Vec<EvidenceCandidate> {
        result
            .records
            .iter()
            .find_map(|r| match &r.outputs {
                StageOutput::Candidates(c) => Some(c.clone()),
                _ => None,
            })
            .expect("the candidate stage produced a record")
    }

    /// One StageRecord per DECLARED stage, in canonical order, each with
    /// digests + tiers + timing from the INJECTED clock — and a second
    /// identical run reproduces every record byte-for-byte (the
    /// fixed-clock determinism pin).
    #[test]
    fn decision_run_trace_carries_every_stage_record() {
        let model = ReferenceModel::from_table(TABLE_JSON);
        let loaded = loaded_config(model.digest());
        let retr = retriever(vec![governed_hit(11, DIGEST_A), governed_hit(12, DIGEST_B)]);
        let first = run_decision_pipeline(
            &loaded,
            &request(),
            model.as_model(),
            &retr,
            &stepping_clock(),
        );

        assert_eq!(first.action, ActionLabel::Act);
        assert!(first.escalation.is_none());
        let declared = &loaded.config.stages;
        assert_eq!(first.records.len(), declared.len());
        for (record, stage) in first.records.iter().zip(declared.iter()) {
            assert_eq!(record.stage, *stage, "records follow the declared order");
            assert_eq!(record.config_hash, loaded.config_hash);
            assert!(!record.algorithm.is_empty(), "the algorithm is recorded");
            assert_eq!(record.inputs_digest.len(), 64);
            assert_eq!(record.outputs_digest.len(), 64);
            assert!(record.duration_ns > 0, "the stepped clock shows in timing");
        }
        // The canonical order, verbatim (no re-rank leg in this config).
        let names: Vec<StageName> = first.records.iter().map(|r| r.stage).collect();
        assert_eq!(
            names,
            vec![
                StageName::Normalize,
                StageName::RetrieveContext,
                StageName::CandidateGeneration,
                StageName::DecisionModel,
                StageName::RulesPolicy,
                StageName::Threshold,
                StageName::ActionEscalation,
            ]
        );
        // The tiers law is input-based: normalize and retrieve see no
        // evidence yet (the hits ARRIVE as the retrieve output); the
        // candidate stage's input carries the governed hits.
        assert!(first.records[0].trust_tiers_seen.is_empty());
        assert!(first.records[1].trust_tiers_seen.is_empty());
        assert_eq!(first.records[2].trust_tiers_seen, vec![SerdeTier::Governed]);
        // Timing is strictly increasing along the run.
        for pair in first.records.windows(2) {
            assert!(pair[1].started_at > pair[0].started_at);
        }
        // The model's proposal rode to the end (proposes-not-disposes is
        // visible in the outcome), produced_at is the context's instant,
        // and the context refs carry digests, never text.
        let decision = first.output.expect("the model decided");
        assert_eq!(decision.model_id, "rules-reference");
        assert_eq!(decision.produced_at, 1_800_000_123);
        assert_eq!(first.context_hits.len(), 2);
        assert_eq!(first.context_hits[0].evidence_id, "11");
        assert_eq!(first.context_hits[0].content_digest, DIGEST_A);

        // Determinism: the same inputs + the same fixed clock reproduce the
        // records exactly (digests and timing included).
        let second = run_decision_pipeline(
            &loaded,
            &request(),
            model.as_model(),
            &retr,
            &stepping_clock(),
        );
        assert_eq!(first.records, second.records);
        assert_eq!(first.input_digest, second.input_digest);
    }

    /// A refusal at ANY stage escalates as honest data: the typed record
    /// names the refusing stage and the closed reason, the records-so-far
    /// are preserved, and NO output is ever synthesized. Pinned at three
    /// stages: normalize (invalid input), retrieval (unavailable seam), and
    /// the model (insufficient evidence).
    #[test]
    fn refusal_at_any_stage_escalates_never_guesses() {
        let model = ReferenceModel::from_table(TABLE_JSON);
        let loaded = loaded_config(model.digest());

        // Normalize refuses an empty request id; nothing else runs.
        let mut bad_ask = request();
        bad_ask.request_id = String::new();
        let out = run_decision_pipeline(
            &loaded,
            &bad_ask,
            model.as_model(),
            &retriever(vec![governed_hit(11, DIGEST_A)]),
            &stepping_clock(),
        );
        assert_eq!(out.action, ActionLabel::Escalate);
        let esc = out.escalation.as_ref().expect("typed escalation");
        assert_eq!(esc.stage, StageName::Normalize);
        assert_eq!(esc.reason, EscalationReason::InvalidInput);
        assert!(out.records.is_empty());
        assert!(out.output.is_none(), "never a synthesized output");

        // The unavailable seam refuses at retrieve_context; the normalize
        // record survives as provenance of how far the run got.
        struct DownRetriever;
        impl ContextRetriever for DownRetriever {
            fn retrieve(
                &self,
                _query: &str,
                _params: &RetrievalParams,
            ) -> Result<Vec<ContextHit>, RetrievalRefusal> {
                Err(RetrievalRefusal::Unavailable)
            }
            fn algorithm(&self) -> &str {
                "test.down"
            }
        }
        let out = run_decision_pipeline(
            &loaded,
            &request(),
            model.as_model(),
            &DownRetriever,
            &stepping_clock(),
        );
        let esc = out.escalation.as_ref().expect("typed escalation");
        assert_eq!(esc.stage, StageName::RetrieveContext);
        assert_eq!(esc.reason, EscalationReason::RetrievalUnavailable);
        assert_eq!(out.records.len(), 1);
        assert!(out.output.is_none());

        // The model's own honest refusal: the rule needs two refs at the
        // vetted floor, one arrives — InsufficientEvidenceTier at
        // decision_model (evidence arrived, not enough at the tier), and
        // the stages after it never ran.
        let out = run_decision_pipeline(
            &loaded,
            &request(),
            model.as_model(),
            &retriever(vec![governed_hit(11, DIGEST_A)]),
            &stepping_clock(),
        );
        let esc = out.escalation.as_ref().expect("typed escalation");
        assert_eq!(esc.stage, StageName::DecisionModel);
        assert_eq!(esc.reason, EscalationReason::InsufficientEvidenceTier);
        assert_eq!(
            esc.detail,
            "decision refused: the evidence does not meet the required trust tier"
        );
        assert_eq!(out.records.len(), 3);
        assert!(out.output.is_none());
        assert_eq!(out.action, ActionLabel::Escalate);
    }

    /// Policy v0: evidence whose tiers are ALL untrusted escalates to a
    /// human — even though the rules model itself (min_tier untrusted)
    /// would answer. The proposal still rides in the outcome (proposes,
    /// never disposes). Mixed tiers clear. Flagged (screen-tripped) hits
    /// never become candidates.
    #[test]
    fn untrusted_only_evidence_escalates() {
        let model = ReferenceModel::from_table(TABLE_JSON);
        let loaded = loaded_config(model.digest());

        // All-untrusted evidence: the model answers "act", the policy law
        // escalates, and the outcome carries BOTH facts.
        let mut ask = request();
        ask.question = Some(AskQuestion {
            id: "untrusted_ok".into(),
            kind: AskKind::Choice,
        });
        ask.question_ids = vec!["untrusted_ok".into()];
        let out = run_decision_pipeline(
            &loaded,
            &ask,
            model.as_model(),
            &retriever(vec![
                hit(21, DIGEST_A, TrustTier::Untrusted),
                hit(22, DIGEST_B, TrustTier::Untrusted),
            ]),
            &stepping_clock(),
        );
        assert_eq!(out.action, ActionLabel::Escalate);
        let esc = out.escalation.as_ref().expect("the policy escalation");
        assert_eq!(esc.stage, StageName::RulesPolicy);
        assert_eq!(esc.reason, EscalationReason::PolicyUntrustedOnly);
        let decision = out.output.as_ref().expect("the proposal still rides");
        assert_eq!(decision.model_id, "rules-reference");
        assert_eq!(
            decision.evidence_refs.len(),
            2,
            "the model consumed the untrusted refs"
        );
        // The threshold record carries the verdict with its provenance.
        let threshold = out
            .records
            .iter()
            .find(|r| r.stage == StageName::Threshold)
            .expect("the threshold ran");
        match &threshold.outputs {
            StageOutput::Verdict(ActionVerdict::Escalate { origin, reason }) => {
                assert_eq!(
                    (*origin, *reason),
                    (
                        StageName::RulesPolicy,
                        EscalationReason::PolicyUntrustedOnly
                    )
                );
            }
            other => panic!("the threshold escalated: {other:?}"),
        }

        // Mixed tiers clear the policy law and the ask acts.
        let out = run_decision_pipeline(
            &loaded,
            &ask,
            model.as_model(),
            &retriever(vec![
                hit(21, DIGEST_A, TrustTier::Untrusted),
                hit(22, DIGEST_B, TrustTier::Vetted),
            ]),
            &stepping_clock(),
        );
        assert_eq!(out.action, ActionLabel::Act);
        assert!(out.escalation.is_none());

        // Flagged (screen-tripped) hits never become candidates: a flagged
        // governed hit plus a real governed hit leaves exactly one usable
        // candidate, and the record proves the flagged id is absent.
        let mut risk_ask = request();
        risk_ask.question = Some(AskQuestion {
            id: "risk".into(),
            kind: AskKind::Score,
        });
        risk_ask.question_ids = vec!["risk".into()];
        let out = run_decision_pipeline(
            &loaded,
            &risk_ask,
            model.as_model(),
            &retriever(vec![flagged_hit(31, DIGEST_A), governed_hit(32, DIGEST_B)]),
            &stepping_clock(),
        );
        assert!(out.escalation.is_none());
        let candidates = candidates_of(&out);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].evidence_id, "32");

        // And a run whose ONLY hit is flagged has no evidence at all — the
        // honest insufficient-evidence escalation.
        let out = run_decision_pipeline(
            &loaded,
            &risk_ask,
            model.as_model(),
            &retriever(vec![flagged_hit(31, DIGEST_A)]),
            &stepping_clock(),
        );
        let esc = out.escalation.as_ref().expect("typed escalation");
        assert_eq!(esc.stage, StageName::CandidateGeneration);
        assert_eq!(esc.reason, EscalationReason::InsufficientEvidence);
    }
}
