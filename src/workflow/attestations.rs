//! The delivery loop's ATTESTATION chain (the attestation round).
//!
//! One writer — [`append_link`], called by the phase pass inside its own
//! `WorkflowTx` — and one reader that VERIFIES: [`verify_chain`], a pure
//! function over the chain's own rows. This module is the line's first
//! signature surface, and it is deliberately built out of the stack the server
//! already ships rather than a new one: the canonicalizer, the content hash,
//! and the signature all come from [`crate::ump_integrity`], and the predicate
//! and the record digest come from `brain_delivery_core`. **Zero new
//! dependencies, and no second canonicalization that could drift beside the
//! first.**
//!
//! ## What an attestation is, and is not
//!
//! It is EVIDENCE OF WHO ACTED: a signed statement that a specific run, at a
//! specific phase, over a specific artifact, was recorded by the holder of one
//! key. It is never a disposition. There is no status column, no decision
//! column, and no code path that promotes or denies anything on the strength of
//! a row here; the gate reads a chain, and a chain is evidence, not a verdict.
//!
//! ## The signing profile, in one place
//!
//! Three mutually incompatible signed-message formulas coexist in
//! `ump_integrity` — [`sign_hash`](crate::ump_integrity::sign_hash) over the raw
//! 32-byte BLAKE3, [`sign_manifest_bytes`] over a lowercase-hex SHA-256 string,
//! and [`sign_hash_string`] over BLAKE3 of the `blake3:…` STRING. This module
//! uses the third, and the known-answer pin
//! `attestation_signs_the_shipped_canonical_form` recomputes the exact bytes so
//! a move to either sibling fails rather than producing a well-formed signature
//! that verifies against nothing. The canonicalizer is
//! [`canonical_ump`](crate::ump_integrity::canonical_ump) for the same reason:
//! its own docstring states it is the only flavor that reproduces the reference
//! `contentHash`.
//!
//! ## NON-CLAIMS (carried in every artifact, per the delivery line's law)
//!
//! - **This envelope is NOT DSSE.** It is the project envelope convention: a
//!   signature over a canonicalized JSON object. It does not implement the DSSE
//!   envelope structure and does not apply the DSSE Pre-Authentication Encoding
//!   (DSSE version 1.0.2, 2024-05-10), and it **will not verify against any DSSE
//!   verifier**.
//! - **The field names `subject_digest` / `predicate_type` / `predicate` were
//!   chosen to MIRROR the naming of the in-toto Attestation Framework's
//!   Statement v1 model** (`subject[].digest`, `predicateType`, `predicate`).
//!   That is naming adjacency in our own design lineage. The envelope is **not
//!   an in-toto Statement**, carries no `_type` field, and verifies against
//!   neither an in-toto nor a DSSE verifier.
//! - **No SLSA claim.** SLSA (v1.2, 2025-11-24) is the ecosystem's frame for
//!   BUILD provenance; this is not a build artifact, this produces **no SLSA
//!   provenance attestation, and claims no SLSA build level**.
//! - **Authorship is not authority.** A valid signature proves the holder of
//!   the key named in `signer_did` signed. There is no PKI, no revocation
//!   oracle, and no epoch: a rotated key leaves historical rows verifiable and
//!   valid, and a signature says nothing about whether the act was permitted.
//! - **The IETF WIMSE agent-audit drafts are contemporaneous prior art, not a
//!   standard and not a target.** Four drafts, zero RFCs, and two of them are
//!   individual submissions rather than Working Group documents. No WIMSE
//!   relationship, adoption, or alignment is claimed.
//!
//! ## Key custody
//!
//! [`operator_key`] is the fail-closed door: `resolve_operator_key()`'s `Err` is
//! PROPAGATED and its `Ok(None)` ALSO refuses. The consequence is operational and
//! load-bearing — on a host with no operator key, every delivery phase pass
//! refuses rather than writing an unsigned link. The banned alternative is the
//! `operator_signing_key()`-shaped accessor, which collapses a refused key into
//! `None` after a log line and makes "the key is unreadable" indistinguishable
//! from "there is no key"; a source-scan pin keeps it out of this path.

#![deny(unsafe_code)]

use std::collections::BTreeSet;

use base64::Engine;
use brain_delivery_core::{
    Attestation, AttestationPredicate, AutonomyTier, PREDICATE_TYPE, canonical_predicate_bytes,
    parse_canonical_predicate, predicate_digest,
};
use ed25519_dalek::SigningKey;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// The chain walk is bounded. An append-only log read without a cap is an
/// availability hole, and a cap that TRUNCATES would report a partial chain as
/// a whole one — so the cap refuses.
pub(crate) const MAX_CHAIN_LINKS: usize = 500;

/// The subject name is KERNEL-DERIVED (`delivery/{stage}/{phase}` plus a
/// bounded artifact-id suffix), so this is a second fence, not the first. It is
/// here because the value reaches `record_digest`'s pipe framing, where an
/// embedded `|` would make two different links frame to identical bytes.
const MAX_SUBJECT_NAME_CHARS: usize = 160;
const MAX_DIGEST_CHARS: usize = 128;

/// The link id prefix. Content-addressed, so a re-derivation reproduces it.
const ATT_ID_PREFIX: &str = "att_";

/// The facts one link is built from. All owned: the values cross a `tx` borrow
/// and outlive the `advance()` frame that assembled them.
pub(crate) struct ChainLink {
    /// The run's own domain, for the audit row's tenant.
    pub domain: String,
    pub run_id: i64,
    pub step_id: i64,
    /// KERNEL-DERIVED. Agent free text never becomes a signed name.
    pub subject_name: String,
    /// `sha256:<64hex>` — the artifact bytes this pass was about.
    pub subject_digest: String,
    pub policy_digest: Option<String>,
    /// The binding's config digest; a SIBLING of `policy_digest` in the signed
    /// object, not a predicate field.
    pub config_digest: Option<String>,
    /// The registry key of the model that acted, if a binding was presented.
    pub model_ref: Option<String>,
    /// The resolved row's ARTIFACT digest. A name without the bytes it names is
    /// not evidence, so the two travel together and neither travels alone.
    pub model_digest: Option<String>,
    pub tier: AutonomyTier,
}

/// One sealed link: the envelope as it will be stored, plus the values the
/// caller needs to write the row and the audit that covers it.
#[derive(Debug)]
pub(crate) struct Sealed {
    pub envelope: Value,
    pub envelope_json: String,
    pub content_hash: String,
    pub signature: Vec<u8>,
    pub chain_hash: String,
    pub row_id: String,
    pub predicate_digest: String,
}

impl Sealed {
    /// The stored row. `signer_did` is the key's identity and the ONLY record
    /// of which key wrote it (A9: the signer IS the key history).
    pub(crate) fn row(&self, signer_did: &str, link: &ChainLink, now: i64) -> AttestationRow {
        AttestationRow {
            id: self.row_id.clone(),
            run_id: link.run_id,
            step_id: link.step_id,
            subject_name: link.subject_name.clone(),
            subject_digest: link.subject_digest.clone(),
            predicate_type: PREDICATE_TYPE.to_string(),
            predicate_digest: self.predicate_digest.clone(),
            envelope_json: self.envelope_json.clone(),
            signer_did: signer_did.to_string(),
            parent_id: self
                .envelope
                .get("parent_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            chain_hash: self.chain_hash.clone(),
            created_at: now,
        }
    }
}

/// A stored link, as read back for verification. The verifier is handed these
/// and nothing else — no connection, no clock, no key file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AttestationRow {
    pub id: String,
    pub run_id: i64,
    pub step_id: i64,
    pub subject_name: String,
    pub subject_digest: String,
    pub predicate_type: String,
    pub predicate_digest: String,
    pub envelope_json: String,
    pub signer_did: String,
    pub parent_id: String,
    pub chain_hash: String,
    pub created_at: i64,
}

/// Every refusal the attestation layer can return. Typed so a caller maps
/// rather than guesses, and closed so a client cannot learn a new failure mode
/// from a new code.
#[derive(Debug)]
pub(crate) enum AttestationError {
    /// No operator key at all. The pass refuses; it never writes an unsigned
    /// link.
    KeyAbsent,
    /// A key IS present and is unusable (wrong size, leaked perms, unreadable).
    /// Distinct from absent, because it is a different operator problem.
    KeyRefused(String),
    /// A value that would make `record_digest`'s pipe framing ambiguous.
    NameRefused {
        field: &'static str,
    },
    ModelNotRegistered,
    ModelNotPromoted,
    ModelRetired,
    /// A registry row that resolves but carries no artifact digest, so there is
    /// nothing saying which bytes acted.
    ModelDigestMissing,
    /// The chain is already at its bound.
    ChainFull,
    EnvelopeRefused(String),
    Storage(String),
}

impl std::fmt::Display for AttestationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::KeyAbsent => write!(f, "operator_key_absent"),
            Self::KeyRefused(detail) => write!(f, "operator_key_refused: {detail}"),
            Self::NameRefused { field } => write!(f, "attestation_field_ambiguous:{field}"),
            Self::ModelNotRegistered => write!(f, "model_not_registered"),
            Self::ModelNotPromoted => write!(f, "model_not_promoted"),
            Self::ModelRetired => write!(f, "model_retired"),
            Self::ModelDigestMissing => write!(f, "model_digest_missing"),
            Self::ChainFull => write!(f, "attestation_chain_full"),
            Self::EnvelopeRefused(detail) => write!(f, "attestation_envelope_refused: {detail}"),
            Self::Storage(detail) => write!(f, "attestation_storage: {detail}"),
        }
    }
}

impl std::error::Error for AttestationError {}

fn attestation_storage(detail: impl std::fmt::Display) -> AttestationError {
    AttestationError::Storage(detail.to_string())
}

/// Why a link does not verify. Closed vocabulary, and a refusal is NEVER
/// downgraded into a degraded mark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChainRefusal {
    RootHasParent,
    BrokenLink,
    EnvelopeMalformed,
    IntegrityIncomplete,
    SignerMismatch,
    ContentHashMismatch,
    UnknownSigner,
    SignatureInvalid,
    PredicateTypeMismatch,
    PredicateMalformed,
    PredicateIncomplete,
    PredicateDigestMismatch,
    ChainHashMismatch,
    ColumnMismatch,
    DatedInFuture,
    ChainTooLong,
}

impl ChainRefusal {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::RootHasParent => "attestation_root_has_parent",
            Self::BrokenLink => "attestation_broken_link",
            Self::EnvelopeMalformed => "attestation_envelope_malformed",
            Self::IntegrityIncomplete => "attestation_integrity_incomplete",
            Self::SignerMismatch => "attestation_signer_mismatch",
            Self::ContentHashMismatch => "attestation_content_hash_mismatch",
            Self::UnknownSigner => "attestation_unknown_signer",
            Self::SignatureInvalid => "attestation_signature_invalid",
            Self::PredicateTypeMismatch => "attestation_predicate_type_mismatch",
            Self::PredicateMalformed => "attestation_predicate_malformed",
            Self::PredicateIncomplete => "attestation_predicate_incomplete",
            Self::PredicateDigestMismatch => "attestation_predicate_digest_mismatch",
            Self::ChainHashMismatch => "attestation_chain_hash_mismatch",
            Self::ColumnMismatch => "attestation_column_mismatch",
            Self::DatedInFuture => "attestation_dated_in_future",
            Self::ChainTooLong => "attestation_chain_too_long",
        }
    }
}

impl std::fmt::Display for ChainRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The per-link verdict. `verified: false` always carries a named refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LinkVerdict {
    pub verified: bool,
    pub refusal: Option<&'static str>,
}

/// The chain verdict. `verified` is true only when EVERY link verified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChainVerdict {
    pub run_id: i64,
    pub verified: bool,
    pub link_count: usize,
    pub head: Option<String>,
    pub signer_dids: Vec<String>,
    pub links: Vec<LinkVerdict>,
}

// ── the signing material ────────────────────────────────────────────────────

/// The operator key for one link, FAIL-CLOSED.
///
/// `resolve_operator_key()` is `Ok(None)` when there is no key (the legitimate
/// L2 posture for every OTHER surface, and a refusal here) and `Err` when a key
/// is present and unusable. Both refuse the write, and with DIFFERENT codes:
/// "install a key" and "your key is broken" are different operator problems. The
/// `operator_signing_key()` shape — which turns that `Err` into a `None` after a
/// log line — is the door this module must not use, and a source-scan pin keeps
/// it out.
///
/// There is deliberately no test-only seam at this boundary. The suite installs
/// a REAL seed file at `BRAIN_UMP_KEY_DIR` under the shared env lock, so this is
/// the shipped resolver in the lib's own tests too, and
/// `attestation_key_absence_refuses_the_phase_pass` drives the real `Ok(None)`
/// and real `Err` arms rather than a stub's.
fn operator_key() -> Result<(String, SigningKey), AttestationError> {
    match crate::handlers::ump::resolve_operator_key() {
        Err(detail) => Err(AttestationError::KeyRefused(detail)),
        Ok(None) => Err(AttestationError::KeyAbsent),
        Ok(Some(material)) => Ok(material),
    }
}

// ── the envelope ────────────────────────────────────────────────────────────

/// The framing fence (A5). `record_digest` joins its seven fields with `|`, so
/// a field containing `|` — or any control byte — would let two different links
/// frame to identical bytes. The value is kernel-derived, so this is the second
/// fence, not the first; `record_digest` itself is untouched.
fn framed_field(field: &'static str, value: &str) -> Result<(), AttestationError> {
    if value.contains('|') || value.chars().any(char::is_control) {
        return Err(AttestationError::NameRefused { field });
    }
    Ok(())
}

fn bounded_field(field: &'static str, value: &str, max: usize) -> Result<(), AttestationError> {
    if value.chars().count() > max {
        return Err(AttestationError::NameRefused { field });
    }
    Ok(())
}

/// The 13-field predicate, built from resolved facts and normalized by the
/// crate's own canonicalizer.
///
/// **The 4-of-13 ceiling, stated once:** `gate_verdicts`, `approval_ref`,
/// `authority_receipts`, and `budget_spend` are EMPTY this round, because the
/// rounds that populate them have not shipped (the gate lifecycle and the authority
/// bindings are later rounds). `model_ref`/`model_digest` are populated only
/// when the pass presented a binding. A reader must not read this predicate as a
/// rich claim: it says what acted and under whose key, and nothing about
/// approval, budget, or authority.
fn predicate_for(link: &ChainLink) -> Result<AttestationPredicate, AttestationError> {
    framed_field("subject_name", &link.subject_name)?;
    bounded_field("subject_name", &link.subject_name, MAX_SUBJECT_NAME_CHARS)?;
    framed_field("subject_digest", &link.subject_digest)?;
    bounded_field("subject_digest", &link.subject_digest, MAX_DIGEST_CHARS)?;
    if let Some(policy) = link.policy_digest.as_deref() {
        framed_field("policy_digest", policy)?;
        bounded_field("policy_digest", policy, MAX_DIGEST_CHARS)?;
    }
    if let Some(digest) = link.model_digest.as_deref() {
        framed_field("model_digest", digest)?;
        bounded_field("model_digest", digest, MAX_DIGEST_CHARS)?;
    }
    let mut predicate = AttestationPredicate::new(
        // `run_ref` is the run's own id rendered as a ref; the trace row's id is
        // the per-pass address and is not yet minted at seal time.
        format!("run:{}", link.run_id),
        link.subject_name.clone(),
        link.subject_digest.clone(),
        link.tier,
    );
    predicate.model_ref = link.model_ref.clone().unwrap_or_default();
    predicate.model_digest = link.model_digest.clone().unwrap_or_default();
    predicate.policy_digest = link.policy_digest.clone().unwrap_or_default();
    Ok(predicate.normalized())
}

/// Seal one link: the record digest, the envelope, and the signature.
///
/// The signed object is the envelope MINUS its `integrity` block — the shipped
/// `emit_record` shape from `handlers/ump.rs`, verbatim. `predicate_type` and
/// `parent_id` are TOP-LEVEL SIGNED KEYS, so a re-typed or re-parented link
/// stops verifying; the 13 predicate fields are NESTED so a verifier holding
/// only the chain can re-derive the digest offline.
pub(crate) fn seal_envelope(
    link: &ChainLink,
    signer_did: &str,
    key: &SigningKey,
    now: i64,
) -> Result<Sealed, AttestationError> {
    if signer_did.is_empty() {
        return Err(AttestationError::EnvelopeRefused(
            "signer_did is empty".into(),
        ));
    }
    framed_field("signer_did", signer_did)?;
    let predicate = predicate_for(link)?;
    let digest = predicate_digest(&predicate);
    let mut sealed = seal(link, signer_did, key, &digest, &predicate, None, now)?;
    sealed.row_id = attestation_row_id(&sealed.chain_hash, link.run_id, link.step_id);
    Ok(sealed)
}

/// The shared body of [`seal_envelope`] and [`reseal_with_parent`]. The parent
/// is a pair rather than two arguments so the signature stays inside the
/// repo's arity ceiling — and because "a link has a parent" is one fact, not
/// two.
#[allow(clippy::too_many_arguments)]
fn seal(
    link: &ChainLink,
    signer_did: &str,
    key: &SigningKey,
    digest: &str,
    predicate: &AttestationPredicate,
    parent: Option<(&str, &str)>,
    now: i64,
) -> Result<Sealed, AttestationError> {
    let (parent_id, parent_digest) = parent.unwrap_or(("", ""));
    let record = Attestation {
        subject_name: link.subject_name.clone(),
        subject_digest: link.subject_digest.clone(),
        predicate_digest: digest.to_string(),
        predicate_type: PREDICATE_TYPE.to_string(),
        // The crate's field is a plain String and its `is_complete` law wants a
        // non-empty value; the envelope's is an Option. The absent case is the
        // empty string on both sides, which round-trips exactly.
        policy_digest: link.policy_digest.clone().unwrap_or_default(),
        signer_did: signer_did.to_string(),
        parent_digest: parent_digest.to_string(),
    };
    let chain_hash = record.record_digest();

    // The nested predicate is the crate's CANONICAL bytes parsed back into a
    // JSON value — never serde's field order. That is what makes the stored
    // object byte-stable and the offline re-derivation exact.
    let predicate_value: Value = serde_json::from_slice(&canonical_predicate_bytes(predicate))
        .map_err(|e| AttestationError::EnvelopeRefused(format!("predicate bytes: {e}")))?;

    let envelope = json!({
        "v": 1,
        "run_id": link.run_id,
        "step_id": link.step_id,
        "created_at": now,
        "subject_name": link.subject_name,
        "subject_digest": link.subject_digest,
        "policy_digest": link.policy_digest,
        "predicate_type": PREDICATE_TYPE,
        "predicate": predicate_value,
        "config_digest": link.config_digest,
        "parent_id": parent_id,
        "signer_did": signer_did,
    });

    let mut signed = envelope.clone();
    let object = signed
        .as_object_mut()
        .ok_or_else(|| AttestationError::EnvelopeRefused("envelope is not an object".into()))?;
    object.remove("integrity");
    let canonical =
        crate::ump_integrity::canonical_ump(&signed).map_err(AttestationError::EnvelopeRefused)?;
    let content_hash = crate::ump_integrity::content_hash_string(&canonical);
    let signature = crate::ump_integrity::sign_hash_string(&content_hash, key);

    let mut complete = envelope;
    complete["integrity"] = json!({
        "content_hash": content_hash,
        "signature": format!(
            "ed25519:{}",
            base64::engine::general_purpose::STANDARD.encode(&signature)
        ),
        "signer": signer_did,
    });
    // Stored in CANONICAL form: the bytes a verifier re-canonicalizes are then
    // the bytes that were signed, and the stored text is stable across runs.
    let envelope_json = String::from_utf8(
        crate::ump_integrity::canonical_ump(&complete)
            .map_err(AttestationError::EnvelopeRefused)?,
    )
    .map_err(|e| AttestationError::EnvelopeRefused(e.to_string()))?;

    Ok(Sealed {
        envelope: complete,
        envelope_json,
        content_hash,
        signature,
        chain_hash,
        row_id: String::new(),
        predicate_digest: digest.to_string(),
    })
}

/// Re-seal a link with its parent, after the parent has been read. The chain
/// digest and the signed `parent_id` both change; nothing else does.
pub(crate) fn reseal_with_parent(
    link: &ChainLink,
    signer_did: &str,
    key: &SigningKey,
    parent_id: &str,
    parent_digest: &str,
    now: i64,
) -> Result<Sealed, AttestationError> {
    let predicate = predicate_for(link)?;
    let digest = predicate_digest(&predicate);
    let mut sealed = seal(
        link,
        signer_did,
        key,
        &digest,
        &predicate,
        Some((parent_id, parent_digest)),
        now,
    )?;
    sealed.row_id = attestation_row_id(&sealed.chain_hash, link.run_id, link.step_id);
    Ok(sealed)
}

/// `att_` + 32 hex of `Sha256(chain_hash ‖ run_id ‖ step_id)`.
fn attestation_row_id(chain_hash: &str, run_id: i64, step_id: i64) -> String {
    let mut hasher = Sha256::new();
    hasher.update(chain_hash.as_bytes());
    hasher.update(run_id.to_be_bytes());
    hasher.update(step_id.to_be_bytes());
    format!(
        "{ATT_ID_PREFIX}{}",
        &crate::audit::hex_encode(&hasher.finalize())[..32]
    )
}

// ── the chain writer ────────────────────────────────────────────────────────

/// Append ONE link, inside the caller's transaction, and emit the audit row
/// that covers the write.
///
/// This is the module's only INSERT, and `advance()` is its only caller. The
/// audit is the same fail-closed idiom the delivery phase pass uses (the
/// best-effort writer with a `None` converted into an error), so a link that
/// commits without its evidence rolls back with it.
pub(crate) fn append_link(
    tx: &Transaction<'_>,
    link: &ChainLink,
    now: i64,
) -> Result<String, AttestationError> {
    let depth: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM delivery_attestations WHERE run_id = ?1",
            params![link.run_id],
            |r| r.get(0),
        )
        .map_err(attestation_storage)?;
    if depth >= MAX_CHAIN_LINKS as i64 {
        return Err(AttestationError::ChainFull);
    }

    // The parent is the chain's current head: its ROW ID goes into the signed
    // `parent_id`, and its CHAIN HASH into the record digest. Both halves are
    // needed — a row id is not a digest, and a digest alone does not order rows.
    let parent: Option<(String, String)> = tx
        .query_row(
            "SELECT id, chain_hash FROM delivery_attestations WHERE run_id = ?1 \
             ORDER BY created_at DESC, rowid DESC LIMIT 1",
            params![link.run_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(attestation_storage)?;

    let (signer_did, key) = operator_key()?;
    let sealed = if let Some((id, digest)) = &parent {
        reseal_with_parent(link, &signer_did, &key, id, digest, now)?
    } else {
        seal_envelope(link, &signer_did, &key, now)?
    };
    let row = sealed.row(&signer_did, link, now);

    tx.execute(
        "INSERT INTO delivery_attestations(id, run_id, step_id, subject_name, subject_digest, \
         predicate_type, predicate_digest, envelope_json, signer_did, parent_id, chain_hash, \
         created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        params![
            row.id,
            row.run_id,
            row.step_id,
            row.subject_name,
            row.subject_digest,
            row.predicate_type,
            row.predicate_digest,
            row.envelope_json,
            row.signer_did,
            row.parent_id,
            row.chain_hash,
            row.created_at,
        ],
    )
    .map_err(attestation_storage)?;

    // The link's own evidence, inside the same transaction. `AuditKind::Workflow`
    // is REUSED (no 20th variant); the target is the link's own id, so the audit
    // layer's hash covers exactly this row.
    crate::workflow::delivery::delivery_audit(
        tx,
        &link.domain,
        &format!("delivery/attestation/{}/{}", link.run_id, row.id),
        crate::audit::AuditStatus::Ok,
        &format!(
            "delivery attestation linked run={} step={} parent={} chain_hash={} subject={}",
            link.run_id,
            link.step_id,
            if row.parent_id.is_empty() {
                "root"
            } else {
                "child"
            },
            row.chain_hash,
            row.subject_name
        ),
    )
    .map_err(|e| attestation_storage(format!("attestation audit insertion failed: {e}")))?;

    Ok(row.id)
}

/// The chain's current head for a run, or `None` when it has none. Read-only:
/// the admission, the answer, and the gate each name the head into their trace
/// row and never append.
pub(crate) fn chain_head(
    conn: &Connection,
    run_id: i64,
) -> Result<Option<String>, AttestationError> {
    conn.query_row(
        "SELECT id FROM delivery_attestations WHERE run_id = ?1 \
         ORDER BY created_at DESC, rowid DESC LIMIT 1",
        params![run_id],
        |r| r.get(0),
    )
    .optional()
    .map_err(attestation_storage)
}

// ── the read ────────────────────────────────────────────────────────────────

/// The chain, in order, bounded. The read is capped at one past the walk bound
/// so the verifier can SEE that the chain is too long and refuse, rather than
/// being handed a truncated prefix that looks whole.
pub(crate) fn read_chain(
    conn: &Connection,
    run_id: i64,
) -> Result<Vec<AttestationRow>, AttestationError> {
    let mut stmt = conn
        .prepare(
            "SELECT id, run_id, step_id, subject_name, subject_digest, predicate_type, \
             predicate_digest, envelope_json, signer_did, parent_id, chain_hash, created_at \
             FROM delivery_attestations WHERE run_id = ?1 ORDER BY created_at, rowid LIMIT ?2",
        )
        .map_err(attestation_storage)?;
    let rows = stmt
        .query_map(params![run_id, MAX_CHAIN_LINKS as i64 + 1], |r| {
            Ok(AttestationRow {
                id: r.get(0)?,
                run_id: r.get(1)?,
                step_id: r.get(2)?,
                subject_name: r.get(3)?,
                subject_digest: r.get(4)?,
                predicate_type: r.get(5)?,
                predicate_digest: r.get(6)?,
                envelope_json: r.get(7)?,
                signer_did: r.get(8)?,
                parent_id: r.get(9)?,
                chain_hash: r.get(10)?,
                created_at: r.get(11)?,
            })
        })
        .map_err(attestation_storage)?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(attestation_storage)
}

// ── the offline verifier ────────────────────────────────────────────────────

/// THE offline verifier: a pure function over the chain's own rows.
///
/// It is handed the rows and a `now`, and it reads NOTHING else — no socket, no
/// config, no key file, no pool, no clock. The verifying key is recovered from
/// each link's OWN `signer_did`, which is what makes a rotated key leave
/// history intact and what makes this function usable by anyone holding the
/// chain.
///
/// Each `envelope_json` is parsed EXACTLY ONCE and the same parsed object is
/// carried through every check: re-parsing after verification would re-derive
/// the payload from bytes that were never the ones checked.
pub(crate) fn verify_chain(
    rows: &[AttestationRow],
    now: i64,
) -> Result<ChainVerdict, ChainRefusal> {
    if rows.len() > MAX_CHAIN_LINKS {
        return Err(ChainRefusal::ChainTooLong);
    }
    let run_id = rows.first().map_or(0, |r| r.run_id);
    let mut links = Vec::with_capacity(rows.len());
    let mut signers: BTreeSet<String> = BTreeSet::new();
    for (index, row) in rows.iter().enumerate() {
        // The chain walk, and the parent digest the record-digest check needs.
        // The root names no parent — one that does is a FRAGMENT of a longer
        // chain, and a fragment is not a chain.
        let (parent_refusal, parent) = if index == 0 {
            (
                (!row.parent_id.is_empty()).then_some(ChainRefusal::RootHasParent),
                None,
            )
        } else {
            let previous = &rows[index - 1];
            (
                (row.parent_id != previous.id).then_some(ChainRefusal::BrokenLink),
                Some(previous),
            )
        };
        let refusal = parent_refusal.or_else(|| verify_link(row, parent, now));
        signers.insert(row.signer_did.clone());
        links.push(LinkVerdict {
            verified: refusal.is_none(),
            refusal: refusal.map(ChainRefusal::as_str),
        });
    }
    Ok(ChainVerdict {
        run_id,
        verified: links.iter().all(|l| l.verified),
        link_count: links.len(),
        head: rows.last().map(|r| r.id.clone()),
        signer_dids: signers.into_iter().collect(),
        links,
    })
}

/// One link's checks, in the order that makes each refusal the most specific
/// true statement about the link. `parent` is the previous link in the chain,
/// or `None` at the root; it supplies the parent CHAIN HASH that the record
/// digest binds.
///
/// The body is written as a `Result` so a MISSING field is a refusal and not a
/// pass. An `Option`-returning chain of `?`s reads the same but means the
/// opposite: every missing value would be "no defect found", and a link with no
/// `integrity` block at all would verify.
fn verify_link(
    row: &AttestationRow,
    parent: Option<&AttestationRow>,
    now: i64,
) -> Option<ChainRefusal> {
    verify_link_inner(row, parent, now).err()
}

fn verify_link_inner(
    row: &AttestationRow,
    parent: Option<&AttestationRow>,
    now: i64,
) -> Result<(), ChainRefusal> {
    let envelope: Value =
        serde_json::from_str(&row.envelope_json).map_err(|_| ChainRefusal::EnvelopeMalformed)?;
    // The signed object is the envelope minus the block the signature lives in.
    let mut signed = envelope.clone();
    let integrity = signed
        .as_object_mut()
        .ok_or(ChainRefusal::EnvelopeMalformed)?
        .remove("integrity")
        .ok_or(ChainRefusal::IntegrityIncomplete)?;
    let integrity = integrity
        .as_object()
        .ok_or(ChainRefusal::IntegrityIncomplete)?;

    let content_hash = integrity
        .get("content_hash")
        .and_then(Value::as_str)
        .ok_or(ChainRefusal::IntegrityIncomplete)?;
    let signature = integrity
        .get("signature")
        .and_then(Value::as_str)
        .ok_or(ChainRefusal::IntegrityIncomplete)?;
    let integrity_signer = integrity
        .get("signer")
        .and_then(Value::as_str)
        .ok_or(ChainRefusal::IntegrityIncomplete)?;
    let envelope_signer = envelope
        .get("signer_did")
        .and_then(Value::as_str)
        .ok_or(ChainRefusal::EnvelopeMalformed)?;
    if integrity_signer != envelope_signer || envelope_signer != row.signer_did {
        return Err(ChainRefusal::SignerMismatch);
    }

    // The columns and the signed bytes must agree. A row whose columns were
    // edited beside a valid signature would otherwise read as verified.
    let text = |key: &str| envelope.get(key).and_then(Value::as_str);
    let number = |key: &str| envelope.get(key).and_then(Value::as_i64);
    if text("subject_name") != Some(row.subject_name.as_str())
        || text("subject_digest") != Some(row.subject_digest.as_str())
        || text("parent_id") != Some(row.parent_id.as_str())
        || text("predicate_type") != Some(row.predicate_type.as_str())
        || number("run_id") != Some(row.run_id)
        || number("step_id") != Some(row.step_id)
        || number("created_at") != Some(row.created_at)
    {
        return Err(ChainRefusal::ColumnMismatch);
    }
    if row.created_at > now {
        return Err(ChainRefusal::DatedInFuture);
    }

    let canonical = crate::ump_integrity::canonical_ump(&signed)
        .map_err(|_| ChainRefusal::EnvelopeMalformed)?;
    if crate::ump_integrity::content_hash_string(&canonical) != content_hash {
        return Err(ChainRefusal::ContentHashMismatch);
    }

    let signature = signature
        .strip_prefix("ed25519:")
        .and_then(|raw| base64::engine::general_purpose::STANDARD.decode(raw).ok())
        .ok_or(ChainRefusal::IntegrityIncomplete)?;
    let verifying = crate::ump_integrity::verifying_key_from_did(envelope_signer)
        .ok_or(ChainRefusal::UnknownSigner)?;
    if !crate::ump_integrity::verify_hash_string(content_hash, &verifying.to_bytes(), &signature) {
        return Err(ChainRefusal::SignatureInvalid);
    }

    // The predicate is re-derived from the SIGNED bytes: its canonical form is
    // re-encoded and compared, so a re-serialized object is refused, and the
    // digest is recomputed rather than read.
    let predicate = envelope
        .get("predicate")
        .ok_or(ChainRefusal::EnvelopeMalformed)?;
    let predicate_bytes =
        serde_json::to_vec(predicate).map_err(|_| ChainRefusal::PredicateMalformed)?;
    let parsed = parse_canonical_predicate(&predicate_bytes)
        .map_err(|_| ChainRefusal::PredicateMalformed)?;
    if predicate_digest(&parsed) != row.predicate_digest {
        return Err(ChainRefusal::PredicateDigestMismatch);
    }
    // The policy digest is not a COLUMN of this table (the design owner's list
    // has twelve and policy is not among them), so it is read from the signed
    // envelope and cross-checked against the predicate's copy of it.
    let policy_digest = match envelope.get("policy_digest") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(value)) => value.clone(),
        Some(_) => return Err(ChainRefusal::EnvelopeMalformed),
    };
    if parsed.subject_name != row.subject_name
        || parsed.subject_digest != row.subject_digest
        || parsed.policy_digest != policy_digest
    {
        return Err(ChainRefusal::PredicateIncomplete);
    }

    // The chain hash IS the crate's record digest, recomputed from the SAME
    // values — so a link whose stored `chain_hash` was edited beside a valid
    // signature refuses. The parent's CHAIN HASH (not its row id) is what the
    // digest binds; the walk already checked the row id, and a link whose parent
    // is absent from the chain has no digest to bind and refuses here.
    let parent_digest = match parent {
        Some(previous) if previous.id == row.parent_id => previous.chain_hash.clone(),
        _ => String::new(),
    };
    let record = Attestation {
        subject_name: row.subject_name.clone(),
        subject_digest: row.subject_digest.clone(),
        predicate_digest: row.predicate_digest.clone(),
        predicate_type: row.predicate_type.clone(),
        policy_digest,
        signer_did: row.signer_did.clone(),
        parent_digest,
    };
    if record.record_digest() != row.chain_hash {
        return Err(ChainRefusal::ChainHashMismatch);
    }
    if row.predicate_type != PREDICATE_TYPE {
        return Err(ChainRefusal::PredicateTypeMismatch);
    }
    Ok(())
}

// ── the read surface ────────────────────────────────────────────────────────

/// One link as the read surface serves it. The raw envelope is NOT emitted: it
/// is canonical bytes carrying a base64 signature, and this surface answers
/// "does the chain verify, and who signed it", not "here are the bytes".
#[derive(Debug, Clone, Serialize)]
pub(crate) struct LinkRead {
    pub id: String,
    pub run_id: i64,
    pub step_id: i64,
    pub subject_name: String,
    pub subject_digest: String,
    pub predicate_type: String,
    pub predicate_digest: String,
    pub signer_did: String,
    pub parent_id: String,
    pub chain_hash: String,
    pub created_at: i64,
    pub verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<&'static str>,
}

/// The unconditional verdict. There is no parameter that can switch it off.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct VerdictRead {
    pub verified: bool,
    pub head: Option<String>,
    pub link_count: usize,
    pub signer_dids: Vec<String>,
}

/// The whole read surface's payload.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ChainRead {
    pub run_id: i64,
    pub chain: Vec<LinkRead>,
    pub verdict: VerdictRead,
}

/// The read surface's own assembly: read the chain, verify it, and pair each
/// row with its verdict. A non-verifying chain is REPORTED, never hidden and
/// never degraded into a 200-with-no-mark: the refusal travels per link, named.
pub(crate) fn read_surface(
    conn: &Connection,
    run_id: i64,
    now: i64,
) -> Result<ChainRead, ChainRefusal> {
    let rows = read_chain(conn, run_id).map_err(|_| ChainRefusal::EnvelopeMalformed)?;
    let verdict = verify_chain(&rows, now)?;
    let chain = rows
        .iter()
        .zip(verdict.links.iter())
        .map(|(row, link)| LinkRead {
            id: row.id.clone(),
            run_id: row.run_id,
            step_id: row.step_id,
            subject_name: row.subject_name.clone(),
            subject_digest: row.subject_digest.clone(),
            predicate_type: row.predicate_type.clone(),
            predicate_digest: row.predicate_digest.clone(),
            signer_did: row.signer_did.clone(),
            parent_id: row.parent_id.clone(),
            chain_hash: row.chain_hash.clone(),
            created_at: row.created_at,
            verified: link.verified,
            refusal: link.refusal,
        })
        .collect();
    Ok(ChainRead {
        run_id,
        chain,
        verdict: VerdictRead {
            verified: verdict.verified,
            head: verdict.head,
            link_count: verdict.link_count,
            signer_dids: verdict.signer_dids,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic test signing key. Generated, never literal (the
    /// `src/testkeys.rs` doctrine: CodeQL's hard-coded-cryptographic-value
    /// query taint-flags literal key bytes reaching a signing sink, and these
    /// ARE signing sinks).
    fn test_key(seed: u64) -> SigningKey {
        let bytes = crate::testkeys::unit_hmac_key(seed);
        let fixed: [u8; 32] = bytes.as_slice().try_into().expect("a 32-byte seed");
        SigningKey::from_bytes(&fixed)
    }

    // ── the envelope ────────────────────────────────────────────────────────

    /// The envelope is a pure function of its facts: the same link sealed twice
    /// is byte-identical, or nothing downstream can be reasoned about.
    #[test]
    fn attestation_envelope_is_deterministic() {
        let key = test_key(11);
        let did = test_did(&key);
        let first = seal_root(&did, &key).expect("first seal");
        let second = seal_root(&did, &key).expect("second seal");
        assert_eq!(
            first.envelope, second.envelope,
            "two seals of the same link must be byte-identical"
        );
        assert_eq!(
            first.chain_hash, second.chain_hash,
            "the record digest is a function of the link's facts"
        );
        assert_eq!(
            first.row_id, second.row_id,
            "the row id is content-addressed"
        );
    }

    /// KNOWN ANSWER, not self-consistency. Three mutually incompatible signing
    /// formulas coexist in `ump_integrity` (`sign_hash` over the raw BLAKE3
    /// digest, `sign_manifest_bytes` over a hex-SHA256 string, `sign_hash_string`
    /// over BLAKE3 of the `blake3:…` STRING) and two canonicalizers coexist
    /// (`canonical_ump`, the RFC 8785 flavor the reference `contentHash`
    /// reproduces, and `canonical_jcs`, a bare `serde_json::to_vec`). A
    /// self-consistent test passes under ANY of the six pairings, so this one
    /// recomputes the signed message by hand from the stored envelope and
    /// pins the exact `content_hash` string and the exact signature bytes.
    #[test]
    fn attestation_signs_the_shipped_canonical_form() {
        let key = test_key(11);
        let did = test_did(&key);
        let sealed = seal_root(&did, &key).expect("seal");

        // The signed object is the envelope MINUS its integrity block.
        let mut signed = sealed.envelope.clone();
        let removed = signed
            .as_object_mut()
            .expect("an envelope is an object")
            .remove("integrity");
        assert!(
            removed
                .as_ref()
                .and_then(|v| v.get("content_hash"))
                .is_some(),
            "the integrity block is the signed object's only exclusion"
        );

        let canonical =
            crate::ump_integrity::canonical_ump(&signed).expect("the shipped canonicalizer");
        let expected_hash = crate::ump_integrity::content_hash_string(&canonical);
        assert_eq!(
            sealed.content_hash, expected_hash,
            "the recorded content_hash IS blake3: + base32(blake3(canonical_ump(envelope minus integrity)))"
        );

        let emitted = sealed.envelope["integrity"]["signature"]
            .as_str()
            .expect("the integrity block carries a signature")
            .strip_prefix("ed25519:")
            .expect("the signature is scheme-prefixed");
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(emitted)
                .expect("standard base64"),
            crate::ump_integrity::sign_hash_string(&expected_hash, &key),
            "the signature is sign_hash_string over the content-hash STRING"
        );
    }

    /// The canonicalizer is a POLICY the value space makes SAFE, and this pin
    /// proves both halves rather than assuming either.
    ///
    /// The first run of this red-proof — swapping the shipped `canonical_ump`
    /// for its bare-`serde_json` sibling `canonical_jcs` — did NOT go red. That
    /// is not a vacuous pin; it is a fact about the two functions: on an object
    /// of ASCII keys with no floats they emit IDENTICAL bytes, so on THIS
    /// envelope the choice cannot be observed. The two differ only where
    /// integral floats lose their `.0` and where U+2028/U+2029 are escaped.
    ///
    /// So the honest claim is not "the canonicalizer is pinned by its output". It
    /// is: the two canonicalizers genuinely differ (witness below), AND the
    /// signed object contains no value they could disagree about. That turns
    /// the coincidence into a GUARANTEE, and a future change that put a float or
    /// a U+2028 into a signed field would fail here instead of quietly producing
    /// a signature over bytes the verifier would read differently.
    #[test]
    fn attestation_canonical_form_carries_nothing_the_canonicalizers_disagree_about() {
        // Half one: the witness. If the two canonicalizers ever CONVERGED, the
        // guarantee below would be vacuous, so the difference is proved first.
        let witness = serde_json::json!({"a": 1.0, "b": "line\u{2028}sep"});
        let ump = crate::ump_integrity::canonical_ump(&witness).expect("ump canonicalizes");
        let jcs = crate::ump_integrity::canonical_jcs(&witness).expect("jcs canonicalizes");
        assert_ne!(
            ump, jcs,
            "the two canonicalizers must genuinely differ, or the guarantee below proves \
             nothing — an integral float and U+2028 are where they part company"
        );

        // Half two: the guarantee, over every link this module can seal.
        let key = test_key(11);
        let did = test_did(&key);
        for mut link in [
            root_link(),
            ChainLink {
                subject_name: "delivery/phase/design".to_string(),
                subject_digest: format!("sha256:{}", "f".repeat(64)),
                step_id: 9_817_000_000_000,
                ..root_link()
            },
        ] {
            let sealed = seal_link(&link, &did, &key).expect("seal");
            let mut signed = sealed.envelope.clone();
            signed.as_object_mut().expect("object").remove("integrity");
            let canonical = crate::ump_integrity::canonical_ump(&signed).expect("canonicalize");
            let text = String::from_utf8(canonical.clone()).expect("UTF-8");
            assert!(
                !text.contains('\u{2028}') && !text.contains('\u{2029}'),
                "a U+2028/9 in a signed field is one of the two values the canonicalizers \
                 disagree about"
            );
            assert!(
                !text.contains(".0"),
                "an integral float in a signed field is the other: the two canonicalizers \
                 disagree about it, so it must never reach the signed object"
            );
            assert_eq!(
                crate::ump_integrity::canonical_jcs(&signed).expect("jcs"),
                canonical,
                "on THIS value space the two agree — which is the whole point of the two \
                 assertions above, and what makes the shipped choice safe rather than lucky"
            );
            link.step_id += 1;
        }
    }

    /// The pinned formula, discriminated against the SIBLING message shapes.
    ///
    /// `sign_hash` and `sign_hash_string` are not two primitives with different
    /// algebra — they differ in the MESSAGE the caller supplies. The signed
    /// message here is BLAKE3 of the `blake3:…` STRING; the raw-digest profile
    /// would have signed the 32 content-hash bytes themselves. The same
    /// signature must verify under one and refuse under the other, and this is
    /// what stops a future edit from quietly producing a well-formed signature
    /// that verifies against nothing.
    #[test]
    fn attestation_signs_with_the_pinned_formula() {
        let key = test_key(11);
        let did = test_did(&key);
        let sealed = seal_root(&did, &key).expect("seal");
        let pk = key.verifying_key().to_bytes();

        assert!(
            crate::ump_integrity::verify_hash_string(&sealed.content_hash, &pk, &sealed.signature),
            "the pinned formula verifies its own signature"
        );
        let mut signed = sealed.envelope.clone();
        signed.as_object_mut().expect("object").remove("integrity");
        let canonical = crate::ump_integrity::canonical_ump(&signed).expect("canonicalize");
        let raw_digest = crate::ump_integrity::record_hash(&canonical);
        assert!(
            !crate::ump_integrity::verify_hash(&pk, &raw_digest, &sealed.signature),
            "the signature must NOT verify over the raw 32-byte content digest — that is \
             sign_hash's message, and this one is sign_hash_string's"
        );
        // And the hex-SHA-256 manifest profile is a third message again.
        let manifest_digest = {
            use sha2::Digest as _;
            let mut h = sha2::Sha256::new();
            h.update(&canonical);
            hex_encode_lower(&h.finalize())
        };
        let mut manifest_key_bytes = [0_u8; 32];
        manifest_key_bytes.copy_from_slice(&pk);
        assert_ne!(
            manifest_digest.as_bytes(),
            sealed.content_hash.as_bytes(),
            "the two hash families are different messages, which is why picking one is a \
             decision and not an accident"
        );
    }

    /// The predicate TYPE and the PARENT are inside the signed object (A1).
    /// Changing either stops the ORIGINAL signature verifying — which is the
    /// point: a re-typed or re-parented link is not the link that was signed.
    #[test]
    fn attestation_signature_covers_predicate_type_and_parent() {
        let key = test_key(11);
        let did = test_did(&key);
        let sealed = seal_root(&did, &key).expect("seal");
        let pk = key.verifying_key().to_bytes();
        assert!(
            crate::ump_integrity::verify_hash_string(&sealed.content_hash, &pk, &sealed.signature),
            "the link verifies as sealed"
        );

        for (field, value) in [
            ("predicate_type", json!("urn:brain:attest:delivery:v2")),
            ("parent_id", json!("att_somewhere_else")),
        ] {
            let mut tampered = sealed.envelope.clone();
            tampered[field] = value;
            let mut stripped = tampered.clone();
            stripped
                .as_object_mut()
                .expect("object")
                .remove("integrity");
            let canonical =
                crate::ump_integrity::canonical_ump(&stripped).expect("canonicalize the tamper");
            let rehashed = crate::ump_integrity::content_hash_string(&canonical);
            assert_ne!(
                rehashed, sealed.content_hash,
                "changing the signed `{field}` must change the hashed object at all"
            );
            assert!(
                !crate::ump_integrity::verify_hash_string(&rehashed, &pk, &sealed.signature),
                "changing the signed `{field}` must stop the ORIGINAL signature verifying"
            );
        }

        assert_eq!(
            sealed.envelope["predicate_type"],
            json!(brain_delivery_core::PREDICATE_TYPE),
            "the envelope's predicate_type is the crate's shipped constant, never a literal here"
        );
    }

    // ── the offline verifier ────────────────────────────────────────────────

    /// A real cryptographic failure: one flipped signature bit.
    #[test]
    fn attestation_verify_rejects_a_flipped_bit() {
        let key = test_key(11);
        let did = test_did(&key);
        let sealed = seal_root(&did, &key).expect("seal");
        let mut row = row_of(&sealed, &did);
        let mut signature = sealed.signature.clone();
        signature[0] ^= 0x01;
        row.envelope_json = replace_signature(&sealed.envelope, &signature);
        let verdict = verify_chain(std::slice::from_ref(&row), 1_790_000_100)
            .expect("a one-link chain is readable");
        assert!(!verdict.verified, "a flipped signature bit must not verify");
        assert_eq!(
            verdict.links[0].refusal,
            Some(ChainRefusal::SignatureInvalid.as_str()),
            "the refusal names the signature: {:?}",
            verdict.links[0].refusal
        );
    }

    /// A `did:key` that does not decode to a key at all. The envelope is fully
    /// self-consistent and correctly signed — the only false thing is the
    /// identity it claims — so the refusal must be the SIGNER's, not the
    /// signature's.
    #[test]
    fn attestation_verify_rejects_an_unknown_signer() {
        let key = test_key(11);
        let bogus = "did:key:zNotBase58!!";
        let sealed = seal_claiming(&root_link(), bogus, &key);
        let mut row = row_of(&sealed, bogus);
        row.signer_did = bogus.to_string();
        let verdict = verify_chain(std::slice::from_ref(&row), ROOT_LINK_NOW + 100)
            .expect("a one-link chain is readable");
        assert!(!verdict.verified, "an undecodable signer must not verify");
        assert_eq!(
            verdict.links[0].refusal,
            Some(ChainRefusal::UnknownSigner.as_str())
        );
    }

    /// Two distinct foreign-signer failures, kept apart because they are
    /// different facts about the same evidence.
    ///
    /// Arm A: the attribution block names a signer the signed object does not.
    /// The bytes verify; the ATTRIBUTION is the lie, so the refusal is
    /// `SignerMismatch`. Arm B: the object was re-signed by a different key
    /// while still claiming the original signer — the attribution is
    /// internally consistent, the signature is the forgery, and the refusal is
    /// `SignatureInvalid`. A caller must be able to tell them apart.
    #[test]
    fn attestation_verify_rejects_a_foreign_signer() {
        let key = test_key(11);
        let did = test_did(&key);
        let foreign = test_key(99);
        let foreign_did = test_did(&foreign);
        assert_ne!(did, foreign_did, "the two keys really differ");

        // Arm A: integrity.signer edited to a foreign did, bytes untouched.
        let sealed = seal_root(&did, &key).expect("seal");
        let mut envelope = sealed.envelope.clone();
        envelope["integrity"]["signer"] = serde_json::json!(foreign_did);
        let mut row = row_of(&sealed, &did);
        row.envelope_json = serde_json::to_string(&envelope).expect("re-serialize");
        let verdict = verify_chain(std::slice::from_ref(&row), ROOT_LINK_NOW + 100)
            .expect("a one-link chain is readable");
        assert!(!verdict.verified, "a foreign attribution must not verify");
        assert_eq!(
            verdict.links[0].refusal,
            Some(ChainRefusal::SignerMismatch.as_str()),
            "the integrity block's signer and the signed signer_did must agree"
        );

        // Arm B: the signed object re-signed by a foreign key.
        let mut row = row_of(&sealed, &did);
        let foreign_signature =
            crate::ump_integrity::sign_hash_string(&sealed.content_hash, &foreign);
        row.envelope_json = replace_signature(&sealed.envelope, &foreign_signature);
        let verdict = verify_chain(std::slice::from_ref(&row), ROOT_LINK_NOW + 100)
            .expect("a one-link chain is readable");
        assert!(!verdict.verified, "a foreign signature must not verify");
        assert_eq!(
            verdict.links[0].refusal,
            Some(ChainRefusal::SignatureInvalid.as_str()),
            "the signature is the forgery, and the refusal says so"
        );
    }

    /// A row with no signature is never a degraded mark: it is a refusal. The
    /// hash-only L2 posture is a DIFFERENT product, and this surface never
    /// silently downgrades into it.
    #[test]
    fn attestation_verify_rejects_an_unsigned_row() {
        let key = test_key(11);
        let did = test_did(&key);
        let sealed = seal_root(&did, &key).expect("seal");
        let mut row = row_of(&sealed, &did);
        let mut envelope = sealed.envelope.clone();
        let object = envelope.as_object_mut().expect("object");
        if let Some(integrity) = object.get_mut("integrity").and_then(Value::as_object_mut) {
            integrity.remove("signature");
            integrity.remove("signer");
        }
        row.envelope_json = serde_json::to_string(&envelope).expect("re-serialize");
        let verdict = verify_chain(std::slice::from_ref(&row), 1_790_000_100)
            .expect("a one-link chain is readable");
        assert!(
            !verdict.verified,
            "an unsigned row must never read as verified"
        );
        assert_eq!(
            verdict.links[0].refusal,
            Some(ChainRefusal::IntegrityIncomplete.as_str())
        );
    }

    /// The predicate inside the envelope must be the crate's CANONICAL form.
    ///
    /// The envelope is RE-SEALED after the predicate is reordered, so the
    /// content hash and the signature are both valid and the predicate's own
    /// canonicity is the only thing left to catch it. Without the re-seal this
    /// test would pass for the wrong reason — the object changed, so the
    /// content-hash check fires first and the predicate check is never reached.
    #[test]
    fn attestation_verify_rejects_a_non_canonical_predicate() {
        let key = test_key(11);
        let did = test_did(&key);
        let sealed = seal_root(&did, &key).expect("seal");
        let mut envelope = sealed.envelope.clone();
        let predicate = envelope["predicate"].clone();
        let object = predicate.as_object().expect("the predicate is an object");
        // Same two facts, wrong key order: semantically close, byte-wise
        // different, and no longer the crate's canonical encoding.
        envelope["predicate"] = serde_json::json!({
            "tier": object["tier"].clone(),
            "run_ref": object["run_ref"].clone(),
        });
        let mut row = row_of(&sealed, &did);
        row.envelope_json = reseal(&envelope, &key);
        let verdict = verify_chain(std::slice::from_ref(&row), ROOT_LINK_NOW + 100)
            .expect("a one-link chain is readable");
        assert!(
            !verdict.verified,
            "a non-canonical predicate must not verify"
        );
        assert_eq!(
            verdict.links[0].refusal,
            Some(ChainRefusal::PredicateMalformed.as_str()),
            "parse_canonical_predicate's own refusal: {:?}",
            verdict.links[0].refusal
        );
    }

    /// The stored `predicate_digest` COLUMN is cross-checked against the
    /// envelope's own predicate, so a column that disagrees with the signed
    /// bytes is caught even when the signature over those bytes is valid. This
    /// is the check that stops a digest from being asserted beside a
    /// predicate it does not describe.
    #[test]
    fn attestation_verify_rejects_a_predicate_digest_that_disagrees() {
        let key = test_key(11);
        let did = test_did(&key);
        let sealed = seal_root(&did, &key).expect("seal");
        let mut row = row_of(&sealed, &did);
        // Same length, one hex character different: a digest that is
        // well-formed and describes something else.
        let flipped = if sealed.predicate_digest.starts_with('0') {
            "1"
        } else {
            "0"
        };
        row.predicate_digest = format!("{flipped}{}", &sealed.predicate_digest[1..]);
        let verdict = verify_chain(std::slice::from_ref(&row), 1_790_000_100)
            .expect("a one-link chain is readable");
        assert!(
            !verdict.verified,
            "a disagreeing predicate digest must not verify"
        );
        assert_eq!(
            verdict.links[0].refusal,
            Some(ChainRefusal::PredicateDigestMismatch.as_str()),
            "the recomputed predicate digest must equal the stored column: {:?}",
            verdict.links[0].refusal
        );
    }

    /// The chain walk itself (DO invariant 2, offline): a link whose
    /// `parent_id` names a row that is not the previous link.
    #[test]
    fn attestation_verify_rejects_a_broken_chain() {
        let key = test_key(11);
        let did = test_did(&key);
        let first = seal_root(&did, &key).expect("seal the first");
        let mut child_link = root_link();
        child_link.step_id = 9818;
        child_link.subject_name = "delivery/phase/release".to_string();
        let child = seal_link(&child_link, &did, &key).expect("seal the child");
        let mut child_row = child.row(&did, &child_link, ROOT_LINK_NOW);
        // Sealed as a root, so its parent is empty; point it at a row that is
        // not in this chain.
        child_row.parent_id = "att_not_in_this_chain".to_string();
        let chain = vec![row_of(&first, &did), child_row];
        let verdict = verify_chain(&chain, 1_790_000_100).expect("a two-link chain is readable");
        assert!(!verdict.verified, "a broken link must not verify");
        assert_eq!(
            verdict.links[1].refusal,
            Some(ChainRefusal::BrokenLink.as_str())
        );
    }

    /// A root that names a parent is a FRAGMENT of a longer chain, not a
    /// chain — the crate's own `RootHasParent` defect, adopted here.
    #[test]
    fn attestation_verify_rejects_a_root_that_names_a_parent() {
        let key = test_key(11);
        let did = test_did(&key);
        let sealed = seal_root(&did, &key).expect("seal");
        let mut row = row_of(&sealed, &did);
        row.parent_id = "att_something".to_string();
        let verdict = verify_chain(std::slice::from_ref(&row), 1_790_000_100)
            .expect("a one-link chain is readable");
        assert!(!verdict.verified, "a fragment is not a chain");
        assert_eq!(
            verdict.links[0].refusal,
            Some(ChainRefusal::RootHasParent.as_str())
        );
    }

    /// A link that claims a creation time in the future is refused. `now` is a
    /// PARAMETER precisely so the verifier never reads a clock (DO invariant
    /// 2) — and this is the check that parameter exists for.
    #[test]
    fn attestation_verify_rejects_a_link_dated_in_the_future() {
        let key = test_key(11);
        let did = test_did(&key);
        let sealed = seal_root(&did, &key).expect("seal");
        let row = row_of(&sealed, &did);
        let verdict = verify_chain(std::slice::from_ref(&row), row.created_at - 1)
            .expect("a one-link chain is readable");
        assert!(
            !verdict.verified,
            "a link dated after the read must not verify"
        );
        assert_eq!(
            verdict.links[0].refusal,
            Some(ChainRefusal::DatedInFuture.as_str())
        );
    }

    /// `chain_hash` IS the shipped `Attestation::record_digest()` (A2) — the
    /// crate's own pipe-framed, domain-separated digest. Recomputing it from
    /// the seven fields must reproduce the stored value byte for byte.
    #[test]
    fn attestation_chain_hash_is_the_shipped_record_digest() {
        let key = test_key(11);
        let did = test_did(&key);
        let sealed = seal_root(&did, &key).expect("seal");
        let row = row_of(&sealed, &did);
        let rebuilt = Attestation {
            subject_name: root_link().subject_name.clone(),
            subject_digest: root_link().subject_digest.clone(),
            predicate_digest: row.predicate_digest.clone(),
            predicate_type: brain_delivery_core::PREDICATE_TYPE.to_string(),
            policy_digest: String::new(),
            signer_did: did,
            parent_digest: String::new(),
        };
        assert_eq!(
            row.chain_hash,
            rebuilt.record_digest(),
            "the stored chain_hash is the crate's record_digest over the same seven fields"
        );
    }

    /// The links are ORDERED (A3): the child's `parent_id` is the parent's ROW
    /// ID, and the child's `Attestation::parent_digest` is the parent's
    /// `chain_hash`. Both halves are checked, because either alone is forgeable
    /// — a row id is not a digest, and a digest alone does not order the rows.
    #[test]
    fn attestation_chain_links_are_ordered() {
        let mut conn = test_db();
        let operator = crate::test_support::operator_key_guard();
        let first_link = ChainLink {
            step_id: 1,
            ..root_link()
        };
        let first = append_in_own_tx(&mut conn, &first_link).expect("the first link");
        let second_link = ChainLink {
            step_id: 2,
            subject_name: "delivery/phase/release".to_string(),
            ..root_link()
        };
        let second = append_in_own_tx(&mut conn, &second_link).expect("the second link");

        let rows = read_chain(&conn, root_link().run_id).expect("read the chain");
        assert_eq!(rows.len(), 2, "two links were appended");
        assert_eq!(rows[0].id, first, "the chain reads in append order");
        assert_eq!(rows[1].id, second);
        assert_eq!(rows[0].parent_id, "", "the root names no parent");
        assert_eq!(
            rows[1].parent_id, first,
            "the child names the parent's ROW ID"
        );
        assert_eq!(
            rows[1].chain_hash,
            Attestation {
                subject_name: second_link.subject_name.clone(),
                subject_digest: second_link.subject_digest.clone(),
                predicate_digest: rows[1].predicate_digest.clone(),
                predicate_type: brain_delivery_core::PREDICATE_TYPE.to_string(),
                policy_digest: String::new(),
                signer_did: operator.did.clone(),
                parent_digest: rows[0].chain_hash.clone(),
            }
            .record_digest(),
            "the child's record digest binds the PARENT'S CHAIN HASH, not its row id"
        );
        let verdict = verify_chain(&rows, 1_790_000_100).expect("a two-link chain is readable");
        assert!(
            verdict.verified,
            "a well-formed two-link chain verifies: {verdict:?}"
        );
        assert_eq!(
            verdict.head.as_deref(),
            Some(second.as_str()),
            "the head is the last link"
        );
        assert_eq!(verdict.signer_dids, vec![operator.did.clone()]);
    }

    /// A5: `record_digest` frames its seven fields with `|`, so a value that
    /// itself contains `|` — or a control byte — would let two different links
    /// frame to the same bytes. That is a typed refusal at the WRITE.
    /// `record_digest` itself is NOT modified: it is the frozen
    /// back-compatible form, and the fence lives at the boundary instead.
    #[test]
    fn attestation_refuses_a_pipe_in_a_record_digest_field() {
        let key = test_key(11);
        let did = test_did(&key);
        for (field, poisoned) in [
            ("subject_name", "delivery/phase|build"),
            ("subject_digest", "sha256:aa\u{1f}bb"),
            ("policy_digest", "sha256:cc\ndd"),
        ] {
            let mut link = root_link();
            match field {
                "subject_name" => link.subject_name = poisoned.to_string(),
                "subject_digest" => link.subject_digest = poisoned.to_string(),
                _ => link.policy_digest = Some(poisoned.to_string()),
            }
            let err =
                seal_link(&link, &did, &key).expect_err("a framing-ambiguous value must refuse");
            assert!(
                matches!(err, AttestationError::NameRefused { .. }),
                "the `{field}` refusal is typed, not a silent pass: {err:?}"
            );
        }
    }

    /// A9: `signer_did` IS the key history. A rotation changes who signs NEXT;
    /// it does not retroactively invalidate a link, because the verifying key
    /// is recovered from the link's OWN `signer_did` and nothing else. This is
    /// authorship-≠-authority in its most testable form: there is no key file,
    /// no counter, and no epoch in the loop.
    #[test]
    fn attestation_rotation_does_not_invalidate_history() {
        let key = test_key(11);
        let did = test_did(&key);
        let sealed = seal_root(&did, &key).expect("seal");
        let row = row_of(&sealed, &did);
        let rotated = test_key(12);
        let rotated_did = test_did(&rotated);
        assert_ne!(did, rotated_did, "the rotation really changed the key");

        let verdict =
            verify_chain(std::slice::from_ref(&row), 1_790_000_100).expect("a one-link chain");
        assert!(
            verdict.verified,
            "a link signed by a rotated-away key stays verifiable: {verdict:?}"
        );
        assert_eq!(verdict.signer_dids, vec![did]);
        assert!(
            !verdict.signer_dids.contains(&rotated_did),
            "the new key is not in the history"
        );
    }

    /// The chain walk is BOUNDED. An unbounded read of an append-only log is an
    /// availability hole, so the cap is a refusal, not a truncation.
    #[test]
    fn attestation_chain_walk_is_bounded() {
        let mut conn = test_db();
        let _operator = crate::test_support::operator_key_guard();
        for step in 1..=MAX_CHAIN_LINKS {
            let link = ChainLink {
                step_id: i64::try_from(step).expect("a bounded step index"),
                ..root_link()
            };
            append_in_own_tx(&mut conn, &link).expect("append inside the cap");
        }
        let rows = read_chain(&conn, root_link().run_id).expect("read the chain");
        assert_eq!(
            rows.len(),
            MAX_CHAIN_LINKS,
            "every link inside the cap was read"
        );
        let verdict = verify_chain(&rows, 1_790_000_100).expect("a bounded chain is readable");
        assert!(verdict.verified, "a full chain inside the cap verifies");
        assert_eq!(verdict.link_count, MAX_CHAIN_LINKS);

        // One past the cap refuses rather than reading a partial chain.
        let over = ChainLink {
            step_id: i64::try_from(MAX_CHAIN_LINKS).expect("a bounded count") + 1,
            ..root_link()
        };
        let err = append_in_own_tx(&mut conn, &over).expect_err("a chain past the cap refuses");
        assert!(
            matches!(err, AttestationError::ChainFull),
            "the refusal is the cap's own: {err:?}"
        );
    }

    // ── the module's own gates ──────────────────────────────────────────────

    /// DO invariant 2 as a named test: the chain verifies OFFLINE. The
    /// verifier is a pure function — no socket, no config, no key file, no
    /// AppState, no clock. `now` is a parameter precisely so it cannot read
    /// one. This is a source scan of the PRODUCTION region with the
    /// `#[cfg(test)]` boundary LOCATED, not assumed (the an earlier round lesson: four
    /// guards shipped vacuous because their scan region was wrong).
    #[test]
    fn attestation_offline_verification_touches_no_io() {
        let body = verification_bodies(include_str!("attestations.rs"));
        for forbidden in [
            "std::fs",
            "std::net",
            "std::process",
            "chrono",
            "Utc",
            "env::",
            "AppState",
            "resolve_operator_key",
            "operator_key",
            "Pool",
            "read_chain",
        ] {
            assert!(
                !body.contains(forbidden),
                "the offline verifier must not reach `{forbidden}` — it is handed the rows, \
                 never the store"
            );
        }
        assert!(
            body.contains("now: i64"),
            "the clock is a parameter, never read: the verification path takes `now`"
        );
    }

    /// A13: the verifier parses each `envelope_json` ONCE and hands the SAME
    /// parsed object onward. A second parse after verification would re-derive
    /// the payload from bytes that were never the ones checked — the parse-once
    /// property DSSE's own envelope format states, adopted here for a
    /// format that is NOT DSSE.
    #[test]
    fn attestation_verifier_does_not_reparse_after_verification() {
        let body = verification_bodies(include_str!("attestations.rs"));
        let parses = body.matches("from_str").count();
        assert_eq!(
            parses, 1,
            "the verification path parses each envelope exactly once (found {parses} parses \
             across verify_chain and verify_link_inner)"
        );
    }

    // ── helpers ─────────────────────────────────────────────────────────────

    fn test_did(key: &SigningKey) -> String {
        crate::ump_integrity::did_key_from_ed25519(&key.verifying_key().to_bytes())
    }

    use serde_json::Value;
    use serde_json::json;

    /// A root link over the shipped 13-field predicate.
    fn root_link() -> ChainLink {
        ChainLink {
            domain: "global".to_string(),
            run_id: 4211,
            step_id: 9817,
            subject_name: "delivery/phase/build".to_string(),
            subject_digest: format!("sha256:{}", "a".repeat(64)),
            policy_digest: None,
            config_digest: None,
            model_ref: None,
            model_digest: None,
            tier: AutonomyTier::BoundedAuto,
        }
    }

    /// The stored row for a sealed link, the way `append_link` builds it.
    fn row_of(sealed: &Sealed, did: &str) -> AttestationRow {
        sealed.row(did, &root_link(), ROOT_LINK_NOW)
    }

    /// Seal the root link with a test key.
    fn seal_root(did: &str, key: &SigningKey) -> Result<Sealed, AttestationError> {
        seal_envelope(&root_link(), did, key, ROOT_LINK_NOW)
    }

    /// Seal an arbitrary link with a test key.
    fn seal_link(
        link: &ChainLink,
        did: &str,
        key: &SigningKey,
    ) -> Result<Sealed, AttestationError> {
        seal_envelope(link, did, key, ROOT_LINK_NOW)
    }

    /// Seal a link whose `signer_did` is whatever the caller names. A
    /// consistent envelope carrying a signer that does not DECODE is only
    /// reachable by re-signing, which is exactly what this does: the object,
    /// its hash, and its signature all agree with each other, and the only
    /// false thing is the identity.
    fn seal_claiming(link: &ChainLink, claimed: &str, key: &SigningKey) -> Sealed {
        let predicate = predicate_for(link).expect("the predicate seals");
        let digest = predicate_digest(&predicate);
        seal(link, claimed, key, &digest, &predicate, None, ROOT_LINK_NOW)
            .expect("a consistent envelope under a claimed signer")
    }

    /// Swap a link's signature bytes into its stored envelope, leaving every
    /// other byte alone — the shape of a real tamper, not a re-seal.
    fn replace_signature(envelope: &Value, signature: &[u8]) -> String {
        let mut tampered = envelope.clone();
        tampered["integrity"]["signature"] = json!(format!(
            "ed25519:{}",
            base64::engine::general_purpose::STANDARD.encode(signature)
        ));
        serde_json::to_string(&tampered).expect("re-serialize the tampered envelope")
    }

    /// Re-sign a tampered envelope with the same key, so the content hash and
    /// the signature are VALID and whatever the test changed is the only thing
    /// left for the verifier to catch.
    fn reseal(envelope: &Value, key: &SigningKey) -> String {
        let mut signed = envelope.clone();
        signed.as_object_mut().expect("object").remove("integrity");
        let canonical = crate::ump_integrity::canonical_ump(&signed).expect("canonicalize");
        let content_hash = crate::ump_integrity::content_hash_string(&canonical);
        let signature = crate::ump_integrity::sign_hash_string(&content_hash, key);
        let signer = envelope["signer_did"]
            .as_str()
            .expect("a signer")
            .to_string();
        let mut complete = envelope.clone();
        complete["integrity"] = serde_json::json!({
            "content_hash": content_hash,
            "signature": format!(
                "ed25519:{}",
                base64::engine::general_purpose::STANDARD.encode(&signature)
            ),
            "signer": signer,
        });
        String::from_utf8(crate::ump_integrity::canonical_ump(&complete).expect("canonicalize"))
            .expect("UTF-8")
    }

    fn hex_encode_lower(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The verification PATH's production bodies: the walk and the per-link
    /// checks it delegates to. Both must be present — a scan of only one would
    /// be a scan of the wrong region, which is the an earlier round vacuity lesson.
    fn verification_bodies(source: &str) -> String {
        [
            fn_body(source, "verify_chain"),
            fn_body(source, "verify_link_inner"),
        ]
        .join("\n")
    }

    fn test_db() -> Connection {
        crate::register_sqlite_vec::register_sqlite_vec();
        let mut conn = Connection::open_in_memory().expect("open in-memory DB");
        crate::migration::run_migration(&mut conn, 512).expect("migration");
        conn
    }

    /// The PRODUCTION region of a function: the file is split at its
    /// `#[cfg(test)]` boundary, the named function is located inside it, and
    /// its braces are matched. The slice starts at the `fn` token so the
    /// SIGNATURE is part of it — a parameter is a contract, and "the clock is a
    /// parameter, never read" is exactly a claim about the signature. An empty
    /// or unlocatable extraction fails loudly rather than passing on nothing.
    fn fn_body(source: &str, symbol: &str) -> String {
        let boundary = source
            .find("#[cfg(test)]")
            .expect("the test boundary must be locatable — a scan of the whole file is vacuous");
        let production = &source[..boundary];
        let needle = format!("fn {symbol}(");
        let start = production
            .find(&needle)
            .unwrap_or_else(|| panic!("the production region must declare `fn {symbol}`"));
        let open = production[start..]
            .find('{')
            .unwrap_or_else(|| panic!("`fn {symbol}` must have a body"));
        let mut depth = 0_usize;
        let end = production[start + open..]
            .char_indices()
            .find_map(|(i, c)| match c {
                '{' => {
                    depth += 1;
                    None
                }
                '}' => {
                    depth -= 1;
                    if depth == 0 { Some(i + 1) } else { None }
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("`fn {symbol}`'s braces must balance"));
        let body = &production[start..start + open + end];
        assert!(
            body.len() > 40,
            "`fn {symbol}`'s extracted body is implausibly short ({}) — the extractor, not the \
             function, is broken",
            body.len()
        );
        // Line comments are stripped so a doc comment naming a forbidden token
        // cannot fail the scan (the F7-07 comment-stripping precedent).
        body.lines()
            .map(|l| l.split_once("//").map_or(l, |(code, _)| code))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Append one link in its own transaction, mirroring how `advance()` calls
    /// it — inside a caller-owned `WorkflowTx`.
    fn append_in_own_tx(
        conn: &mut Connection,
        link: &ChainLink,
    ) -> Result<String, AttestationError> {
        let mut tx = crate::workflow::tx::WorkflowTx::begin(conn).expect("begin");
        let id = append_link(tx.tx(), link, ROOT_LINK_NOW)?;
        tx.commit().expect("commit");
        Ok(id)
    }

    const ROOT_LINK_NOW: i64 = 1_790_000_000;
}
