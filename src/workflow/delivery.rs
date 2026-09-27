//! The delivery loop's persistence and run lifecycle.
//!
//! Four writes, one `WorkflowTx` each, and the phase law borrowed — never
//! re-derived — from the pure core. `workflow_runs` (kind = `delivery`),
//! `workflow_steps`, and the four normative routing keys are REUSED with no
//! migration; the two new tables carry what those could not.
//!
//! What this module deliberately does not know. It does not decide whether an
//! artifact may be PROMOTED — that is a pure function over an attestation
//! chain, a digest-bound approval, and a budget headroom, none of which exist
//! yet. It does not enforce a budget: `delivery_budgets` rows are STORED, and
//! the enforcement round is the one that reads them. It does not sign, verify
//! a signature, or attest to authorship; a digest is not a signature. And it
//! never writes `law_version` — a delivery run has no jurisdiction, and the
//! report layer documents the empty stamp as "advisory unavailable", never a
//! refusal.
//!
//! The storage boundary is a `WorkflowTx`: the step row, the CAS, the trace
//! row, and the fail-closed audit are one transition or none of them are. The
//! audit is the LAST statement before the commit, so a phase pass can never
//! land without its evidence.

#![deny(unsafe_code)]

use brain_delivery_core::{
    AutonomyTier, Phase, StageDiff, StageDigest, StageMismatch, is_legal_phase_transition,
    terminal_phase, trace_mode_for_tier,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::audit::{AuditKind, AuditStatus};
use crate::workflow::session_log::SessionEventRow;
use crate::workflow::state;

/// The run kind. ONE run engine — the delivery loop rides the existing
/// `workflow_runs` table with a free-TEXT `kind`, and never a second engine.
pub(crate) const RUN_KIND: &str = "delivery";

/// The engine identity stamped on every trace row. This is the engine, not
/// the schema: the schema stamp moves to 1.32.15 and this does not.
pub(crate) const PIPELINE_VERSION: &str = crate::workflow::harness::PIPELINE_VERSION;

/// The closed run-status set (the engine surface, untouched). A phase pass
/// keeps a live run `active` and closes it `completed` at the terminal phase;
/// it never invents a status and never writes one outside a CAS.
const STATUS_ACTIVE: &str = "active";
const STATUS_COMPLETED: &str = "completed";

/// Bounded caller text. The answer is operator-authored prose on a run the
/// operator owns; it is stored verbatim in the run's own state, bounded here,
/// and never copied into a trace row.
const MAX_ANSWER_CHARS: usize = 2000;
const MAX_GOAL_CHARS: usize = 2000;
const MAX_BUDGETS: usize = 5;
const MAX_REFS: usize = 32;
/// The typed artifact's own caps. An artifact body is a proposal's content, so
/// it is bounded like every other bounded caller text — and the caps are named
/// so the refusal names them.
const MAX_ARTIFACT_ID_CHARS: usize = 128;
const MAX_ARTIFACT_CHARS: usize = 8000;
const MAX_GATE_CHARS: usize = 8000;
/// the attestation round: the presented model's own caps. A key and a config digest are caller
/// text at this boundary, so they are bounded like every other bounded caller
/// text — and the caps are named so the refusal names them.
const MAX_MODEL_KEY_CHARS: usize = 128;
const MAX_DIGEST_INPUT_CHARS: usize = 128;
/// The subject name's bounded artifact suffix (A4). The name is signed, so its
/// length is a property of the signature's domain, not a display choice.
const MAX_SUBJECT_SUFFIX_CHARS: usize = 64;

/// The delivery loop's typed-artifact proposal kind. `proposals.kind` is free
/// text with no CHECK and no global closed vocabulary, so a new kind is
/// admissible at the schema level with no migration — and it is deliberately
/// NOT one of the nine kinds `POST /propose` accepts, because an executor
/// artifact is not operator-authored knowledge and never becomes a knowledge
/// row. It stays a proposal or it does not exist.
const ARTIFACT_PROPOSAL_KIND: &str = "delivery/artifact";

/// The `ddl_*` session-log family — the delivery trace's narrative, exactly as
/// the design owner describes it ("the `agent_session_events` `ddl_*`
/// narrative"). It is NOT the reserved `control:` family, so these rows are
/// visible to `replay` and to the context projection; that visibility is
/// intended, because the narrative is what a replaying agent reads.
///
/// There is deliberately NO `ddl_artifact_refused` kind. A gate refusal is
/// raised BEFORE the transaction writes anything, so it leaves no residue to
/// narrate — the refusal is the error, and the fail-closed audit of the
/// *passing* transaction is the evidence. Naming a kind nothing can emit would
/// be the same validated-but-dropped vocabulary the executor core just shed.
pub(crate) const DDL_ARTIFACT_KIND: &str = "ddl_artifact";

/// The typed artifact the phase pass may carry. A REUSED shape, not a new
/// type: [`Self::typed`] builds the shipped
/// [`brain_consensus_core::Artifact`], whose `hash` field is the same
/// `sha256(content)` the shipped [`brain_executor_core::artifact_hash`]
/// computes, so the id recorded in the audit and the digest an approver
/// computes are the same digest of the same bytes.
///
/// `quality_gate` is the checkpoint gate, validated by the shipped
/// [`brain_executor_core::validate_gate_json`] rather than reimplemented here.
/// It is consulted on the `build` phase pass; on every other phase it is
/// carried and ignored, because a scope or design artifact has no QA evidence
/// to offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeliveryArtifact {
    pub id: String,
    pub content: String,
    pub quality_gate: Option<String>,
}

impl DeliveryArtifact {
    /// The shipped typed artifact, with its content digest derived.
    pub(crate) fn typed(&self) -> brain_consensus_core::Artifact {
        brain_consensus_core::Artifact::new(&self.id, &self.content)
    }

    /// Run the shipped QA gate. The refusal is the executor's own vocabulary,
    /// carried verbatim so the caller learns which law refused.
    fn quality_gate(&self) -> Result<(), String> {
        let Some(raw) = self.quality_gate.as_deref() else {
            return Ok(());
        };
        brain_executor_core::validate_gate_json(raw).map(|_| ())
    }
}

// ── the persisted run state ────────────────────────────────────────────────

/// The run's `state_json`. Opaque engine-owned state: the phase, the tier, the
/// attempt count, and the pending question.
///
/// `law_version` is DELIBERATELY ABSENT and must stay so — the engines CAS
/// against these exact bytes, so a law stamp placed here would be a stamp
/// nothing reads and a byte sequence the CAS compares against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DeliveryState {
    pub phase: String,
    pub tier: String,
    pub attempt: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_question: Option<String>,
}

impl DeliveryState {
    fn fresh(tier: AutonomyTier) -> Self {
        Self {
            phase: Phase::Scope.as_str().to_string(),
            tier: tier_core_to_wire(tier).to_string(),
            attempt: 0,
            pending_question: None,
        }
    }
}

// ── the closed error vocabulary ────────────────────────────────────────────

/// Every refusal a delivery core can return. Typed, so the handler maps rather
/// than guesses, and closed, so a caller cannot learn a new failure mode from
/// a new error.
#[derive(Debug)]
pub(crate) enum DeliveryError {
    /// A value outside a closed vocabulary, carried verbatim for the refusal.
    UnknownVocabulary {
        field: &'static str,
        value: String,
    },
    /// The run is absent, is not a delivery run, or is not the caller's — the
    /// handler collapses all three into one probe-blind answer.
    RunAbsent,
    /// The caller lost the CAS: another writer moved the revision first.
    Stale {
        actual_revision: i64,
    },
    /// The requested phase move is not in the forward-only machine.
    IllegalPhaseTransition {
        from: String,
        to: String,
    },
    /// The run already sits in the terminal phase.
    TerminalPhase {
        phase: String,
    },
    /// A run with no pending question cannot be answered.
    NoPendingQuestion,
    /// The run already carries a pending question.
    QuestionPending,
    /// A bounded input exceeded its cap.
    TooLong {
        field: &'static str,
        max: usize,
    },
    /// Too many budget rows, or too many artifact refs, in one request.
    TooMany {
        field: &'static str,
        max: usize,
    },
    /// The checkpoint gate refused the artifact. Carries the executor's own
    /// refusal verbatim — the QA law, not a nearest-match guess.
    QualityGate {
        reason: String,
    },
    /// the attestation round: the attestation layer refused the pass. The detail is the
    /// attestation's own closed vocabulary (`operator_key_absent`,
    /// `operator_key_refused`, `model_retired`, `attestation_chain_full`, …),
    /// so a caller learns WHICH law stopped the phase pass.
    AttestationRefused {
        reason: String,
    },
    /// the model-citation law: the model binding named a row the registry does not hold, or
    /// a key that is not registry-shaped at all. Three DISTINCT refusals, one
    /// per registry refusal — an unregistered row and an unpromoted one are
    /// different operator problems.
    ModelNotRegistered,
    ModelNotPromoted,
    ModelRetired,
    /// the model-citation law: the row resolves but carries no artifact digest, so nothing
    /// says which bytes acted. A name without its digest is not a citation.
    ModelDigestMissing,
    /// The storage boundary refused. The detail never reaches a caller
    /// verbatim; it exists so the failure is diagnosable.
    Storage(String),
}

impl std::fmt::Display for DeliveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownVocabulary { field, value } => {
                write!(f, "delivery_unknown_vocabulary:{field}:{value}")
            }
            Self::RunAbsent => write!(f, "delivery_run_not_found"),
            Self::Stale { actual_revision } => {
                write!(f, "delivery_gate_stale_revision:{actual_revision}")
            }
            Self::IllegalPhaseTransition { from, to } => {
                write!(f, "delivery_illegal_phase_transition:{from}->{to}")
            }
            Self::TerminalPhase { phase } => write!(f, "delivery_terminal_phase:{phase}"),
            Self::NoPendingQuestion => write!(f, "delivery_no_pending_question"),
            Self::QuestionPending => write!(f, "delivery_question_pending"),
            Self::TooLong { field, max } => write!(f, "delivery_{field}_too_long:{max}"),
            Self::TooMany { field, max } => write!(f, "delivery_{field}_too_many:{max}"),
            Self::QualityGate { reason } => write!(f, "delivery_quality_gate_refused:{reason}"),
            Self::AttestationRefused { reason } => {
                write!(f, "delivery_attestation_refused:{reason}")
            }
            Self::ModelNotRegistered => write!(f, "delivery_model_not_registered"),
            Self::ModelNotPromoted => write!(f, "delivery_model_not_promoted"),
            Self::ModelRetired => write!(f, "delivery_model_retired"),
            Self::ModelDigestMissing => write!(f, "delivery_model_digest_missing"),
            Self::Storage(detail) => write!(f, "delivery_storage: {detail}"),
        }
    }
}

impl std::error::Error for DeliveryError {}

fn storage(detail: impl std::fmt::Display) -> DeliveryError {
    DeliveryError::Storage(detail.to_string())
}

/// The attestation layer's error, mapped into the delivery vocabulary. Every
/// attestation refusal becomes a TYPED delivery refusal carrying the
/// attestation's own reason, never a flattened "storage" or a 500.
fn attestation(error: crate::workflow::attestations::AttestationError) -> DeliveryError {
    use crate::workflow::attestations::AttestationError as E;
    match error {
        E::ModelNotRegistered => DeliveryError::ModelNotRegistered,
        E::ModelNotPromoted => DeliveryError::ModelNotPromoted,
        E::ModelRetired => DeliveryError::ModelRetired,
        E::ModelDigestMissing => DeliveryError::ModelDigestMissing,
        // A failed audit INSERT is a STORAGE failure, not an attestation law
        // refusal: conflating the two would make the rollback pin's boundary
        // message lie, and "the evidence substrate is gone" is a different
        // operator problem from "there is no key".
        E::Storage(detail) => DeliveryError::Storage(detail),
        other => DeliveryError::AttestationRefused {
            reason: other.to_string(),
        },
    }
}

/// Parse against a closed vocabulary, never guessing a near match.
fn closed(field: &'static str, value: &str) -> Result<Phase, DeliveryError> {
    Phase::parse(value).map_err(|_| DeliveryError::UnknownVocabulary {
        field,
        value: value.to_string(),
    })
}

/// Parse the DESIGN OWNER's autonomy-tier vocabulary — the one that reaches
/// the wire and the stored `delivery_traces.tier` column.
///
/// The pure crate spells its own variants `snake_case` (`bounded_auto`) while
/// the governing spec spells the same closed set `kebab-case`
/// (`bounded-auto`, DO L108). The design owner is the sole governing source
/// and the crate is an implementation artifact of a shipped round, so the DO's
/// spelling is what is STORED and what a client sends; the crate's spelling
/// stays inside the crate, where its own pins hold it.
///
/// This is a closed, total, four-arm bijection between the two spellings —
/// not a normalization pass, and not a nearest-match guess. Both directions are
/// pinned by `delivery_tier_vocabulary_is_a_bijection_onto_the_design_owners`.
fn tier_wire_to_core(s: &str) -> Result<AutonomyTier, DeliveryError> {
    match s {
        "observe" => Ok(AutonomyTier::Observe),
        "propose" => Ok(AutonomyTier::Propose),
        "bounded-auto" => Ok(AutonomyTier::BoundedAuto),
        "delegated" => Ok(AutonomyTier::Delegated),
        other => Err(DeliveryError::UnknownVocabulary {
            field: "tier",
            value: other.to_string(),
        }),
    }
}

/// The inverse: what goes in the column and on the wire.
fn tier_core_to_wire(t: AutonomyTier) -> &'static str {
    match t {
        AutonomyTier::Observe => "observe",
        AutonomyTier::Propose => "propose",
        AutonomyTier::BoundedAuto => "bounded-auto",
        AutonomyTier::Delegated => "delegated",
    }
}

fn closed_tier(value: &str) -> Result<AutonomyTier, DeliveryError> {
    tier_wire_to_core(value)
}

fn bounded_input(field: &'static str, value: &str, max: usize) -> Result<(), DeliveryError> {
    if value.chars().count() > max {
        return Err(DeliveryError::TooLong { field, max });
    }
    Ok(())
}

// ── the trace row ──────────────────────────────────────────────────────────

/// What a trace row commits to. The id IS the commitment: the content digest
/// over the canonical field order, so a trace row is addressable by its own
/// contents and a replay can re-derive the same id from the same facts.
///
/// No raw query, evidence text, model bytes, rules bytes, or secrets: the row
/// carries refs, digests, and closed labels only.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct TraceRow {
    pub id: String,
    pub run_id: i64,
    /// the stored-ordinal law: the STORED ordinal inside the run. 1-based, allocated in the
    /// caller's transaction as `MAX(seq)+1`. It is a field and not a parameter
    /// because `content_id` digests it and a re-derivation must reproduce the
    /// id from the row alone.
    pub seq: i64,
    pub stage: String,
    pub phase: String,
    pub status: String,
    pub tier: String,
    pub actor: String,
    pub model_ref: Option<String>,
    pub policy_digest: Option<String>,
    pub config_digest: Option<String>,
    pub pipeline_version: String,
    pub budget_digest: Option<String>,
    pub artifact_refs_json: String,
    pub attestation_root: Option<String>,
    pub created_at: i64,
}

impl TraceRow {
    /// The canonical bytes the id digests: the closed labels and digests in a
    /// fixed order, the STORED ORDINAL last. Two rows with the same facts and
    /// the same position produce the same id, which is what makes the replay
    /// index trustworthy.
    ///
    /// `seq` is framed and `created_at` is NOT: the ordinal is part of the
    /// row's identity (the same disposition can be recorded twice) and the
    /// wall clock is not (a replay that crosses a second boundary must still
    /// reproduce the id). The docstring here said the opposite for years, and
    /// the attestation round is the round that made the distinction load-bearing.
    fn canonical_bytes(&self) -> Vec<u8> {
        let mut s = String::new();
        s.push_str(&self.seq.to_string());
        s.push('\u{1f}');
        s.push_str(&self.run_id.to_string());
        s.push('\u{1f}');
        s.push_str(&self.stage);
        s.push('\u{1f}');
        s.push_str(&self.phase);
        s.push('\u{1f}');
        s.push_str(&self.status);
        s.push('\u{1f}');
        s.push_str(&self.tier);
        s.push('\u{1f}');
        s.push_str(&self.actor);
        s.push('\u{1f}');
        s.push_str(self.model_ref.as_deref().unwrap_or(""));
        s.push('\u{1f}');
        s.push_str(self.policy_digest.as_deref().unwrap_or(""));
        s.push('\u{1f}');
        s.push_str(self.config_digest.as_deref().unwrap_or(""));
        s.push('\u{1f}');
        s.push_str(&self.pipeline_version);
        s.push('\u{1f}');
        s.push_str(self.budget_digest.as_deref().unwrap_or(""));
        s.push('\u{1f}');
        s.push_str(&self.artifact_refs_json);
        s.push('\u{1f}');
        s.push_str(self.attestation_root.as_deref().unwrap_or(""));
        s.into_bytes()
    }

    /// `trc_<32 hex>` — a content address over the canonical bytes AND the
    /// row's STORED ordinal within its run.
    ///
    /// The ordinal is part of the identity on purpose. An append-only evidence
    /// log can legitimately record the same disposition twice (the same gate
    /// asked twice, at the same revision, in the same second), and a pure
    /// content address would collide on `delivery_traces.id` — which is
    /// precisely the bug the first run of this test found. Folding the ordinal
    /// in makes the address unique AND deterministic: replaying the same
    /// sequence of facts on a fresh database reproduces the same ids, which is
    /// the property the replay-verify surface will read.
    ///
    /// the stored-ordinal law folds the STORED `seq` rather than a passed-in counter, so
    /// this is a function of the row alone and the id is re-derivable from
    /// storage.
    pub(crate) fn content_id(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.canonical_bytes());
        let digest = hasher.finalize();
        format!("trc_{}", &crate::audit::hex_encode(&digest)[..32])
    }

    /// Read a stored row back into the struct, so an id can be re-derived from
    /// the database alone. `None` when the row is absent — absence is not an
    /// error at this boundary, the caller decides what it means.
    pub(crate) fn read_back(conn: &Connection, id: &str) -> Result<Option<Self>, DeliveryError> {
        conn.query_row(
            "SELECT id, run_id, seq, stage, phase, status, tier, actor, model_ref, policy_digest, \
             config_digest, pipeline_version, budget_digest, artifact_refs_json, attestation_root, \
             created_at FROM delivery_traces WHERE id = ?1",
            params![id],
            |r| {
                Ok(Self {
                    id: r.get(0)?,
                    run_id: r.get(1)?,
                    seq: r.get(2)?,
                    stage: r.get(3)?,
                    phase: r.get(4)?,
                    status: r.get(5)?,
                    tier: r.get(6)?,
                    actor: r.get(7)?,
                    model_ref: r.get(8)?,
                    policy_digest: r.get(9)?,
                    config_digest: r.get(10)?,
                    pipeline_version: r.get(11)?,
                    budget_digest: r.get(12)?,
                    artifact_refs_json: r.get(13)?,
                    attestation_root: r.get(14)?,
                    created_at: r.get(15)?,
                })
            },
        )
        .optional()
        .map_err(storage)
    }
}

/// The next ordinal inside the run, read inside the caller's transaction.
///
/// `MAX(seq)+1` and NEVER `COUNT(*)`: the count reissues an ordinal after a
/// row is removed, and the `UNIQUE(run_id, seq)` index then refuses the write
/// as a constraint violation rather than as the law it is. Under
/// `BEGIN IMMEDIATE` the read and the insert are serialized, so two writers
/// cannot claim the same ordinal.
fn trace_next_seq(conn: &Connection, run_id: i64) -> Result<i64, DeliveryError> {
    conn.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM delivery_traces WHERE run_id = ?1",
        params![run_id],
        |r| r.get(0),
    )
    .map_err(storage)
}

/// Write the trace row inside the caller's transaction. `artifact_refs_json` is
/// a JSON array of refs, never content.
fn write_trace(conn: &Connection, row: &TraceRow) -> Result<(), DeliveryError> {
    conn.execute(
        "INSERT INTO delivery_traces(id, run_id, seq, stage, phase, status, tier, actor, \
         model_ref, policy_digest, config_digest, pipeline_version, budget_digest, \
         artifact_refs_json, attestation_root, created_at) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
        params![
            row.id,
            row.run_id,
            row.seq,
            row.stage,
            row.phase,
            row.status,
            row.tier,
            row.actor,
            row.model_ref,
            row.policy_digest,
            row.config_digest,
            row.pipeline_version,
            row.budget_digest,
            row.artifact_refs_json,
            row.attestation_root,
            row.created_at,
        ],
    )
    .map_err(storage)?;
    Ok(())
}

// ── the fail-closed audit ───────────────────────────────────────────────────

/// The one audit shape for this round. `record_tenant` is the best-effort writer; calling it
/// here and CONVERTING `None` into an error is the house idiom for "this row
/// is required evidence" — the same one the GDL phase pass uses, and the
/// opposite of letting a dropped row pass for a write that already happened.
///
/// The detail carries closed labels and digests. The target is hashed before
/// insert by the audit layer itself.
/// the audit law: shared with the attestation chain writer, which must emit the same
/// kind with the same fail-closed idiom for its own write.
pub(crate) fn delivery_audit(
    conn: &Connection,
    tenant: &str,
    target: &str,
    status: AuditStatus,
    detail: &str,
) -> Result<(), DeliveryError> {
    crate::audit::record_tenant(
        conn,
        AuditKind::Workflow,
        crate::workflow::ACTOR,
        target,
        status,
        detail,
        tenant,
    )
    .ok_or_else(|| storage("delivery checked audit insertion failed"))?;
    Ok(())
}

// ── the run read ───────────────────────────────────────────────────────────

/// The delivery run head: (domain, kind, status, state_json, state_revision).
/// A non-delivery row reads as absent rather than as a wrong-kind error, so a
/// caller cannot probe for the existence of another kind's run.
fn delivery_head(
    conn: &Connection,
    run_id: i64,
) -> Result<Option<(String, String, String, i64)>, DeliveryError> {
    conn.query_row(
        "SELECT domain, status, state_json, state_revision FROM workflow_runs \
          WHERE id = ?1 AND kind = ?2",
        params![run_id, RUN_KIND],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )
    .optional()
    .map_err(storage)
}

fn decode_state(state_json: &str) -> Result<DeliveryState, DeliveryError> {
    serde_json::from_str(state_json).map_err(storage)
}

fn encode_state(state: &DeliveryState) -> Result<String, DeliveryError> {
    serde_json::to_string(state).map_err(storage)
}

// ── the gate disposition ────────────────────────────────────────────

/// What a phase-gate evaluation concluded. Deny wins: a refusal is never
/// downgraded to a prompt, and a prompt is never read as an allow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Disposition {
    Allow,
    Prompt,
    Deny,
}

impl Disposition {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allowed",
            Self::Prompt => "prompt",
            Self::Deny => "denied",
        }
    }
}

/// The closed refusal vocabulary. `None` on an allow or a prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DenyReason {
    TerminalPhase,
    IllegalPhaseTransition,
    ClosedVocabulary,
    ExhaustedBudget,
}

impl DenyReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::TerminalPhase => "terminal-phase",
            Self::IllegalPhaseTransition => "illegal-phase-transition",
            Self::ClosedVocabulary => "closed-vocabulary",
            Self::ExhaustedBudget => "exhausted-budget",
        }
    }
}

/// The pure disposition. It reads the phase, the tier, and nothing else — in
/// particular it does NOT read a budget ceiling, because no budget in this
/// round is enforced and a row consulted here would be enforcement by
/// accident.
fn dispose(
    current: Phase,
    proposed: Option<Phase>,
    tier: AutonomyTier,
) -> (Disposition, Option<DenyReason>) {
    if current == terminal_phase() {
        return (Disposition::Deny, Some(DenyReason::TerminalPhase));
    }
    match proposed {
        None => (Disposition::Deny, Some(DenyReason::ClosedVocabulary)),
        Some(to) => {
            if !is_legal_phase_transition(current, to) {
                return (Disposition::Deny, Some(DenyReason::IllegalPhaseTransition));
            }
            // A non-promoting tier asks; it does not self-advance. The human's
            // advance route IS the disposal, so a prompt is a gate outcome, not
            // a failure.
            if tier.is_promoting() {
                (Disposition::Allow, None)
            } else {
                (Disposition::Prompt, None)
            }
        }
    }
}

// ── the four cores ──────────────────────────────────────────────────────────

/// A budget ceiling as REQUESTED. Stored, never consulted: the round that
/// enforces budgets reads these rows, and until then a ceiling is a statement
/// of intent recorded as evidence.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct BudgetCeiling {
    pub kind: String,
    pub ceiling: i64,
}

/// Open a delivery run: the run row, its budget rows, the admission trace, and
/// the audit — one transaction.
pub(crate) struct CreateRun<'a> {
    pub domain: &'a str,
    pub goal: &'a str,
    pub tier: &'a str,
    pub policy_digest: Option<&'a str>,
    pub config_digest: Option<&'a str>,
    pub budgets: &'a [BudgetCeiling],
    pub now: i64,
}

/// What the run-create route serves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Created {
    pub run_id: i64,
    pub phase: String,
    pub tier: String,
    pub trace_mode: String,
    pub trace_id: String,
    pub state_revision: i64,
}

pub(crate) fn create_run(
    conn: &mut Connection,
    req: &CreateRun<'_>,
) -> Result<Created, DeliveryError> {
    bounded_input("goal", req.goal, MAX_GOAL_CHARS)?;
    bounded_input("policy_digest", req.policy_digest.unwrap_or(""), 128)?;
    bounded_input("config_digest", req.config_digest.unwrap_or(""), 128)?;
    if req.budgets.len() > MAX_BUDGETS {
        return Err(DeliveryError::TooMany {
            field: "budgets",
            max: MAX_BUDGETS,
        });
    }
    let tier = closed_tier(req.tier)?;

    let mut tx = crate::workflow::tx::WorkflowTx::begin(conn).map_err(storage)?;
    let state = DeliveryState::fresh(tier);
    let state_json = encode_state(&state)?;

    let run_id =
        state::open_run(tx.tx(), req.domain, RUN_KIND, &state_json, req.now).map_err(storage)?;

    // Budget ceilings are STORED. They are not read back by anything in this
    // round; the kind CHECK is the only gate, and it is the database's.
    for b in req.budgets {
        if b.ceiling < 0 {
            return Err(DeliveryError::Storage("negative budget ceiling".into()));
        }
        tx.tx()
            .execute(
                "INSERT INTO delivery_budgets(run_id, kind, ceiling, spent, updated_at) \
                 VALUES (?1, ?2, ?3, 0, ?4)",
                params![run_id, b.kind, b.ceiling, req.now],
            )
            .map_err(storage)?;
    }

    let mut row = TraceRow {
        id: String::new(),
        run_id,
        seq: trace_next_seq(tx.tx(), run_id)?,
        stage: "run".into(),
        phase: state.phase.clone(),
        status: "admitted".into(),
        tier: tier_core_to_wire(tier).into(),
        actor: crate::workflow::ACTOR.into(),
        model_ref: None,
        policy_digest: req.policy_digest.map(str::to_string),
        config_digest: req.config_digest.map(str::to_string),
        pipeline_version: PIPELINE_VERSION.into(),
        budget_digest: None,
        artifact_refs_json: "[]".into(),
        // the attestation round: the admission PREDATES every link, so there is no head yet and
        // `None` is the honest value. This site only ever READS the chain.
        attestation_root: crate::workflow::attestations::chain_head(tx.tx(), run_id)
            .map_err(attestation)?,
        created_at: req.now,
    };
    row.id = row.content_id();
    write_trace(tx.tx(), &row)?;

    // LAST statement before the commit.
    delivery_audit(
        tx.tx(),
        req.domain,
        &format!("delivery_run:{run_id}"),
        AuditStatus::Ok,
        &format!(
            "delivery run admitted phase={} tier={} budgets={} goal_len={}",
            state.phase,
            tier.as_str(),
            req.budgets.len(),
            req.goal.chars().count()
        ),
    )?;
    tx.commit().map_err(storage)?;

    Ok(Created {
        run_id,
        phase: state.phase,
        tier: tier_core_to_wire(tier).to_string(),
        trace_mode: trace_mode_for_tier(tier).as_str().to_string(),
        trace_id: row.id,
        state_revision: 0,
    })
}

/// Advance a run one phase. THE phase pass, and the shape the round is named
/// for: step row + CAS + trace + audit, one `WorkflowTx`, audit last.
pub(crate) struct Advance<'a> {
    pub run_id: i64,
    pub expected_revision: i64,
    pub to_phase: &'a str,
    pub artifact_refs: &'a [String],
    /// The typed artifact this phase pass carries, if any. Absent is the
    /// previous behaviour byte for byte; present files exactly one proposal
    /// inside this same `WorkflowTx`.
    pub artifact: Option<&'a DeliveryArtifact>,
    /// the model-citation law: the OPTIONAL model binding this pass executes under. Absent is
    /// the previous behaviour byte for byte — the trace rows carry honest
    /// `None`s and the predicate's `model_ref`/`model_digest` stay empty. It is
    /// optional because it is the only way the model citation can be REAL rather
    /// than vacuous: a citation nobody can present proves nothing.
    pub model: Option<&'a ModelBinding>,
    pub actor: &'a str,
    pub now: i64,
}

/// A model's registry binding, as the caller names it: the registry key shape
/// and the config digest that selects the row. The server resolves both — a
/// client may name a model, never vouch for one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelBinding {
    pub key: String,
    pub config_digest: String,
}

/// What a resolved binding actually cites: the key, the config digest that
/// selected the row, and the row's ARTIFACT digest — the bytes. The last one is
/// why a binding with no artifact digest refuses instead of being recorded.
struct Citation {
    key: String,
    config_digest: String,
    artifact_digest: String,
}

/// Resolve a presented binding through the registry, in the run's own trace
/// mode, and return the citation. Each registry refusal maps to a DISTINCT
/// delivery error: unregistered, unpromoted, and retired are three different
/// operator problems and a caller must be able to tell them apart.
fn resolve_citation(
    conn: &Connection,
    binding: &ModelBinding,
    tier: AutonomyTier,
) -> Result<Citation, DeliveryError> {
    use crate::workflow::registry::{ResolveRefusal, resolve_for_execution};
    bounded_input("model_key", &binding.key, MAX_MODEL_KEY_CHARS)?;
    bounded_input(
        "model_config_digest",
        &binding.config_digest,
        MAX_DIGEST_INPUT_CHARS,
    )?;
    let mode = trace_mode_for_tier(tier).as_str();
    let resolved =
        resolve_for_execution(conn, &binding.key, &binding.config_digest, mode).map_err(storage)?;
    let row = match resolved {
        Ok(row) => row,
        Err(ResolveRefusal::NotRegistered) => return Err(DeliveryError::ModelNotRegistered),
        Err(ResolveRefusal::NotPromoted) => return Err(DeliveryError::ModelNotPromoted),
        Err(ResolveRefusal::Retired) => return Err(DeliveryError::ModelRetired),
    };
    let artifact_digest = row
        .artifact_digest
        .filter(|d| !d.trim().is_empty())
        .ok_or(DeliveryError::ModelDigestMissing)?;
    Ok(Citation {
        key: binding.key.clone(),
        config_digest: binding.config_digest.clone(),
        artifact_digest,
    })
}

/// The KERNEL-DERIVED subject name (A4). Never agent prose: the signed name is
/// `delivery/{phase}` plus, when a typed artifact rode the pass, a bounded
/// suffix drawn from the artifact's OWN id and filtered to `[a-z0-9-]`. An id
/// that does not survive the filter contributes nothing rather than being
/// escaped into the signed bytes — the name says what the pass was, and the
/// digest beside it says which bytes.
fn subject_name(phase: &str, artifact: Option<&DeliveryArtifact>) -> String {
    let Some(artifact) = artifact else {
        return format!("delivery/phase/{phase}");
    };
    let suffix: String = artifact
        .id
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
        .take(MAX_SUBJECT_SUFFIX_CHARS)
        .collect();
    if suffix.is_empty() {
        format!("delivery/phase/{phase}")
    } else {
        format!("delivery/phase/{phase}/{suffix}")
    }
}

/// What the advance route serves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Advanced {
    pub run_id: i64,
    pub phase: String,
    pub tier: String,
    pub trace_mode: String,
    pub trace_id: String,
    pub state_revision: i64,
    pub step_id: i64,
    /// The proposal the typed artifact filed, or `0` when the pass carried no
    /// artifact. A caller holding a non-zero id has evidence that the
    /// proposal, the trace, and the audit all committed together.
    pub proposal_id: i64,
}

pub(crate) fn advance(conn: &mut Connection, req: &Advance<'_>) -> Result<Advanced, DeliveryError> {
    if req.artifact_refs.len() > MAX_REFS {
        return Err(DeliveryError::TooMany {
            field: "artifact_refs",
            max: MAX_REFS,
        });
    }
    for r in req.artifact_refs {
        bounded_input("artifact_refs", r, 256)?;
    }

    // The artifact's own bounds, before any transaction opens. A typed artifact
    // is executor-produced and therefore untrusted input at this boundary; it
    // is screened by the handler and bounded HERE, and stored verbatim so the
    // approval digest is computed over one shape.
    if let Some(artifact) = req.artifact {
        bounded_input("artifact_id", &artifact.id, MAX_ARTIFACT_ID_CHARS)?;
        bounded_input("artifact_content", &artifact.content, MAX_ARTIFACT_CHARS)?;
        if artifact.content.trim().is_empty() {
            return Err(DeliveryError::Storage("artifact_content_empty".into()));
        }
        if let Some(gate) = artifact.quality_gate.as_deref() {
            bounded_input("artifact_quality_gate", gate, MAX_GATE_CHARS)?;
        }
    }

    let mut tx = crate::workflow::tx::WorkflowTx::begin(conn).map_err(storage)?;

    // 1. ownership + existence re-verify, inside the transaction.
    let (domain, _status, state_json, revision) =
        delivery_head(tx.tx(), req.run_id)?.ok_or(DeliveryError::RunAbsent)?;
    if revision != req.expected_revision {
        return Err(DeliveryError::Stale {
            actual_revision: revision,
        });
    }

    let state = decode_state(&state_json)?;
    let current = closed("phase", &state.phase)?;
    let tier = closed_tier(&state.tier)?;
    let proposed = closed("to_phase", req.to_phase)?;

    // 2. legality, BEFORE any write — the forward-only machine.
    if current == terminal_phase() {
        return Err(DeliveryError::TerminalPhase {
            phase: state.phase.clone(),
        });
    }
    if !is_legal_phase_transition(current, proposed) {
        return Err(DeliveryError::IllegalPhaseTransition {
            from: state.phase.clone(),
            to: proposed.as_str().to_string(),
        });
    }

    // 2a. the checkpoint gate — BEFORE any write. The shipped executor
    // validator decides whether this artifact's evidence is a live surface, and
    // a refusal aborts the whole pass with nothing landed. This is engine
    // CONSUMPTION: the law lives in the crate and is not restated here.
    if let Some(artifact) = req.artifact
        && proposed == Phase::Build
        && let Err(reason) = artifact.quality_gate()
    {
        return Err(DeliveryError::QualityGate { reason });
    }

    // 2b. the model citation, resolved BEFORE any write so a refusal leaves
    // nothing behind. A presented binding must resolve through the registry and
    // must carry the row's ARTIFACT digest: a model name with no bytes behind it
    // is not evidence, and recording it as if it were would be a citation the
    // verifier cannot check. An ABSENT binding is the honest `None`s below, not
    // a forged citation.
    let citation = req
        .model
        .map(|binding| resolve_citation(tx.tx(), binding, tier))
        .transpose()?;

    // 3. the step row — the phase pass's own durable artifact.
    let refs_json = serde_json::to_string(req.artifact_refs).map_err(storage)?;
    tx.tx()
        .execute(
            "INSERT INTO workflow_steps(run_id, phase, step_key, state_json) \
             VALUES (?1, ?2, ?2, ?3)",
            params![req.run_id, proposed.as_str(), refs_json],
        )
        .map_err(storage)?;
    let step_id = tx.tx().last_insert_rowid();

    // 4. the CAS — fail-closed. A stale revision aborts the whole pass.
    let next = DeliveryState {
        phase: proposed.as_str().to_string(),
        tier: tier_core_to_wire(tier).to_string(),
        attempt: state.attempt + 1,
        pending_question: state.pending_question.clone(),
    };
    let next_status = if proposed == terminal_phase() {
        STATUS_COMPLETED
    } else {
        STATUS_ACTIVE
    };
    state::cas_update(
        tx.tx(),
        req.run_id,
        req.expected_revision,
        &encode_state(&next)?,
        next_status,
        req.now,
    )
    .map_err(|e| match e {
        state::CasError::Stale { actual_revision } => DeliveryError::Stale { actual_revision },
        other => storage(other),
    })?;

    // 5. the trace row — the phase-advance transaction is the first writer.
    //    The model citation rides it, and the attestation head is the PREVIOUS
    //    link: this pass's own link is appended just after, so naming itself
    //    would be circular.
    let mut row = TraceRow {
        id: String::new(),
        run_id: req.run_id,
        seq: trace_next_seq(tx.tx(), req.run_id)?,
        stage: "phase".into(),
        phase: proposed.as_str().into(),
        status: "advanced".into(),
        tier: tier_core_to_wire(tier).into(),
        actor: req.actor.to_string(),
        model_ref: citation.as_ref().map(|c| c.key.clone()),
        policy_digest: None,
        config_digest: citation.as_ref().map(|c| c.config_digest.clone()),
        pipeline_version: PIPELINE_VERSION.into(),
        budget_digest: None,
        artifact_refs_json: refs_json.clone(),
        attestation_root: crate::workflow::attestations::chain_head(tx.tx(), req.run_id)
            .map_err(attestation)?,
        created_at: req.now,
    };
    row.id = row.content_id();
    write_trace(tx.tx(), &row)?;

    // 5a-bis. THE CHAIN LINK (the attestation round). The ONE place the chain is written, inside
    // the same `WorkflowTx` as the step row, the CAS, and the trace: a phase
    // pass can never commit without its signed evidence, and a link can never
    // name a pass that did not commit. It runs before the proposal seam and the
    // audit, both of which stay where they are — the audit remains the LAST
    // statement before the commit.
    // A pass with no artifact still attests SOMETHING: the phase it moved to,
    // under the run's own pipeline version. The digest is real and
    // kernel-derived; it is simply not an artifact's.
    let subject_digest = if let Some(artifact) = req.artifact {
        artifact.typed().hash
    } else {
        brain_executor_core::artifact_hash(&format!("{PIPELINE_VERSION}:{}", proposed.as_str()))
    };
    let attestation_id = crate::workflow::attestations::append_link(
        tx.tx(),
        &crate::workflow::attestations::ChainLink {
            domain: domain.clone(),
            run_id: req.run_id,
            step_id,
            subject_name: subject_name(proposed.as_str(), req.artifact),
            subject_digest,
            policy_digest: None,
            config_digest: citation.as_ref().map(|c| c.config_digest.clone()),
            model_ref: citation.as_ref().map(|c| c.key.clone()),
            model_digest: citation.as_ref().map(|c| c.artifact_digest.clone()),
            tier,
        },
        req.now,
    )
    .map_err(attestation)?;

    // 5a. the typed-artifact proposal seam. The artifact becomes a PENDING
    // proposal in the SAME transaction as the step row, the CAS, and the trace:
    // a reviewable artifact can never cite a phase pass that did not commit,
    // and a phase pass can never commit without its artifact's evidence.
    //
    // The executor PROPOSES. It writes no disposition, no `decided_at`, and no
    // routing key — the gate disposes, and the four normative keys stay the
    // engine's. The content is stored VERBATIM so `review_digest` binds one
    // shape: a seam that pre-sanitized here would move the digest of every
    // outstanding approval and fail them closed with 409 at approve time.
    let mut proposal_id = 0_i64;
    if let Some(artifact) = req.artifact {
        let typed = artifact.typed();
        proposal_id = tx
            .tx()
            .query_row(
                "INSERT INTO proposals(kind, content, title, source, novelty, salience, \
                                    created_at, owner, domain, decision_run_ref)
                 VALUES (?1, ?2, ?3, ?4, 0, 0, ?5, ?6, ?7, ?8)
                 RETURNING id",
                params![
                    ARTIFACT_PROPOSAL_KIND,
                    typed.content,
                    typed.id,
                    format!("delivery:{}", RUN_KIND),
                    req.now,
                    req.actor,
                    domain,
                    format!("trc:{}", row.id),
                ],
                |r| r.get(0),
            )
            .map_err(storage)?;

        // The narrative row. The payload carries the digest, the ids, and the
        // gate flag — never the artifact body, which is the proposal's job.
        let narrative = serde_json::json!({
            "proposal_id": proposal_id,
            "trace_id": row.id,
            "artifact_id": typed.id,
            "artifact_hash": typed.hash,
            "phase": proposed.as_str(),
            "tier": tier.as_str(),
            "quality_gate": artifact.quality_gate.is_some(),
        })
        .to_string();
        crate::workflow::session_log::append(
            tx.tx(),
            req.run_id,
            DDL_ARTIFACT_KIND,
            &narrative,
            &format!("ddl-artifact:{proposal_id}"),
            req.now,
        )
        .map_err(|e| storage(format!("delivery session append failed: {e}")))?;
    }

    // 6. the audit — LAST, fail-closed, so a pass cannot land without it.
    // The pass's audit detail is byte-identical to an earlier round's, so an earlier round's
    // detail-hash pin keeps passing unchanged. The tie between this transition
    // and its signed link is the LINK'S OWN audit row, whose target is
    // `delivery/attestation/{run_id}/{id}` — one evidence row per write, rather
    // than a second mention of the link in the pass's row.
    //
    // The id is read here so the linkage is visible at the tie point. It was a
    // discarded reference, which reads like a swallowed error (AGENTS.md
    // forbids the discard idiom on writes) while asserting nothing: the binding
    // is what documents the tie, and naming it does the same job.
    debug_assert!(!attestation_id.is_empty(), "the sealed link carries an id");
    delivery_audit(
        tx.tx(),
        &domain,
        &format!("delivery_run:{}", req.run_id),
        AuditStatus::Ok,
        &format!(
            "delivery phase pass {}->{} tier={} refs={} status={next_status} \
             proposal={proposal_id}",
            state.phase,
            proposed.as_str(),
            tier.as_str(),
            req.artifact_refs.len()
        ),
    )?;
    tx.commit().map_err(storage)?;

    Ok(Advanced {
        run_id: req.run_id,
        phase: next.phase,
        tier: next.tier,
        trace_mode: trace_mode_for_tier(tier).as_str().to_string(),
        trace_id: row.id,
        state_revision: req.expected_revision + 1,
        step_id,
        proposal_id,
    })
}

/// Answer the run's pending question. The answer is operator-authored prose on
/// a run the operator owns: it is stored in the run's own state, bounded, and
/// never copied into a trace row — the trace records that an answer happened
/// and nothing about what it said.
pub(crate) struct Answer<'a> {
    pub run_id: i64,
    pub expected_revision: i64,
    pub answer: &'a str,
    pub actor: &'a str,
    pub now: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Answered {
    pub run_id: i64,
    pub trace_id: String,
    pub state_revision: i64,
}

pub(crate) fn answer(conn: &mut Connection, req: &Answer<'_>) -> Result<Answered, DeliveryError> {
    bounded_input("answer", req.answer, MAX_ANSWER_CHARS)?;

    let mut tx = crate::workflow::tx::WorkflowTx::begin(conn).map_err(storage)?;
    let (domain, _status, state_json, revision) =
        delivery_head(tx.tx(), req.run_id)?.ok_or(DeliveryError::RunAbsent)?;
    if revision != req.expected_revision {
        return Err(DeliveryError::Stale {
            actual_revision: revision,
        });
    }
    let mut state = decode_state(&state_json)?;
    if state.pending_question.is_none() {
        return Err(DeliveryError::NoPendingQuestion);
    }
    state.pending_question = None;
    state.attempt += 1;

    let tier = closed_tier(&state.tier)?;
    state::cas_update(
        tx.tx(),
        req.run_id,
        req.expected_revision,
        &encode_state(&state)?,
        STATUS_ACTIVE,
        req.now,
    )
    .map_err(|e| match e {
        state::CasError::Stale { actual_revision } => DeliveryError::Stale { actual_revision },
        other => storage(other),
    })?;

    let mut row = TraceRow {
        id: String::new(),
        run_id: req.run_id,
        seq: trace_next_seq(tx.tx(), req.run_id)?,
        stage: "answer".into(),
        phase: state.phase.clone(),
        status: "answered".into(),
        tier: tier_core_to_wire(tier).into(),
        actor: req.actor.to_string(),
        model_ref: None,
        policy_digest: None,
        config_digest: None,
        pipeline_version: PIPELINE_VERSION.into(),
        budget_digest: None,
        artifact_refs_json: "[]".into(),
        // the attestation round: the answer reads the head and appends nothing. An attestation
        // records that a PHASE PASS happened; an answer is not one.
        attestation_root: crate::workflow::attestations::chain_head(tx.tx(), req.run_id)
            .map_err(attestation)?,
        created_at: req.now,
    };
    row.id = row.content_id();
    write_trace(tx.tx(), &row)?;
    delivery_audit(
        tx.tx(),
        &domain,
        &format!("delivery_run:{}", req.run_id),
        AuditStatus::Ok,
        &format!("delivery question answered phase={}", state.phase),
    )?;
    tx.commit().map_err(storage)?;

    Ok(Answered {
        run_id: req.run_id,
        trace_id: row.id,
        state_revision: req.expected_revision + 1,
    })
}

/// Evaluate the phase gate. It is a DISPOSITION, not a mutation: the run's
/// phase, status, and revision are untouched, and the only writes are the
/// trace row and its audit.
pub(crate) struct Gates<'a> {
    pub run_id: i64,
    pub to_phase: Option<&'a str>,
    pub actor: &'a str,
    pub now: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct GateVerdict {
    pub run_id: i64,
    pub phase: String,
    pub next_phase: Option<String>,
    pub tier: String,
    pub trace_mode: String,
    pub disposition: String,
    pub deny_reason: Option<String>,
    pub trace_id: String,
    pub evaluated_at: i64,
}

pub(crate) fn gates(conn: &mut Connection, req: &Gates<'_>) -> Result<GateVerdict, DeliveryError> {
    // A value outside the closed phase vocabulary is a 400 at the handler
    // boundary and a deny here, never a nearest-match guess.
    let proposed = req.to_phase.and_then(|v| closed("to_phase", v).ok());
    let unknown_vocab = proposed.is_none() && req.to_phase.is_some();

    let mut tx = crate::workflow::tx::WorkflowTx::begin(conn).map_err(storage)?;
    let (domain, _status, state_json, _revision) =
        delivery_head(tx.tx(), req.run_id)?.ok_or(DeliveryError::RunAbsent)?;
    let state = decode_state(&state_json)?;
    let current = closed("phase", &state.phase)?;
    let tier = closed_tier(&state.tier)?;

    let (mut disposition, mut reason) = dispose(current, proposed, tier);
    if unknown_vocab {
        // Deny wins: a closed-vocabulary failure is a denial, not a prompt.
        disposition = Disposition::Deny;
        reason = Some(DenyReason::ClosedVocabulary);
    }

    let mut row = TraceRow {
        id: String::new(),
        run_id: req.run_id,
        seq: trace_next_seq(tx.tx(), req.run_id)?,
        stage: "gate".into(),
        phase: current.as_str().into(),
        status: disposition.as_str().into(),
        tier: tier_core_to_wire(tier).into(),
        actor: req.actor.to_string(),
        model_ref: None,
        policy_digest: None,
        config_digest: None,
        pipeline_version: PIPELINE_VERSION.into(),
        budget_digest: None,
        artifact_refs_json: "[]".into(),
        // the attestation round: the gate reads the head and appends nothing. A DISPOSITION is
        // not an attestation — the chain has no disposition column, and a deny
        // or a prompt leaves no link behind.
        attestation_root: crate::workflow::attestations::chain_head(tx.tx(), req.run_id)
            .map_err(attestation)?,
        created_at: req.now,
    };
    row.id = row.content_id();
    write_trace(tx.tx(), &row)?;
    delivery_audit(
        tx.tx(),
        &domain,
        &format!("delivery_run:{}", req.run_id),
        if disposition == Disposition::Deny {
            AuditStatus::Denied
        } else {
            AuditStatus::Ok
        },
        &format!(
            "delivery gate phase={} proposed={} disposition={} reason={}",
            current.as_str(),
            req.to_phase.unwrap_or("none"),
            disposition.as_str(),
            reason.map(DenyReason::as_str).unwrap_or("none")
        ),
    )?;
    tx.commit().map_err(storage)?;

    Ok(GateVerdict {
        run_id: req.run_id,
        phase: current.as_str().to_string(),
        next_phase: proposed.map(|p| p.as_str().to_string()),
        tier: tier_core_to_wire(tier).to_string(),
        trace_mode: trace_mode_for_tier(tier).as_str().to_string(),
        disposition: disposition.as_str().to_string(),
        deny_reason: reason.map(DenyReason::as_str).map(str::to_string),
        trace_id: row.id,
        evaluated_at: req.now,
    })
}

// ── the replay read surface ─────────────────────────────────────────────

/// The trace window's cap (bounds law). A run's trace is an append-only log
/// with no natural ceiling, so the read is bounded and the bound is DISCLOSED
/// in the payload — a bounded window that does not announce itself is a silent
/// short read, and a reader who cannot tell a window from the whole run will
/// draw conclusions from rows that were never shown to them.
pub(crate) const MAX_TRACE_ROWS: usize = 500;

/// One stage's digests, as they travel the wire. The frozen crate's
/// `StageDigest` deliberately does not derive `Serialize` — the crate is
/// untouchable this round — so the wire form is HAND-MAPPED here. `StageDigest`
/// gains no derive; this type is the server's own projection of it, and it is
/// the only place the mapping is written.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct StageDigestRead {
    pub stage: String,
    pub input_digest: String,
    pub output_digest: String,
}

/// One mismatch, hand-mapped for the same reason.
///
/// `mismatch` is the crate's `StageMismatch` rendered as its own name, so a
/// client reads `output_digest_differs` rather than a position in an enum. The
/// mapping is exhaustive over the five variants and panics on a sixth: a new
/// variant must be given a wire name deliberately, and an unmapped one reaching
/// a client as a number would be unreadable.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct StageDiffRead {
    pub stage: String,
    pub mismatch: &'static str,
    pub recorded: Option<StageDigestRead>,
    pub rederived: Option<StageDigestRead>,
}

/// The disclosed bounds of a read. `cap` and `truncated` travel together: a
/// caller can always tell a bounded window from the whole run.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct WindowRead {
    pub rows: usize,
    pub cap: usize,
    pub truncated: bool,
}

/// One session event, as it travels the wire. `SessionEventRow` gains no
/// `Serialize` derive — the read seam applies to this projection, and adding a
/// derive to another module for one caller's convenience is how a stored row
/// starts being emitted un-shaped.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct EventRowRead {
    pub seq: i64,
    pub kind: String,
    pub payload_json: String,
    pub created_at: i64,
}

/// The narrative appendix: the `ddl_*` session log, human-readable and
/// additive. It is NEVER the re-derivation source — `delivery_traces` is the
/// spine — and a mismatch is never read out of it.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct EventLogRead {
    pub rows: Vec<EventRowRead>,
    pub cap: usize,
    pub truncated: bool,
}

/// The replay verdict. DATA, never a status: no run status, no approval, no
/// disposition, and nothing persisted.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ReplayReport {
    pub run_id: i64,
    pub window: WindowRead,
    /// Whether the stored ordinal series is contiguous ascending.
    pub order_ok: bool,
    pub compared: usize,
    pub matched: usize,
    pub mismatched: usize,
    pub diffs: Vec<StageDiffRead>,
    pub event_log: EventLogRead,
    pub generated_at: i64,
}

impl ReplayReport {
    /// Whether the report is CLEAN — no digest mismatch and no order violation.
    ///
    /// This is NOT the crate's `ReplayDiff::all_digests_match()`, and the
    /// difference matters: the crate's is `mismatched == 0` over the
    /// COMPARATOR's count, while this report's `mismatched` has order
    /// violations folded in. A run with a broken ordinal series and no digest
    /// mismatch reads `false` here and `true` there. This is the stricter rule
    /// and the one a reader of THIS payload wants.
    fn is_clean(&self) -> bool {
        self.mismatched == 0
    }
}

/// The trace listing. It rides the SAME read function and the SAME window
/// function as the verdict.
///
/// **What "the same read function" does and does not promise.** The two are
/// separate HTTP requests, on separate pooled connections, at separate
/// wall-clock times, with no shared transaction or snapshot. What sharing
/// guarantees is that both apply IDENTICAL logic — the same ordering, the same
/// window, the same truncation rule — so **any difference you observe between
/// the two is a change in storage, not a difference of method.** It does not
/// promise the two saw the same bytes: a row appended between the two calls
/// shows up as a difference, which is the honest reading of a live run.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct TraceListing {
    pub run_id: i64,
    pub rows: Vec<TraceRow>,
    pub window: WindowRead,
    /// The chain head this listing actually read, or `None` before the first
    /// link. It is the same head the attestation read reports, and it is read
    /// rather than recomputed.
    pub attestation_root: Option<String>,
    pub event_log: EventLogRead,
    pub generated_at: i64,
}

/// The stored stage column's four legal values, mirrored from the DDL's CHECK
/// (`migration.rs`). The projection reads the STORED column and adds nothing —
/// `StageDigest.stage` is a bare `String` with no vocabulary in the crate, so
/// this mirror is the only examination a stored value gets before it reaches
/// the wire.
///
/// It is enforced by a real check, not a `debug_assert!`. A debug-only guard
/// compiles to nothing in release, and the round's stated threat model is
/// exactly "a stored column was edited" — the one case a release build must
/// still catch. The `CHECK` binds today, so reaching this needs an
/// out-of-band edit, and a bad value is reported as a mismatch rather than
/// panicked on.
const LEGAL_STAGES: [&str; 4] = ["run", "phase", "gate", "answer"];

/// Read the run's trace rows in ORDINAL order, bounded by [`MAX_TRACE_ROWS`].
///
/// `ORDER BY seq` is served by `idx_delivery_traces_seq (run_id, seq)`, the
/// UNIQUE index the attestation round created. (The `(run_id, created_at)`
/// replay index CANNOT serve this order — `created_at` is second-granularity,
/// so it is neither total nor unique. That index stays for the walk that reads
/// by time.)
///
/// The bound is applied in SQL with `LIMIT cap + 1` and the overflow row
/// discarded in Rust, so `truncated` is a FACT rather than a guess: a run of
/// exactly `cap` rows is not truncated, and a run of `cap + 1` is. Reading
/// `cap` and inferring truncation from the length would report a full window
/// as truncated.
pub(crate) fn read_run_traces(
    conn: &Connection,
    run_id: i64,
    cap: usize,
) -> Result<Vec<TraceRow>, DeliveryError> {
    // The run must be a DELIVERY run. Every write path resolves its head
    // through `delivery_head`, which filters `kind = 'delivery'` and answers a
    // foreign run as absent; a read that skipped the filter would serve a
    // structurally-valid delivery payload for a GDL or account run that merely
    // shares the id space. Worse, it would turn "this id is not a delivery run"
    // into a 200 where every write says 404 — an existence answer the write
    // paths deliberately refuse to give. One kind, one answer.
    delivery_head(conn, run_id)?.ok_or(DeliveryError::RunAbsent)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, run_id, seq, stage, phase, status, tier, actor, model_ref, \
             policy_digest, config_digest, pipeline_version, budget_digest, \
             artifact_refs_json, attestation_root, created_at \
             FROM delivery_traces WHERE run_id = ?1 ORDER BY seq LIMIT ?2",
        )
        .map_err(storage)?;
    let rows = stmt
        .query_map(params![run_id, cap as i64 + 1], |r| {
            Ok(TraceRow {
                id: r.get(0)?,
                run_id: r.get(1)?,
                seq: r.get(2)?,
                stage: r.get(3)?,
                phase: r.get(4)?,
                status: r.get(5)?,
                tier: r.get(6)?,
                actor: r.get(7)?,
                model_ref: r.get(8)?,
                policy_digest: r.get(9)?,
                config_digest: r.get(10)?,
                pipeline_version: r.get(11)?,
                budget_digest: r.get(12)?,
                artifact_refs_json: r.get(13)?,
                attestation_root: r.get(14)?,
                created_at: r.get(15)?,
            })
        })
        .map_err(storage)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(storage)?;
    Ok(rows.into_iter().take(cap).collect())
}

/// B2, the projection law: one stored row becomes the pair of stage digests
/// the comparator consumes.
///
/// **The honest reading of what this proves.** `input_digest` is derived from
/// `canonical_bytes`, which is a pure function of the STORED columns — and the
/// re-derived side recomputes the same function over the same stored columns.
/// So the `input_digest` arms can never disagree: their equality is a statement
/// that the PROJECTION is well-formed, NOT that the row is unmodified.
///
/// The one comparison in this round that is NOT tautological is
/// `output_digest`: the recorded side carries the STORED content address and
/// the re-derived side carries the address recomputed from the stored columns.
/// A row whose stored `id` no longer follows from its columns disagrees there
/// and only there. That is the entire detection surface, and it detects a row
/// whose id and columns have fallen out of agreement — NOT an attacker who
/// edits a column AND recomputes the id, and it binds nothing to the
/// attestation chain. The chain is what binds; this checks.
fn stage_digest_projection(row: &TraceRow) -> (StageDigest, StageDigest) {
    // The stage is PROJECTED from the stored column, never invented: the DDL
    // CHECKs it to four values and the crate's `StageDigest.stage` is an
    // unvalidated `String`, so this is the only place the vocabulary is
    // examined. An out-of-band value is passed through with a marker prefix
    // rather than dropped or panicked on: this surface's whole job is to
    // REPORT what storage holds, and a row whose stage is out of vocabulary is
    // a finding a reader needs, not a request to be refused.
    let stage = if LEGAL_STAGES.contains(&row.stage.as_str()) {
        row.stage.clone()
    } else {
        format!("unexpected:{}", row.stage)
    };
    let input = format!(
        "sha256:{}",
        crate::audit::hex_encode(&row.canonical_bytes())
    );
    (
        StageDigest::new(stage.clone(), input.clone(), row.id.clone()),
        StageDigest::new(stage, input, row.content_id()),
    )
}

/// The wire name for the order fold's own mismatch.
///
/// It is deliberately NOT one of the crate's five. `StageMismatch` is frozen
/// this round, and borrowing `InputDigestDiffers` for an ordinal violation
/// would have contradicted the spec: the schema says the `input_digest` arms
/// "always agree", and then emitted `input_digest_differs` on every gap. A
/// client applying that rule would mis-diagnose a hole in the evidence log as
/// a malformed projection — two different operator responses.
///
/// So the order fold carries its own code on the wire, mapped here.
const ORDER_MISMATCH: &str = "order_not_contiguous";

/// B3, the order-integrity fold, and the order violations it reports as data.
///
/// The series must be `1..=n` contiguous ascending — 1-based, because the
/// writer allocates `MAX(seq)+1` and the backfill numbered pre-existing rows
/// `1..n`. A gap, a duplicate, or a descent is a MISMATCHING DIFF with stage
/// `"order"`, never an error status: mismatches are the product, and a reader
/// who is told "this run's evidence log has a hole" by an exception has learned
/// less than one handed the hole.
///
/// A duplicate is unreachable through the write path — `UNIQUE(run_id, seq)`
/// refuses it — so it is reported here anyway, because a database whose index
/// was dropped or rebuilt is exactly the state a reader needs to be told about.
///
/// **A TAIL DELETION IS INVISIBLE HERE, and that is a stated ceiling.** Removing
/// the last row leaves a shorter but still contiguous `1..n` series, so a run
/// whose evidence log was truncated at the end reports a CLEAN verdict. The
/// same vacuous-truth shape as an empty chain, and disclosed for the same
/// reason: only the head of the log is a window, and a window cannot detect
/// what fell out of it. `window.rows` is the only signal, and a reader
/// comparing it against the run's own phase count is doing that comparison
/// themselves.
fn order_diff(expected: i64, actual: i64) -> StageDiff {
    StageDiff {
        stage: "order".to_string(),
        recorded: Some(StageDigest::new(
            "order",
            format!("seq:{actual}"),
            String::new(),
        )),
        rederived: Some(StageDigest::new(
            "order",
            format!("seq:{expected}"),
            String::new(),
        )),
        mismatch: StageMismatch::InputDigestDiffers,
    }
}

/// The shared, truncated window both surfaces report.
///
/// This existed as the SAME twelve lines pasted into `replay_verify` and
/// `trace_listing` — and the round's own headline claim was that the two
/// surfaces cannot disagree. Duplicating the one field a reader consults to
/// decide whether the window IS the whole run is the exact failure that claim
/// was meant to foreclose. One function, two callers.
///
/// `rows_len` is what `read_run_traces` returned. The overflow row was already
/// discarded there, so truncation is a FACT and not an inference: the read was
/// `LIMIT cap + 1`, and this asks whether that extra row existed.
fn trace_window(
    conn: &Connection,
    run_id: i64,
    rows_len: usize,
) -> Result<WindowRead, DeliveryError> {
    let truncated = rows_len == MAX_TRACE_ROWS && {
        let more: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM (SELECT 1 FROM delivery_traces WHERE run_id = ?1 \
                 ORDER BY seq LIMIT ?2)",
                params![run_id, MAX_TRACE_ROWS as i64 + 1],
                |r| r.get(0),
            )
            .map_err(storage)?;
        more as usize > MAX_TRACE_ROWS
    };
    Ok(WindowRead {
        rows: rows_len,
        cap: MAX_TRACE_ROWS,
        truncated,
    })
}

/// Assemble the replay verdict over the run's stored trace rows.
///
/// `compare_replay` does the comparison; this function's own work is the read,
/// the projection, the order fold, and the hand-mapped wire form. It persists
/// NOTHING, calls no model, and reaches no network — the verdict is computable
/// from the stored bytes alone.
pub(crate) fn replay_verify(
    conn: &Connection,
    run_id: i64,
    now: i64,
) -> Result<ReplayReport, DeliveryError> {
    let rows = read_run_traces(conn, run_id, MAX_TRACE_ROWS)?;
    let window = trace_window(conn, run_id, rows.len())?;

    // The order fold. The stage digests are projected in the SAME order, so the
    // positional comparison lines up with the stored series by construction —
    // there is no second sort that could disagree with the first.
    let mut order_diffs: Vec<StageDiff> = Vec::new();
    let mut recorded: Vec<StageDigest> = Vec::with_capacity(rows.len());
    let mut rederived: Vec<StageDigest> = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let expected = index as i64 + 1;
        if row.seq != expected {
            order_diffs.push(order_diff(expected, row.seq));
        }
        let (rec, red) = stage_digest_projection(row);
        recorded.push(rec);
        rederived.push(red);
    }

    let diff = brain_delivery_core::compare_replay(&recorded, &rederived);
    let order_ok = order_diffs.is_empty();

    // The order violations join the comparator's diffs, and they are counted as
    // COMPARED positions that failed — so `matched + mismatched == compared`
    // holds over the report exactly as it does inside the crate.
    let mut diffs: Vec<StageDiffRead> = diff.diffs.iter().map(stage_diff_read).collect();
    let extra = order_diffs.len();
    diffs.extend(order_diffs.iter().map(stage_diff_read));

    let event_log = read_event_log(conn, run_id)?;

    Ok(ReplayReport {
        run_id,
        window,
        order_ok,
        compared: diff.compared + extra,
        matched: diff.matched,
        mismatched: diff.mismatched + extra,
        diffs,
        event_log,
        generated_at: now,
    })
}

/// The raw listing, over the SAME read function as the verdict.
pub(crate) fn trace_listing(
    conn: &Connection,
    run_id: i64,
    now: i64,
) -> Result<TraceListing, DeliveryError> {
    let rows = read_run_traces(conn, run_id, MAX_TRACE_ROWS)?;
    let window = trace_window(conn, run_id, rows.len())?;
    // The head is READ, not recomputed, so the two surfaces apply the same
    // logic to it.
    let attestation_root =
        crate::workflow::attestations::chain_head(conn, run_id).map_err(attestation)?;
    Ok(TraceListing {
        run_id,
        window,
        rows,
        attestation_root,
        event_log: read_event_log(conn, run_id)?,
        generated_at: now,
    })
}

/// The narrative appendix, bounded by the session log's own cap and disclosing
/// it. `session_log::replay` returns the `cap` MOST RECENT events oldest-first,
/// so the window is a tail — which is stated by the shape, not left for a reader
/// to infer.
///
/// **The `ddl_*` filter is load-bearing, not cosmetic.** `agent_session_events`
/// is the AGENT LOOP's conversation log, shared with the GDL engine: the loop
/// appends `user`, `assistant`, `tool_result` and `compaction` rows to the same
/// table. `session_log::replay` excludes only the `control:*` family, so
/// without this filter a route documented in three places as "the `ddl_*`
/// narrative appendix" would serve the model conversation transcript of any run
/// whose id also carried loop history. The read filters to the family it
/// promises, so the appendix can only ever be the delivery narrative.
fn read_event_log(conn: &Connection, run_id: i64) -> Result<EventLogRead, DeliveryError> {
    let cap = crate::workflow::session_log::REPLAY_CAP;
    let read = |limit: usize| -> Result<Vec<SessionEventRow>, DeliveryError> {
        let mut stmt = conn
            .prepare(
                "SELECT seq, kind, payload_json, created_at FROM agent_session_events \
                 WHERE run_id = ?1 AND kind GLOB 'ddl_*' ORDER BY seq DESC LIMIT ?2",
            )
            .map_err(|e| storage(format!("delivery event log read failed: {e}")))?;
        let mut rows = stmt
            .query_map(params![run_id, limit as i64], |r| {
                Ok(SessionEventRow {
                    seq: r.get(0)?,
                    kind: r.get(1)?,
                    payload_json: r.get(2)?,
                    created_at: r.get(3)?,
                })
            })
            .map_err(|e| storage(format!("delivery event log read failed: {e}")))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| storage(format!("delivery event log read failed: {e}")))?;
        // DESC + reverse: the tail window, oldest-first, prefix-stable.
        rows.reverse();
        Ok(rows)
    };
    let rows = read(cap)?;
    let truncated = rows.len() == cap && read(cap + 1)?.len() > cap;
    Ok(EventLogRead {
        rows: rows.into_iter().map(event_row_read).collect(),
        cap,
        truncated,
    })
}

fn event_row_read(e: SessionEventRow) -> EventRowRead {
    EventRowRead {
        seq: e.seq,
        kind: e.kind,
        payload_json: e.payload_json,
        created_at: e.created_at,
    }
}

/// The hand-mapped wire form of one diff. `StageDiff` gains no `Serialize`
/// derive — the crate is frozen this round — so the mapping lives here, once.
fn stage_diff_read(diff: &StageDiff) -> StageDiffRead {
    StageDiffRead {
        stage: diff.stage.clone(),
        // The order fold reuses a crate variant internally but has its OWN wire
        // code: the schema states the `input_digest` arms always agree, so
        // emitting `input_digest_differs` for an ordinal gap would contradict
        // the contract the same document publishes.
        mismatch: if diff.stage == "order" {
            ORDER_MISMATCH
        } else {
            mismatch_name(diff.mismatch)
        },
        recorded: diff.recorded.as_ref().map(stage_digest_read),
        rederived: diff.rederived.as_ref().map(stage_digest_read),
    }
}

fn stage_digest_read(d: &StageDigest) -> StageDigestRead {
    StageDigestRead {
        stage: d.stage.clone(),
        input_digest: d.input_digest.clone(),
        output_digest: d.output_digest.clone(),
    }
}

/// The wire name for a mismatch.
///
/// EXHAUSTIVE over the crate's five variants, and a sixth is a **COMPILE
/// ERROR**, not a runtime panic: `StageMismatch` is not `#[non_exhaustive]`, so
/// a new variant cannot be added without this match failing to build. That is
/// the stronger guarantee — a new mismatch cannot reach a client unlabelled, and
/// no deployed binary can crash here. A previous version of this comment
/// claimed a runtime panic, which described a failure that cannot occur and
/// would have misled the next reader into expecting a crash path.
fn mismatch_name(m: StageMismatch) -> &'static str {
    match m {
        StageMismatch::MissingRecorded => "missing_recorded",
        StageMismatch::MissingRederived => "missing_rederived",
        StageMismatch::StageDiffers => "stage_differs",
        StageMismatch::InputDigestDiffers => "input_digest_differs",
        StageMismatch::OutputDigestDiffers => "output_digest_differs",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migration;
    use crate::register_sqlite_vec::register_sqlite_vec;

    fn seed() -> Connection {
        register_sqlite_vec();
        let mut conn = Connection::open_in_memory().unwrap();
        run_migration(&mut conn, 1).unwrap();
        conn
    }

    fn open(conn: &mut Connection, tier: &str) -> Created {
        create_run(
            conn,
            &CreateRun {
                domain: "global",
                goal: "ship the thing",
                tier,
                policy_digest: None,
                config_digest: None,
                budgets: &[],
                now: 1,
            },
        )
        .expect("a delivery run opens")
    }

    fn advance_one(
        conn: &mut Connection,
        run_id: i64,
        rev: i64,
        to: &str,
    ) -> Result<Advanced, DeliveryError> {
        advance(
            conn,
            &Advance {
                run_id,
                expected_revision: rev,
                to_phase: to,
                artifact_refs: &[],
                artifact: None,
                model: None,
                actor: "tester",
                now: 2,
            },
        )
    }

    fn count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0)).unwrap_or(0)
    }

    /// The machine is forward-only and TOTAL: the five adjacent moves are the
    /// only legal ones, and every other pair is refused rather than guessed.
    #[test]
    fn delivery_phase_machine_is_forward_only_and_total() {
        let mut conn = seed();
        let legal = [
            ("scope", "design"),
            ("design", "build"),
            ("build", "release"),
            ("release", "operate"),
            ("operate", "done"),
        ];
        for (from, to) in legal {
            assert!(
                is_legal_phase_transition(Phase::parse(from).unwrap(), Phase::parse(to).unwrap()),
                "{from}->{to} is one of the five adjacent moves"
            );
        }
        // TOTAL: every non-adjacent pair is refused, including the rewind and
        // the same-phase hop.
        for from in Phase::ALL {
            for to in Phase::ALL {
                let expect = is_legal_phase_transition(from, to);
                assert_eq!(
                    expect,
                    legal.contains(&(from.as_str(), to.as_str())),
                    "the machine's verdict for {}->{} must be the closed set's",
                    from.as_str(),
                    to.as_str()
                );
            }
        }

        // ...and the CORE enforces it, not just the pure function.
        let run = open(&mut conn, "bounded-auto");
        assert_eq!(
            advance_one(&mut conn, run.run_id, 0, "build")
                .unwrap_err()
                .to_string(),
            "delivery_illegal_phase_transition:scope->build",
            "skipping a phase must be refused"
        );
        assert_eq!(
            advance_one(&mut conn, run.run_id, 0, "scope")
                .unwrap_err()
                .to_string(),
            "delivery_illegal_phase_transition:scope->scope",
            "a same-phase hop must be refused"
        );
    }

    /// The phase pass is one transaction: the step row, the CAS, the trace row,
    /// and the audit row all land, or none of them do.
    #[test]
    fn delivery_phase_advance_writes_step_row_cas_and_audit_atomically() {
        let mut conn = seed();
        // the key law: a phase pass signs its attestation link, and a pass with
        // no usable operator key REFUSES. A real seed is installed at
        // `BRAIN_UMP_KEY_DIR` so the shipped resolver runs for real; the
        // refusal half is pinned by `attestation_key_absence_refuses_the_phase_pass`.
        let _operator = crate::test_support::operator_key_guard();
        let run = open(&mut conn, "bounded-auto");
        let audits_before = count(&conn, "SELECT COUNT(*) FROM audit_events");

        let out = advance_one(&mut conn, run.run_id, 0, "design").expect("advance");

        assert_eq!(out.phase, "design");
        assert_eq!(out.state_revision, 1, "the CAS moved the revision by one");
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM workflow_steps WHERE run_id = 1 AND phase = 'design'"
            ),
            1,
            "the step row landed"
        );
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM delivery_traces WHERE run_id = 1 AND stage = 'phase' \
                   AND status = 'advanced'"
            ),
            1,
            "the trace row landed"
        );
        assert!(
            count(&conn, "SELECT COUNT(*) FROM audit_events") > audits_before,
            "the audit row landed in the same transaction"
        );
        // The revision is the real one, read back.
        let rev: i64 = conn
            .query_row(
                "SELECT state_revision FROM workflow_runs WHERE id = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rev, 1);
    }

    /// The load-bearing atomicity proof. A failing audit must leave NO step
    /// row, NO CAS, and NO trace row: a phase pass that lands without its
    /// evidence is a transition the audit chain cannot explain.
    #[test]
    fn delivery_phase_advance_rolls_back_completely_when_the_audit_fails() {
        let mut conn = seed();
        // the key law: a phase pass signs its attestation link, and a pass with
        // no usable operator key REFUSES. A real seed is installed at
        // `BRAIN_UMP_KEY_DIR` so the shipped resolver runs for real; the
        // refusal half is pinned by `attestation_key_absence_refuses_the_phase_pass`.
        let _operator = crate::test_support::operator_key_guard();
        let run = open(&mut conn, "bounded-auto");

        // Break the evidence substrate, not the write path: the audit INSERT
        // can no longer land.
        conn.execute_batch("DROP TABLE audit_events").unwrap();

        let err = advance_one(&mut conn, run.run_id, 0, "design")
            .expect_err("a pass whose audit cannot land must fail");
        assert!(
            err.to_string().starts_with("delivery_storage"),
            "the refusal names the storage boundary: {err}"
        );

        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM workflow_steps WHERE run_id = 1"
            ),
            0,
            "NO step row survived the failed audit"
        );
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM delivery_traces WHERE run_id = 1 AND stage = 'phase'"
            ),
            0,
            "NO trace row survived the failed audit"
        );
        let (rev, phase): (i64, String) = conn
            .query_row(
                "SELECT state_revision, state_json FROM workflow_runs WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(rev, 0, "the CAS was rolled back");
        assert!(
            phase.contains("\"scope\""),
            "the run never left its opening phase: {phase}"
        );
    }

    /// The CAS is fail-closed: a stale revision aborts the whole pass, and it
    /// is the `Continue`-past-stale precedent that is the counterexample, not
    /// the template.
    #[test]
    fn delivery_cas_is_fail_closed_on_a_stale_revision() {
        let mut conn = seed();
        // the key law: a phase pass signs its attestation link, and a pass with
        // no usable operator key REFUSES. A real seed is installed at
        // `BRAIN_UMP_KEY_DIR` so the shipped resolver runs for real; the
        // refusal half is pinned by `attestation_key_absence_refuses_the_phase_pass`.
        let _operator = crate::test_support::operator_key_guard();
        let run = open(&mut conn, "bounded-auto");
        advance_one(&mut conn, run.run_id, 0, "design").expect("first pass");

        // Revision is now 1; advance claiming revision 0 must lose.
        let err = advance_one(&mut conn, run.run_id, 0, "build")
            .expect_err("a stale revision must be refused");
        assert!(
            err.to_string().starts_with("delivery_gate_stale_revision"),
            "the refusal names the contention: {err}"
        );
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM workflow_steps WHERE run_id = 1 AND phase = 'build'"
            ),
            0,
            "the losing writer left no step row"
        );
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM delivery_traces WHERE run_id = 1 AND stage = 'phase'"
            ),
            1,
            "the losing writer left no trace row"
        );
    }

    /// The four lifecycle laws the DO names by name: the closed run-status set,
    /// the four normative routing keys, the phase vocabulary, and a run that
    /// still carries the CASE law's vocabulary after its phase passes.
    #[test]
    fn delivery_run_lifecycle_laws_unchanged() {
        let mut conn = seed();
        // the key law: a phase pass signs its attestation link, and a pass with
        // no usable operator key REFUSES. A real seed is installed at
        // `BRAIN_UMP_KEY_DIR` so the shipped resolver runs for real; the
        // refusal half is pinned by `attestation_key_absence_refuses_the_phase_pass`.
        let _operator = crate::test_support::operator_key_guard();
        let run = open(&mut conn, "propose");

        // 1. the closed run-status set — a delivery run is inside the SAME set
        //    the engine already uses, not a delivery-specific vocabulary.
        let status: String = conn
            .query_row("SELECT status FROM workflow_runs WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(
            [
                "active",
                "cancelled",
                "closed",
                "completed",
                "fired",
                "resolved"
            ]
            .contains(&status.as_str()),
            "status {status} must be inside the engine's closed set"
        );

        // 2. the four normative routing keys are untouched: the delivery state
        //    adds none of its own and removes none of the engine's.
        let state: String = conn
            .query_row(
                "SELECT state_json FROM workflow_runs WHERE id = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&state).unwrap();
        for key in ["status", "pending_question", "next_step", "next_state"] {
            assert!(
                !parsed.as_object().unwrap().contains_key(key) || key == "pending_question",
                "the delivery state must not shadow the engine routing key `{key}`"
            );
        }
        assert_eq!(parsed["phase"], "scope", "the phase lives in state_json");
        assert!(
            !parsed.as_object().unwrap().contains_key("law_version"),
            "law_version must never enter state_json"
        );

        // 3. the phase vocabulary is the DO's, closed.
        assert_eq!(parsed["phase"], "scope");
        assert_eq!(Phase::ALL.len(), 6);

        // 4. after a full walk the run CLOSES through the engine's set, at the
        //    terminal phase, and not before.
        let mut rev = 0i64;
        for to in ["design", "build", "release", "operate"] {
            rev += 1;
            advance_one(&mut conn, run.run_id, rev - 1, to).expect("walk");
        }
        let mid: String = conn
            .query_row("SELECT status FROM workflow_runs WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(mid, "active", "a run stays active until the terminal phase");
        advance_one(&mut conn, run.run_id, 4, "done").expect("terminal pass");
        let done: String = conn
            .query_row("SELECT status FROM workflow_runs WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(done, "completed", "the run closes inside the closed set");
        assert_eq!(
            advance_one(&mut conn, run.run_id, 5, "done")
                .unwrap_err()
                .to_string(),
            "delivery_terminal_phase:done",
            "the terminal phase is terminal"
        );
    }

    /// The gate is a DISPOSITION. It writes its trace and its audit and moves
    /// nothing else: the run's revision, status, and phase are untouched.
    #[test]
    fn delivery_gate_is_a_disposition_and_mutates_nothing() {
        let mut conn = seed();
        let run = open(&mut conn, "bounded-auto");
        let before: (i64, String, String) = conn
            .query_row(
                "SELECT state_revision, status, state_json FROM workflow_runs WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();

        let verdict = gates(
            &mut conn,
            &Gates {
                run_id: run.run_id,
                to_phase: Some("design"),
                actor: "tester",
                now: 5,
            },
        )
        .expect("the gate evaluates");
        assert_eq!(verdict.disposition, "allowed");
        assert_eq!(verdict.deny_reason, None);
        assert_eq!(
            verdict.trace_mode, "deterministic",
            "a promoting tier traces deterministically"
        );

        let after: (i64, String, String) = conn
            .query_row(
                "SELECT state_revision, status, state_json FROM workflow_runs WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(before, after, "the gate must not move the run");
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM delivery_traces WHERE run_id = 1 AND stage = 'gate'"
            ),
            1,
            "the gate leaves its disposition on the record"
        );
    }

    /// Deny wins, and a non-promoting tier asks rather than self-advancing.
    #[test]
    fn delivery_gate_denies_the_illegal_and_prompts_the_narrow() {
        let mut conn = seed();
        let run = open(&mut conn, "observe");
        let ask = gates(
            &mut conn,
            &Gates {
                run_id: run.run_id,
                to_phase: Some("design"),
                actor: "tester",
                now: 5,
            },
        )
        .unwrap();
        assert_eq!(
            ask.disposition, "prompt",
            "an observe tier asks; it does not advance"
        );
        assert_eq!(ask.deny_reason, None);

        let skip = gates(
            &mut conn,
            &Gates {
                run_id: run.run_id,
                to_phase: Some("release"),
                actor: "tester",
                now: 5,
            },
        )
        .unwrap();
        assert_eq!(skip.disposition, "denied");
        assert_eq!(
            skip.deny_reason.as_deref(),
            Some("illegal-phase-transition")
        );

        // A value outside the closed vocabulary is a DENIAL, never a
        // nearest-match guess and never a prompt.
        let unknown = gates(
            &mut conn,
            &Gates {
                run_id: run.run_id,
                to_phase: Some("shipped"),
                actor: "tester",
                now: 5,
            },
        )
        .unwrap();
        assert_eq!(unknown.disposition, "denied");
        assert_eq!(unknown.deny_reason.as_deref(), Some("closed-vocabulary"));
        assert_eq!(unknown.next_phase, None);
    }

    /// Absence is probe-blind and UNAUDITED: the core refuses before it writes,
    /// so the audit log never becomes an existence oracle for a probed id.
    #[test]
    fn delivery_absent_row_is_probe_blind_and_unaudited() {
        let mut conn = seed();
        let audits_before = count(&conn, "SELECT COUNT(*) FROM audit_events");

        for err in [
            advance_one(&mut conn, 424_242, 0, "design").unwrap_err(),
            gates(
                &mut conn,
                &Gates {
                    run_id: 424_242,
                    to_phase: Some("design"),
                    actor: "tester",
                    now: 5,
                },
            )
            .unwrap_err(),
            answer(
                &mut conn,
                &Answer {
                    run_id: 424_242,
                    expected_revision: 0,
                    answer: "yes",
                    actor: "tester",
                    now: 5,
                },
            )
            .unwrap_err(),
        ] {
            assert_eq!(
                err.to_string(),
                "delivery_run_not_found",
                "an absent run and a foreign run must be indistinguishable"
            );
        }
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM audit_events"),
            audits_before,
            "absence writes NO audit row — the chain must not record that the id was probed"
        );
    }

    /// The trace id is a CONTENT ADDRESS over the row's facts AND its ordinal
    /// in the run. The first run of this pin caught the real bug: two
    /// identical gate evaluations in the same second collided on
    /// `delivery_traces.id`. What a replay needs is DETERMINISM, not uniqueness of
    /// facts — replaying the same sequence on a fresh database must reproduce
    /// the same ids.
    #[test]
    fn delivery_trace_id_is_a_deterministic_content_address() {
        fn replay() -> (String, String, String) {
            let mut conn = seed();
            let run = open(&mut conn, "bounded-auto");
            let mut g = |now: i64| {
                gates(
                    &mut conn,
                    &Gates {
                        run_id: run.run_id,
                        to_phase: Some("design"),
                        actor: "tester",
                        now,
                    },
                )
                .unwrap()
                .trace_id
            };
            (g(7), g(7), g(8))
        }
        let (a1, b1, c1) = replay();
        let (a2, b2, c2) = replay();

        assert_eq!(
            a1, a2,
            "the same sequence of facts must reproduce the same ids"
        );
        assert_eq!(b1, b2);
        assert_eq!(c1, c2);
        assert_ne!(
            a1, b1,
            "an identical second evaluation is still a distinct row"
        );
        assert_ne!(a1, c1, "a different fact must address differently");
        assert!(
            a1.starts_with("trc_") && a1.len() == "trc_".len() + 32,
            "{a1}"
        );
        assert_eq!(
            count(
                &{
                    let mut conn = seed();
                    let run = open(&mut conn, "bounded-auto");
                    for now in [7, 7, 8] {
                        gates(
                            &mut conn,
                            &Gates {
                                run_id: run.run_id,
                                to_phase: Some("design"),
                                actor: "tester",
                                now,
                            },
                        )
                        .unwrap();
                    }
                    conn
                },
                "SELECT COUNT(*) FROM delivery_traces"
            ),
            4,
            "three gate rows plus the admission row, all distinct"
        );
    }

    /// The stored tier is the DESIGN OWNER's spelling, and the translation to
    /// the pure crate's `snake_case` is a closed bijection — not a
    /// normalization pass and not a nearest-match guess. The DO governs the
    /// wire and the column; the crate governs the pure semantics.
    #[test]
    fn delivery_tier_vocabulary_is_a_bijection_onto_the_design_owners() {
        let wire = ["observe", "propose", "bounded-auto", "delegated"];
        let mut seen = std::collections::BTreeSet::new();
        for w in wire {
            let core = tier_wire_to_core(w).unwrap_or_else(|e| panic!("{w} must parse: {e}"));
            assert_eq!(tier_core_to_wire(core), w, "{w} must round-trip");
            assert!(seen.insert(w), "the wire vocabulary must be a set");
        }
        assert_eq!(seen.len(), 4);
        // The crate's OWN spelling is never what we store.
        assert_eq!(tier_core_to_wire(AutonomyTier::BoundedAuto), "bounded-auto");
        assert_ne!(
            AutonomyTier::BoundedAuto.as_str(),
            "bounded-auto",
            "the crate still spells it snake_case — that divergence is what this boundary absorbs"
        );
        // And an unknown tier is a refusal, not a default.
        assert!(
            tier_wire_to_core("bounded_auto").is_err(),
            "the crate's spelling is NOT accepted on the wire"
        );
        assert!(tier_wire_to_core("auto").is_err());
        assert!(tier_wire_to_core("").is_err());

        // ...and the stored column carries the DO's spelling.
        let mut conn = seed();
        let run = open(&mut conn, "bounded-auto");
        let stored: String = conn
            .query_row(
                "SELECT tier FROM delivery_traces WHERE run_id = 1 AND stage = 'run'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            stored, "bounded-auto",
            "the column speaks the DO's vocabulary"
        );
        let _ = run;
    }

    /// Budget rows are STORED and never consulted. A run can be opened with
    /// every documented kind — including the unenforced one — and the gate's
    /// disposition is unchanged by any of them.
    #[test]
    fn delivery_budget_rows_store_without_enforcement() {
        let mut conn = seed();
        let budgets = vec![
            BudgetCeiling {
                kind: "tokens".into(),
                ceiling: 100,
            },
            BudgetCeiling {
                kind: "tool_calls".into(),
                ceiling: 5,
            },
            BudgetCeiling {
                kind: "files".into(),
                ceiling: 9,
            },
            BudgetCeiling {
                kind: "minutes".into(),
                ceiling: 30,
            },
            // Admitted by the kind CHECK because the DO names it. Nothing in
            // this round reads it: the disposition below is identical.
            BudgetCeiling {
                kind: "blast_radius".into(),
                ceiling: 1,
            },
        ];
        let run = create_run(
            &mut conn,
            &CreateRun {
                domain: "global",
                goal: "g",
                tier: "bounded-auto",
                policy_digest: None,
                config_digest: None,
                budgets: &budgets,
                now: 1,
            },
        )
        .unwrap();
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM delivery_budgets WHERE run_id = 1"
            ),
            5,
            "all five documented kinds are stored"
        );
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM delivery_budgets WHERE run_id = 1 AND spent != 0"
            ),
            0,
            "nothing is spent: an earlier round has no executor to spend against"
        );
        let verdict = gates(
            &mut conn,
            &Gates {
                run_id: run.run_id,
                to_phase: Some("design"),
                actor: "t",
                now: 2,
            },
        )
        .unwrap();
        assert_eq!(
            verdict.disposition, "allowed",
            "a stored ceiling must not become enforcement by accident"
        );
    }

    /// The stamp and the const move as ONE unit — this is the red-proof twin of
    /// the lockstep guard. It asserts the guard's two arms are both present and
    /// both named, so a future one-sided edit is caught by the guard rather
    /// than by this mirror.
    #[test]
    fn delivery_schema_stamp_and_const_move_as_one_atomic_unit() {
        let src =
            std::fs::read_to_string(format!("{}/src/migration.rs", env!("CARGO_MANIFEST_DIR")))
                .expect("read migration.rs");
        let values: Vec<&str> = src
            .match_indices("VALUES ('schema_version', '")
            .map(|(i, _)| {
                let rest = &src[i + "VALUES ('schema_version', '".len()..];
                &rest[..rest.find('\'').unwrap()]
            })
            .collect();
        let upserts: Vec<&str> = src
            .match_indices("DO UPDATE SET value = '")
            .map(|(i, _)| {
                let rest = &src[i + "DO UPDATE SET value = '".len()..];
                &rest[..rest.find('\'').unwrap()]
            })
            .filter(|v| src[..i_of(&src, v)].contains("schema_version"))
            .collect();
        assert_eq!(values.len(), 1, "exactly one VALUES arm stamps the version");
        assert_eq!(
            upserts.len(),
            1,
            "exactly one ON CONFLICT arm stamps the version"
        );
        assert_eq!(
            values[0],
            crate::storage_layout::LATEST_KNOWN_SCHEMA,
            "the VALUES arm and the const are one unit"
        );
        assert_eq!(
            upserts[0],
            crate::storage_layout::LATEST_KNOWN_SCHEMA,
            "the ON CONFLICT arm and the const are one unit — editing one and not \
             the other makes refuse_newer lie"
        );
    }

    /// Helper for the upsert arm: the byte offset of the literal's value.
    fn i_of(src: &str, v: &str) -> usize {
        src.find(&format!("DO UPDATE SET value = '{v}'"))
            .unwrap_or(0)
    }

    // ── the an earlier round seam: D2/D3 wiring + the typed-artifact proposal ──────────

    fn artifact(id: &str, content: &str) -> DeliveryArtifact {
        DeliveryArtifact {
            id: id.to_string(),
            content: content.to_string(),
            quality_gate: None,
        }
    }

    /// The typed artifact is a SHIPPED type, not an invention, and the two
    /// shipped digests agree. `consensus_core::Artifact::new` derives its hash
    /// as `sha256(content)`; `executor_core::artifact_hash` is the same digest
    /// as a function. If they ever diverge, the id recorded in the audit and
    /// the id an approver sees would be different digests of the same bytes —
    /// so this is a cross-crate consistency pin, not a restatement.
    #[test]
    fn delivery_typed_artifact_is_a_shipped_type() {
        let a = artifact("plan-1", "the plan body");
        let typed = a.typed();
        assert_eq!(typed.id, "plan-1");
        assert_eq!(typed.content, "the plan body");
        assert_eq!(
            typed.hash,
            brain_executor_core::artifact_hash("the plan body"),
            "the consensus shape and the executor digest fn must agree byte for byte"
        );
        assert_eq!(typed.hash.len(), 64, "sha256 hex is 64 chars");
    }

    /// The seam lands the proposal, the trace, and the audit as ONE transition.
    /// A caller that reads a proposal id back has evidence that all three
    /// committed; a partial write would be a proposal nobody can audit.
    #[test]
    fn delivery_proposal_seam_lands_in_one_transaction() {
        let mut conn = seed();
        // the key law: a phase pass signs its attestation link, and a pass with
        // no usable operator key REFUSES. A real seed is installed at
        // `BRAIN_UMP_KEY_DIR` so the shipped resolver runs for real; the
        // refusal half is pinned by `attestation_key_absence_refuses_the_phase_pass`.
        let _operator = crate::test_support::operator_key_guard();
        let created = open(&mut conn, "propose");
        let a = artifact("plan-1", "the plan body");
        let advanced = advance(
            &mut conn,
            &Advance {
                run_id: created.run_id,
                expected_revision: 0,
                to_phase: "design",
                artifact_refs: &[],
                artifact: Some(&a),
                model: None,
                actor: "tester",
                now: 2,
            },
        )
        .expect("the seam pass lands");

        assert!(advanced.proposal_id > 0, "the pass filed a proposal");
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM proposals WHERE kind = 'delivery/artifact'"
            ),
            1,
            "exactly one delivery artifact proposal"
        );
        let (content, status): (String, String) = conn
            .query_row(
                "SELECT content, status FROM proposals WHERE id = ?1",
                params![advanced.proposal_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(content, "the plan body", "content is stored verbatim");
        assert_eq!(
            status, "pending",
            "a proposal lands PENDING — the executor proposes, only the gate disposes"
        );
        // the trace and the audit rode the same pass. The audit chain stores
        // HASHES, not the detail text, so the row is proved by recomputing the
        // detail hash — a `LIKE` over a plaintext `detail` column would query a
        // column that does not exist and read back 0 rows.
        assert!(count(&conn, "SELECT COUNT(*) FROM delivery_traces") >= 2);
        let expected_detail = format!(
            "delivery phase pass scope->design tier=propose refs=0 status=active \
             proposal={proposal_id}",
            proposal_id = advanced.proposal_id
        );
        let detail_hash: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE detail_hash = ?1",
                params![crate::audit::hash(&expected_detail)],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            detail_hash, 1,
            "the pass audit names the proposal it filed, and it is the last \
             statement before the commit"
        );
    }

    /// The commit-or-rollback law, proven by a poisoned statement. A seam that
    /// filed the proposal and THEN failed would leave a reviewable artifact
    /// citing a phase pass that never happened.
    #[test]
    fn delivery_proposal_and_trace_commit_or_roll_back_together() {
        let mut conn = seed();
        // the key law: a phase pass signs its attestation link, and a pass with
        // no usable operator key REFUSES. A real seed is installed at
        // `BRAIN_UMP_KEY_DIR` so the shipped resolver runs for real; the
        // refusal half is pinned by `attestation_key_absence_refuses_the_phase_pass`.
        let _operator = crate::test_support::operator_key_guard();
        let created = open(&mut conn, "propose");
        let a = artifact("plan-1", "the plan body");

        // A trigger that poisons the audit insert: the proposal and the trace
        // are written BEFORE it, so a non-rollback implementation leaves both.
        conn.execute_batch(
            "CREATE TRIGGER poison_delivery_audit BEFORE INSERT ON audit_events
             WHEN NEW.detail LIKE '%delivery phase pass%'
             BEGIN SELECT RAISE(ABORT, 'poisoned'); END;",
        )
        .unwrap();

        let err = advance(
            &mut conn,
            &Advance {
                run_id: created.run_id,
                expected_revision: 0,
                to_phase: "design",
                artifact_refs: &[],
                artifact: Some(&a),
                model: None,
                actor: "tester",
                now: 2,
            },
        )
        .expect_err("the poisoned audit must fail the whole pass");
        assert!(matches!(err, DeliveryError::Storage(_)), "got {err:?}");

        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM proposals"),
            0,
            "the proposal rolled back with the pass"
        );
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM delivery_traces"),
            1,
            "only the admission trace survives; the phase trace rolled back"
        );
        let revision: i64 = conn
            .query_row(
                "SELECT state_revision FROM workflow_runs WHERE id = ?1",
                params![created.run_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(revision, 0, "the CAS rolled back with the pass");
    }

    /// The round's structural property: an executor-produced artifact has NO
    /// write path to a disposition. It files a PENDING proposal and stops. The
    /// gate's column is not reachable from this seam — not by parameter, not by
    /// a defaulted value, not by a later write in the same transaction.
    #[test]
    fn delivery_executor_has_no_write_path_to_gate_disposition() {
        let mut conn = seed();
        // the key law: a phase pass signs its attestation link, and a pass with
        // no usable operator key REFUSES. A real seed is installed at
        // `BRAIN_UMP_KEY_DIR` so the shipped resolver runs for real; the
        // refusal half is pinned by `attestation_key_absence_refuses_the_phase_pass`.
        let _operator = crate::test_support::operator_key_guard();
        let created = open(&mut conn, "propose");
        let a = artifact("plan-1", "the plan body");
        let advanced = advance(
            &mut conn,
            &Advance {
                run_id: created.run_id,
                expected_revision: 0,
                to_phase: "design",
                artifact_refs: &[],
                artifact: Some(&a),
                model: None,
                actor: "tester",
                now: 2,
            },
        )
        .unwrap();

        // 1. The proposal carries no disposition, and its status is the queue's.
        let status: String = conn
            .query_row(
                "SELECT status FROM proposals WHERE id = ?1",
                params![advanced.proposal_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "pending");

        // 2. The seam wrote NO approval evidence: no decided_at, and no digest
        //    bound to an approver.
        let decided: Option<i64> = conn
            .query_row(
                "SELECT decided_at FROM proposals WHERE id = ?1",
                params![advanced.proposal_id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(decided.is_none(), "an artifact is never self-decided");

        // 3. The run's own status is the ENGINE's, not the executor's: the
        //    phase pass set `active`, and nothing in the seam chose it.
        let (run_status, disposition): (String, String) = conn
            .query_row(
                "SELECT r.status, t.status FROM workflow_runs r
                 JOIN delivery_traces t ON t.run_id = r.id
                 WHERE r.id = ?1 AND t.stage = 'phase'",
                params![created.run_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(run_status, "active");
        assert_eq!(
            disposition, "advanced",
            "the trace records that a pass happened, not that anything was approved"
        );
    }

    /// The four normative routing keys are untouched by the seam. `status`,
    /// `pending_question`, `next_step`, `next_state` belong to the engine; an
    /// artifact rides the pass without moving any of them beyond the documented
    /// phase advance.
    #[test]
    fn delivery_executor_cannot_move_a_normative_routing_key() {
        let mut conn = seed();
        // the key law: a phase pass signs its attestation link, and a pass with
        // no usable operator key REFUSES. A real seed is installed at
        // `BRAIN_UMP_KEY_DIR` so the shipped resolver runs for real; the
        // refusal half is pinned by `attestation_key_absence_refuses_the_phase_pass`.
        let _operator = crate::test_support::operator_key_guard();
        let created = open(&mut conn, "propose");
        let before: (String, String) = conn
            .query_row(
                "SELECT status, state_json FROM workflow_runs WHERE id = ?1",
                params![created.run_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        let before_state: DeliveryState = decode_state(&before.1).unwrap();
        assert!(before_state.pending_question.is_none());

        let a = artifact("plan-1", "the plan body");
        advance(
            &mut conn,
            &Advance {
                run_id: created.run_id,
                expected_revision: 0,
                to_phase: "design",
                artifact_refs: &[],
                artifact: Some(&a),
                model: None,
                actor: "tester",
                now: 2,
            },
        )
        .unwrap();

        let after: (String, String) = conn
            .query_row(
                "SELECT status, state_json FROM workflow_runs WHERE id = ?1",
                params![created.run_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        let after_state: DeliveryState = decode_state(&after.1).unwrap();
        assert_eq!(after.0, "active", "the seam never closes or cancels a run");
        assert_eq!(
            after_state.pending_question, before_state.pending_question,
            "an artifact does not ask a question"
        );
        assert_eq!(after_state.attempt, before_state.attempt + 1);
        assert_eq!(after_state.tier, before_state.tier, "autonomy never widens");
    }

    /// The D2/D3 QA gate is CONSUMED, not reimplemented: advancing into
    /// `build` (D3) with a quality gate runs the shipped executor validator,
    /// and an artifact whose evidence is not a live surface is refused before
    /// anything is written. This is the round's actual engine consumption.
    #[test]
    fn delivery_d3_quality_gate_is_consumed_fail_closed() {
        let mut conn = seed();
        // the key law: a phase pass signs its attestation link, and a pass with
        // no usable operator key REFUSES. A real seed is installed at
        // `BRAIN_UMP_KEY_DIR` so the shipped resolver runs for real; the
        // refusal half is pinned by `attestation_key_absence_refuses_the_phase_pass`.
        let _operator = crate::test_support::operator_key_guard();
        let created = open(&mut conn, "propose");
        advance_one(&mut conn, created.run_id, 0, "design").unwrap();

        // No live-surface evidence → refused, and nothing landed.
        let no_evidence = DeliveryArtifact {
            id: "impl-1".into(),
            content: "the implementation".into(),
            quality_gate: Some(
                r#"{"executorQa":{"contractCoverage":"x","surfaceEvidence":[],"adversarialCases":[],"artifactRefs":[],"iteration":1}}"#
                    .into(),
            ),
        };
        let err = advance(
            &mut conn,
            &Advance {
                run_id: created.run_id,
                expected_revision: 1,
                to_phase: "build",
                artifact_refs: &[],
                artifact: Some(&no_evidence),
                model: None,
                actor: "tester",
                now: 3,
            },
        )
        .expect_err("an artifact with no live-surface evidence must not advance");
        assert!(
            matches!(err, DeliveryError::QualityGate { .. }),
            "the refusal is the named QA refusal, got {err:?}"
        );
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM proposals"), 0);

        // Live-surface evidence → the pass lands.
        let with_evidence = DeliveryArtifact {
            id: "impl-1".into(),
            content: "the implementation".into(),
            quality_gate: Some(
                r#"{"executorQa":{"contractCoverage":"x","surfaceEvidence":[{"kind":"cli","receipt":"exit 0"}],"adversarialCases":[],"artifactRefs":[],"iteration":1}}"#
                    .into(),
            ),
        };
        let advanced = advance(
            &mut conn,
            &Advance {
                run_id: created.run_id,
                expected_revision: 1,
                to_phase: "build",
                artifact_refs: &[],
                artifact: Some(&with_evidence),
                model: None,
                actor: "tester",
                now: 3,
            },
        )
        .expect("evidence-backed artifact advances");
        assert_eq!(advanced.phase, "build");
        assert!(advanced.proposal_id > 0);
    }

    /// The `ddl_*` session-log family round-trips through the REAL append and
    /// read-back, and is NOT the reserved `control:` family. The harness pin
    /// round-trips only one of its three kinds; this one proves every kind the
    /// delivery seam writes is representable AND visible to replay (which
    /// filters `control:*`).
    #[test]
    fn delivery_ddl_kinds_round_trip_and_avoid_control() {
        let mut conn = seed();
        // the key law: a phase pass signs its attestation link, and a pass with
        // no usable operator key REFUSES. A real seed is installed at
        // `BRAIN_UMP_KEY_DIR` so the shipped resolver runs for real; the
        // refusal half is pinned by `attestation_key_absence_refuses_the_phase_pass`.
        let _operator = crate::test_support::operator_key_guard();
        let created = open(&mut conn, "propose");
        let a = artifact("plan-1", "the plan body");
        advance(
            &mut conn,
            &Advance {
                run_id: created.run_id,
                expected_revision: 0,
                to_phase: "design",
                artifact_refs: &[],
                artifact: Some(&a),
                model: None,
                actor: "tester",
                now: 2,
            },
        )
        .unwrap();

        let kind = DDL_ARTIFACT_KIND;
        assert!(!kind.is_empty());
        assert!(
            !kind.starts_with("control:"),
            "the control: family stays reserved: {kind}"
        );
        assert!(
            kind.starts_with("ddl_"),
            "the family prefix is ddl_: {kind}"
        );

        // The pass really appended the narrative row, and replay sees it.
        let replayed = crate::workflow::session_log::replay(
            &conn,
            created.run_id,
            crate::workflow::session_log::REPLAY_CAP,
        )
        .unwrap();
        assert!(
            replayed.iter().any(|r| r.kind == DDL_ARTIFACT_KIND),
            "the ddl_ narrative row must be visible to replay — control:* would not be"
        );
    }

    /// A phase pass with NO artifact is unchanged: the seam is additive, so
    /// every an earlier round caller that passes no artifact must still work byte for byte.
    #[test]
    fn delivery_advance_without_an_artifact_is_unchanged() {
        let mut conn = seed();
        // the key law: a phase pass signs its attestation link, and a pass with
        // no usable operator key REFUSES. A real seed is installed at
        // `BRAIN_UMP_KEY_DIR` so the shipped resolver runs for real; the
        // refusal half is pinned by `attestation_key_absence_refuses_the_phase_pass`.
        let _operator = crate::test_support::operator_key_guard();
        let created = open(&mut conn, "propose");
        let advanced = advance_one(&mut conn, created.run_id, 0, "design").unwrap();
        assert_eq!(advanced.phase, "design");
        assert_eq!(advanced.proposal_id, 0, "no artifact, no proposal");
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM proposals"), 0);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM delivery_traces"), 2);
    }

    // ── the attestation round: the attestation chain ──────────────────────────────────────────

    /// Seed a `decision_model_registry` row the R29 citation can resolve. The
    /// binding is looked up by (id, config_digest, kind) and the artifact digest
    /// is the model BYTES digest — a name without a digest is not evidence.
    fn seed_model(
        conn: &Connection,
        id: &str,
        config_digest: &str,
        status: &str,
        artifact_digest: Option<&str>,
    ) {
        // The registry's digests are BARE lowercase 64-hex (its own
        // `is_registry_lower_hex_digest` law), not `sha256:`-prefixed.
        conn.execute(
            "INSERT INTO decision_model_registry(id, version, kind, name, output_vocabulary, \
             artifact_digest, config_digest, calibration_ref, status, evaluation_refs, \
             proposed_by, approved_by, created_at, updated_at) \
             VALUES (?1,'1',?2,'m',?3,?4,?5,NULL,?6,'[]','tester','tester',1,1)",
            params![
                id,
                crate::workflow::registry::KIND_DETERMINISTIC_RULES,
                "[\"choice\"]",
                artifact_digest,
                config_digest,
                status,
            ],
        )
        .expect("seed a registry row");
    }

    fn advance_with_model(
        conn: &mut Connection,
        run_id: i64,
        rev: i64,
        to: &str,
        model: Option<&ModelBinding>,
    ) -> Result<Advanced, DeliveryError> {
        advance(
            conn,
            &Advance {
                run_id,
                expected_revision: rev,
                to_phase: to,
                artifact_refs: &[],
                artifact: None,
                model,
                actor: "tester",
                now: 2,
            },
        )
    }

    /// A7, fail-closed. A pass with NO key at all refuses, and a pass whose key
    /// is PRESENT BUT UNUSABLE refuses too — and the two refuse differently,
    /// because "there is no key" and "the key is unreadable" are different
    /// operator problems. Neither degrades into an unsigned link.
    ///
    /// This is pinned against the SHIPPED resolver (a real key directory, a
    /// real 0600 check, a real size check) rather than a test seam, because the
    /// seam would be the thing under test.
    #[test]
    fn attestation_key_absence_refuses_the_phase_pass() {
        // Arm 1: Ok(None) — the key directory exists and holds no seed.
        let mut conn = seed();
        {
            let _empty = crate::test_support::empty_key_dir_guard();
            let run = open(&mut conn, "bounded-auto");
            let err = advance_one(&mut conn, run.run_id, 0, "design")
                .expect_err("a pass with no key must refuse");
            assert!(
                matches!(err, DeliveryError::AttestationRefused { .. }),
                "an absent key is the attestation's refusal: {err:?}"
            );
            assert_eq!(
                err.to_string(),
                "delivery_attestation_refused:operator_key_absent",
                "the refusal names the cause"
            );
            // Nothing landed: the step row, the CAS, and the trace are one
            // transaction and the refusal rolls all of them back.
            assert_eq!(count(&conn, "SELECT COUNT(*) FROM workflow_steps"), 0);
            assert_eq!(
                count(
                    &conn,
                    "SELECT COUNT(*) FROM delivery_traces WHERE stage='phase'"
                ),
                0
            );
            assert_eq!(
                count(&conn, "SELECT COUNT(*) FROM delivery_attestations"),
                0
            );
        }

        // Arm 2: Err — the seed is present but the wrong size, which the
        // resolver refuses LOUDLY rather than skipping.
        let mut conn = seed();
        {
            let _wrong = crate::test_support::wrong_size_key_dir_guard();
            let run = open(&mut conn, "bounded-auto");
            let err = advance_one(&mut conn, run.run_id, 0, "design")
                .expect_err("a pass with an unusable key must refuse");
            assert!(
                err.to_string()
                    .starts_with("delivery_attestation_refused:operator_key_refused"),
                "a REFUSED key is distinct from an absent one — the Err arm is propagated, \
                 never collapsed into None. Got: {err}"
            );
            assert_eq!(
                count(&conn, "SELECT COUNT(*) FROM delivery_attestations"),
                0
            );
        }
    }

    /// A3: `advance()` is the ONLY chain writer. The admission, the answer, and
    /// the gate each write a trace row and none of them appends a link — they
    /// only READ the head. One writer, one place to audit.
    #[test]
    fn attestation_advance_is_the_only_chain_writer() {
        let mut conn = seed();
        let _operator = crate::test_support::operator_key_guard();
        let run = open(&mut conn, "bounded-auto");
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM delivery_attestations"),
            0,
            "the admission writes no link — the chain starts at the first phase pass"
        );

        gates(
            &mut conn,
            &Gates {
                run_id: run.run_id,
                to_phase: Some("design"),
                actor: "tester",
                now: 2,
            },
        )
        .expect("the gate evaluates");
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM delivery_attestations"),
            0,
            "a gate evaluation is a DISPOSITION, never an attestation"
        );

        advance_one(&mut conn, run.run_id, 0, "design").expect("the phase pass");
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM delivery_attestations"),
            1,
            "the phase pass appends exactly one link"
        );
    }

    /// DO invariant 1: an attestation is EVIDENCE OF WHO ACTED, never a
    /// disposition. A denial attests nothing, a prompt attests nothing, and no
    /// route reads a link to decide anything. Proven behaviourally: a denying
    /// gate writes no link, and the run's routing keys are untouched by the
    /// presence of a link.
    #[test]
    fn attestation_is_not_a_disposition() {
        let mut conn = seed();
        let _operator = crate::test_support::operator_key_guard();
        let run = open(&mut conn, "bounded-auto");
        // A skip is a DENY. It must leave no attestation behind — a link records
        // that a phase pass happened, and no phase pass happened.
        let verdict = gates(
            &mut conn,
            &Gates {
                run_id: run.run_id,
                to_phase: Some("done"),
                actor: "tester",
                now: 2,
            },
        )
        .expect("the gate evaluates");
        assert_eq!(verdict.disposition, "denied", "scope->done is not adjacent");
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM delivery_attestations"),
            0,
            "a denial is not an attestation"
        );

        // And the link carries no disposition vocabulary at all: the predicate
        // is a fixed 13-field shape whose only free field is the closed tier.
        advance_one(&mut conn, run.run_id, 0, "design").expect("the phase pass");
        let envelope: String = conn
            .query_row(
                "SELECT envelope_json FROM delivery_attestations WHERE run_id = ?1",
                params![run.run_id],
                |r| r.get(0),
            )
            .expect("the link is stored");
        let parsed: serde_json::Value =
            serde_json::from_str(&envelope).expect("the envelope is JSON");
        let keys: Vec<&str> = parsed
            .as_object()
            .expect("an envelope is an object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            vec![
                "config_digest",
                "created_at",
                "integrity",
                "parent_id",
                "policy_digest",
                "predicate",
                "predicate_type",
                "run_id",
                "signer_did",
                "step_id",
                "subject_digest",
                "subject_name",
                "v",
            ],
            "the envelope's keys are a fixed shape — no disposition, no routing key, no status"
        );
        assert!(
            !envelope.contains("allowed")
                && !envelope.contains("denied")
                && !envelope.contains("prompt"),
            "no gate disposition vocabulary may appear in a signed envelope"
        );
    }

    /// The four trace sites all write a real `attestation_root` (A3) — closing
    /// the an earlier round/an earlier round debt where all four hard-coded `None`. The admission and
    /// the gate read a head that does not exist yet, so theirs are `None` for
    /// an honest reason: there IS no head. The pass that appends names itself.
    #[test]
    fn delivery_trace_attestation_root_is_written() {
        let mut conn = seed();
        let _operator = crate::test_support::operator_key_guard();
        let run = open(&mut conn, "bounded-auto");
        let head = |conn: &Connection, stage: &str| -> Option<String> {
            conn.query_row(
                "SELECT attestation_root FROM delivery_traces \
                 WHERE run_id = ?1 AND stage = ?2 ORDER BY seq DESC LIMIT 1",
                params![run.run_id, stage],
                |r| r.get::<_, Option<String>>(0),
            )
            .expect("read the root")
        };
        assert_eq!(
            head(&conn, "run"),
            None,
            "the admission trace predates every link: there is no head yet, and None is the \
             honest value rather than a forged one"
        );
        advance_one(&mut conn, run.run_id, 0, "design").expect("the phase pass");
        assert_eq!(
            head(&conn, "phase"),
            None,
            "the first pass's trace row is written BEFORE its own link is appended, so it \
             names the head that existed BEFORE it — which is none. Naming itself would be \
             circular."
        );
        let first = conn
            .query_row(
                "SELECT id FROM delivery_attestations WHERE run_id = ?1",
                params![run.run_id],
                |r| r.get::<_, String>(0),
            )
            .expect("the first link");
        assert!(
            first.starts_with("att_"),
            "the link id is content-addressed: {first}"
        );

        // A second pass: the trace row names the PREVIOUS link, because the chain
        // is appended after the trace. The chain itself still resolves.
        advance_one(&mut conn, run.run_id, 1, "build").expect("the second pass");
        assert_eq!(
            head(&conn, "phase"),
            Some(first.clone()),
            "the second pass names the head that existed before it"
        );
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM delivery_attestations"),
            2,
            "two passes, two links"
        );
        let rows =
            crate::workflow::attestations::read_chain(&conn, run.run_id).expect("read the chain");
        let verdict =
            crate::workflow::attestations::verify_chain(&rows, 3).expect("a readable chain");
        assert!(
            verdict.verified,
            "the chain the trace rows point into verifies: {verdict:?}"
        );
    }

    /// A11: the ordinal is `MAX(seq)+1`, so it stays unique and monotonic even
    /// after a row is removed — which is exactly where `COUNT(*)` failed, and
    /// this is the case the existing content-address pin cannot reach because
    /// it never deletes a row.
    #[test]
    fn delivery_trace_seq_is_unique_and_monotonic() {
        let mut conn = seed();
        let _operator = crate::test_support::operator_key_guard();
        let run = open(&mut conn, "bounded-auto");
        advance_one(&mut conn, run.run_id, 0, "design").expect("the first pass");
        advance_one(&mut conn, run.run_id, 1, "build").expect("the second pass");
        advance_one(&mut conn, run.run_id, 2, "release").expect("the third pass");

        let seqs = |conn: &Connection| -> Vec<i64> {
            let mut stmt = conn
                .prepare("SELECT seq FROM delivery_traces WHERE run_id = ?1 ORDER BY seq")
                .expect("prepare");
            let rows = stmt
                .query_map(params![run.run_id], |r| r.get::<_, i64>(0))
                .expect("query");
            rows.map(std::result::Result::unwrap).collect()
        };
        assert_eq!(
            seqs(&conn),
            vec![1, 2, 3, 4],
            "the admission plus three passes, 1-based"
        );
        let distinct = count(&conn, "SELECT COUNT(DISTINCT seq) FROM delivery_traces");
        let total = count(&conn, "SELECT COUNT(*) FROM delivery_traces");
        assert_eq!(
            distinct, total,
            "seq is unique inside a run — the (run_id, seq) UNIQUE index is not decorative"
        );

        // Remove a middle row. `COUNT(*)` would reissue 3 and collide on the
        // (run_id, seq) UNIQUE index; `MAX(seq)+1` does not.
        conn.execute(
            "DELETE FROM delivery_traces WHERE run_id = ?1 AND seq = 2",
            params![run.run_id],
        )
        .expect("remove the middle trace row");
        assert_eq!(seqs(&conn), vec![1, 3, 4], "the gap is real");
        let advanced = advance_one(&mut conn, run.run_id, 3, "operate").expect("the fourth pass");
        assert!(!advanced.trace_id.is_empty());
        assert_eq!(
            seqs(&conn),
            vec![1, 3, 4, 5],
            "the next ordinal is MAX(seq)+1, never COUNT(*) — no collision after a delete"
        );
    }

    /// A11: `content_id` folds the STORED seq, so the same facts at a different
    /// position address differently, and a re-derivation from the stored row
    /// reproduces the stored id. That re-derivation is a later round's law; the attestation round stores
    /// the input it needs.
    #[test]
    fn delivery_trace_content_id_folds_the_stored_seq() {
        let mut conn = seed();
        let _operator = crate::test_support::operator_key_guard();
        let run = open(&mut conn, "bounded-auto");
        advance_one(&mut conn, run.run_id, 0, "design").expect("the pass");

        let row: (String, i64) = conn
            .query_row(
                "SELECT id, seq FROM delivery_traces WHERE run_id = ?1 AND stage = 'phase'",
                params![run.run_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("read the phase trace row");
        let rebuilt = TraceRow::read_back(&conn, &row.0)
            .expect("the row reads back")
            .expect("the row exists");
        assert_eq!(
            rebuilt.content_id(),
            row.0,
            "content_id is re-derivable from the stored row alone — the stored seq IS the input"
        );
        assert!(
            row.0.starts_with("trc_") && row.0.len() == "trc_".len() + 32,
            "the id shape is unchanged: {row:?}"
        );
        // A different position over identical facts addresses differently.
        let mut moved = rebuilt.clone();
        moved.seq = row.1 + 1;
        assert_ne!(
            moved.content_id(),
            row.0,
            "the same facts at another ordinal are another row — that is why seq is folded in"
        );
    }

    /// A6: the model citation is digest-pinned. A binding that resolves carries
    /// the registry row's ARTIFACT digest in the signed predicate — a name
    /// without the bytes it names is not evidence.
    #[test]
    fn attestation_carries_the_digest_pinned_model() {
        let mut conn = seed();
        let _operator = crate::test_support::operator_key_guard();
        let digest = "b".repeat(64);
        let config = "c".repeat(64);
        seed_model(
            &conn,
            "mb-elastic",
            &config,
            crate::workflow::registry::STATUS_PROMOTED,
            Some(&digest),
        );
        let run = open(&mut conn, "bounded-auto");
        let binding = ModelBinding {
            key: "rules:mb-elastic".to_string(),
            config_digest: config.clone(),
        };
        advance_with_model(&mut conn, run.run_id, 0, "design", Some(&binding))
            .expect("the bound pass");

        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM delivery_traces WHERE model_ref = 'rules:mb-elastic'"
            ),
            1,
            "the trace row names the model that acted"
        );
        let (model_ref, config_digest): (String, String) = conn
            .query_row(
                "SELECT model_ref, config_digest FROM delivery_traces \
                 WHERE model_ref IS NOT NULL",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("the bound trace row");
        assert_eq!(
            model_ref, "rules:mb-elastic",
            "the citation is the registry key shape"
        );
        assert_eq!(
            config_digest, config,
            "the trace row carries the binding's config digest"
        );
        // The envelope lives on the CHAIN table, not the trace table.
        let envelope: String = conn
            .query_row(
                "SELECT envelope_json FROM delivery_attestations WHERE run_id = ?1",
                params![run.run_id],
                |r| r.get(0),
            )
            .expect("the link's envelope");
        let parsed: serde_json::Value = serde_json::from_str(&envelope).expect("the envelope");
        assert_eq!(
            parsed["predicate"]["model_ref"],
            serde_json::json!("rules:mb-elastic"),
            "the signed predicate carries the model reference"
        );
        assert_eq!(
            parsed["predicate"]["model_digest"],
            serde_json::json!(digest),
            "the signed predicate carries the model BYTES digest"
        );
        assert_eq!(
            parsed["config_digest"],
            serde_json::json!(config),
            "the binding's config digest is a SIGNED SIBLING key, not a predicate field"
        );
    }

    /// A6: a registry row that resolves but carries NO artifact digest is
    /// refused. The row exists, the name is real, and there is still nothing
    /// that says which bytes acted — so the citation is incomplete and the pass
    /// refuses rather than attesting a name.
    #[test]
    fn attestation_refuses_a_model_without_artifact_digest() {
        let mut conn = seed();
        let _operator = crate::test_support::operator_key_guard();
        seed_model(
            &conn,
            "mb-nodigest",
            &"c".repeat(64),
            crate::workflow::registry::STATUS_PROMOTED,
            None,
        );
        let run = open(&mut conn, "bounded-auto");
        let binding = ModelBinding {
            key: "rules:mb-nodigest".to_string(),
            config_digest: "c".repeat(64),
        };
        let err = advance_with_model(&mut conn, run.run_id, 0, "design", Some(&binding))
            .expect_err("a name with no bytes must refuse");
        assert_eq!(
            err.to_string(),
            "delivery_model_digest_missing",
            "the refusal is its own code, distinct from the three registry refusals"
        );
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM delivery_attestations"),
            0
        );
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM workflow_steps"),
            0,
            "the refusal rolls the whole pass back"
        );
    }

    /// §4.4: the link and its audit row are one transaction. Dropping the
    /// evidence substrate must leave NO link, NO step row, and NO trace row — a
    /// phase pass that landed without its signed evidence would be a transition
    /// the audit chain cannot explain. This EXTENDS the an earlier round rollback pin.
    #[test]
    fn attestation_audit_row_is_written_last_and_rolls_back_together() {
        let mut conn = seed();
        let _operator = crate::test_support::operator_key_guard();
        let run = open(&mut conn, "bounded-auto");
        conn.execute_batch("DROP TABLE audit_events").unwrap();
        let err = advance_one(&mut conn, run.run_id, 0, "design")
            .expect_err("a pass whose audit cannot land must fail");
        assert!(
            err.to_string().starts_with("delivery_storage"),
            "the refusal names the boundary: {err}"
        );
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM delivery_attestations"),
            0,
            "a link that commits without its audit row would be unattested evidence"
        );
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM workflow_steps"), 0);
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM delivery_traces WHERE stage='phase'"
            ),
            0
        );
    }

    /// The pass that DOES land carries the link's OWN evidence row, targeting
    /// the link (A8). `audit_events.target_hash` is a hash of the target, so
    /// the assertion recomputes it — a `LIKE` over a plaintext column would
    /// query a column that does not exist and read back zero rows.
    ///
    /// The pass writes more than two audit rows in total: `state::cas_update`
    /// emits its own under the module's "audit-per-write, the fence holds of the
    /// FUNCTION" law, and that is pre-existing. What the attestation round adds is exactly one
    /// row, the link's, and this proves it by target rather than by counting.
    #[test]
    fn attestation_audit_row_covers_the_link_write() {
        let mut conn = seed();
        let _operator = crate::test_support::operator_key_guard();
        let run = open(&mut conn, "bounded-auto");
        advance_one(&mut conn, run.run_id, 0, "design").expect("the pass");

        let (attestation_id, chain_hash): (String, String) = conn
            .query_row(
                "SELECT id, chain_hash FROM delivery_attestations WHERE run_id = ?1",
                params![run.run_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("the link");
        assert!(
            attestation_id.starts_with("att_"),
            "the link id is content-addressed"
        );

        assert_eq!(
            count(
                &conn,
                &format!(
                    "SELECT COUNT(*) FROM audit_events WHERE target_hash = '{}'",
                    crate::audit::hash(&format!(
                        "delivery/attestation/{}/{}",
                        run.run_id, attestation_id
                    ))
                )
            ),
            1,
            "the link write's evidence row targets the link itself — one audit row per write, \
             inside the caller's transaction"
        );
        assert_eq!(
            count(
                &conn,
                &format!(
                    "SELECT COUNT(*) FROM audit_events WHERE detail_hash = '{}'",
                    crate::audit::hash(&format!(
                        "delivery attestation linked run={} step=1 parent=root chain_hash={} \
                         subject=delivery/phase/design",
                        run.run_id, chain_hash
                    ))
                )
            ),
            1,
            "the link's row names the chain hash and the derived subject — evidence, not a \
             disposition"
        );
    }

    // ── R41: the replay read surface ───────────────────────────────────────

    /// A run with several trace rows, each recorded through the real writers, so
    /// the read surface is exercised over rows production actually produced
    /// rather than over hand-inserted fixtures.
    fn seeded_run(conn: &mut Connection) -> Created {
        let run = open(conn, "observe");
        // A phase pass and a gate evaluation: three rows, seq 1..3.
        advance_one(conn, run.run_id, 0, "design").expect("the phase pass lands");
        gates(
            conn,
            &Gates {
                run_id: run.run_id,
                to_phase: Some("build"),
                actor: "tester",
                now: 3,
            },
        )
        .expect("the gate evaluates");
        run
    }

    /// The wire form the spec DECLARES, asserted against the struct that
    /// produces it. The openapi validator says a spec is well-formed; it cannot
    /// say the spec still DESCRIBES the response. A field renamed in Rust and
    /// left in openapi is a contract that lies, and with
    /// `additionalProperties: false` a client that trusts the spec rejects the
    /// real response.
    ///
    /// The nullable columns are the ones this round got wrong once: they were
    /// first written as OpenAPI 3.1's `type: [string, "null"]`, which is not
    /// valid in this file's 3.0.3. The spec now uses `nullable: true`, and this
    /// pins that the Rust side really does serialize them as `null` and not as
    /// an absent key.
    #[test]
    fn delivery_replay_wire_form_is_the_form_the_spec_declares() {
        let mut conn = seed();
        let run = open(&mut conn, "observe");

        let report = serde_json::to_value(replay_verify(&conn, run.run_id, 7).unwrap())
            .expect("the report serializes");
        let keys: Vec<&String> = report.as_object().expect("an object").keys().collect();
        for key in [
            "run_id",
            "window",
            "order_ok",
            "compared",
            "matched",
            "mismatched",
            "diffs",
            "event_log",
            "generated_at",
        ] {
            assert!(
                report.get(key).is_some(),
                "the spec's `required` names `{key}` and the report must carry it (keys: {keys:?})"
            );
        }
        assert!(
            keys.len() == 9,
            "the report carries exactly the nine declared keys, so `additionalProperties: false` \
             is honest (keys: {keys:?})"
        );
        // The window and the appendix both disclose their bound.
        for path in [report["window"].clone(), report["event_log"].clone()] {
            let obj = path.as_object().expect("an object");
            for key in ["rows", "cap", "truncated"] {
                assert!(
                    obj.contains_key(key),
                    "the spec's window schema requires `{key}` and the payload must carry it"
                );
            }
            assert_eq!(obj.len(), 3, "exactly the three declared window keys");
        }

        // The trace listing, same question.
        let listing = serde_json::to_value(trace_listing(&conn, run.run_id, 7).unwrap())
            .expect("the listing serializes");
        for key in [
            "run_id",
            "window",
            "rows",
            "attestation_root",
            "event_log",
            "generated_at",
        ] {
            assert!(
                listing.get(key).is_some(),
                "the spec's `required` names `{key}` and the listing must carry it"
            );
        }
        // ...and the NULLABLE columns serialize as a present `null`, never as an
        // absent key. This is the exact property the 3.0.3 `nullable: true`
        // declares; a `#[serde(skip_serializing_if)]` would break it silently.
        let row = listing["rows"][0].as_object().expect("a row object");
        for key in [
            "model_ref",
            "policy_digest",
            "config_digest",
            "budget_digest",
            "attestation_root",
        ] {
            assert!(
                row.contains_key(key) && row[key].is_null(),
                "`{key}` is declared `nullable: true`, so the row must carry the key with a null \
                 value — an ABSENT key is a different wire shape and a client reading the spec \
                 would see a required-less field (row: {row:?})"
            );
        }
        assert_eq!(
            row.len(),
            16,
            "the trace row carries exactly the sixteen declared columns, so the spec's closed \
             property set is honest"
        );
        // `attestation_root` is null before the first link — never a fabricated
        // address, which the spec says in prose.
        assert!(
            listing["attestation_root"].is_null(),
            "a run with no signed link has no head, and the spec says `null`, not a fabricated \
             address"
        );
    }

    /// A non-delivery run is ABSENT, not an empty delivery run. The two must be
    /// one answer: `workflow_runs` is a shared table whose id space also holds
    /// GDL, account, and valet runs, and every WRITE path filters
    /// `kind = 'delivery'`. A read that skipped the filter answered 200 with an
    /// empty window for a foreign run — an existence oracle the write paths
    /// deliberately refuse to give, and a payload shaped exactly like a real one.
    #[test]
    fn delivery_replay_treats_a_non_delivery_run_as_absent() {
        let conn = seed();
        // A run of another kind, sharing the id space and holding NO trace rows.
        conn.execute(
            "INSERT INTO workflow_runs(id, kind, domain, state_json, state_revision, status, \
             created_at, updated_at) VALUES (77, 'troubleshoot', 'global', '{}', 0, 'active', 1, 1)",
            [],
        )
        .expect("the foreign run lands");

        let report = replay_verify(&conn, 77, 2);
        assert!(
            matches!(report, Err(DeliveryError::RunAbsent)),
            "a run that is not a delivery run reads as ABSENT, identically to a missing one — \
             a 200 with an empty window would be an existence oracle across kinds. Got: \
             {report:?}"
        );
        let listing = trace_listing(&conn, 77, 2);
        assert!(
            matches!(listing, Err(DeliveryError::RunAbsent)),
            "the listing must answer the foreign run the same way the verdict does — the two \
             ride one read for a reason"
        );
        // And a genuinely absent id is the SAME error, so the two collapse.
        assert!(
            matches!(replay_verify(&conn, 999, 2), Err(DeliveryError::RunAbsent)),
            "a missing run and a foreign-kind run must be one indistinguishable answer"
        );
    }

    /// The narrative appendix is the DELIVERY narrative, not the agent loop's
    /// conversation transcript.
    ///
    /// `agent_session_events` is shared: the loop appends `user`, `assistant`,
    /// `tool_result` and `compaction` rows to the same table, and
    #[test]
    fn delivery_event_log_serves_only_the_ddl_family() {
        let mut conn = seed();
        let run = open(&mut conn, "observe");

        // A loop transcript and a delivery narrative on the SAME run.
        crate::workflow::session_log::append(
            &conn,
            run.run_id,
            "user",
            "{\"text\":\"a user turn\"}",
            "k1",
            1,
        )
        .expect("the loop writes its turns");
        crate::workflow::session_log::append(
            &conn,
            run.run_id,
            "assistant",
            "{\"text\":\"a model turn\"}",
            "k2",
            1,
        )
        .expect("the loop writes its turns");
        crate::workflow::session_log::append(
            &conn,
            run.run_id,
            DDL_ARTIFACT_KIND,
            "{\"text\":\"a delivery narrative\"}",
            "k3",
            1,
        )
        .expect("the delivery narrative appends");

        let report = replay_verify(&conn, run.run_id, 2).expect("the report assembles");
        let kinds: Vec<&str> = report
            .event_log
            .rows
            .iter()
            .map(|r| r.kind.as_str())
            .collect();
        assert_eq!(
            kinds,
            vec!["ddl_artifact"],
            "the appendix carries the `ddl_*` family and nothing else — the agent loop's \
             `user`/`assistant` transcript is not this route's to serve (got {kinds:?})"
        );
    }

    /// B2, the projection law, pinned once and for all: a `TraceRow` becomes a
    /// pair of `StageDigest`s, and the two sides differ in exactly one place —
    /// the content address, recorded on the left and re-derived on the right.
    ///
    /// The golden vector is the anti-vacuity device. A pin that only asserts
    /// `compare_replay` returns no diffs would pass on a projection that mapped
    /// EVERY field to a constant. This asserts the concrete bytes, so a
    /// projection that drifts in either direction is caught.
    #[test]
    fn delivery_replay_projection_is_pinned() {
        let row = TraceRow {
            id: "trc_ignored_by_the_projection".into(),
            run_id: 7,
            seq: 3,
            stage: "phase".into(),
            phase: "design".into(),
            status: "advanced".into(),
            tier: "observe".into(),
            actor: "operator".into(),
            model_ref: None,
            policy_digest: None,
            config_digest: None,
            pipeline_version: PIPELINE_VERSION.into(),
            budget_digest: None,
            artifact_refs_json: "[]".into(),
            attestation_root: None,
            created_at: 1_700_000_000,
        };
        let (recorded, rederived) = stage_digest_projection(&row);

        // The stage is the STORED column, projected — never invented.
        assert_eq!(recorded.stage, "phase");
        assert_eq!(rederived.stage, "phase");

        // `input_digest` is the row's committed facts: the sha256 of the
        // canonical bytes, prefixed. It does NOT frame `id` and does NOT frame
        // `created_at` — so it is stable across a re-read of the same row.
        let expected_input = format!(
            "sha256:{}",
            crate::audit::hex_encode(&row.canonical_bytes())
        );
        assert_eq!(recorded.input_digest, expected_input);
        assert_eq!(rederived.input_digest, expected_input);

        // `output_digest` is where the two sides DIVERGE: the recorded side
        // carries the stored address, the re-derived side the recomputed one.
        assert_eq!(
            recorded.output_digest, "trc_ignored_by_the_projection",
            "the recorded side carries the STORED content address"
        );
        assert_eq!(
            rederived.output_digest,
            row.content_id(),
            "the re-derived side carries the address recomputed from the stored columns"
        );

        // ...and the recomputed address ignores the stored one, which is the
        // whole reason the check is not a tautology.
        assert_ne!(
            recorded.output_digest, rederived.output_digest,
            "a row whose stored id does not match its columns MUST produce two different \
             output digests — this is the one comparison in the round that is not tautological"
        );
    }

    /// The DO's named test. A run that has not been tampered with re-derives
    /// exactly: every row's recomputed content address equals the stored one, and
    /// the verdict is "compared == matched, mismatched == 0".
    ///
    /// **And it makes no model call.** That is not asserted by reading this
    /// test; it is asserted structurally by
    /// `delivery_replay_makes_no_model_call_and_persists_nothing` below, which
    /// scans the read path for every provider call site.
    #[test]
    fn delivery_replay_verify_is_byte_exact_and_zero_model_calls() {
        let mut conn = seed();
        let run = seeded_run(&mut conn);

        let report = replay_verify(&conn, run.run_id, 2).expect("the replay report assembles");

        assert_eq!(report.window.rows, 3, "three recorded rows");
        assert!(!report.window.truncated, "three rows is far below the cap");
        assert!(report.order_ok, "seq 1..3 is contiguous ascending");
        assert_eq!(report.compared, 3);
        assert_eq!(report.matched, 3);
        assert_eq!(
            report.mismatched, 0,
            "an untampered run re-derives byte-exactly; diffs: {:?}",
            report.diffs
        );
        assert!(
            report.is_clean(),
            "the verdict is the report's own, and folds order violations in"
        );
    }

    /// B5: a mismatch is DATA. Corrupting one stored `trc_` address must
    /// produce a diff row and a non-clean verdict — never an error, never a
    /// status, and never a 500.
    #[test]
    fn delivery_replay_reports_mismatches_as_data_not_status() {
        let mut conn = seed();
        let run = seeded_run(&mut conn);

        // Flip one stored address, directly — the shape an out-of-band edit
        // leaves behind.
        conn.execute(
            "UPDATE delivery_traces SET id = 'trc_0000000000000000000000000000dead' \
             WHERE run_id = ?1 AND seq = 2",
            params![run.run_id],
        )
        .expect("the tamper lands");

        let report = replay_verify(&conn, run.run_id, 2).expect("a mismatch is never an error");

        assert_eq!(report.compared, 3, "every position is still compared");
        assert_eq!(report.mismatched, 1, "exactly the tampered row");
        assert_eq!(report.diffs.len(), 1);
        assert!(
            !report.is_clean(),
            "the verdict must be non-clean: the stored address no longer follows from the columns"
        );
        let diff = &report.diffs[0];
        assert_eq!(diff.stage, "phase", "the diff names the row's own stage");
        assert_eq!(diff.mismatch, "output_digest_differs");
        let recorded = diff
            .recorded
            .as_ref()
            .expect("the recorded side is present");
        let rederived = diff
            .rederived
            .as_ref()
            .expect("the re-derived side is present");
        assert_eq!(
            recorded.output_digest, "trc_0000000000000000000000000000dead",
            "the report shows WHAT was stored"
        );
        assert!(
            rederived.output_digest.starts_with("trc_") && rederived.output_digest.len() == 36,
            "the re-derived side is a well-formed content address recomputed from the stored \
             columns: {}",
            rederived.output_digest
        );
        assert_ne!(
            recorded.output_digest, rederived.output_digest,
            "and the two sides disagree, which is the finding: the stored address no longer \
             follows from the row's own columns"
        );
        assert!(
            diff.recorded.is_some() && diff.rederived.is_some(),
            "a field mismatch carries BOTH sides: a reader learns what was stored AND what the \
             columns imply"
        );
    }

    /// B3: the ordinal series must be contiguous ascending. A deleted middle
    /// row leaves a GAP, and the gap is reported as an `order` mismatch — a
    /// diff row, never an error status, and never a silently shorter window.
    #[test]
    fn delivery_replay_orders_by_seq_and_requires_contiguity() {
        let mut conn = seed();
        let run = seeded_run(&mut conn);

        // Delete the middle row. `UNIQUE(run_id, seq)` does not defend against
        // a DELETE, and `MAX(seq)+1` would reissue 3 — so the gap is real
        // state a reader must be able to see.
        conn.execute(
            "DELETE FROM delivery_traces WHERE run_id = ?1 AND seq = 2",
            params![run.run_id],
        )
        .expect("the delete lands");

        let report = replay_verify(&conn, run.run_id, 2).expect("a gap is never an error");

        assert_eq!(report.window.rows, 2, "the window holds what is stored");
        assert!(
            !report.order_ok,
            "seq 1,3 is not contiguous — the ordinal series is broken"
        );
        let order: Vec<&StageDiffRead> =
            report.diffs.iter().filter(|d| d.stage == "order").collect();
        assert!(
            !order.is_empty(),
            "the gap is REPORTED as an `order` diff, not merely flagged: {:?}",
            report.diffs
        );
        assert!(
            report.mismatched >= 1,
            "an order violation counts as a mismatch — it is the product, not a status"
        );
    }

    /// B6: no silent short read. The trace window is capped, and the cap is
    /// DISCLOSED in the payload with a truncation flag, so a reader can never
    /// mistake a bounded window for the whole run.
    #[test]
    fn delivery_replay_discloses_a_capped_event_log() {
        let mut conn = seed();
        let run = open(&mut conn, "observe");
        let baseline = count(
            &conn,
            &format!(
                "SELECT COUNT(*) FROM delivery_traces WHERE run_id = {}",
                run.run_id
            ),
        );
        // Add rows until the run is OVER the cap, whichever the admission
        // already contributed. Asserting the fixture's exact size is the kind
        // of detail that breaks when a writer adds a row; what matters is that
        // the run is past the cap and the report says so.
        while count(
            &conn,
            &format!(
                "SELECT COUNT(*) FROM delivery_traces WHERE run_id = {}",
                run.run_id
            ),
        ) <= MAX_TRACE_ROWS as i64
        {
            gates(
                &mut conn,
                &Gates {
                    run_id: run.run_id,
                    to_phase: Some("build"),
                    actor: "tester",
                    now: 9,
                },
            )
            .expect("the gate evaluates");
        }
        let total = count(
            &conn,
            &format!(
                "SELECT COUNT(*) FROM delivery_traces WHERE run_id = {}",
                run.run_id
            ),
        );
        assert!(
            total as usize > MAX_TRACE_ROWS,
            "the fixture is over the cap: {total} rows (baseline {baseline} plus the gate passes)"
        );

        let report = replay_verify(&conn, run.run_id, 2).expect("the report assembles");

        assert_eq!(report.window.cap, MAX_TRACE_ROWS, "the cap is DISCLOSED");
        assert!(report.window.truncated, "the window is short and says so");
        assert_eq!(
            report.window.rows, MAX_TRACE_ROWS,
            "the cap is a real bound, not a label"
        );
        // The appendix is bounded and discloses its own cap, separately.
        assert_eq!(
            report.event_log.cap,
            crate::workflow::session_log::REPLAY_CAP
        );
        assert!(
            report.event_log.rows.len() <= report.event_log.cap,
            "the appendix is bounded by its own cap"
        );
    }

    /// The `/trace` surface: the run's rows in ordinal order, plus the chain
    /// head, plus the same disclosed appendix. It rides the SAME read
    /// function, so the two surfaces can never disagree about what is stored.
    #[test]
    fn delivery_trace_read_returns_rows_and_head() {
        let mut conn = seed();
        let run = seeded_run(&mut conn);

        let listing = trace_listing(&conn, run.run_id, 2).expect("the listing assembles");

        assert_eq!(listing.run_id, run.run_id);
        assert_eq!(listing.window.rows, 3);
        let seqs: Vec<i64> = listing.rows.iter().map(|r| r.seq).collect();
        assert_eq!(
            seqs,
            vec![1, 2, 3],
            "the listing is in ORDINAL order — the stored series, not insertion accident"
        );
        // The head is the chain's head, or None before the first link.
        let head = crate::workflow::attestations::chain_head(&conn, run.run_id)
            .ok()
            .flatten();
        assert_eq!(
            listing.attestation_root, head,
            "the listing names the chain head it actually read"
        );
        assert_eq!(listing.window.cap, MAX_TRACE_ROWS);
    }

    /// Zero model calls, and nothing persisted. Both are STRUCTURAL claims, so
    /// they are checked against the source rather than against a run that
    /// happened not to need a model.
    ///
    /// The scan is scoped to the production region with the `#[cfg(test)]`
    /// boundary LOCATED (R39 found four vacuous checks and R40 a fifth; a guard
    /// is believed only after it has been deliberately broken), and it asserts
    /// its own symbols are locatable before asserting anything about them.
    #[test]
    fn delivery_replay_makes_no_model_call_and_persists_nothing() {
        let full = include_str!("delivery.rs");
        let boundary = full
            .find("#[cfg(test)]")
            .expect("delivery.rs has a #[cfg(test)] boundary");
        assert!(boundary > 0, "the test region must not start at byte 0");
        let production = &full[..boundary];

        for symbol in [
            "read_run_traces",
            "replay_verify",
            "stage_digest_projection",
        ] {
            assert!(
                production.contains(&format!("fn {symbol}")),
                "`fn {symbol}` must exist in the PRODUCTION region — a scan that cannot find \
                 its subject proves nothing"
            );
        }

        // No write SQL anywhere in the read path's own functions.
        for symbol in ["read_run_traces", "replay_verify", "trace_listing"] {
            let start = production
                .find(&format!("fn {symbol}"))
                .expect("locate the read path");
            let body = &production[start..];
            let end = body.find("\nfn ").unwrap_or(body.len());
            let body = &body[..end];
            for verb in ["INSERT", "UPDATE ", "DELETE FROM", "REPLACE INTO"] {
                assert!(
                    !body.contains(verb),
                    "`{symbol}` contains `{verb}` — the replay surface persists NOTHING: a \
                     replay verdict is a report, and a report that writes is a mutation"
                );
            }
        }

        // No model provider on the READ PATH. This is scanned per-function,
        // not over the whole production region: `resolve_for_execution` is the
        // R40 model-citation path and belongs to `advance`, which is a WRITE.
        // A module-wide ban would forbid correct code and get deleted. What is
        // forbidden is the read path reaching a provider.
        //
        // The pure comparator's own crate carries no provider and no network
        // stack (`delivery_r41_adds_no_dependency` pins its dependency set
        // exactly), so a provider call here would be a NEW edge.
        for symbol in [
            "read_run_traces",
            "replay_verify",
            "trace_listing",
            "read_event_log",
        ] {
            let start = production
                .find(&format!("fn {symbol}"))
                .unwrap_or_else(|| panic!("`fn {symbol}` must be locatable in delivery.rs"));
            let body = &production[start..];
            let end = body.find("\nfn ").unwrap_or(body.len());
            let body = &body[..end];
            for banned in [
                "resolve_for_execution",
                "run_model",
                "embed_text",
                "reqwest::",
                "http://",
                "https://",
            ] {
                assert!(
                    !body.contains(banned),
                    "`{symbol}` reaches `{banned}` — the replay verdict is computable from stored \
                     bytes alone, and a model or network call on the read path would make the \
                     verdict depend on something the evidence does not"
                );
            }
        }
    }

    /// D5: AGENTS.md forbids `let _ =` on writes because it reads like a
    /// swallowed error. This one sat in PRODUCTION on the attestation tie.
    /// Pinned so the idiom cannot come back on this file.
    #[test]
    fn delivery_production_carries_no_swallowed_let_underscore() {
        let full = include_str!("delivery.rs");
        let boundary = full
            .find("#[cfg(test)]")
            .expect("delivery.rs has a #[cfg(test)] boundary");
        let production = &full[..boundary];
        for (i, line) in production.lines().enumerate() {
            let trimmed = line.trim();
            assert!(
                !trimmed.starts_with("let _ ="),
                "delivery.rs:{}: `let _ =` in the production region reads like a swallowed \
                 error (AGENTS.md forbids it on writes). Line: {trimmed}",
                i + 1
            );
        }
    }
}
