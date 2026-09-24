//! Decision-evaluation records: bounded operator-declared judgments over
//! persisted decision traces.
//!
//! This module is intentionally conservative about what it knows. The existing
//! gold packs are quality-scorer oracles, not decision-output labels, and the
//! Kappa labels are reflection labels, not a decision judgment set. This
//! module therefore accepts only a strict, digest-pinned operator declaration
//! and marks it non-authoritative. No raw query, evidence text, model bytes, or
//! unrestricted prose is accepted or persisted.
//!
//! The evaluator is pure once traces and the registry row have been loaded.
//! The storage boundary is a `WorkflowTx`: the evaluation row, its checked
//! audit row, and the commit are one transition. Missing observations become
//! explicit unavailable legs, never numeric zeroes.

#![deny(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::eval;
use crate::workflow::harness::pipeline::{ActionLabel, SerdeDecisionOutput, SerdeDecisionValue};
use crate::workflow::harness::trace::DecisionRunTrace;
use crate::workflow::kappa::cohen_kappa_units;
use crate::workflow::registry::{self, RegistryError, RegistryRow};
use crate::workflow::tx::WorkflowTx;

/// Maximum number of judgment cases in one bounded evaluation.
pub(crate) const MAX_JUDGMENT_CASES: usize = 128;
/// Maximum number of relevant evidence references per case.
pub(crate) const MAX_RELEVANT_EVIDENCE_IDS: usize = 32;
/// Maximum length of a bounded identifier/ref.
pub(crate) const MAX_EVAL_TEXT_LEN: usize = 256;
/// Maximum length of the idempotency key.
pub(crate) const MAX_IDEMPOTENCY_KEY_LEN: usize = 128;
/// Maximum serialized manifest size.
pub(crate) const MAX_MANIFEST_BYTES: usize = 128 * 1024;
/// Maximum serialized report size.
pub(crate) const MAX_REPORT_BYTES: usize = 64 * 1024;
/// Maximum list page size.
pub(crate) const MAX_LIST_LIMIT: usize = 50;
/// Default list page size.
pub(crate) const DEFAULT_LIST_LIMIT: usize = 20;

const JUDGMENT_SOURCE: &str = "operator_declared";
const ACCEPTANCE_STATE: &str = "operator_accepted_non_authoritative";
const NO_OUTPUT_DIGEST_INPUT: &[u8] = b"decision-evaluation:none-output";
const REPORT_VERSION: &str = "decision-evaluation/v1";

/// Domain failures are named at the handler boundary. Messages contain no raw
/// request content; database details are logged by the handler, not rendered.
#[derive(Debug)]
pub(crate) enum EvaluationError {
    Invalid { code: &'static str, message: String },
    JudgmentSetUnavailable,
    TargetMismatch,
    ModelArtifactDigestRequired,
    IdempotencyConflict,
    RecordTampered,
    Audit(String),
    Db(rusqlite::Error),
}

impl EvaluationError {
    pub(crate) fn wire_code(&self) -> &'static str {
        match self {
            Self::Invalid { code, .. } => code,
            Self::JudgmentSetUnavailable => "judgment_set_unavailable",
            Self::TargetMismatch => "evaluation_target_mismatch",
            Self::ModelArtifactDigestRequired => "model_artifact_digest_required",
            Self::IdempotencyConflict => "evaluation_idempotency_conflict",
            Self::RecordTampered => "evaluation_record_tampered",
            Self::Audit(_) => "evaluation_audit_failed",
            Self::Db(_) => "evaluation_storage_failed",
        }
    }
}

impl fmt::Display for EvaluationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid { message, .. } => write!(f, "{message}"),
            Self::JudgmentSetUnavailable => write!(
                f,
                "a valid decision judgment set could not be proven from the supplied manifest and persisted traces"
            ),
            Self::TargetMismatch => write!(
                f,
                "the evaluation target does not match the persisted trace/model binding"
            ),
            Self::ModelArtifactDigestRequired => {
                write!(f, "a learned model target requires an artifact digest")
            }
            Self::IdempotencyConflict => write!(
                f,
                "the idempotency key is already bound to a different evaluation request"
            ),
            Self::RecordTampered => {
                write!(f, "the stored evaluation record digest does not verify")
            }
            Self::Audit(message) => write!(f, "checked evaluation audit write failed: {message}"),
            Self::Db(error) => write!(f, "evaluation storage error: {error}"),
        }
    }
}

impl std::error::Error for EvaluationError {}

impl From<rusqlite::Error> for EvaluationError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Db(error)
    }
}

fn evaluation_invalid(code: &'static str, message: impl Into<String>) -> EvaluationError {
    EvaluationError::Invalid {
        code,
        message: message.into(),
    }
}

fn evaluation_valid_text(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && !value.chars().any(|character| {
            character.is_control() || crate::strip_invisible::is_invisible(character)
        })
}

fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn digest_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn digest_json<T: Serialize>(value: &T) -> Result<String, EvaluationError> {
    let bytes = serde_json::to_vec(value).map_err(|error| {
        evaluation_invalid("evaluation_canonicalization_failed", error.to_string())
    })?;
    Ok(digest_bytes(&bytes))
}

/// The target binding carried by an evaluation request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvaluationTarget {
    pub(crate) pipeline_version: String,
    pub(crate) config_hash: String,
    pub(crate) model_registry_id: String,
    pub(crate) model_registry_version: String,
    pub(crate) model_registry_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) model_artifact_digest: Option<String>,
}

/// One closed decision-output kind label.
fn valid_output_label(value: &str) -> bool {
    matches!(value, "choice" | "score" | "noul" | "none")
}

/// One closed action label.
fn valid_action_label(value: &str) -> bool {
    matches!(value, "act" | "approve" | "reject" | "escalate")
}

/// One operator-declared judgment case. All fields are references, closed
/// labels, or digests; no raw content can be represented.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JudgmentCase {
    pub(crate) case_id: String,
    pub(crate) trace_id: i64,
    pub(crate) relevant_evidence_ids: Vec<String>,
    pub(crate) expected_action: String,
    pub(crate) expected_output_label: String,
    pub(crate) expected_output_digest: String,
    pub(crate) case_digest: String,
}

#[derive(Serialize)]
struct JudgmentCaseDigestInput<'a> {
    case_id: &'a str,
    trace_id: i64,
    relevant_evidence_ids: &'a [String],
    expected_action: &'a str,
    expected_output_label: &'a str,
    expected_output_digest: &'a str,
}

/// The server-recomputable per-case label digest.
pub(crate) fn judgment_case_digest(case: &JudgmentCase) -> Result<String, EvaluationError> {
    digest_json(&JudgmentCaseDigestInput {
        case_id: &case.case_id,
        trace_id: case.trace_id,
        relevant_evidence_ids: &case.relevant_evidence_ids,
        expected_action: &case.expected_action,
        expected_output_label: &case.expected_output_label,
        expected_output_digest: &case.expected_output_digest,
    })
}

#[derive(Serialize)]
struct JudgmentManifestDigestInput<'a> {
    source: &'a str,
    authoritative: bool,
    judgment_set_ref: &'a str,
    frozen_at: i64,
    cases: &'a [JudgmentCase],
}

/// The server-recomputable manifest digest, including every per-case digest.
pub(crate) fn judgment_manifest_digest(
    manifest: &JudgmentSetManifest,
) -> Result<String, EvaluationError> {
    digest_json(&JudgmentManifestDigestInput {
        source: &manifest.source,
        authoritative: manifest.authoritative,
        judgment_set_ref: &manifest.judgment_set_ref,
        frozen_at: manifest.frozen_at,
        cases: &manifest.cases,
    })
}

/// A strict, non-authoritative judgment-set declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JudgmentSetManifest {
    pub(crate) source: String,
    pub(crate) authoritative: bool,
    pub(crate) judgment_set_ref: String,
    pub(crate) frozen_at: i64,
    pub(crate) manifest_digest: String,
    pub(crate) cases: Vec<JudgmentCase>,
}

/// The wire body accepted by the evaluation creation route.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateEvaluationBody {
    pub(crate) idempotency_key: String,
    pub(crate) target: EvaluationTarget,
    pub(crate) judgment_set: JudgmentSetManifest,
}

#[derive(Serialize)]
struct RequestDigestInput<'a> {
    idempotency_key: &'a str,
    target: &'a EvaluationTarget,
    judgment_set: &'a JudgmentSetManifest,
}

/// The stable request identity used for idempotency and evaluation ids.
pub(crate) fn evaluation_request_digest(
    body: &CreateEvaluationBody,
) -> Result<String, EvaluationError> {
    digest_json(&RequestDigestInput {
        idempotency_key: &body.idempotency_key,
        target: &body.target,
        judgment_set: &body.judgment_set,
    })
}

fn validate_target(target: &EvaluationTarget) -> Result<(), EvaluationError> {
    if !evaluation_valid_text(&target.pipeline_version, 64) {
        return Err(evaluation_invalid(
            "evaluation_target_invalid",
            "pipeline_version must be a bounded non-control string",
        ));
    }
    if !is_digest(&target.config_hash) {
        return Err(evaluation_invalid(
            "evaluation_target_invalid",
            "config_hash must be 64 lowercase hexadecimal characters",
        ));
    }
    if !evaluation_valid_text(&target.model_registry_id, MAX_EVAL_TEXT_LEN)
        || target.model_registry_id.contains('@')
    {
        return Err(evaluation_invalid(
            "evaluation_target_invalid",
            "model_registry_id must be bounded and contain no @",
        ));
    }
    if !evaluation_valid_text(&target.model_registry_version, 128)
        || target.model_registry_version.contains('@')
    {
        return Err(evaluation_invalid(
            "evaluation_target_invalid",
            "model_registry_version must be bounded and contain no @",
        ));
    }
    if !is_digest(&target.model_registry_digest) {
        return Err(evaluation_invalid(
            "evaluation_target_invalid",
            "model_registry_digest must be 64 lowercase hexadecimal characters",
        ));
    }
    if target
        .model_artifact_digest
        .as_deref()
        .is_some_and(|value| !is_digest(value))
    {
        return Err(evaluation_invalid(
            "evaluation_target_invalid",
            "model_artifact_digest must be 64 lowercase hexadecimal characters",
        ));
    }
    Ok(())
}

fn validate_manifest_structure(manifest: &JudgmentSetManifest) -> Result<(), EvaluationError> {
    if manifest.source != JUDGMENT_SOURCE {
        return Err(evaluation_invalid(
            "judgment_set_source_invalid",
            "source must be operator_declared; no other judgment source is accepted",
        ));
    }
    if manifest.authoritative {
        return Err(evaluation_invalid(
            "judgment_set_not_authoritative",
            "this evaluation path accepts only an explicitly non-authoritative operator declaration",
        ));
    }
    if !evaluation_valid_text(&manifest.judgment_set_ref, MAX_EVAL_TEXT_LEN) {
        return Err(evaluation_invalid(
            "judgment_set_ref_invalid",
            "judgment_set_ref must be a bounded non-control string",
        ));
    }
    if !is_digest(&manifest.manifest_digest) {
        return Err(evaluation_invalid(
            "judgment_set_digest_invalid",
            "manifest_digest must be 64 lowercase hexadecimal characters",
        ));
    }
    if manifest.cases.is_empty() || manifest.cases.len() > MAX_JUDGMENT_CASES {
        return Err(evaluation_invalid(
            "judgment_set_size_invalid",
            format!("cases must contain 1..={MAX_JUDGMENT_CASES} entries"),
        ));
    }

    let mut case_ids = BTreeSet::new();
    let mut previous_case: Option<&str> = None;
    for case in &manifest.cases {
        if !evaluation_valid_text(&case.case_id, MAX_EVAL_TEXT_LEN) {
            return Err(evaluation_invalid(
                "judgment_case_id_invalid",
                "case_id must be a bounded non-control string",
            ));
        }
        if !case_ids.insert(case.case_id.as_str()) {
            return Err(evaluation_invalid(
                "judgment_case_id_duplicate",
                "case_id values must be unique",
            ));
        }
        if previous_case.is_some_and(|previous| case.case_id.as_str() <= previous) {
            return Err(evaluation_invalid(
                "judgment_set_order_invalid",
                "cases must be in strictly increasing case_id order",
            ));
        }
        previous_case = Some(&case.case_id);
        if case.trace_id <= 0 {
            return Err(evaluation_invalid(
                "judgment_trace_id_invalid",
                "trace_id must be a positive persisted decision-trace id",
            ));
        }
        if case.relevant_evidence_ids.is_empty()
            || case.relevant_evidence_ids.len() > MAX_RELEVANT_EVIDENCE_IDS
        {
            return Err(evaluation_invalid(
                "judgment_evidence_ids_invalid",
                format!(
                    "relevant_evidence_ids must contain 1..={MAX_RELEVANT_EVIDENCE_IDS} entries"
                ),
            ));
        }
        let mut previous_evidence: Option<&str> = None;
        for evidence_id in &case.relevant_evidence_ids {
            if !evaluation_valid_text(evidence_id, MAX_EVAL_TEXT_LEN) {
                return Err(evaluation_invalid(
                    "judgment_evidence_id_invalid",
                    "evidence ids must be bounded non-control strings",
                ));
            }
            if previous_evidence.is_some_and(|previous| evidence_id.as_str() <= previous) {
                return Err(evaluation_invalid(
                    "judgment_evidence_order_invalid",
                    "relevant_evidence_ids must be strictly increasing",
                ));
            }
            if evidence_id.parse::<i64>().is_err() {
                return Err(EvaluationError::JudgmentSetUnavailable);
            }
            previous_evidence = Some(evidence_id);
        }
        if !valid_action_label(&case.expected_action) {
            return Err(evaluation_invalid(
                "judgment_action_label_invalid",
                "expected_action must be act, approve, reject, or escalate",
            ));
        }
        if !valid_output_label(&case.expected_output_label) {
            return Err(evaluation_invalid(
                "judgment_output_label_invalid",
                "expected_output_label must be choice, score, noul, or none",
            ));
        }
        if !is_digest(&case.expected_output_digest) || !is_digest(&case.case_digest) {
            return Err(evaluation_invalid(
                "judgment_case_digest_invalid",
                "case and expected-output digests must be 64 lowercase hexadecimal characters",
            ));
        }
        let computed = judgment_case_digest(case)?;
        if computed != case.case_digest {
            return Err(evaluation_invalid(
                "judgment_case_digest_mismatch",
                "the case digest does not match its bounded label/reference fields",
            ));
        }
    }
    let computed = judgment_manifest_digest(manifest)?;
    if computed != manifest.manifest_digest {
        return Err(evaluation_invalid(
            "judgment_set_digest_mismatch",
            "manifest_digest does not match the supplied judgment cases",
        ));
    }
    Ok(())
}

fn validate_freeze(manifest: &JudgmentSetManifest, now: i64) -> Result<(), EvaluationError> {
    if manifest.frozen_at <= 0 || manifest.frozen_at > now + 86_400 {
        return Err(evaluation_invalid(
            "judgment_set_freeze_invalid",
            "frozen_at must be positive and no more than one day in the future",
        ));
    }
    Ok(())
}

fn validate_body(body: &CreateEvaluationBody, now: i64) -> Result<(), EvaluationError> {
    if !evaluation_valid_text(&body.idempotency_key, MAX_IDEMPOTENCY_KEY_LEN) {
        return Err(evaluation_invalid(
            "evaluation_idempotency_key_invalid",
            "idempotency_key must be a bounded non-control string",
        ));
    }
    validate_target(&body.target)?;
    validate_manifest_structure(&body.judgment_set)?;
    validate_freeze(&body.judgment_set, now)?;
    Ok(())
}

fn action_label(action: ActionLabel) -> &'static str {
    match action {
        ActionLabel::Act => "act",
        ActionLabel::Approve => "approve",
        ActionLabel::Reject => "reject",
        ActionLabel::Escalate => "escalate",
    }
}

fn output_label(output: Option<&SerdeDecisionOutput>) -> &'static str {
    match output.map(|value| &value.value) {
        Some(SerdeDecisionValue::Choice { .. }) => "choice",
        Some(SerdeDecisionValue::Score { .. }) => "score",
        Some(SerdeDecisionValue::Noul { .. }) => "noul",
        None => "none",
    }
}

/// The digest law for an expected/persisted typed decision output.
pub(crate) fn decision_output_digest(
    output: Option<&SerdeDecisionOutput>,
) -> Result<String, EvaluationError> {
    if let Some(output) = output {
        digest_json(output)
    } else {
        Ok(digest_bytes(NO_OUTPUT_DIGEST_INPUT))
    }
}

fn unavailable_leg(reason: &str) -> LegReport {
    LegReport {
        status: "unavailable".to_string(),
        reason: Some(reason.to_string()),
        value: None,
    }
}

fn available_leg(value: Value) -> LegReport {
    LegReport {
        status: "available".to_string(),
        reason: None,
        value: Some(value),
    }
}

/// One explicit metric-leg status. Missing data is never encoded as zero.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct LegReport {
    pub(crate) status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) value: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct CaseResult {
    pub(crate) case_id: String,
    pub(crate) trace_id: i64,
    pub(crate) action_match: bool,
    pub(crate) output_match: bool,
    pub(crate) retrieved_evidence_count: usize,
}

/// The deterministic report persisted with an evaluation run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct EvaluationReport {
    pub(crate) report_version: String,
    pub(crate) case_results: Vec<CaseResult>,
    pub(crate) retrieval: LegReport,
    pub(crate) agreement: LegReport,
    pub(crate) abstention: LegReport,
    pub(crate) ood_abstention: LegReport,
    pub(crate) calibration: LegReport,
    pub(crate) red_team: LegReport,
    pub(crate) acceptance_reported_as_data_only: bool,
    pub(crate) acceptance_bars: Value,
}

fn validate_trace_target(
    body: &CreateEvaluationBody,
    trace: &DecisionRunTrace,
) -> Result<(), EvaluationError> {
    if trace.pipeline_version != body.target.pipeline_version
        || trace.config_hash != body.target.config_hash
    {
        return Err(EvaluationError::TargetMismatch);
    }
    if trace.model_refs.len() != 1 {
        return Err(EvaluationError::TargetMismatch);
    }
    let model = &trace.model_refs[0];
    if model.id != body.target.model_registry_id
        || model.version != body.target.model_registry_version
    {
        return Err(EvaluationError::TargetMismatch);
    }
    let Some(registry_ref) = model.registry_ref.as_ref() else {
        return Err(EvaluationError::TargetMismatch);
    };
    if registry_ref.registry_id != body.target.model_registry_id
        || registry_ref.registry_version != body.target.model_registry_version
    {
        return Err(EvaluationError::TargetMismatch);
    }
    match (
        model.weights_digest.as_deref(),
        body.target.model_artifact_digest.as_deref(),
    ) {
        (None, None) => {}
        (Some(actual), Some(expected)) if actual == expected => {}
        (Some(_), None) => return Err(EvaluationError::ModelArtifactDigestRequired),
        (None, Some(_)) | (Some(_), Some(_)) => {
            return Err(EvaluationError::TargetMismatch);
        }
    }
    Ok(())
}

fn numeric_evidence_ids(ids: &[String]) -> Result<Vec<i64>, EvaluationError> {
    ids.iter()
        .map(|id| {
            id.parse::<i64>()
                .map_err(|_| EvaluationError::JudgmentSetUnavailable)
        })
        .collect()
}

/// Pure evaluation over already-loaded traces. No database, HTTP, pool,
/// environment, or clock access occurs here; `now` is only the injected
/// validation timestamp.
pub(crate) fn evaluate_prepared(
    body: &CreateEvaluationBody,
    traces: &BTreeMap<i64, DecisionRunTrace>,
    now: i64,
) -> Result<EvaluationReport, EvaluationError> {
    validate_body(body, now)?;

    let mut precision_sum = 0.0_f32;
    let mut recall_sum = 0.0_f32;
    let mut mrr_sum = 0.0_f32;
    let mut ndcg_sum = 0.0_f32;
    let mut actual_actions = Vec::with_capacity(body.judgment_set.cases.len());
    let mut expected_actions = Vec::with_capacity(body.judgment_set.cases.len());
    let mut case_results = Vec::with_capacity(body.judgment_set.cases.len());
    let mut confidences = Vec::new();
    let mut correctness = Vec::new();
    let mut confidence_observations = 0usize;
    let mut abstentions = 0usize;

    for case in &body.judgment_set.cases {
        let trace = traces
            .get(&case.trace_id)
            .ok_or(EvaluationError::JudgmentSetUnavailable)?;
        validate_trace_target(body, trace)?;
        let relevant = numeric_evidence_ids(&case.relevant_evidence_ids)?;
        let mut retrieved = Vec::with_capacity(trace.context_refs.len());
        for hit in &trace.context_refs {
            retrieved.push(
                hit.evidence_id
                    .parse::<i64>()
                    .map_err(|_| EvaluationError::JudgmentSetUnavailable)?,
            );
        }
        precision_sum += eval::precision_at_k(&retrieved, &relevant, 5);
        recall_sum += eval::recall_at_k(&retrieved, &relevant, 5);
        mrr_sum += eval::mrr(&retrieved, &relevant);
        ndcg_sum += eval::ndcg(&retrieved, &relevant, 5);

        let actual_action = action_label(trace.outcome.action);
        let actual_output_digest = decision_output_digest(trace.outcome.output.as_ref())?;
        let actual_output_label = output_label(trace.outcome.output.as_ref());
        let action_match = actual_action == case.expected_action;
        let output_match = actual_output_label == case.expected_output_label
            && actual_output_digest == case.expected_output_digest;
        actual_actions.push(actual_action.to_string());
        expected_actions.push(case.expected_action.clone());
        case_results.push(CaseResult {
            case_id: case.case_id.clone(),
            trace_id: case.trace_id,
            action_match,
            output_match,
            retrieved_evidence_count: retrieved.len(),
        });

        if let Some(output) = trace.outcome.output.as_ref()
            && let SerdeDecisionValue::Choice {
                confidence: Some(confidence),
                ..
            } = output.value
            && confidence.is_finite()
            && (0.0..=1.0).contains(&confidence)
        {
            confidence_observations += 1;
            confidences.push(confidence as f32);
            correctness.push(output_match);
        }
        let abstains = trace.outcome.action == ActionLabel::Escalate
            || matches!(
                trace.outcome.output.as_ref().map(|output| &output.value),
                Some(SerdeDecisionValue::Noul { value: true })
            );
        if abstains {
            abstentions += 1;
        }
    }

    let n = body.judgment_set.cases.len() as f32;
    let retrieval = available_leg(serde_json::json!({
        "n": body.judgment_set.cases.len(),
        "precision_at_5": precision_sum / n,
        "recall_at_5": recall_sum / n,
        "mrr": mrr_sum / n,
        "ndcg_at_5": ndcg_sum / n,
    }));
    let agreement = cohen_kappa_units(&actual_actions, &expected_actions).map_or_else(
        |reason| unavailable_leg(&format!("agreement_unavailable:{reason}")),
        |value| {
            available_leg(serde_json::json!({
                "cohen_kappa_units": value,
                "n": body.judgment_set.cases.len(),
            }))
        },
    );
    let abstention = available_leg(serde_json::json!({
        "abstain_rate_units": (abstentions as i64 * 10_000)
            / body.judgment_set.cases.len() as i64,
        "n": body.judgment_set.cases.len(),
    }));
    let calibration = if confidence_observations == 0 {
        unavailable_leg("confidence_observations_not_provided")
    } else if confidence_observations != body.judgment_set.cases.len() {
        unavailable_leg("confidence_observations_incomplete")
    } else {
        let raw = super::decide::calibration::ece_score(&confidences, &correctness, 15);
        if let Some(units) = super::decide::calibration::score_to_units(raw) {
            available_leg(serde_json::json!({
                "ece_units": units,
                "n": confidence_observations,
            }))
        } else {
            unavailable_leg("ece_not_on_integer_units_grid")
        }
    };
    let acceptance_bars = serde_json::json!({
        "reported_as_data_only": true,
        "kappa_units": crate::workflow::kappa::KAPPA_BAR_UNITS,
    });
    Ok(EvaluationReport {
        report_version: REPORT_VERSION.to_string(),
        case_results,
        retrieval,
        agreement,
        abstention,
        ood_abstention: unavailable_leg("ood_judgment_set_not_provided"),
        calibration,
        red_team: unavailable_leg("red_team_judgment_set_not_provided"),
        acceptance_reported_as_data_only: true,
        acceptance_bars,
    })
}

#[derive(Serialize)]
struct RecordDigestInput<'a> {
    evaluation_id: &'a str,
    idempotency_key: &'a str,
    request_digest: &'a str,
    pipeline_version: &'a str,
    config_hash: &'a str,
    model_registry_id: &'a str,
    model_registry_version: &'a str,
    model_registry_digest: &'a str,
    model_artifact_digest: Option<&'a str>,
    judgment_set_ref: &'a str,
    judgment_set_source: &'a str,
    judgment_set_digest: &'a str,
    judgment_set_count: i64,
    judgment_set_frozen_at: i64,
    manifest_json: &'a str,
    report_json: &'a str,
    acceptance_state: &'a str,
    acceptance_bars_json: &'a str,
    creator_id: &'a str,
    reviewer_id: &'a str,
    created_at: i64,
    audit_target: &'a str,
}

struct RecordDigestParts<'a> {
    evaluation_id: &'a str,
    idempotency_key: &'a str,
    request_digest: &'a str,
    target: &'a EvaluationTarget,
    judgment_set: &'a JudgmentSetManifest,
    manifest_json: &'a str,
    report_json: &'a str,
    acceptance_bars_json: &'a str,
    creator_id: &'a str,
    reviewer_id: &'a str,
    created_at: i64,
    audit_target: &'a str,
}

fn record_digest_from_parts(parts: RecordDigestParts<'_>) -> Result<String, EvaluationError> {
    let target = parts.target;
    let judgment_set = parts.judgment_set;
    digest_json(&RecordDigestInput {
        evaluation_id: parts.evaluation_id,
        idempotency_key: parts.idempotency_key,
        request_digest: parts.request_digest,
        pipeline_version: &target.pipeline_version,
        config_hash: &target.config_hash,
        model_registry_id: &target.model_registry_id,
        model_registry_version: &target.model_registry_version,
        model_registry_digest: &target.model_registry_digest,
        model_artifact_digest: target.model_artifact_digest.as_deref(),
        judgment_set_ref: &judgment_set.judgment_set_ref,
        judgment_set_source: &judgment_set.source,
        judgment_set_digest: &judgment_set.manifest_digest,
        judgment_set_count: judgment_set.cases.len() as i64,
        judgment_set_frozen_at: judgment_set.frozen_at,
        manifest_json: parts.manifest_json,
        report_json: parts.report_json,
        acceptance_state: ACCEPTANCE_STATE,
        acceptance_bars_json: parts.acceptance_bars_json,
        creator_id: parts.creator_id,
        reviewer_id: parts.reviewer_id,
        created_at: parts.created_at,
        audit_target: parts.audit_target,
    })
}

/// The full stored/evaluation response projection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct EvaluationRecord {
    pub(crate) evaluation_id: String,
    pub(crate) idempotency_key: String,
    pub(crate) request_digest: String,
    pub(crate) target: EvaluationTarget,
    pub(crate) judgment_set: JudgmentSetManifest,
    pub(crate) report: EvaluationReport,
    pub(crate) acceptance_state: String,
    pub(crate) acceptance_bars: Value,
    pub(crate) creator_id: String,
    pub(crate) reviewer_id: String,
    pub(crate) created_at: i64,
    pub(crate) record_digest: String,
    pub(crate) audit_target: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EvaluationWriteReceipt {
    pub(crate) record: EvaluationRecord,
    pub(crate) created: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EvaluationListRow {
    pub(crate) evaluation_id: String,
    pub(crate) pipeline_version: String,
    pub(crate) config_hash: String,
    pub(crate) model_registry_id: String,
    pub(crate) model_registry_version: String,
    pub(crate) judgment_set_ref: String,
    pub(crate) judgment_set_digest: String,
    pub(crate) judgment_set_count: i64,
    pub(crate) acceptance_state: String,
    pub(crate) created_at: i64,
}

const EVALUATION_ROW_COLUMNS: &str = "evaluation_id, idempotency_key, request_digest, \
     pipeline_version, config_hash, model_registry_id, model_registry_version, \
     model_registry_digest, model_artifact_digest, judgment_set_ref, \
     judgment_set_source, judgment_set_digest, judgment_set_count, \
     judgment_set_frozen_at, manifest_json, report_json, acceptance_state, \
     acceptance_bars_json, creator_id, reviewer_id, created_at, record_digest, \
     audit_target";

struct RawEvaluationRow {
    evaluation_id: String,
    idempotency_key: String,
    request_digest: String,
    pipeline_version: String,
    config_hash: String,
    model_registry_id: String,
    model_registry_version: String,
    model_registry_digest: String,
    model_artifact_digest: Option<String>,
    judgment_set_ref: String,
    judgment_set_source: String,
    judgment_set_digest: String,
    judgment_set_count: i64,
    judgment_set_frozen_at: i64,
    manifest_json: String,
    report_json: String,
    acceptance_state: String,
    acceptance_bars_json: String,
    creator_id: String,
    reviewer_id: String,
    created_at: i64,
    record_digest: String,
    audit_target: String,
}

impl RawEvaluationRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            evaluation_id: row.get(0)?,
            idempotency_key: row.get(1)?,
            request_digest: row.get(2)?,
            pipeline_version: row.get(3)?,
            config_hash: row.get(4)?,
            model_registry_id: row.get(5)?,
            model_registry_version: row.get(6)?,
            model_registry_digest: row.get(7)?,
            model_artifact_digest: row.get(8)?,
            judgment_set_ref: row.get(9)?,
            judgment_set_source: row.get(10)?,
            judgment_set_digest: row.get(11)?,
            judgment_set_count: row.get(12)?,
            judgment_set_frozen_at: row.get(13)?,
            manifest_json: row.get(14)?,
            report_json: row.get(15)?,
            acceptance_state: row.get(16)?,
            acceptance_bars_json: row.get(17)?,
            creator_id: row.get(18)?,
            reviewer_id: row.get(19)?,
            created_at: row.get(20)?,
            record_digest: row.get(21)?,
            audit_target: row.get(22)?,
        })
    }
}

fn record_from_raw(raw: RawEvaluationRow) -> Result<EvaluationRecord, EvaluationError> {
    let target = EvaluationTarget {
        pipeline_version: raw.pipeline_version,
        config_hash: raw.config_hash,
        model_registry_id: raw.model_registry_id,
        model_registry_version: raw.model_registry_version,
        model_registry_digest: raw.model_registry_digest,
        model_artifact_digest: raw.model_artifact_digest,
    };
    let judgment_set: JudgmentSetManifest =
        serde_json::from_str(&raw.manifest_json).map_err(|_| EvaluationError::RecordTampered)?;
    let report: EvaluationReport =
        serde_json::from_str(&raw.report_json).map_err(|_| EvaluationError::RecordTampered)?;
    let acceptance_bars: Value = serde_json::from_str(&raw.acceptance_bars_json)
        .map_err(|_| EvaluationError::RecordTampered)?;
    let expected_digest = record_digest_from_parts(RecordDigestParts {
        evaluation_id: &raw.evaluation_id,
        idempotency_key: &raw.idempotency_key,
        request_digest: &raw.request_digest,
        target: &target,
        judgment_set: &judgment_set,
        manifest_json: &raw.manifest_json,
        report_json: &raw.report_json,
        acceptance_bars_json: &raw.acceptance_bars_json,
        creator_id: &raw.creator_id,
        reviewer_id: &raw.reviewer_id,
        created_at: raw.created_at,
        audit_target: &raw.audit_target,
    })?;
    if raw.record_digest != expected_digest
        || raw.judgment_set_ref != judgment_set.judgment_set_ref
        || raw.judgment_set_source != judgment_set.source
        || raw.judgment_set_digest != judgment_set.manifest_digest
        || raw.judgment_set_count != judgment_set.cases.len() as i64
        || raw.judgment_set_frozen_at != judgment_set.frozen_at
        || raw.acceptance_state != ACCEPTANCE_STATE
    {
        return Err(EvaluationError::RecordTampered);
    }
    Ok(EvaluationRecord {
        evaluation_id: raw.evaluation_id,
        idempotency_key: raw.idempotency_key,
        request_digest: raw.request_digest,
        target,
        judgment_set,
        report,
        acceptance_state: raw.acceptance_state,
        acceptance_bars,
        creator_id: raw.creator_id,
        reviewer_id: raw.reviewer_id,
        created_at: raw.created_at,
        record_digest: raw.record_digest,
        audit_target: raw.audit_target,
    })
}

fn row_by_idempotency(
    conn: &Connection,
    idempotency_key: &str,
) -> Result<Option<EvaluationRecord>, EvaluationError> {
    let sql = format!(
        "SELECT {EVALUATION_ROW_COLUMNS} FROM decision_evaluation_runs WHERE idempotency_key = ?1"
    );
    let raw = conn
        .query_row(&sql, [idempotency_key], RawEvaluationRow::from_row)
        .optional()?;
    raw.map(record_from_raw).transpose()
}

fn load_trace(conn: &Connection, trace_id: i64) -> Result<DecisionRunTrace, EvaluationError> {
    let raw = conn
        .query_row(
            "SELECT trace_json FROM decision_run_traces WHERE id = ?1",
            [trace_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(raw) = raw else {
        return Err(EvaluationError::JudgmentSetUnavailable);
    };
    serde_json::from_str(&raw).map_err(|_| EvaluationError::JudgmentSetUnavailable)
}

fn validate_registry_target(
    target: &EvaluationTarget,
    row: &RegistryRow,
) -> Result<(), EvaluationError> {
    if row.id != target.model_registry_id
        || row.version != target.model_registry_version
        || row.config_digest.as_deref() != Some(target.model_registry_digest.as_str())
    {
        return Err(EvaluationError::TargetMismatch);
    }
    if row.kind == registry::KIND_LEARNED && target.model_artifact_digest.is_none() {
        return Err(EvaluationError::ModelArtifactDigestRequired);
    }
    if let Some(expected) = target.model_artifact_digest.as_deref()
        && row.artifact_digest.as_deref() != Some(expected)
    {
        return Err(EvaluationError::TargetMismatch);
    }
    if row.kind != registry::KIND_LEARNED && target.model_artifact_digest.is_some() {
        return Err(EvaluationError::TargetMismatch);
    }
    Ok(())
}

fn registry_row(
    conn: &Connection,
    target: &EvaluationTarget,
) -> Result<RegistryRow, EvaluationError> {
    match registry::row_by_ref(
        conn,
        &target.model_registry_id,
        &target.model_registry_version,
    ) {
        Ok(Some(row)) => Ok(row),
        Ok(None) => Err(EvaluationError::TargetMismatch),
        Err(error) => Err(match error {
            RegistryError::Db(database_error) => EvaluationError::Db(database_error),
            _ => EvaluationError::TargetMismatch,
        }),
    }
}

/// Create one evaluation and its checked audit row atomically.
pub(crate) fn create_evaluation(
    conn: &mut Connection,
    body: CreateEvaluationBody,
    creator_id: &str,
    now: i64,
) -> Result<EvaluationWriteReceipt, EvaluationError> {
    validate_body(&body, now)?;
    if !evaluation_valid_text(creator_id, MAX_EVAL_TEXT_LEN) {
        return Err(evaluation_invalid(
            "evaluation_creator_invalid",
            "the authenticated creator identity is not bounded",
        ));
    }
    let request_digest = evaluation_request_digest(&body)?;
    let mut tx = WorkflowTx::begin(conn)?;
    if let Some(existing) = row_by_idempotency(tx.tx(), &body.idempotency_key)? {
        if existing.request_digest != request_digest {
            return Err(EvaluationError::IdempotencyConflict);
        }
        tx.commit()?;
        return Ok(EvaluationWriteReceipt {
            record: existing,
            created: false,
        });
    }

    let manifest_json = serde_json::to_string(&body.judgment_set).map_err(|error| {
        evaluation_invalid(
            "evaluation_manifest_serialization_failed",
            error.to_string(),
        )
    })?;
    if manifest_json.len() > MAX_MANIFEST_BYTES {
        return Err(evaluation_invalid(
            "judgment_set_too_large",
            "the bounded judgment manifest exceeds its serialized size limit",
        ));
    }
    let model = registry_row(tx.tx(), &body.target)?;
    validate_registry_target(&body.target, &model)?;
    let mut traces = BTreeMap::new();
    for case in &body.judgment_set.cases {
        if traces.contains_key(&case.trace_id) {
            return Err(evaluation_invalid(
                "judgment_trace_duplicate",
                "each judgment case must cite a distinct persisted trace",
            ));
        }
        let trace = load_trace(tx.tx(), case.trace_id)?;
        validate_trace_target(&body, &trace)?;
        traces.insert(case.trace_id, trace);
    }
    let report = evaluate_prepared(&body, &traces, now)?;
    let report_json = serde_json::to_string(&report).map_err(|error| {
        evaluation_invalid("evaluation_report_serialization_failed", error.to_string())
    })?;
    if report_json.len() > MAX_REPORT_BYTES {
        return Err(evaluation_invalid(
            "evaluation_report_too_large",
            "the deterministic evaluation report exceeds its serialized size limit",
        ));
    }
    let acceptance_bars_json = serde_json::to_string(&report.acceptance_bars).map_err(|error| {
        evaluation_invalid("evaluation_bars_serialization_failed", error.to_string())
    })?;
    let evaluation_id = format!("eval_{}", &request_digest[..32]);
    let audit_target = format!("decision_eval:{evaluation_id}");
    let record_digest = record_digest_from_parts(RecordDigestParts {
        evaluation_id: &evaluation_id,
        idempotency_key: &body.idempotency_key,
        request_digest: &request_digest,
        target: &body.target,
        judgment_set: &body.judgment_set,
        manifest_json: &manifest_json,
        report_json: &report_json,
        acceptance_bars_json: &acceptance_bars_json,
        creator_id,
        reviewer_id: creator_id,
        created_at: now,
        audit_target: &audit_target,
    })?;

    tx.tx().execute(
        "INSERT INTO decision_evaluation_runs(
            evaluation_id, idempotency_key, request_digest, pipeline_version,
            config_hash, model_registry_id, model_registry_version,
            model_registry_digest, model_artifact_digest, judgment_set_ref,
            judgment_set_source, judgment_set_digest, judgment_set_count,
            judgment_set_frozen_at, manifest_json, report_json, acceptance_state,
            acceptance_bars_json, creator_id, reviewer_id, created_at,
            record_digest, audit_target
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                   ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)",
        rusqlite::params![
            evaluation_id,
            body.idempotency_key,
            request_digest,
            body.target.pipeline_version,
            body.target.config_hash,
            body.target.model_registry_id,
            body.target.model_registry_version,
            body.target.model_registry_digest,
            body.target.model_artifact_digest,
            body.judgment_set.judgment_set_ref,
            body.judgment_set.source,
            body.judgment_set.manifest_digest,
            body.judgment_set.cases.len() as i64,
            body.judgment_set.frozen_at,
            manifest_json,
            report_json,
            ACCEPTANCE_STATE,
            acceptance_bars_json,
            creator_id,
            creator_id,
            now,
            record_digest,
            audit_target,
        ],
    )?;
    crate::audit::record_tenant_checked(
        tx.tx(),
        crate::audit::AuditKind::Workflow,
        creator_id,
        &audit_target,
        crate::audit::AuditStatus::Ok,
        &format!(
            "evaluation acceptance={ACCEPTANCE_STATE} record_digest={record_digest} source={JUDGMENT_SOURCE}"
        ),
        "global",
    )
    .map_err(|error| EvaluationError::Audit(error.to_string()))?;
    tx.commit()?;

    Ok(EvaluationWriteReceipt {
        record: EvaluationRecord {
            evaluation_id,
            idempotency_key: body.idempotency_key,
            request_digest,
            target: body.target,
            judgment_set: body.judgment_set,
            report,
            acceptance_state: ACCEPTANCE_STATE.to_string(),
            acceptance_bars: serde_json::from_str(&acceptance_bars_json).map_err(|error| {
                evaluation_invalid("evaluation_bars_read_failed", error.to_string())
            })?,
            creator_id: creator_id.to_string(),
            reviewer_id: creator_id.to_string(),
            created_at: now,
            record_digest,
            audit_target,
        },
        created: true,
    })
}

/// Read one evaluation by stable id and verify its canonical record digest.
pub(crate) fn get_evaluation(
    conn: &Connection,
    evaluation_id: &str,
) -> Result<Option<EvaluationRecord>, EvaluationError> {
    let sql = format!(
        "SELECT {EVALUATION_ROW_COLUMNS} FROM decision_evaluation_runs WHERE evaluation_id = ?1"
    );
    let raw = conn
        .query_row(&sql, [evaluation_id], RawEvaluationRow::from_row)
        .optional()?;
    raw.map(record_from_raw).transpose()
}

/// A bounded newest-first listing. It never parses or emits manifests/reports.
pub(crate) fn list_evaluations(
    conn: &Connection,
    limit: usize,
) -> Result<Vec<EvaluationListRow>, EvaluationError> {
    if limit == 0 || limit > MAX_LIST_LIMIT {
        return Err(evaluation_invalid(
            "evaluation_limit_out_of_bounds",
            format!("limit must land inside 1..={MAX_LIST_LIMIT}"),
        ));
    }
    let mut statement = conn.prepare(
        "SELECT evaluation_id, pipeline_version, config_hash, model_registry_id,
                model_registry_version, judgment_set_ref, judgment_set_digest,
                judgment_set_count, acceptance_state, created_at
         FROM decision_evaluation_runs
         ORDER BY created_at DESC, evaluation_id DESC
         LIMIT ?1",
    )?;
    let rows = statement.query_map([limit as i64], |row| {
        Ok(EvaluationListRow {
            evaluation_id: row.get(0)?,
            pipeline_version: row.get(1)?,
            config_hash: row.get(2)?,
            model_registry_id: row.get(3)?,
            model_registry_version: row.get(4)?,
            judgment_set_ref: row.get(5)?,
            judgment_set_digest: row.get(6)?,
            judgment_set_count: row.get(7)?,
            acceptance_state: row.get(8)?,
            created_at: row.get(9)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(EvaluationError::from)
}

/// Verify a stored row without exposing its report. Used by focused tests.
pub(crate) fn verify_record_digest(record: &EvaluationRecord) -> Result<(), EvaluationError> {
    let manifest_json = serde_json::to_string(&record.judgment_set).map_err(|error| {
        evaluation_invalid(
            "evaluation_manifest_serialization_failed",
            error.to_string(),
        )
    })?;
    let report_json = serde_json::to_string(&record.report).map_err(|error| {
        evaluation_invalid("evaluation_report_serialization_failed", error.to_string())
    })?;
    let bars_json = serde_json::to_string(&record.acceptance_bars).map_err(|error| {
        evaluation_invalid("evaluation_bars_serialization_failed", error.to_string())
    })?;
    let expected = record_digest_from_parts(RecordDigestParts {
        evaluation_id: &record.evaluation_id,
        idempotency_key: &record.idempotency_key,
        request_digest: &record.request_digest,
        target: &record.target,
        judgment_set: &record.judgment_set,
        manifest_json: &manifest_json,
        report_json: &report_json,
        acceptance_bars_json: &bars_json,
        creator_id: &record.creator_id,
        reviewer_id: &record.reviewer_id,
        created_at: record.created_at,
        audit_target: &record.audit_target,
    })?;
    if expected == record.record_digest {
        Ok(())
    } else {
        Err(EvaluationError::RecordTampered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migration;
    use crate::register_sqlite_vec::register_sqlite_vec;
    use crate::workflow::harness::config::{RetrievalLeg, RetrievalParams};
    use crate::workflow::harness::pipeline::{ModelRefRecord, RegistryRef};
    use crate::workflow::harness::trace::{EnvFingerprint, TraceOutcome};
    use crate::workflow::registry::test_support::{seed_promoted_rules_model, seed_rules_model};

    const NOW: i64 = 1_800_000_000;
    const RULES: &str = r#"{
      "model_id": "rules-eval",
      "model_version": "1.0.0",
      "rules": [
        { "question_id": "q", "min_evidence": 1, "min_tier": "untrusted",
          "output": { "Choice": { "options": ["a", "b"], "label": "a" } } }
      ]
    }"#;

    fn db() -> Connection {
        register_sqlite_vec();
        let mut connection = Connection::open_in_memory().unwrap();
        run_migration(&mut connection, 1).unwrap();
        connection
    }

    fn trace(_trace_id: i64, run_id: i64, _model_digest: &str) -> DecisionRunTrace {
        DecisionRunTrace {
            run_id,
            pipeline_version: "1.32.11".into(),
            mode: "deterministic".into(),
            config_hash: "a".repeat(64),
            model_refs: vec![ModelRefRecord {
                id: "rules-eval".into(),
                version: "1.0.0".into(),
                weights_digest: None,
                registry_ref: Some(RegistryRef {
                    registry_id: "rules-eval".into(),
                    registry_version: "1.0.0".into(),
                }),
            }],
            input_digest: "b".repeat(64),
            context_refs: vec![crate::workflow::harness::pipeline::ContextHit {
                evidence_id: "42".into(),
                content_digest: "c".repeat(64),
                tier: crate::workflow::harness::pipeline::SerdeTier::Vetted,
                vector_rank: Some(1),
                fts_rank: None,
                graph_rank: None,
                fused_score: Some(1.0),
                flagged: false,
                untrusted: false,
            }],
            retrieval_params: RetrievalParams {
                rrf_k: 60,
                limit: 5,
                leg: RetrievalLeg::Both,
            },
            env_fingerprint: EnvFingerprint {
                kernel_version: "test".into(),
                feature_flags: Vec::new(),
            },
            stages: Vec::new(),
            outcome: TraceOutcome {
                action: ActionLabel::Act,
                escalation: None,
                output: Some(SerdeDecisionOutput {
                    model_id: "rules-eval".into(),
                    model_version: "1.0.0".into(),
                    produced_at: NOW,
                    value: SerdeDecisionValue::Choice {
                        label: "a".into(),
                        probabilities: Some(vec![("a".into(), 0.9), ("b".into(), 0.1)]),
                        confidence: Some(0.8),
                    },
                    evidence_refs: Vec::new(),
                }),
            },
        }
    }

    fn body(trace_id: i64, config_hash: &str, registry_digest: &str) -> CreateEvaluationBody {
        let expected = SerdeDecisionOutput {
            model_id: "rules-eval".into(),
            model_version: "1.0.0".into(),
            produced_at: NOW,
            value: SerdeDecisionValue::Choice {
                label: "a".into(),
                probabilities: Some(vec![("a".into(), 0.9), ("b".into(), 0.1)]),
                confidence: Some(0.8),
            },
            evidence_refs: Vec::new(),
        };
        let mut case = JudgmentCase {
            case_id: "case-1".into(),
            trace_id,
            relevant_evidence_ids: vec!["42".into()],
            expected_action: "act".into(),
            expected_output_label: "choice".into(),
            expected_output_digest: decision_output_digest(Some(&expected)).unwrap(),
            case_digest: "0".repeat(64),
        };
        case.case_digest = judgment_case_digest(&case).unwrap();
        let mut manifest = JudgmentSetManifest {
            source: JUDGMENT_SOURCE.into(),
            authoritative: false,
            judgment_set_ref: "operator-set-1".into(),
            frozen_at: NOW - 10,
            manifest_digest: "0".repeat(64),
            cases: vec![case],
        };
        manifest.manifest_digest = judgment_manifest_digest(&manifest).unwrap();
        CreateEvaluationBody {
            idempotency_key: "eval-request-1".into(),
            target: EvaluationTarget {
                pipeline_version: "1.32.11".into(),
                config_hash: config_hash.into(),
                model_registry_id: "rules-eval".into(),
                model_registry_version: "1.0.0".into(),
                model_registry_digest: registry_digest.into(),
                model_artifact_digest: None,
            },
            judgment_set: manifest,
        }
    }

    fn seed_trace(connection: &Connection, trace_id: i64, registry_digest: &str) {
        let value = trace(trace_id, 900 + trace_id, registry_digest);
        connection
            .execute(
                "INSERT INTO decision_run_traces(id, run_id, mode, pipeline_version, config_hash, trace_json, created_at)
                 VALUES (?1, ?2, 'deterministic', '1.32.11', ?3, ?4, ?5)",
                rusqlite::params![
                    trace_id,
                    value.run_id,
                    value.config_hash,
                    serde_json::to_string(&value).unwrap(),
                    NOW,
                ],
            )
            .unwrap();
    }

    #[test]
    fn evaluation_record_binds_target_and_judgment_set_by_digest() {
        let mut connection = db();
        let (id, version, digest) = seed_promoted_rules_model(&connection, RULES);
        assert_eq!(id, "rules-eval");
        assert_eq!(version, "1.0.0");
        seed_trace(&connection, 7, &digest);
        let request = body(7, &"a".repeat(64), &digest);
        let receipt = create_evaluation(&mut connection, request, "operator", NOW).unwrap();
        assert!(receipt.created);
        assert_eq!(receipt.record.judgment_set.manifest_digest.len(), 64);
        assert_eq!(receipt.record.target.model_registry_digest, digest);
        assert!(verify_record_digest(&receipt.record).is_ok());
    }

    #[test]
    fn evaluation_report_is_reproducible_for_identical_inputs() {
        let mut connection = db();
        let (_, _, digest) = seed_promoted_rules_model(&connection, RULES);
        seed_trace(&connection, 7, &digest);
        let first = create_evaluation(
            &mut connection,
            body(7, &"a".repeat(64), &digest),
            "operator",
            NOW,
        )
        .unwrap();
        let mut replay_connection = db();
        let (_, _, replay_digest) = seed_promoted_rules_model(&replay_connection, RULES);
        seed_trace(&replay_connection, 7, &replay_digest);
        let second = create_evaluation(
            &mut replay_connection,
            body(7, &"a".repeat(64), &replay_digest),
            "operator",
            NOW,
        )
        .unwrap();
        assert_eq!(first.record.report, second.record.report);
        assert_eq!(first.record.record_digest, second.record.record_digest);
    }

    #[test]
    fn acceptance_bars_never_auto_gate_promotion() {
        let mut connection = db();
        let (_, _, digest) = seed_rules_model(&connection, RULES, "candidate");
        seed_trace(&connection, 7, &digest);
        let receipt = create_evaluation(
            &mut connection,
            body(7, &"a".repeat(64), &digest),
            "operator",
            NOW,
        )
        .unwrap();
        assert!(receipt.record.report.acceptance_reported_as_data_only);
        let status: Option<String> = connection
            .query_row(
                "SELECT status FROM decision_model_registry WHERE id = 'rules-eval' AND version = '1.0.0'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status.as_deref(), Some("candidate"));
    }

    #[test]
    fn evaluation_acceptance_audit_verifies_and_tamper_fails() {
        let mut connection = db();
        let (_, _, digest) = seed_promoted_rules_model(&connection, RULES);
        seed_trace(&connection, 7, &digest);
        let receipt = create_evaluation(
            &mut connection,
            body(7, &"a".repeat(64), &digest),
            "operator",
            NOW,
        )
        .unwrap();
        assert!(crate::audit::verify_chain(&connection));
        let mut tampered = receipt.record.clone();
        tampered.report.case_results[0].action_match =
            !tampered.report.case_results[0].action_match;
        assert!(matches!(
            verify_record_digest(&tampered),
            Err(EvaluationError::RecordTampered)
        ));
    }

    #[test]
    fn evaluation_of_learned_model_without_digest_is_rejected() {
        let mut connection = db();
        let artifact = "d".repeat(64);
        connection
            .execute(
                "INSERT INTO decision_model_registry(id, version, kind, name, output_vocabulary,
                    artifact_digest, config_digest, calibration_ref, status, evaluation_refs,
                    proposed_by, approved_by, created_at, updated_at)
                 VALUES ('learned-eval', '1.0.0', 'learned', 'learned-eval', '[\"choice\"]',
                    ?1, ?2, NULL, 'candidate', '[]', 'test', NULL, ?3, ?3)",
                rusqlite::params![artifact, "e".repeat(64), NOW],
            )
            .unwrap();
        let mut request = body(7, &"a".repeat(64), &"e".repeat(64));
        request.target.model_registry_id = "learned-eval".into();
        request.target.model_artifact_digest = None;
        let mut trace = trace(7, 907, &"e".repeat(64));
        trace.model_refs[0].id = "learned-eval".into();
        trace.model_refs[0].weights_digest = Some(artifact.clone());
        trace.model_refs[0].registry_ref = Some(RegistryRef {
            registry_id: "learned-eval".into(),
            registry_version: "1.0.0".into(),
        });
        connection
            .execute(
                "INSERT INTO decision_run_traces(id, run_id, mode, pipeline_version, config_hash, trace_json, created_at)
                 VALUES (7, 907, 'deterministic', '1.32.11', ?1, ?2, ?3)",
                rusqlite::params![
                    "a".repeat(64),
                    serde_json::to_string(&trace).unwrap(),
                    NOW
                ],
            )
            .unwrap();
        assert!(matches!(
            create_evaluation(&mut connection, request, "operator", NOW),
            Err(EvaluationError::ModelArtifactDigestRequired)
        ));
    }

    #[test]
    fn judgment_set_digest_mismatch_refuses_record() {
        let mut connection = db();
        let (_, _, digest) = seed_promoted_rules_model(&connection, RULES);
        seed_trace(&connection, 7, &digest);
        let mut request = body(7, &"a".repeat(64), &digest);
        request.judgment_set.cases[0].expected_action = "reject".into();
        assert!(matches!(
            create_evaluation(&mut connection, request.clone(), "operator", NOW),
            Err(EvaluationError::Invalid { code, .. }) if code == "judgment_case_digest_mismatch"
        ));
        request.judgment_set.cases[0].case_digest =
            judgment_case_digest(&request.judgment_set.cases[0]).unwrap();
        assert!(matches!(
            create_evaluation(&mut connection, request, "operator", NOW),
            Err(EvaluationError::Invalid { code, .. }) if code == "judgment_set_digest_mismatch"
        ));
    }

    #[test]
    fn frozen_set_mutation_after_record_creation_refuses() {
        let mut connection = db();
        let (_, _, digest) = seed_promoted_rules_model(&connection, RULES);
        seed_trace(&connection, 7, &digest);
        let request = body(7, &"a".repeat(64), &digest);
        let first = create_evaluation(&mut connection, request.clone(), "operator", NOW).unwrap();
        let mut changed = request;
        changed.judgment_set.cases[0].expected_action = "reject".into();
        changed.judgment_set.cases[0].case_digest =
            judgment_case_digest(&changed.judgment_set.cases[0]).unwrap();
        changed.judgment_set.manifest_digest =
            judgment_manifest_digest(&changed.judgment_set).unwrap();
        assert!(matches!(
            create_evaluation(&mut connection, changed, "operator", NOW),
            Err(EvaluationError::IdempotencyConflict)
        ));
        let stored = get_evaluation(&connection, &first.record.evaluation_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            stored.judgment_set.manifest_digest,
            first.record.judgment_set.manifest_digest
        );
    }

    #[test]
    fn empty_and_oversized_judgment_set_refuses() {
        let mut connection = db();
        let (_, _, digest) = seed_promoted_rules_model(&connection, RULES);
        seed_trace(&connection, 7, &digest);
        let mut empty = body(7, &"a".repeat(64), &digest);
        empty.judgment_set.cases.clear();
        empty.judgment_set.manifest_digest = judgment_manifest_digest(&empty.judgment_set).unwrap();
        assert!(matches!(
            create_evaluation(&mut connection, empty, "operator", NOW),
            Err(EvaluationError::Invalid { code, .. }) if code == "judgment_set_size_invalid"
        ));
        let mut oversized = body(7, &"a".repeat(64), &digest);
        oversized.judgment_set.cases = (0..=MAX_JUDGMENT_CASES)
            .map(|index| JudgmentCase {
                case_id: format!("case-{index:04}"),
                trace_id: 7,
                relevant_evidence_ids: vec!["42".into()],
                expected_action: "act".into(),
                expected_output_label: "choice".into(),
                expected_output_digest: "a".repeat(64),
                case_digest: "0".repeat(64),
            })
            .collect();
        assert!(matches!(
            create_evaluation(&mut connection, oversized, "operator", NOW),
            Err(EvaluationError::Invalid { code, .. }) if code == "judgment_set_size_invalid"
        ));
    }

    #[test]
    fn unavailable_leg_is_not_zero() {
        let connection = db();
        let (_, _, digest) = seed_promoted_rules_model(&connection, RULES);
        seed_trace(&connection, 7, &digest);
        let report = evaluate_prepared(
            &body(7, &"a".repeat(64), &digest),
            &BTreeMap::from([(7, trace(7, 907, &digest))]),
            NOW,
        )
        .unwrap();
        assert_eq!(report.ood_abstention.status, "unavailable");
        assert!(report.ood_abstention.value.is_none());
        assert_eq!(report.red_team.status, "unavailable");
    }

    #[test]
    fn raw_query_and_evidence_text_are_absent_from_durable_record() {
        let mut connection = db();
        let (_, _, digest) = seed_promoted_rules_model(&connection, RULES);
        seed_trace(&connection, 7, &digest);
        let receipt = create_evaluation(
            &mut connection,
            body(7, &"a".repeat(64), &digest),
            "operator",
            NOW,
        )
        .unwrap();
        let (manifest, report): (String, String) = connection
            .query_row(
                "SELECT manifest_json, report_json FROM decision_evaluation_runs WHERE evaluation_id = ?1",
                [&receipt.record.evaluation_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let emitted = serde_json::to_string(&receipt.record).unwrap();
        assert!(!manifest.contains("query"));
        assert!(!manifest.contains("evidence text"));
        assert!(!report.contains("raw"));
        assert!(!emitted.contains("query"));
        assert!(!emitted.contains("evidence text"));
    }

    #[test]
    fn target_model_config_and_pipeline_mismatch_refuses() {
        let mut connection = db();
        let (_, _, digest) = seed_promoted_rules_model(&connection, RULES);
        seed_trace(&connection, 7, &digest);
        let mut request = body(7, &"a".repeat(64), &digest);
        request.target.pipeline_version = "1.32.10".into();
        assert!(matches!(
            create_evaluation(&mut connection, request, "operator", NOW),
            Err(EvaluationError::TargetMismatch)
        ));
    }

    #[test]
    fn duplicate_idempotent_evaluation_returns_original() {
        let mut connection = db();
        let (_, _, digest) = seed_promoted_rules_model(&connection, RULES);
        seed_trace(&connection, 7, &digest);
        let request = body(7, &"a".repeat(64), &digest);
        let first = create_evaluation(&mut connection, request.clone(), "operator", NOW).unwrap();
        let second = create_evaluation(&mut connection, request, "operator", NOW + 1).unwrap();
        assert!(!second.created);
        assert_eq!(first.record.record_digest, second.record.record_digest);
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM decision_evaluation_runs", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn evaluation_audit_failure_rolls_back_record() {
        let mut connection = db();
        let (_, _, digest) = seed_promoted_rules_model(&connection, RULES);
        seed_trace(&connection, 7, &digest);
        connection.execute("DROP TABLE audit_events", []).unwrap();
        assert!(
            create_evaluation(
                &mut connection,
                body(7, &"a".repeat(64), &digest),
                "operator",
                NOW,
            )
            .is_err()
        );
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM decision_evaluation_runs", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn evaluation_record_commit_race_does_not_duplicate() {
        let mut connection = db();
        let (_, _, digest) = seed_promoted_rules_model(&connection, RULES);
        seed_trace(&connection, 7, &digest);
        let request = body(7, &"a".repeat(64), &digest);
        let first = create_evaluation(&mut connection, request.clone(), "operator", NOW).unwrap();
        let second = create_evaluation(&mut connection, request, "operator", NOW).unwrap();
        assert_eq!(first.record.evaluation_id, second.record.evaluation_id);
        let ids: i64 = connection
            .query_row(
                "SELECT COUNT(DISTINCT evaluation_id) FROM decision_evaluation_runs",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(ids, 1);
    }

    #[test]
    fn unknown_evaluation_json_fields_refuse() {
        let raw = r#"{"idempotency_key":"k","target":{},"judgment_set":{},"raw_query":"secret"}"#;
        assert!(serde_json::from_str::<CreateEvaluationBody>(raw).is_err());
    }

    #[test]
    fn evaluation_numeric_and_string_bounds_refuse() {
        let mut connection = db();
        let (_, _, digest) = seed_promoted_rules_model(&connection, RULES);
        seed_trace(&connection, 7, &digest);
        let mut request = body(7, &"a".repeat(64), &digest);
        request.idempotency_key = "x".repeat(MAX_IDEMPOTENCY_KEY_LEN + 1);
        assert!(matches!(
            create_evaluation(&mut connection, request, "operator", NOW),
            Err(EvaluationError::Invalid { code, .. }) if code == "evaluation_idempotency_key_invalid"
        ));
    }

    #[test]
    fn absent_and_unauthorized_evaluation_are_probe_blind() {
        let connection = db();
        assert!(
            get_evaluation(&connection, "eval_missing")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn evaluation_creation_never_changes_registry_status() {
        let mut connection = db();
        let (_, _, digest) = seed_promoted_rules_model(&connection, RULES);
        seed_trace(&connection, 7, &digest);
        let before: String = connection
            .query_row(
                "SELECT status FROM decision_model_registry WHERE id='rules-eval' AND version='1.0.0'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        create_evaluation(
            &mut connection,
            body(7, &"a".repeat(64), &digest),
            "operator",
            NOW,
        )
        .unwrap();
        let after: String = connection
            .query_row(
                "SELECT status FROM decision_model_registry WHERE id='rules-eval' AND version='1.0.0'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn evaluation_refs_remain_fail_closed_without_verified_producer() {
        let connection = db();
        let (_, _, digest) = seed_promoted_rules_model(&connection, RULES);
        seed_trace(&connection, 7, &digest);
        let mut row = registry::row_by_ref(&connection, "rules-eval", "1.0.0")
            .unwrap()
            .unwrap();
        row.evaluation_refs = vec!["eval_forged".into()];
        let payload = serde_json::json!({
            "action": "promote",
            "id": row.id,
            "version": row.version,
            "row_digest": registry::row_digest(&row).unwrap(),
            "row": row,
        });
        assert!(registry::parse_lifecycle_payload(&payload.to_string()).is_err());
        let _ = digest;
    }
}
