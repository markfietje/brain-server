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
    AutonomyTier, Phase, is_legal_phase_transition, terminal_phase, trace_mode_for_tier,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::audit::{AuditKind, AuditStatus};
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
    UnknownVocabulary { field: &'static str, value: String },
    /// The run is absent, is not a delivery run, or is not the caller's — the
    /// handler collapses all three into one probe-blind answer.
    RunAbsent,
    /// The caller lost the CAS: another writer moved the revision first.
    Stale { actual_revision: i64 },
    /// The requested phase move is not in the forward-only machine.
    IllegalPhaseTransition { from: String, to: String },
    /// The run already sits in the terminal phase.
    TerminalPhase { phase: String },
    /// A run with no pending question cannot be answered.
    NoPendingQuestion,
    /// The run already carries a pending question.
    QuestionPending,
    /// A bounded input exceeded its cap.
    TooLong { field: &'static str, max: usize },
    /// Too many budget rows, or too many artifact refs, in one request.
    TooMany { field: &'static str, max: usize },
    /// The checkpoint gate refused the artifact. Carries the executor's own
    /// refusal verbatim — the QA law, not a nearest-match guess.
    QualityGate { reason: String },
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
            Self::Storage(detail) => write!(f, "delivery_storage: {detail}"),
        }
    }
}

impl std::error::Error for DeliveryError {}

fn storage(detail: impl std::fmt::Display) -> DeliveryError {
    DeliveryError::Storage(detail.to_string())
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
    /// fixed order, the created_at last. Two rows with the same facts and the
    /// same position produce the same id, which is what makes the replay index
    /// trustworthy.
    fn canonical_bytes(&self) -> Vec<u8> {
        let mut s = String::new();
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
    /// row's ordinal within its run.
    ///
    /// The ordinal is part of the identity on purpose. An append-only evidence
    /// log can legitimately record the same disposition twice (the same gate
    /// asked twice, at the same revision, in the same second), and a pure
    /// content address would collide on `delivery_traces.id` — which is
    /// precisely the bug the first run of this test found. Folding the ordinal
    /// in makes the address unique AND deterministic: replaying the same
    /// sequence of facts on a fresh database reproduces the same ids, which is
    /// the property the replay-verify surface will read.
    pub(crate) fn content_id(&self, ordinal: i64) -> String {
        let mut hasher = Sha256::new();
        hasher.update(ordinal.to_be_bytes());
        hasher.update(self.canonical_bytes());
        let digest = hasher.finalize();
        format!("trc_{}", &crate::audit::hex_encode(&digest)[..32])
    }
}

/// The row's ordinal within its run, read inside the caller's transaction. The
/// count is monotonic under the run's own transaction, so two writers cannot
/// claim the same ordinal.
fn trace_ordinal(conn: &Connection, run_id: i64) -> Result<i64, DeliveryError> {
    conn.query_row(
        "SELECT COUNT(*) FROM delivery_traces WHERE run_id = ?1",
        params![run_id],
        |r| r.get(0),
    )
    .map_err(storage)
}

/// Write the trace row inside the caller's transaction. `artifact_refs_json` is
/// a JSON array of refs, never content.
fn write_trace(conn: &Connection, row: &TraceRow) -> Result<(), DeliveryError> {
    conn.execute(
        "INSERT INTO delivery_traces(id, run_id, stage, phase, status, tier, actor, model_ref, \
         policy_digest, config_digest, pipeline_version, budget_digest, artifact_refs_json, \
         attestation_root, created_at) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
        params![
            row.id,
            row.run_id,
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
fn delivery_audit(
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
        attestation_root: None,
        created_at: req.now,
    };
    row.id = row.content_id(trace_ordinal(tx.tx(), run_id)?);
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
    pub actor: &'a str,
    pub now: i64,
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
    let (domain, status, state_json, revision) =
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
    let mut row = TraceRow {
        id: String::new(),
        run_id: req.run_id,
        stage: "phase".into(),
        phase: proposed.as_str().into(),
        status: "advanced".into(),
        tier: tier_core_to_wire(tier).into(),
        actor: req.actor.to_string(),
        model_ref: None,
        policy_digest: None,
        config_digest: None,
        pipeline_version: PIPELINE_VERSION.into(),
        budget_digest: None,
        artifact_refs_json: refs_json,
        attestation_root: None,
        created_at: req.now,
    };
    row.id = row.content_id(trace_ordinal(tx.tx(), req.run_id)?);
    write_trace(tx.tx(), &row)?;

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

    let _ = status;
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
        attestation_root: None,
        created_at: req.now,
    };
    row.id = row.content_id(trace_ordinal(tx.tx(), req.run_id)?);
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
        attestation_root: None,
        created_at: req.now,
    };
    row.id = row.content_id(trace_ordinal(tx.tx(), req.run_id)?);
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
            "nothing is spent: R38 has no executor to spend against"
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

    // ── the R39 seam: D2/D3 wiring + the typed-artifact proposal ──────────

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
    /// every R38 caller that passes no artifact must still work byte for byte.
    #[test]
    fn delivery_advance_without_an_artifact_is_unchanged() {
        let mut conn = seed();
        let created = open(&mut conn, "propose");
        let advanced = advance_one(&mut conn, created.run_id, 0, "design").unwrap();
        assert_eq!(advanced.phase, "design");
        assert_eq!(advanced.proposal_id, 0, "no artifact, no proposal");
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM proposals"), 0);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM delivery_traces"), 2);
    }
}
