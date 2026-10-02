//! The decision-model registry: the substrate's digest-pinned model
//! identities and their governed lifecycle.
//!
//! One row per (id, version): the identity a model declares (the
//! `DecisionModel::metadata()` law — a model registers by declaring
//! itself), the digests that pin it (the canonical config digest; the
//! artifact digest a learned kind MUST carry), and a lifecycle no route
//! can write directly: `candidate → promoted` and any live state →
//! `retired` move ONLY through the human gate's `registry_lifecycle`
//! proposal kind (propose through `POST /ingest/proposal`, dispose
//! through the approve gate — the dual-leg law; the gate's branch flips
//! the row inside the approval transaction with the digest re-verified
//! against the live row). Registration content is never stored: the
//! canonical digest IS the pin, so a row carries identity + digests +
//! vocabulary and nothing else.
//!
//! The harness consumes this table by LAW: a deterministic pipeline run
//! resolves its `(config.model.key, config.model.digest)` binding here
//! and refuses named unless the bound row is promoted; an exploratory run
//! may ride a candidate (that is what exploration is for) but never an
//! unregistered or retired one. The citation travels on the run's trace.
//!
//! Protocol adapters carry no statements of their own (the no-SQL law
//! counts the handler files live) — every read and write in this module
//! takes the caller's connection or transaction, and every write lands
//! its audit row inside the caller's own transition.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use brain_engine_sdk::decision::{DecisionModel, ModelMetadata, OutputKind};

use crate::workflow::tx::WorkflowTx;

/// The closed registration kinds. `deterministic-rules` registers by
/// submitting its rules document (the server derives the identity from
/// the table); `learned` and `reranker` declare identity + digests
/// directly (no in-tree builder exists — the operator vouches for the
/// artifact, the digest is the pin).
pub(crate) const KIND_DETERMINISTIC_RULES: &str = "deterministic-rules";
pub(crate) const KIND_LEARNED: &str = "learned";
pub(crate) const KIND_RERANKER: &str = "reranker";

/// The closed lifecycle statuses. `evaluated` is reachable only when
/// signed evaluation records exist (the evaluation-record line); no
/// write path produces it before then.
pub(crate) const STATUS_CANDIDATE: &str = "candidate";
pub(crate) const STATUS_EVALUATED: &str = "evaluated";
pub(crate) const STATUS_PROMOTED: &str = "promoted";
pub(crate) const STATUS_RETIRED: &str = "retired";

/// The proposal kind that moves a row through the human gate.
pub(crate) const PROP_KIND_REGISTRY_LIFECYCLE: &str = "registry_lifecycle";

/// The closed output vocabulary every registered model must stay inside
/// (the QType law the rule tables already enforce at load).
const OUTPUT_VOCABULARY: &[&str] = &["choice", "score", "noul"];

/// The metadata vocabulary's wire labels (the closed three, mirrored).
fn vocabulary_labels(kinds: &[OutputKind]) -> Vec<String> {
    kinds
        .iter()
        .map(|k| match k {
            OutputKind::Choice => "choice",
            OutputKind::Score => "score",
            OutputKind::Noul => "noul",
        })
        .map(str::to_string)
        .collect()
}

fn sdk_output_kinds(labels: &[String]) -> Result<Vec<OutputKind>, RegistryError> {
    labels
        .iter()
        .map(|label| match label.as_str() {
            "choice" => Ok(OutputKind::Choice),
            "score" => Ok(OutputKind::Score),
            "noul" => Ok(OutputKind::Noul),
            _ => Err(RegistryError::VocabularyInvalid),
        })
        .collect()
}

const MAX_ID_LEN: usize = 256;
const MAX_VERSION_LEN: usize = 64;
const REGISTRY_MAX_NAME_LEN: usize = 256;

fn is_registry_lower_hex_digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn valid_text(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && !value
            .chars()
            .any(|c| c.is_control() || crate::strip_invisible::is_invisible(c))
}

fn valid_declared_kind(kind: &str) -> bool {
    matches!(kind, KIND_LEARNED | KIND_RERANKER)
}

fn validate_stored_row(row: &RegistryRow) -> Result<(), rusqlite::Error> {
    if !valid_declared_kind(&row.kind) && row.kind != KIND_DETERMINISTIC_RULES {
        return Err(rusqlite::Error::InvalidParameterName(
            "registry row carries an unknown kind".to_string(),
        ));
    }
    if !valid_text(&row.id, MAX_ID_LEN)
        || row.id.contains('@')
        || !valid_text(&row.version, MAX_VERSION_LEN)
        || row.version.contains('@')
        || !valid_text(&row.name, REGISTRY_MAX_NAME_LEN)
        || row
            .calibration_ref
            .as_deref()
            .is_some_and(|value| !valid_text(value, REGISTRY_MAX_NAME_LEN))
    {
        return Err(rusqlite::Error::InvalidParameterName(
            "registry row carries invalid identity text".to_string(),
        ));
    }
    if row.output_vocabulary.is_empty()
        || !row
            .output_vocabulary
            .iter()
            .all(|value| OUTPUT_VOCABULARY.contains(&value.as_str()))
    {
        return Err(rusqlite::Error::InvalidParameterName(
            "registry row carries an invalid output vocabulary".to_string(),
        ));
    }
    if row.kind == KIND_LEARNED && row.artifact_digest.is_none() {
        return Err(rusqlite::Error::InvalidParameterName(
            "learned registry row has no artifact digest".to_string(),
        ));
    }
    if row
        .artifact_digest
        .as_deref()
        .is_some_and(|d| !is_registry_lower_hex_digest(d))
        || row
            .config_digest
            .as_deref()
            .is_some_and(|d| !is_registry_lower_hex_digest(d))
    {
        return Err(rusqlite::Error::InvalidParameterName(
            "registry row carries an invalid digest".to_string(),
        ));
    }
    if !matches!(
        row.status.as_str(),
        STATUS_CANDIDATE | STATUS_EVALUATED | STATUS_PROMOTED | STATUS_RETIRED
    ) {
        return Err(rusqlite::Error::InvalidParameterName(
            "registry row carries an invalid status".to_string(),
        ));
    }
    if !row.evaluation_refs.is_empty() {
        return Err(rusqlite::Error::InvalidParameterName(
            "registry row carries unavailable evaluation references".to_string(),
        ));
    }
    Ok(())
}

fn parse_json_column<T>(value: &str) -> rusqlite::Result<T>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_str(value).map_err(|e| {
        rusqlite::Error::InvalidParameterName(format!("registry JSON column refused: {e}"))
    })
}

/// A registry row: identity, digests, vocabulary, lifecycle. This serde
/// shape is ALSO the canonical form — the row digest a lifecycle proposal
/// pins (and the approval re-verifies) is the sha256 of this struct's
/// compact serialization, field order fixed by the declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegistryRow {
    pub(crate) id: String,
    pub(crate) version: String,
    pub(crate) kind: String,
    pub(crate) name: String,
    pub(crate) output_vocabulary: Vec<String>,
    pub(crate) artifact_digest: Option<String>,
    pub(crate) config_digest: Option<String>,
    pub(crate) calibration_ref: Option<String>,
    pub(crate) status: String,
    pub(crate) evaluation_refs: Vec<String>,
    pub(crate) proposed_by: String,
    pub(crate) approved_by: Option<String>,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}

/// The canonical bytes a `row_digest` pins: the row's compact serde form.
pub(crate) fn canonical_row_json(row: &RegistryRow) -> Result<String, String> {
    serde_json::to_string(row).map_err(|e| format!("registry row canonicalization failed: {e}"))
}

/// sha256 over [`canonical_row_json`] — the row's content pin.
pub(crate) fn row_digest(row: &RegistryRow) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let canonical = canonical_row_json(row)?;
    let mut h = Sha256::new();
    h.update(canonical.as_bytes());
    Ok(hex::encode(h.finalize()))
}

const ROW_COLUMNS: &str = "id, version, kind, name, output_vocabulary, artifact_digest, \
     config_digest, calibration_ref, status, evaluation_refs, proposed_by, approved_by, \
     created_at, updated_at";

fn registry_row_from(r: &rusqlite::Row<'_>) -> rusqlite::Result<RegistryRow> {
    let vocab_json: String = r.get(4)?;
    let eval_json: String = r.get(9)?;
    let row = RegistryRow {
        id: r.get(0)?,
        version: r.get(1)?,
        kind: r.get(2)?,
        name: r.get(3)?,
        output_vocabulary: parse_json_column(&vocab_json)?,
        artifact_digest: r.get(5)?,
        config_digest: r.get(6)?,
        calibration_ref: r.get(7)?,
        status: r.get(8)?,
        evaluation_refs: parse_json_column(&eval_json)?,
        proposed_by: r.get(10)?,
        approved_by: r.get(11)?,
        created_at: r.get(12)?,
        updated_at: r.get(13)?,
    };
    validate_stored_row(&row)?;
    Ok(row)
}

/// The machinery's error vocabulary: named at the wire by the surfaces.
#[derive(Debug)]
pub(crate) enum RegistryError {
    /// 400 `rules_config_invalid` — the rules document is hostile input.
    RulesConfig(String),
    /// 400 `registry_identity_declared` — the deterministic arm takes its
    /// identity from the table; body identity fields refuse.
    IdentityDeclared,
    /// 400 `registry_identity_required` — a declared kind without identity.
    IdentityRequired,
    /// 400 `registry_vocabulary_invalid` — outside the closed QType set.
    VocabularyInvalid,
    /// 400 `artifact_digest_required` — a learned kind must pin its artifact.
    ArtifactDigestRequired,
    /// 400 `artifact_digest_invalid` — not a sha256 hex pin.
    ArtifactDigestInvalid,
    /// 400 `config_digest_invalid` — not a sha256 hex pin.
    ConfigDigestInvalid,
    /// 400 `registry_id_invalid`.
    IdInvalid(&'static str),
    /// 400 `registry_version_invalid`.
    VersionInvalid,
    /// 400 `registry_kind_invalid`.
    KindInvalid,
    /// 409 `model_already_registered` — (id, version) is taken, loud.
    AlreadyRegistered { id: String, version: String },
    /// 400 `registry_payload_invalid` — the lifecycle payload is malformed.
    PayloadInvalid(String),
    /// 404 `registry_row_absent`.
    RowAbsent,
    /// 400 `registry_row_digest_mismatch` — the proposed bytes are not the
    /// stored bytes (creation-time check).
    RowDigestMismatch,
    /// 409 `registry_row_changed` — the row moved between propose and
    /// approve (approval-time closure).
    RowChanged,
    /// 409 `registry_transition_illegal`.
    TransitionIllegal { action: String, from: String },
    /// 500 — a required audit row could not be written.
    Audit(String),
    /// 500 — never a silent drop.
    Db(rusqlite::Error),
}

impl From<rusqlite::Error> for RegistryError {
    fn from(e: rusqlite::Error) -> Self {
        RegistryError::Db(e)
    }
}

/// A registration request, parsed by the surface from the request body.
/// The deterministic arm carries the RAW rules document (the artifact
/// bytes, operator-supplied in the body, never network-fetched); the
/// declared arms carry the identity directly.
#[derive(Debug)]
pub(crate) enum Registration {
    DeterministicRules {
        rules_json: String,
    },
    Declared {
        kind: &'static str,
        id: String,
        version: String,
        name: String,
        output_vocabulary: Vec<String>,
        artifact_digest: Option<String>,
        config_digest: Option<String>,
        calibration_ref: Option<String>,
    },
}

fn validate_declared_identity(
    id: &str,
    version: &str,
    name: &str,
    output_vocabulary: &[String],
) -> Result<(), RegistryError> {
    if !valid_text(id, MAX_ID_LEN) || id.contains('@') {
        return Err(RegistryError::IdInvalid(
            "id must be 1..=256 chars and carry no '@' (the citation-split law)",
        ));
    }
    if !valid_text(version, MAX_VERSION_LEN) || version.contains('@') {
        return Err(RegistryError::VersionInvalid);
    }
    if !valid_text(name, REGISTRY_MAX_NAME_LEN) {
        return Err(RegistryError::IdentityRequired);
    }
    if output_vocabulary.is_empty()
        || !output_vocabulary
            .iter()
            .all(|v| OUTPUT_VOCABULARY.contains(&v.as_str()))
    {
        return Err(RegistryError::VocabularyInvalid);
    }
    Ok(())
}

/// Build the row a registration declares: identity from the table for the
/// deterministic arm (the model declares itself — the metadata law), from
/// the request for the declared arms. Validation is total: the returned
/// row is well-formed by construction. NO writes here — the caller owns
/// the transition.
fn build_row(
    reg: &Registration,
    proposed_by: &str,
    now: i64,
) -> Result<RegistryRow, RegistryError> {
    match reg {
        Registration::DeterministicRules { rules_json } => {
            let model = crate::workflow::harness::models::rules::RulesModel::from_canonical_json(
                rules_json,
            )
            .map_err(RegistryError::RulesConfig)?;
            let meta = model.metadata();
            let expected_key = format!("rules:{}", meta.id());
            if model.config_key() != expected_key {
                return Err(RegistryError::RulesConfig(
                    "the model config key does not match its declared id".into(),
                ));
            }
            let vocabulary = vocabulary_labels(meta.output_vocabulary());
            let id = meta.id().to_string();
            let version = meta.version().to_string();
            if !valid_text(&id, MAX_ID_LEN) || id.contains('@') {
                return Err(RegistryError::IdInvalid(
                    "the table's model_id is not a bounded citation-safe id",
                ));
            }
            if !valid_text(&version, MAX_VERSION_LEN) || version.contains('@') {
                return Err(RegistryError::VersionInvalid);
            }
            if vocabulary.is_empty() {
                return Err(RegistryError::VocabularyInvalid);
            }
            Ok(RegistryRow {
                id,
                version: meta.version().to_string(),
                kind: KIND_DETERMINISTIC_RULES.to_string(),
                name: meta.id().to_string(),
                output_vocabulary: vocabulary,
                artifact_digest: None,
                config_digest: Some(model.digest().to_string()),
                calibration_ref: None,
                status: STATUS_CANDIDATE.to_string(),
                evaluation_refs: Vec::new(),
                proposed_by: proposed_by.to_string(),
                approved_by: None,
                created_at: now,
                updated_at: now,
            })
        }
        Registration::Declared {
            kind,
            id,
            version,
            name,
            output_vocabulary,
            artifact_digest,
            config_digest,
            calibration_ref,
        } => {
            if !valid_declared_kind(kind) {
                return Err(RegistryError::KindInvalid);
            }
            validate_declared_identity(id, version, name, output_vocabulary)?;
            if *kind == KIND_LEARNED {
                let Some(digest) = artifact_digest.as_deref() else {
                    return Err(RegistryError::ArtifactDigestRequired);
                };
                let _metadata = ModelMetadata::learned(
                    id.clone(),
                    version.clone(),
                    digest.to_string(),
                    sdk_output_kinds(output_vocabulary)?,
                    calibration_ref.clone(),
                );
            }
            if let Some(d) = artifact_digest.as_deref()
                && !is_registry_lower_hex_digest(d)
            {
                return Err(RegistryError::ArtifactDigestInvalid);
            }
            if let Some(d) = config_digest.as_deref()
                && !is_registry_lower_hex_digest(d)
            {
                return Err(RegistryError::ConfigDigestInvalid);
            }
            if calibration_ref
                .as_deref()
                .is_some_and(|value| !valid_text(value, REGISTRY_MAX_NAME_LEN))
            {
                return Err(RegistryError::IdentityRequired);
            }
            Ok(RegistryRow {
                id: id.clone(),
                version: version.clone(),
                kind: (*kind).to_string(),
                name: name.clone(),
                output_vocabulary: output_vocabulary.clone(),
                artifact_digest: artifact_digest.clone(),
                config_digest: config_digest.clone(),
                calibration_ref: calibration_ref.clone(),
                status: STATUS_CANDIDATE.to_string(),
                evaluation_refs: Vec::new(),
                proposed_by: proposed_by.to_string(),
                approved_by: None,
                created_at: now,
                updated_at: now,
            })
        }
    }
}

/// Register one model: validate the declaration, refuse a duplicate
/// (id, version) LOUD, insert the identity row, audit — all inside the
/// caller's own transition. Returns the stored row (status `candidate`).
pub(crate) fn register_model(
    tx: &mut WorkflowTx<'_>,
    reg: &Registration,
    proposed_by: &str,
    now: i64,
) -> Result<RegistryRow, RegistryError> {
    let row = build_row(reg, proposed_by, now)?;
    let existing: Option<String> = tx
        .tx()
        .query_row(
            "SELECT id FROM decision_model_registry WHERE id = ?1 AND version = ?2",
            rusqlite::params![row.id, row.version],
            |r| r.get(0),
        )
        .optional()?;
    if existing.is_some() {
        return Err(RegistryError::AlreadyRegistered {
            id: row.id.clone(),
            version: row.version.clone(),
        });
    }
    let vocab_json = serde_json::to_string(&row.output_vocabulary)
        .map_err(|e| RegistryError::PayloadInvalid(e.to_string()))?;
    tx.tx().execute(
        "INSERT INTO decision_model_registry(id, version, kind, name, output_vocabulary,
            artifact_digest, config_digest, calibration_ref, status, evaluation_refs,
            proposed_by, approved_by, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        rusqlite::params![
            row.id,
            row.version,
            row.kind,
            row.name,
            vocab_json,
            row.artifact_digest,
            row.config_digest,
            row.calibration_ref,
            row.status,
            "[]",
            row.proposed_by,
            row.approved_by,
            row.created_at,
            row.updated_at,
        ],
    )?;
    crate::audit::record_tenant_checked(
        tx.tx(),
        crate::audit::AuditKind::Workflow,
        proposed_by,
        &format!("registry:{}@{}", row.id, row.version),
        crate::audit::AuditStatus::Ok,
        &format!("register kind={} status={}", row.kind, row.status),
        "global",
    )
    .map_err(|e| RegistryError::Audit(e.to_string()))?;
    Ok(row)
}

/// The row for one `"{id}@{version}"` citation — the single-row read
/// (the digest VALUES ride only here, never on a listing).
pub(crate) fn row_by_ref(
    conn: &Connection,
    id: &str,
    version: &str,
) -> Result<Option<RegistryRow>, RegistryError> {
    let row = conn
        .query_row(
            &format!(
                "SELECT {ROW_COLUMNS} FROM decision_model_registry WHERE id = ?1 AND version = ?2"
            ),
            rusqlite::params![id, version],
            registry_row_from,
        )
        .optional()?;
    Ok(row)
}

/// One bounded listing row: the bounded columns ONLY — the artifact
/// digest VALUE never rides a listing (presence is the datum).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RegistryListRow {
    pub(crate) id: String,
    pub(crate) version: String,
    pub(crate) kind: String,
    pub(crate) name: String,
    pub(crate) output_vocabulary: Vec<String>,
    pub(crate) artifact_digest_present: bool,
    pub(crate) config_digest: Option<String>,
    pub(crate) status: String,
    pub(crate) proposed_by: String,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}

/// The bounded listing page: newest-first, optional closed-vocabulary
/// filters, caller clamps the limit.
pub(crate) fn list_rows(
    conn: &Connection,
    status: Option<&str>,
    kind: Option<&str>,
    limit: usize,
) -> rusqlite::Result<Vec<RegistryListRow>> {
    let sql = format!(
        "SELECT {ROW_COLUMNS} FROM decision_model_registry
         WHERE (?1 IS NULL OR status = ?1) AND (?2 IS NULL OR kind = ?2)
         ORDER BY rowid DESC
         LIMIT ?3"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params![status, kind, limit as i64], |r| {
        let row = registry_row_from(r)?;
        Ok(RegistryListRow {
            id: row.id,
            version: row.version,
            kind: row.kind,
            name: row.name,
            output_vocabulary: row.output_vocabulary,
            artifact_digest_present: row.artifact_digest.is_some(),
            config_digest: row.config_digest,
            status: row.status,
            proposed_by: row.proposed_by,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    })?;
    rows.collect()
}

/// The lifecycle payload a `registry_lifecycle` proposal carries: the
/// action, the row it names, the digest pinning the reviewed bytes, and
/// the FULL row for the reviewer's exact-bytes display (the Rule of Two).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LifecyclePayload {
    pub(crate) action: String,
    pub(crate) id: String,
    pub(crate) version: String,
    pub(crate) row_digest: String,
    pub(crate) row: RegistryRow,
}

/// Is the transition legal from this state? `promote`:
/// candidate|evaluated → promoted; `retire`: any live state → retired.
fn transition_legal(action: &str, from: &str) -> bool {
    match action {
        "promote" => matches!(from, STATUS_CANDIDATE | STATUS_EVALUATED),
        "retire" => matches!(from, STATUS_CANDIDATE | STATUS_EVALUATED | STATUS_PROMOTED),
        _ => false,
    }
}

/// Validate a lifecycle proposal at CREATION time (the queue only ever
/// holds lawful proposals): the payload parses, the row exists, the
/// pinned digest matches the LIVE row's canonical digest, and the
/// transition is legal. Anyone may propose; the gate disposes.
pub(crate) fn parse_lifecycle_payload(content: &str) -> Result<LifecyclePayload, RegistryError> {
    let payload: LifecyclePayload =
        serde_json::from_str(content).map_err(|e| RegistryError::PayloadInvalid(e.to_string()))?;
    if payload.action != "promote" && payload.action != "retire" {
        return Err(RegistryError::PayloadInvalid(
            "action must be promote or retire".into(),
        ));
    }
    if !payload.row.evaluation_refs.is_empty() {
        return Err(RegistryError::PayloadInvalid(
            "evaluation_refs are unavailable until the signed-record producer exists".into(),
        ));
    }
    Ok(payload)
}

pub(crate) fn validate_lifecycle_payload(
    conn: &Connection,
    content: &str,
) -> Result<LifecyclePayload, RegistryError> {
    let payload = parse_lifecycle_payload(content)?;
    let live = row_by_ref(conn, &payload.id, &payload.version)?.ok_or(RegistryError::RowAbsent)?;
    let live_digest = row_digest(&live).map_err(RegistryError::PayloadInvalid)?;
    let displayed_digest = row_digest(&payload.row).map_err(RegistryError::PayloadInvalid)?;
    if payload.row != live
        || displayed_digest != payload.row_digest
        || live_digest != payload.row_digest
    {
        return Err(RegistryError::RowDigestMismatch);
    }
    if !transition_legal(&payload.action, &live.status) {
        return Err(RegistryError::TransitionIllegal {
            action: payload.action,
            from: live.status,
        });
    }
    Ok(payload)
}

/// THE GATE'S flip: re-load the row INSIDE the approval transaction,
/// re-verify the digest against the LIVE row (a row that moved between
/// propose and approve refuses by name — the closure), re-check the
/// transition, CAS the status, audit. NO knowledge chunk is created —
/// the caller returns before the generic promote.
pub(crate) fn apply_lifecycle(
    tx: &rusqlite::Transaction<'_>,
    payload: &LifecyclePayload,
    approver: &str,
    now: i64,
) -> Result<String, RegistryError> {
    if !payload.row.evaluation_refs.is_empty() {
        return Err(RegistryError::PayloadInvalid(
            "evaluation_refs are unavailable until the signed-record producer exists".into(),
        ));
    }
    let live = row_by_ref(tx, &payload.id, &payload.version)?.ok_or(RegistryError::RowAbsent)?;
    let live_digest = row_digest(&live).map_err(RegistryError::PayloadInvalid)?;
    let displayed_digest = row_digest(&payload.row).map_err(RegistryError::PayloadInvalid)?;
    if payload.row != live
        || displayed_digest != payload.row_digest
        || live_digest != payload.row_digest
    {
        return Err(RegistryError::RowChanged);
    }
    if !transition_legal(&payload.action, &live.status) {
        return Err(RegistryError::TransitionIllegal {
            action: payload.action.clone(),
            from: live.status.clone(),
        });
    }
    let new_status = if payload.action == "promote" {
        STATUS_PROMOTED
    } else {
        STATUS_RETIRED
    };
    let moved = tx.execute(
        "UPDATE decision_model_registry
         SET status = ?1, approved_by = ?2, updated_at = ?3
         WHERE id = ?4 AND version = ?5 AND status = ?6",
        rusqlite::params![
            new_status,
            approver,
            now,
            payload.id,
            payload.version,
            live.status
        ],
    )?;
    if moved == 0 {
        return Err(RegistryError::RowChanged);
    }
    crate::audit::record_tenant_checked(
        tx,
        crate::audit::AuditKind::Workflow,
        approver,
        &format!("registry:{}@{}", payload.id, payload.version),
        crate::audit::AuditStatus::Ok,
        &format!(
            "gate/{} status {}->{}",
            payload.action, live.status, new_status
        ),
        "global",
    )
    .map_err(|e| RegistryError::Audit(e.to_string()))?;
    Ok(new_status.to_string())
}

/// The consumption law's refusal vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ResolveRefusal {
    /// The binding names no registered row (or the key is not a
    /// registry-shaped key at all).
    NotRegistered,
    /// Registered, but no promoted row exists for a deterministic run.
    NotPromoted,
    /// Every row this binding resolves to is retired.
    Retired,
}

/// The status preference for a resolved binding: the most-lifecycle-
/// advanced acceptable row wins (promoted over evaluated over candidate),
/// newest on ties.
fn status_rank(status: &str) -> u8 {
    match status {
        STATUS_PROMOTED => 0,
        STATUS_EVALUATED => 1,
        STATUS_CANDIDATE => 2,
        _ => 3,
    }
}

/// A binding's row with its physical insertion order, so the pick is
/// stable across equal-status rows.
#[derive(Debug, Clone, PartialEq)]
struct BindingRow {
    row: RegistryRow,
    rowid: i64,
}

fn row_query_for_binding(
    conn: &Connection,
    id: &str,
    digest: &str,
) -> rusqlite::Result<Vec<BindingRow>> {
    let sql = format!(
        "SELECT rowid, {ROW_COLUMNS} FROM decision_model_registry
         WHERE id = ?1 AND config_digest = ?2 AND kind = ?3"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        rusqlite::params![id, digest, KIND_DETERMINISTIC_RULES],
        |r| {
            let rowid: i64 = r.get(0)?;
            let row = row_from_shifted(r)?;
            Ok(BindingRow { row, rowid })
        },
    )?;
    rows.collect()
}

/// The row mapping over a rowid-shifted projection: [`row_from`] with
/// every column index +1.
fn row_from_shifted(r: &rusqlite::Row<'_>) -> rusqlite::Result<RegistryRow> {
    let vocab_json: String = r.get(5)?;
    let eval_json: String = r.get(10)?;
    let row = RegistryRow {
        id: r.get(1)?,
        version: r.get(2)?,
        kind: r.get(3)?,
        name: r.get(4)?,
        output_vocabulary: parse_json_column(&vocab_json)?,
        artifact_digest: r.get(6)?,
        config_digest: r.get(7)?,
        calibration_ref: r.get(8)?,
        status: r.get(9)?,
        evaluation_refs: parse_json_column(&eval_json)?,
        proposed_by: r.get(11)?,
        approved_by: r.get(12)?,
        created_at: r.get(13)?,
        updated_at: r.get(14)?,
    };
    validate_stored_row(&row)?;
    Ok(row)
}

/// THE consumption law: resolve a run's model binding `(key, digest)`
/// through the registry. The key must name a registry model (the
/// `rules:{id}` config-key shape); the rows are looked up by
/// (id, config_digest). Deterministic mode accepts ONLY `promoted`;
/// exploratory mode accepts candidate|evaluated|promoted. Unregistered
/// and retired refuse named — a pipeline never executes a model the
/// substrate cannot vouch for. The outer error is the store's (a 500),
/// the inner the law's (a named 400).
pub(crate) fn resolve_for_execution(
    conn: &Connection,
    key: &str,
    digest: &str,
    mode: &str,
) -> Result<Result<RegistryRow, ResolveRefusal>, rusqlite::Error> {
    let Some(id) = key.strip_prefix("rules:") else {
        return Ok(Err(ResolveRefusal::NotRegistered));
    };
    if id.is_empty() {
        return Ok(Err(ResolveRefusal::NotRegistered));
    }
    let rows = row_query_for_binding(conn, id, digest)?;
    if rows.is_empty() {
        return Ok(Err(ResolveRefusal::NotRegistered));
    }
    let deterministic = match mode {
        "deterministic" => true,
        "exploratory" => false,
        _ => return Ok(Err(ResolveRefusal::NotRegistered)),
    };
    let mut acceptable: Vec<BindingRow> = rows
        .iter()
        .filter(|b| {
            if deterministic {
                b.row.status == STATUS_PROMOTED
            } else {
                matches!(
                    b.row.status.as_str(),
                    STATUS_CANDIDATE | STATUS_EVALUATED | STATUS_PROMOTED
                )
            }
        })
        .cloned()
        .collect();
    acceptable.sort_by_key(|b| (status_rank(&b.row.status), std::cmp::Reverse(b.rowid)));
    if let Some(best) = acceptable.first() {
        return Ok(Ok(best.row.clone()));
    }
    if rows.iter().all(|b| b.row.status == STATUS_RETIRED) {
        Ok(Err(ResolveRefusal::Retired))
    } else {
        Ok(Err(ResolveRefusal::NotPromoted))
    }
}

/// The artifact digest a resolved row CITES, or `None` when the row has no
/// artifact to name.
///
/// R67D-CLOSURE. This closes a measured contradiction between three laws, each
/// individually correct and jointly unsatisfiable:
///
/// | # | law | site | requires |
/// |---|---|---|---|
/// | 1 | the execution binding | `resolve_for_execution` | `kind = deterministic-rules` |
/// | 2 | the artifact rule | `validate_stored_row:136` | `artifact_digest` only for `learned` |
/// | 3 | the citation | `delivery::resolve_citation` | a non-`NULL` artifact digest |
///
/// A `deterministic-rules` row is the only bindable kind, and it is the one kind
/// that carries no `artifact_digest` — by construction, not by configuration:
/// `validate_stored_row` requires the artifact only for `learned`, the
/// registration handler REFUSES a rules row that declares one
/// (`handlers/model_registry.rs:177` → `IdentityDeclared`), and the registry's
/// own pin asserts `config_digest: Some(..)` with `artifact_digest: None`. So
/// law 3 could never be satisfied by law 1, no delivery run could acquire a
/// `model_ref`, and the agreement queue was structurally empty.
///
/// **The resolution is that for a rules row the two digests name the SAME
/// bytes.** The module doc already says it: *"Registration content is never
/// stored: the canonical digest IS the pin."* For a rules table the content is
/// the rules document, so the canonical digest of that document is precisely
/// what a citation must name — and the row already carries it, as
/// `config_digest`. Demanding a second name for the same bytes and refusing
/// the only row kind that has them was the defect.
///
/// **This does not weaken the learned law.** The fallback is gated on the KIND,
/// not on "the artifact happened to be absent": a `learned` or `reranker` row
/// with no `artifact_digest` returns `None` and the citation refuses it, exactly
/// as before, so a model that declared no bytes still cannot be cited. Pinned
/// by `a_learned_row_still_requires_its_own_artifact_digest`.
///
/// The gate is an explicit `if` on `row.kind` rather than a `debug_assert` on
/// the same condition, for two reasons. First, a debug assertion disappears in
/// release, and the law must hold in the shipped binary. Second — and this was
/// found by the pin that exists to catch it — an assertion *fired* the moment a
/// caller passed a `learned` row directly, which is a legitimate thing for a
/// unit test to do and is precisely the input the law must refuse. The right
/// response to "an unexpected row kind reached the fallback" is to REFUSE it
/// by name, not to panic.
pub(crate) fn cited_artifact_digest(row: &RegistryRow) -> Option<String> {
    if let Some(artifact) = row
        .artifact_digest
        .as_deref()
        .filter(|d| !d.trim().is_empty())
    {
        return Some(artifact.to_string());
    }
    // The rules arm: the canonical digest of the rules document IS the artifact.
    // Gated on the kind so a learned/reranker row can never borrow its config
    // digest as an artifact, whatever its shape.
    if row.kind != KIND_DETERMINISTIC_RULES {
        return None;
    }
    row.config_digest.clone().filter(|d| !d.trim().is_empty())
}

#[cfg(test)]
pub(crate) mod test_support {
    //! Test-only seeds and probes: the handler test modules stay SQL-free
    //! (the no-SQL law counts handler files live), so the fixtures'
    //! write/read steps live here beside the code they exercise.

    use super::*;

    /// Seed one deterministic-rules row in the given status for the
    /// supplied table (the consumption-law fixtures' green path).
    /// Returns (id, version, config_digest).
    pub(crate) fn seed_rules_model(
        conn: &Connection,
        rules_json: &str,
        status: &str,
    ) -> (String, String, String) {
        let model =
            crate::workflow::harness::models::rules::RulesModel::from_canonical_json(rules_json)
                .unwrap();
        let meta = model.metadata();
        let id = meta.id().to_string();
        let version = meta.version().to_string();
        let digest = model.digest().to_string();
        let vocab = serde_json::to_string(&vocabulary_labels(meta.output_vocabulary())).unwrap();
        conn.execute(
            "INSERT INTO decision_model_registry(id, version, kind, name, output_vocabulary,
                artifact_digest, config_digest, calibration_ref, status, evaluation_refs,
                proposed_by, approved_by, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?1, ?4, NULL, ?5, NULL, ?6, '[]', 'test', ?7, 1800000000, 1800000000)
             ON CONFLICT(id, version) DO UPDATE SET status = excluded.status",
            rusqlite::params![
                id,
                version,
                KIND_DETERMINISTIC_RULES,
                vocab,
                digest,
                status,
                if status == STATUS_PROMOTED {
                    Some("test".to_string())
                } else {
                    None
                },
            ],
        )
        .unwrap();
        (id, version, digest)
    }

    /// A PROMOTED row (the deterministic mode's green path).
    pub(crate) fn seed_promoted_rules_model(
        conn: &Connection,
        rules_json: &str,
    ) -> (String, String, String) {
        seed_rules_model(conn, rules_json, STATUS_PROMOTED)
    }

    pub(crate) fn remove_model(conn: &Connection, id: &str, version: &str) {
        conn.execute(
            "DELETE FROM decision_model_registry WHERE id = ?1 AND version = ?2",
            rusqlite::params![id, version],
        )
        .unwrap();
    }

    /// The row count (the bounded-listing + refusal pins).
    pub(crate) fn registry_row_count(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM decision_model_registry", [], |r| {
            r.get(0)
        })
        .unwrap()
    }

    /// One row's status (the lifecycle pins).
    pub(crate) fn registry_row_status(
        conn: &Connection,
        id: &str,
        version: &str,
    ) -> Option<String> {
        conn.query_row(
            "SELECT status FROM decision_model_registry WHERE id = ?1 AND version = ?2",
            rusqlite::params![id, version],
            |r| r.get(0),
        )
        .optional()
        .unwrap()
    }

    pub(crate) fn audit_rows_for_target(conn: &Connection, target: &str) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM audit_events WHERE target_hash = ?1",
            [crate::audit::hash(target)],
            |r| r.get(0),
        )
        .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migration;
    use crate::register_sqlite_vec::register_sqlite_vec;

    const TABLE_JSON: &str = r#"{
      "model_id": "rules-core-test",
      "model_version": "2.0.0",
      "rules": [
        { "question_id": "q", "min_evidence": 1, "min_tier": "untrusted",
          "output": { "Choice": { "options": ["a", "b"], "label": "a" } } }
      ]
    }"#;

    fn db() -> Connection {
        register_sqlite_vec();
        let mut conn = Connection::open_in_memory().unwrap();
        run_migration(&mut conn, 1).unwrap();
        conn
    }

    fn wtx(conn: &mut Connection) -> WorkflowTx<'_> {
        WorkflowTx::begin(conn).unwrap()
    }

    fn declared(id: &str) -> Registration {
        Registration::Declared {
            kind: KIND_LEARNED,
            id: id.to_string(),
            version: "1.0.0".to_string(),
            name: id.to_string(),
            output_vocabulary: vec!["choice".into()],
            artifact_digest: Some("a".repeat(64)),
            config_digest: None,
            calibration_ref: None,
        }
    }

    fn deterministic_rules() -> Registration {
        Registration::DeterministicRules {
            rules_json: TABLE_JSON.into(),
        }
    }

    /// The registration law is total at the core: the deterministic arm
    /// derives identity + digest from the table; the declared laws
    /// (learned ⇒ artifact digest, the closed vocabulary, the `@` ban)
    /// and the duplicate (id, version) refusal are named.
    #[test]
    fn register_law_is_total_at_the_core() {
        let mut c = db();
        let mut tx = wtx(&mut c);
        let row =
            register_model(&mut tx, &deterministic_rules(), "operator", 1_800_000_000).unwrap();
        tx.commit().unwrap();
        assert_eq!(row.id, "rules-core-test");
        assert_eq!(row.version, "2.0.0");
        assert_eq!(row.status, STATUS_CANDIDATE);
        assert_eq!(row.output_vocabulary, vec!["choice".to_string()]);
        assert!(row.config_digest.is_some());
        assert_eq!(row.artifact_digest, None);

        let mut tx = wtx(&mut c);
        // Duplicate (id, version) — loud, never silent.
        match register_model(&mut tx, &deterministic_rules(), "operator", 1_800_000_000) {
            Err(RegistryError::AlreadyRegistered { id, version }) => {
                assert_eq!(
                    (id.as_str(), version.as_str()),
                    ("rules-core-test", "2.0.0")
                );
            }
            other => panic!("expected the loud duplicate refusal, got {other:?}"),
        }
        // Learned without the artifact digest — the named refusal.
        let mut learned = declared("m1");
        if let Registration::Declared {
            artifact_digest, ..
        } = &mut learned
        {
            *artifact_digest = None;
        }
        assert!(matches!(
            register_model(&mut tx, &learned, "operator", 1),
            Err(RegistryError::ArtifactDigestRequired)
        ));
        // The `@` ban (the citation-split law).
        assert!(matches!(
            register_model(&mut tx, &declared("bad@id"), "operator", 1),
            Err(RegistryError::IdInvalid(_))
        ));
        // The closed vocabulary.
        let mut wide = declared("m2");
        if let Registration::Declared {
            output_vocabulary, ..
        } = &mut wide
        {
            *output_vocabulary = vec!["chat".into()];
        }
        assert!(matches!(
            register_model(&mut tx, &wide, "operator", 1),
            Err(RegistryError::VocabularyInvalid)
        ));
        // A table whose model_id carries `@` refuses named.
        let at_table = Registration::DeterministicRules {
            rules_json: r#"{"model_id":"x@y","model_version":"1","rules":[]}"#.into(),
        };
        assert!(matches!(
            register_model(&mut tx, &at_table, "operator", 1),
            Err(RegistryError::IdInvalid(_))
        ));
        tx.commit().unwrap();
    }

    /// The lifecycle transition matrix is enforced in the caller's tx:
    /// promote candidate → promoted; retire any live state → retired;
    /// promote promoted → refused; retire retired → refused; a stale
    /// digest refuses `registry_row_changed`; an absent row refuses
    /// named; an unknown payload field refuses at creation.
    #[test]
    fn lifecycle_transition_legality_is_enforced_in_tx() {
        let mut c = db();
        let mut tx = wtx(&mut c);
        let row =
            register_model(&mut tx, &deterministic_rules(), "operator", 1_800_000_000).unwrap();
        tx.commit().unwrap();

        let payload_for = |action: &str, c2: &Connection| -> LifecyclePayload {
            let live = row_by_ref(c2, &row.id, &row.version).unwrap().unwrap();
            let digest = row_digest(&live).unwrap();
            LifecyclePayload {
                action: action.to_string(),
                id: live.id.clone(),
                version: live.version.clone(),
                row_digest: digest,
                row: live,
            }
        };

        // promote candidate → promoted (the legal path).
        let payload = payload_for("promote", &c);
        {
            let tx = c.transaction().unwrap();
            let new_status = apply_lifecycle(&tx, &payload, "approver", 2).unwrap();
            assert_eq!(new_status, STATUS_PROMOTED);
            tx.commit().unwrap();
        }
        // promote promoted → illegal, with the displayed row re-read from
        // the live promoted state.
        let promoted_payload = payload_for("promote", &c);
        {
            let tx = c.transaction().unwrap();
            assert!(matches!(
                apply_lifecycle(&tx, &promoted_payload, "approver", 3),
                Err(RegistryError::TransitionIllegal { .. })
            ));
            tx.commit().unwrap();
        }
        // retire promoted → legal; retire retired → illegal.
        let payload = payload_for("retire", &c);
        {
            let tx = c.transaction().unwrap();
            assert_eq!(
                apply_lifecycle(&tx, &payload, "approver", 4).unwrap(),
                STATUS_RETIRED
            );
            tx.commit().unwrap();
        }
        let retired_payload = payload_for("retire", &c);
        {
            let tx = c.transaction().unwrap();
            assert!(matches!(
                apply_lifecycle(&tx, &retired_payload, "approver", 5),
                Err(RegistryError::TransitionIllegal { .. })
            ));
            tx.commit().unwrap();
        }
        // A STALE digest (the payload was built from other bytes).
        let mut stale = payload_for("retire", &c);
        stale.row_digest = "0".repeat(64);
        {
            let tx = c.transaction().unwrap();
            assert!(matches!(
                apply_lifecycle(&tx, &stale, "approver", 6),
                Err(RegistryError::RowChanged)
            ));
            tx.commit().unwrap();
        }
        // An absent row.
        let mut absent = payload_for("retire", &c);
        absent.id = "no-such-model".into();
        {
            let tx = c.transaction().unwrap();
            assert!(matches!(
                apply_lifecycle(&tx, &absent, "approver", 7),
                Err(RegistryError::RowAbsent)
            ));
            tx.commit().unwrap();
        }
        // Creation-time validation: a digest mismatch refuses
        // `registry_row_digest_mismatch` at the queue.
        let mut bad = payload_for("retire", &c);
        bad.row_digest = "1".repeat(64);
        assert!(matches!(
            validate_lifecycle_payload(&c, &serde_json::to_string(&bad).unwrap()),
            Err(RegistryError::RowDigestMismatch)
        ));
        // An unknown payload field refuses named — evaluation_refs has NO
        // write path (the signed-records law).
        let hostile = format!(
            r#"{{"action":"retire","id":"x","version":"1","row_digest":"{}","row":{{}},"evaluation_refs":["e1"]}}"#,
            "0".repeat(64)
        );
        assert!(matches!(
            validate_lifecycle_payload(&c, &hostile),
            Err(RegistryError::PayloadInvalid(_))
        ));
    }

    /// The consumption law's resolve matrix: deterministic accepts only
    /// `promoted`; exploratory accepts candidate|evaluated|promoted;
    /// unregistered and retired refuse named; the `rules:` key shape is
    /// the only registry-shaped key.
    #[test]
    fn resolve_for_execution_maps_the_status_law() {
        let c = db();
        let model =
            crate::workflow::harness::models::rules::RulesModel::from_canonical_json(TABLE_JSON)
                .unwrap();
        let key = model.config_key().to_string();
        let digest = model.digest().to_string();

        // Unregistered.
        assert_eq!(
            resolve_for_execution(&c, &key, &digest, "deterministic").unwrap(),
            Err(ResolveRefusal::NotRegistered)
        );
        // Not a registry-shaped key.
        assert_eq!(
            resolve_for_execution(&c, "embedding:foo", &digest, "deterministic").unwrap(),
            Err(ResolveRefusal::NotRegistered)
        );

        // Candidate: exploratory resolves, deterministic refuses
        // `NotPromoted`.
        test_support::seed_rules_model(&c, TABLE_JSON, STATUS_CANDIDATE);
        assert!(
            resolve_for_execution(&c, &key, &digest, "exploratory")
                .unwrap()
                .is_ok()
        );
        assert_eq!(
            resolve_for_execution(&c, &key, &digest, "deterministic").unwrap(),
            Err(ResolveRefusal::NotPromoted)
        );

        // Promoted (a NEWER version row, same digest): both modes resolve
        // and cite the promoted row.
        let (id, version, _) = test_support::seed_rules_model(&c, TABLE_JSON, STATUS_PROMOTED);
        let det = resolve_for_execution(&c, &key, &digest, "deterministic")
            .unwrap()
            .unwrap();
        assert_eq!(
            (det.id.as_str(), det.version.as_str(), det.status.as_str()),
            (id.as_str(), version.as_str(), STATUS_PROMOTED)
        );
        assert!(
            resolve_for_execution(&c, &key, &digest, "exploratory")
                .unwrap()
                .is_ok()
        );

        // Retire every row for the binding: deterministic refuses
        // `Retired` (retirement is sticky), exploratory too.
        c.execute(
            "UPDATE decision_model_registry SET status = 'retired' WHERE config_digest = ?1",
            rusqlite::params![digest],
        )
        .unwrap();
        assert_eq!(
            resolve_for_execution(&c, &key, &digest, "deterministic").unwrap(),
            Err(ResolveRefusal::Retired)
        );
        assert_eq!(
            resolve_for_execution(&c, &key, &digest, "exploratory").unwrap(),
            Err(ResolveRefusal::Retired)
        );
    }
}
