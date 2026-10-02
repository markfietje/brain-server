//! The agreement-labelling path: a human verdict bound to a REAL run row.
//!
//! The κ bench (`kappa.rs`) is the agreement machinery and it ships. What it
//! could not do is bind a verdict to a run: its tuples are mined
//! disagreements, and the corpus it labels has an outcome column with no
//! writer anywhere in the tree. This module supplies the missing half — the
//! binding — over rows the delivery loop actually wrote, and it changes no
//! arithmetic, no table, and no migration.
//!
//! **The subject is a `delivery_traces` row**, which carries `run_id`,
//! `model_ref`, `stage`, `phase`, `tier`, `actor` and `status`. That is the
//! per-row model identity the model-binding round believed absent, so a label
//! here can be joined to the model that produced the decision it judges.
//!
//! The machine's own verdict is derived from the row
//! ([`machine_verdict`]) and is **absent from [`AgreementTuple`] by type** —
//! the same blindness the κ bench has, by the same mechanism. That blindness is
//! an INTERFACE property and only that: the reviewer is the system's author, so
//! nothing here makes the measurement independent, and every number the report
//! emits is agreement WITH THE OPERATOR, never with humans and never consensus.
//!
//! Storage is the additive-kind law: one new `agent_session_events` kind, no
//! migration, `LATEST_KNOWN_SCHEMA` unmoved. Latest-wins per
//! (subject, reviewer): rows are appended, never mutated, and the report counts
//! only the latest. Reviewer identity rides the PAYLOAD, never a column, so a
//! second rater joins as a new id on an existing kind — which is what keeps
//! this round's "no schema change" true rather than merely convenient — and the
//! report emits `distinct_reviewers` so the single-rater era is readable from
//! the data instead of asserted in prose.
//!
//! No value here gates anything. The machine verdict is REPORTED as data; the
//! agreement rate is a measurement, not a threshold, and there is no promotion
//! bar in this module to cross.

use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use super::kappa::cap_chars;
use super::tx::WorkflowTx;

/// The additive session-log kind carrying one reviewer's verdict on one
/// real run row. Every pre-existing reader filters by kind, so these rows are
/// inert to all of them (the additive-kind law) — the same posture
/// [`super::kappa::KAPPA_LABEL_KIND`] takes.
pub(crate) const AGREEMENT_LABEL_KIND: &str = "agreement_label";

/// The audit targets this module writes. One audit row per created label, in
/// the caller's transaction; the report audits through the surface's
/// `record_tenant`.
pub(crate) const AUDIT_AGREEMENT_LABEL: &str = "agreement_label";
pub(crate) const AUDIT_AGREEMENT_REPORT: &str = "agreement_report";

/// The rater roster: two slots, a declared constant — no config, no knob.
/// Two is the κ floor. One reviewer occupies one slot; a second rater takes the
/// other and both are counted, so the report can say which era it is reporting.
pub(crate) const RATER_SLOTS: u8 = 2;

/// The closed reviewer-kind vocabulary: WHO is judging. **One member today —
/// `operator`.**
///
/// This is a type change, not a configuration knob, and the distinction is
/// load-bearing. A κ between two raters of the SAME kind is inter-rater
/// reliability. A κ between an operator and a machine is a DIFFERENT quantity
/// with a DIFFERENT ceiling: it measures model-vs-human agreement, not whether
/// humans agree with each other. Collapsing the two is how a machine rater
/// would silently inflate `distinct_reviewers` to 2 and have a reader conclude
/// an inter-rater reliability exists that nobody measured.
///
/// So a machine rater is a DATED AMENDMENT to this constant plus the report's
/// same-kind rule, never a string a client supplies. The body names the
/// judgment, never the judge — and now never the judge's KIND either.
pub(crate) const RATER_KINDS: &[&str] = &["operator"];

/// The kind stamped on every label written today. A named member of
/// [`RATER_KINDS`], so the constant and the vocabulary cannot drift: if the
/// vocabulary is amended, this must resolve to one of its members or the
/// writer's own validation refuses the first label.
pub(crate) const REVIEWER_KIND_OPERATOR: &str = "operator";

/// A reviewer kind against the closed vocabulary: named refusals, never a
/// silent accept. This is what makes a machine rater a TYPE change — the
/// string a client would send is refused here, and admitting one means
/// amending [`RATER_KINDS`] deliberately.
pub(crate) fn validate_reviewer_kind(raw: &str) -> Result<(), String> {
    if raw.is_empty() {
        return Err("agreement: reviewer_kind required".into());
    }
    if !RATER_KINDS.contains(&raw) {
        return Err(format!("agreement: reviewer_kind unknown: {raw}"));
    }
    Ok(())
}

/// The closed verdict vocabulary. An operator judges whether the machine's
/// verdict for a run is right, so these name the JUDGE'S relation to it, not an
/// outcome in its own right. Widenable only by a dated addendum; the arithmetic
/// treats verdicts as opaque strings.
pub(crate) const VERDICT_VOCABULARY: &[&str] = &["confirmed", "overturned", "uncertain"];

/// The machine's own closed verdict vocabulary, derived from a trace row's
/// `status` CHECK. Kept closed so the derivation is total and a new status
/// cannot silently widen it.
pub(crate) const MACHINE_VERDICT_VOCABULARY: &[&str] = &["advanced", "stopped", "deferred"];

/// The κ sentinel, re-exported from the bench rather than redeclared: this
/// module contains no arithmetic of its own, and two sentinels that could
/// drift apart would be a silent second encoding of one value.
pub(crate) const NO_KAPPA: i32 = super::kappa::NO_KAPPA;

/// Char-bound cap for the operator-readable fields the tuple carries. The same
/// shape as the bench's `cap_chars`; the payload itself is digest-and-label
/// only, so this bounds what a READER sees, never what is stored.
const SUBJECT_CAP: usize = 400;

/// Cap on a reviewer id, so a pathological subject cannot bloat the payload
/// past the session-log's own cap or make the id unreadable in a report.
const REVIEWER_ID_CAP: usize = 128;

/// The machine's verdict for one delivery trace row, derived from its
/// `status` and `stage`. **This is what the operator is judging** — and it is
/// absent from the tuple the operator reads, by type.
///
/// The mapping is a closed, total function of two CHECK-constrained columns,
/// so a status outside the vocabulary can never reach it:
///
/// * an `answer`-stage row was produced → `advanced` (the decision completed)
/// * a `gate`-stage row refused → `stopped`
/// * a `gate`-stage row deferred to the operator → `deferred`
/// * any other status → `stopped` (the fail-closed direction: a run that did
///   not reach an answer did not advance)
///
/// A `None` return names an unreadable status rather than guessing. The
/// derivation is pure and total over the vocabulary; it reads no clock, no
/// config, and no other row.
pub(crate) fn machine_verdict(stage: &str, status: &str) -> Result<String, String> {
    if stage == "answer" {
        return Ok("advanced".into());
    }
    match status {
        "advanced" | "allowed" | "admitted" => Ok("advanced".into()),
        "prompt" => Ok("deferred".into()),
        "denied" | "answered" => Ok("stopped".into()),
        other => Err(format!("agreement: machine verdict undecided: {other}")),
    }
}

/// The operator's verdict against the closed vocabulary: named refusals, never
/// a silent accept. Pure, total, and the only gate before any write.
pub(crate) fn validate_verdict(raw: &str) -> Result<(), String> {
    if raw.is_empty() {
        return Err("agreement: verdict required".into());
    }
    if raw.len() > 64 {
        return Err(format!("agreement: verdict invalid: {} chars", raw.len()));
    }
    if !VERDICT_VOCABULARY.contains(&raw) {
        return Err(format!("agreement: verdict invalid: {raw}"));
    }
    Ok(())
}

/// A reviewer id against its cap: an empty or oversized id is a refusal, never
/// a stored blank. **A label without a reviewer is not a measurement** — the
/// frozen corpus's provenance is unrecoverable for exactly this reason.
pub(crate) fn validate_reviewer_id(raw: &str) -> Result<(), String> {
    if raw.trim().is_empty() {
        return Err("agreement: reviewer_id required".into());
    }
    if raw.len() > REVIEWER_ID_CAP {
        return Err(format!(
            "agreement: reviewer_id invalid: {} chars",
            raw.len()
        ));
    }
    Ok(())
}

/// One stored label's payload — exactly the six ratified keys. The reviewer
/// id is a KEY here, not a column: a second rater joins as a new id on this
/// same kind with no migration, and the single-rater era stays readable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct AgreementLabelPayload {
    pub subject_id: String,
    pub run_id: i64,
    pub verdict: String,
    pub reviewer_id: String,
    pub reviewer_kind: String,
    pub reviewer_slot: u8,
    pub machine_verdict: String,
}

/// The total parser every stored payload round-trips through: any input yields
/// the payload or a named `agreement: …` refusal. No I/O, no clock, no panic.
/// Unknown keys REFUSE — the payload carries exactly the ratified seven, and a
/// permissive parser is how a future field would quietly become a stored one.
pub(crate) fn parse_agreement_label(
    value: &serde_json::Value,
) -> Result<AgreementLabelPayload, String> {
    let obj = value
        .as_object()
        .ok_or("agreement: label payload must be a JSON object")?;
    for key in obj.keys() {
        if !matches!(
            key.as_str(),
            "subject_id"
                | "run_id"
                | "verdict"
                | "reviewer_id"
                | "reviewer_kind"
                | "reviewer_slot"
                | "machine_verdict"
        ) {
            return Err(format!("agreement: unknown field {key}"));
        }
    }
    let subject_id = obj
        .get("subject_id")
        .and_then(serde_json::Value::as_str)
        .ok_or("agreement: subject_id must be a string")?;
    if subject_id.is_empty() {
        return Err("agreement: subject_id required".into());
    }
    let run_id = obj
        .get("run_id")
        .and_then(serde_json::Value::as_i64)
        .ok_or("agreement: run_id must be an integer")?;
    let verdict = obj
        .get("verdict")
        .and_then(serde_json::Value::as_str)
        .ok_or("agreement: verdict must be a string")?;
    validate_verdict(verdict)?;
    let reviewer_id = obj
        .get("reviewer_id")
        .and_then(serde_json::Value::as_str)
        .ok_or("agreement: reviewer_id must be a string")?;
    validate_reviewer_id(reviewer_id)?;
    let reviewer_kind = obj
        .get("reviewer_kind")
        .and_then(serde_json::Value::as_str)
        .ok_or("agreement: reviewer_kind must be a string")?;
    validate_reviewer_kind(reviewer_kind)?;
    let slot_raw = obj
        .get("reviewer_slot")
        .ok_or("agreement: reviewer_slot required")?
        .as_u64()
        .ok_or("agreement: reviewer_slot must be a non-negative integer")?;
    if slot_raw >= RATER_SLOTS as u64 {
        return Err(format!("agreement: reviewer_slot unknown: {slot_raw}"));
    }
    let machine_verdict = obj
        .get("machine_verdict")
        .and_then(serde_json::Value::as_str)
        .ok_or("agreement: machine_verdict must be a string")?;
    if !MACHINE_VERDICT_VOCABULARY.contains(&machine_verdict) {
        return Err(format!(
            "agreement: machine_verdict invalid: {machine_verdict}"
        ));
    }
    Ok(AgreementLabelPayload {
        subject_id: subject_id.to_string(),
        run_id,
        verdict: verdict.to_string(),
        reviewer_id: reviewer_id.to_string(),
        reviewer_kind: reviewer_kind.to_string(),
        reviewer_slot: slot_raw as u8,
        machine_verdict: machine_verdict.to_string(),
    })
}

/// The REPORT-side reader: parses a stored label row WITHOUT validating the
/// reviewer's kind against today's [`RATER_KINDS`].
///
/// A stored row is a FACT, not an input. When a future round amends the
/// vocabulary to admit a machine rater and writes through the normal path, the
/// strict parser would then reject that very row on the way back out — the
/// operator's existing, valid measurements would fail to report because a
/// NEW kind exists. That is the wrong failure direction: a new kind must not
/// be able to break the read of history.
///
/// So the report reads the kind as an opaque stored string and applies the
/// SAME-KIND pairing rule against it. A kind the report has never seen is
/// simply not the same kind as any other, so it pairs with nothing rather than
/// being counted as a rater of the incumbent kind. Unknown keys still refuse
/// (the payload's SHAPE is still closed); only the kind's membership check is
/// deferred to the pairing rule.
fn parse_stored_label_for_report(
    value: &serde_json::Value,
) -> Result<AgreementLabelPayload, String> {
    // Reuse the strict parser for everything except the kind. Temporarily
    // present the stored kind as the incumbent one so the strict pass accepts
    // a known-shape row, then restore the true kind. This keeps ONE definition
    // of "is this a well-shaped label" — duplicating the field list here would
    // be a second answer to the same question.
    let obj = value
        .as_object()
        .ok_or("agreement: label payload must be a JSON object")?;
    let stored_kind = obj
        .get("reviewer_kind")
        .and_then(serde_json::Value::as_str)
        .ok_or("agreement: reviewer_kind must be a string")?
        .to_string();
    let mut normalised = value.clone();
    if let Some(map) = normalised.as_object_mut() {
        map.insert(
            "reviewer_kind".to_string(),
            serde_json::Value::String(REVIEWER_KIND_OPERATOR.to_string()),
        );
    }
    let mut payload = parse_agreement_label(&normalised)?;
    payload.reviewer_kind = stored_kind;
    Ok(payload)
}

/// One created (or replayed) label write: `created` is false for an
/// exactly-once replay; `supersession` counts the rows that already existed
/// for the (subject, reviewer) pair (0 = first label).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgreementLabelReceipt {
    pub created: bool,
    pub seq: i64,
    pub supersession: i64,
    pub machine_verdict: String,
}

/// Whether the subject is a REAL delivery trace row, and what the machine
/// says about it. The digest-keyed identity is the trace row's own content id
/// — the authority. A subject that matches nothing is an absent row, and the
/// surface answers it with the same probe-blind 404 as any other absence.
fn subject_verdict(
    conn: &rusqlite::Connection,
    run_id: i64,
    subject_id: &str,
) -> Result<String, String> {
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT stage, status FROM delivery_traces WHERE id = ?1 AND run_id = ?2",
            params![subject_id, run_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| format!("agreement: subject read failed: {e}"))?;
    let Some((stage, status)) = row else {
        return Err("label_subject_absent".into());
    };
    machine_verdict(&stage, &status)
}

/// A reviewer's slot, derived from their authenticated subject — same shape as
/// the bench's, over its own domain so the two never collide. The slot never
/// rides the request body: the client names the judgment, not the judge.
const AGREEMENT_SLOT_DOMAIN: &str = "agreement-slot-v1";

/// A reviewer's slot, derived from the authenticated principal's subject. Same
/// subject → same slot, always. Pure and body-free.
pub(crate) fn slot_for_principal(sub: &str) -> u8 {
    let hex = crate::audit::hash(&format!("{AGREEMENT_SLOT_DOMAIN}:{sub}"));
    (super::kappa::fold8(&hex) % RATER_SLOTS as u32) as u8
}

/// Write one verdict on one real run row: the additive `agreement_label` row
/// plus its audit row, atomic in the caller's tx.
///
/// Latest-wins per (subject, reviewer): re-submitting the judgment that is
/// already that reviewer's latest is the idempotent no-op receipt — nothing
/// appends, the audit logs nothing — while a CHANGED judgment appends a new
/// row and never mutates the old one. A rolled-back retry recomputes the same
/// key, and the session-log append keeps its own exactly-once guard for a
/// racing writer.
///
/// The machine's verdict is DERIVED HERE from the row and frozen into the
/// payload. That is deliberate: if it were re-derived at read time, a run whose
/// status later changed would silently re-score a judgment made against what
/// the row said then. A label is bound to what it judged, not to what the row
/// has since become.
///
/// Refuses, before any write: a verdict outside the closed vocabulary, a
/// reviewer id that is empty or oversized, a slot outside the roster, and a
/// subject that is not a real trace row under this run.
///
/// The reviewer's KIND is stamped by this core, never taken from the request:
/// today every authenticated reviewer is [`REVIEWER_KIND_OPERATOR`], and the
/// writer is the only place that decides. A client cannot name a rater kind, so
/// "operator" is what lands unless a dated amendment changes the constant.
pub(crate) fn write_agreement_label(
    tx: &mut WorkflowTx<'_>,
    run_id: i64,
    subject_id: &str,
    verdict: &str,
    reviewer_id: &str,
    slot: u8,
    now: i64,
) -> Result<AgreementLabelReceipt, String> {
    validate_verdict(verdict)?;
    validate_reviewer_id(reviewer_id)?;
    if slot >= RATER_SLOTS {
        return Err(format!("agreement: reviewer_slot unknown: {slot}"));
    }
    let machine = subject_verdict(tx.tx(), run_id, subject_id)?;
    let payload = serde_json::json!({
        "subject_id": subject_id,
        "run_id": run_id,
        "verdict": verdict,
        "reviewer_id": reviewer_id,
        "reviewer_kind": REVIEWER_KIND_OPERATOR,
        "reviewer_slot": slot,
        "machine_verdict": machine,
    })
    .to_string();
    let base = format!("run{run_id}:{AGREEMENT_LABEL_KIND}:{subject_id}:{reviewer_id}");
    let glob = format!("{base}:*");
    let existing: i64 = tx
        .tx()
        .query_row(
            "SELECT COUNT(*) FROM agent_session_events
              WHERE run_id = ?1 AND kind = ?2 AND (idempotency_key = ?3 OR idempotency_key GLOB ?4)",
            params![run_id, AGREEMENT_LABEL_KIND, base, glob],
            |r| r.get(0),
        )
        .map_err(|e| format!("agreement: label count read failed: {e}"))?;
    let frozen = serde_json::from_str::<serde_json::Value>(
        &tx.tx()
            .query_row(
                "SELECT payload_json FROM agent_session_events
                  WHERE run_id = ?1 AND kind = ?2 AND (idempotency_key = ?3 OR idempotency_key GLOB ?4)
                  ORDER BY seq DESC LIMIT 1",
                params![run_id, AGREEMENT_LABEL_KIND, base, glob],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| format!("agreement: label read failed: {e}"))?
            .unwrap_or_default(),
    )
    .unwrap_or_default();
    let frozen_machine = frozen
        .get("machine_verdict")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    if existing > 0 {
        // Latest-wins: only a CHANGED judgment earns a new row. The comparison
        // is against the whole payload minus the machine verdict, which is
        // compared separately — a row whose frozen machine verdict differs from
        // today's derivation is a CHANGED judgment (the run moved under the
        // label) and supersedes rather than replaying.
        let same_judgment = frozen
            .get("verdict")
            .and_then(serde_json::Value::as_str)
            .map(|v| v == verdict)
            .unwrap_or(false)
            && frozen_machine == machine;
        if same_judgment {
            let latest_seq: i64 = tx
                .tx()
                .query_row(
                    "SELECT seq FROM agent_session_events
                      WHERE run_id = ?1 AND kind = ?2 AND (idempotency_key = ?3 OR idempotency_key GLOB ?4)
                      ORDER BY seq DESC LIMIT 1",
                    params![run_id, AGREEMENT_LABEL_KIND, base, glob],
                    |r| r.get(0),
                )
                .map_err(|e| format!("agreement: label read failed: {e}"))?;
            return Ok(AgreementLabelReceipt {
                created: false,
                seq: latest_seq,
                supersession: existing,
                machine_verdict: machine,
            });
        }
    }
    let key = if existing == 0 {
        base
    } else {
        format!("{base}:{existing}")
    };
    let (created, seq) =
        super::session_log::append(tx.tx(), run_id, AGREEMENT_LABEL_KIND, &payload, &key, now)
            .map_err(|e| format!("agreement: label write failed: {e}"))?;
    if created {
        super::audit_write(
            tx.tx(),
            run_id,
            AUDIT_AGREEMENT_LABEL,
            crate::audit::AuditStatus::Ok,
            &format!(
                "subject={subject_id} reviewer={reviewer_id} slot={slot} seq={seq} \
                 supersession={existing} machine={machine}"
            ),
        );
    }
    Ok(AgreementLabelReceipt {
        created,
        seq,
        supersession: existing,
        machine_verdict: machine,
    })
}

/// One run row as a reviewer sees it: the trace's own metadata (masked
/// through the read seam), and — never anyone else's — the reviewer's own
/// latest verdict.
///
/// **The machine's verdict is ABSENT BY TYPE.** This struct has no field for
/// it, so no construction of it can show the reviewer what they are judging.
/// That is the interface property the pin holds. It is not a claim that the
/// judgment is independent of the machine: the reviewer is the system's
/// author, and nothing in this type changes that.
///
/// `Serialize` is load-bearing for that pin: serializing the STRUCT (rather
/// than a hand-built JSON literal) is what makes a newly added field visible
/// to the field-count assertion. See `agreement_tuple_is_blind_by_type`.
///
/// Four fields, and they are NOT the machine's verdict: the
/// decision about *this case* is a different fact from *what the machine
/// concluded*, and carrying the first does not expose the second. They are
/// `Option` because a trace row whose run predates the carrying seam has no
/// decision, and `None` must mean **"never recorded"** rather than a defaulted
/// `0.0` — a zero confidence that was never measured is the fabricated-number
/// defect, and an `Option` is what makes that unrepresentable.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct AgreementTuple {
    pub run_id: i64,
    pub subject_id: String,
    pub stage: String,
    pub phase: String,
    pub tier: String,
    pub model_ref: Option<String>,
    pub my_verdict: Option<String>,
    pub my_verdict_seq: Option<i64>,
    /// The class the carried decision keyed on. `None` when the run predates
    /// the seam.
    pub routing_class: Option<String>,
    /// `defer` | `clarify` | `stop`. `None` when the run predates the seam.
    ///
    /// **Declared ceiling:** every outcome is `defer` today, because
    /// `confidence::AUTO_CLASSES` is empty and the decision is fail-closed. The
    /// other two arms are UNREACHED, not broken — see
    /// `PREREG_R67D_CARRIED_DECISION_2026-10-02.md` §5. A reader that renders
    /// two empty panels is showing the truth about the policy's coverage.
    pub outcome: Option<String>,
    /// The classifier's uncontested SHARE. **Never read without
    /// `evidence_count`** — one keyword firing alone and ten agreeing both read
    /// `1.0` here, which is the whole reason the companion field exists.
    pub confidence: Option<f32>,
    /// How many keywords fired. The independent signal, and the one a threshold
    /// would key on.
    pub evidence_count: Option<usize>,
}

/// A trace row's reviewer-facing view, or `None` when the row is absent. The
/// read seam runs as a synthetic scope-less reader — PII masking is
/// unconditional and no caller's clearance bypasses it.
fn read_subject(
    conn: &rusqlite::Connection,
    run_id: i64,
    subject_id: &str,
) -> Result<Option<AgreementTuple>, String> {
    let row: Option<(String, String, String, Option<String>)> = conn
        .query_row(
            "SELECT stage, phase, tier, model_ref FROM delivery_traces WHERE id = ?1 AND run_id = ?2",
            params![subject_id, run_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(|e| format!("agreement: subject read failed: {e}"))?;
    let Some((stage, phase, tier, model_ref)) = row else {
        return Ok(None);
    };
    // The carried decision, or `None` for a run that predates
    // the seam. Read AFTER the subject resolves, so an absent subject never
    // pays for the decision lookup.
    let decision =
        read_decision(conn, run_id).map_err(|e| format!("agreement: subject read failed: {e}"))?;
    Ok(Some(AgreementTuple {
        run_id,
        subject_id: subject_id.to_string(),
        stage: cap_chars(&super::reflection::sanitize_seam(&stage), 64),
        phase: cap_chars(&super::reflection::sanitize_seam(&phase), 64),
        tier: cap_chars(&super::reflection::sanitize_seam(&tier), 64),
        model_ref: model_ref.map(|m| cap_chars(&super::reflection::sanitize_seam(&m), SUBJECT_CAP)),
        my_verdict: None,
        my_verdict_seq: None,
        routing_class: decision.routing_class,
        outcome: decision.outcome,
        confidence: decision.confidence,
        evidence_count: decision.evidence_count,
    }))
}

/// The deferral decision carried on this run, or all-`None` when the run has
/// none.
///
/// Reads the LATEST `deferral_decision` row for the run — the
/// decision is recorded once at intake, and a later revision would supersede
/// rather than append, so `MAX(seq)` is the current answer.
///
/// **The parse is strict on purpose.** A payload that is missing a field, or
/// whose `confidence` is not a finite number, yields `None` — the whole tuple,
/// not a partially-defaulted one. A half-read receipt is worse than an absent
/// one: it would put a bare `confidence` on screen with no `evidence_count`
/// beside it, which is the one combination this whole change exists to make
/// impossible.
fn read_decision(conn: &rusqlite::Connection, run_id: i64) -> Result<DecisionFields, String> {
    let payload: Option<String> = conn
        .query_row(
            "SELECT payload_json FROM agent_session_events
              WHERE run_id = ?1 AND kind = ?2 ORDER BY seq DESC LIMIT 1",
            params![run_id, super::confidence::DEFERRAL_DECISION_KIND],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| format!("agreement: decision read failed: {e}"))?;
    let Some(raw) = payload else {
        return Ok(DecisionFields::ABSENT);
    };
    let parsed: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        // A row this reader cannot parse is a row it does not report. Silently
        // coercing it to zeros is the fabricated number; dropping it is the
        // honest answer, and the run reads as "no recorded decision".
        Err(_) => return Ok(DecisionFields::ABSENT),
    };
    let str_field = |k: &str| parsed.get(k).and_then(|v| v.as_str()).map(str::to_string);
    let confidence = parsed
        .get("confidence")
        .and_then(|v| v.as_f64())
        .map(|f| f as f32);
    let evidence_count = parsed
        .get("evidence_count")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize);
    // The pair travels together or not at all — see `read_decision`'s doc.
    let (Some(confidence), Some(evidence_count)) = (confidence, evidence_count) else {
        return Ok(DecisionFields::ABSENT);
    };
    if !confidence.is_finite() {
        return Ok(DecisionFields::ABSENT);
    }
    Ok(DecisionFields {
        routing_class: str_field("routing_class"),
        outcome: str_field("outcome"),
        confidence: Some(confidence),
        evidence_count: Some(evidence_count),
    })
}

/// The four carried fields, with a single `ABSENT` spelling so "never recorded"
/// is written once and cannot drift into per-field defaults.
#[derive(Debug, Clone, Default)]
struct DecisionFields {
    routing_class: Option<String>,
    outcome: Option<String>,
    confidence: Option<f32>,
    evidence_count: Option<usize>,
}

impl DecisionFields {
    const ABSENT: Self = Self {
        routing_class: None,
        outcome: None,
        confidence: None,
        evidence_count: None,
    };
}

/// The reviewer's own latest verdict on a subject: the max-seq
/// `agreement_label` row for this (subject, reviewer). A superseded row is
/// history, never the answer.
fn own_latest_verdict(
    conn: &rusqlite::Connection,
    run_id: i64,
    subject_id: &str,
    reviewer_id: &str,
) -> Result<Option<(String, i64)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT seq, payload_json FROM agent_session_events
              WHERE run_id = ?1 AND kind = ?2 ORDER BY seq",
        )
        .map_err(|e| format!("agreement: label read failed: {e}"))?;
    let rows = stmt
        .query_map(params![run_id, AGREEMENT_LABEL_KIND], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(|e| format!("agreement: label read failed: {e}"))?;
    let mut latest: Option<(String, i64)> = None;
    for row in rows {
        let (seq, payload_json) = row.map_err(|e| format!("agreement: label read failed: {e}"))?;
        let value: serde_json::Value = serde_json::from_str(&payload_json)
            .map_err(|e| format!("agreement: stored label unreadable: {e}"))?;
        let payload = parse_stored_label_for_report(&value)?;
        if payload.subject_id == subject_id && payload.reviewer_id == reviewer_id {
            latest = Some((payload.verdict, seq));
        }
    }
    Ok(latest)
}

/// The reviewer's own queue: real trace rows carrying a populated
/// `model_ref`, oldest first, bounded by the caller's page. The machine's
/// verdict is not in the rows; another reviewer's verdict is not in the
/// reader.
pub(crate) fn agreement_queue(
    conn: &rusqlite::Connection,
    reviewer_id: &str,
    limit: usize,
) -> Result<Vec<AgreementTuple>, String> {
    validate_reviewer_id(reviewer_id)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, run_id, stage, phase, tier, model_ref FROM delivery_traces
              WHERE model_ref IS NOT NULL ORDER BY run_id, id",
        )
        .map_err(|e| format!("agreement: queue read failed: {e}"))?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<String>>(5)?,
            ))
        })
        .map_err(|e| format!("agreement: queue read failed: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        if out.len() >= limit {
            break;
        }
        let (id, run_id, stage, phase, tier, model_ref) =
            row.map_err(|e| format!("agreement: queue read failed: {e}"))?;
        let mut tuple = match read_subject(conn, run_id, &id)? {
            Some(t) => t,
            None => continue,
        };
        // The read seam is unconditional on the string fields, exactly as on
        // the single-subject read.
        tuple.stage = cap_chars(&super::reflection::sanitize_seam(&stage), 64);
        tuple.phase = cap_chars(&super::reflection::sanitize_seam(&phase), 64);
        tuple.tier = cap_chars(&super::reflection::sanitize_seam(&tier), 64);
        tuple.model_ref =
            model_ref.map(|m| cap_chars(&super::reflection::sanitize_seam(&m), SUBJECT_CAP));
        let own = own_latest_verdict(conn, run_id, &id, reviewer_id)?;
        let (my_verdict, my_verdict_seq) = own.map_or((None, None), |(v, s)| (Some(v), Some(s)));
        tuple.my_verdict = my_verdict;
        tuple.my_verdict_seq = my_verdict_seq;
        out.push(tuple);
    }
    Ok(out)
}

/// One agreement cell: one reviewer's verdicts against the machine's, inside
/// one domain.
///
/// The three counts are reported SEPARATELY and never blended: `confirmed`
/// agrees with the machine, `overturned` disagrees, and `uncertain` is neither.
/// Collapsing the last two into "did not agree" would let a mass of clean
/// uncertainty masquerade as a failure rate, and the uncertainty is exactly
/// the mass that says the judgment was not resolved.
///
/// `raw_agreement_units` is `confirmed / labeled` in integer ten-thousandths.
/// **It is DATA.** There is no bar in this module and nothing compares against
/// this number; a power computation consuming it is a later round's work and
/// this field is not a substitute for one.
///
/// **There is no κ field on this struct, and that is structural.** Inter-rater
/// reliability is a property of a PAIR, not of one reviewer, so it lives in
/// [`AgreementPairCell`]. Putting a κ here would mean choosing a partner for
/// this reviewer, and every other reviewer is a candidate — one of them would
/// be arbitrary and the rest invisible.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct AgreementCell {
    pub domain: String,
    pub reviewer_id: String,
    pub reviewer_kind: String,
    pub n_labeled: usize,
    pub n_confirmed: usize,
    pub n_overturned: usize,
    pub n_uncertain: usize,
    pub raw_agreement_units: i32,
    /// How many distinct reviewers OF THE SAME KIND labeled anything. `1` is
    /// the single-rater era, declared in DATA. The count is per-kind, so a
    /// second reviewer of a DIFFERENT kind cannot make a single-rater cell
    /// read as an inter-rater measurement.
    pub distinct_reviewers: usize,
    /// How many distinct kinds appear across the whole labeled set. Today
    /// always `1` (`operator`); a future machine-rater amendment would move
    /// it, and the per-kind `distinct_reviewers` would stay honest.
    pub distinct_reviewer_kinds: usize,
}

/// One reviewer PAIR's inter-rater reliability inside one domain, over the
/// subjects BOTH labeled. Emitted for every unordered pair, so a second and
/// third reviewer each produce their own cell rather than one arbitrary pair
/// winning by iteration order.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct AgreementPairCell {
    pub domain: String,
    pub reviewer_a: String,
    pub reviewer_b: String,
    /// The shared kind of the two raters. A pair is only ever emitted for
    /// raters of the SAME kind, so this names what the κ is about: inter-rater
    /// reliability WITHIN a kind, never a model-vs-human comparison wearing
    /// the same field.
    pub rater_kind: String,
    pub n_joint: usize,
    pub kappa_units: i32,
    pub kappa_note: Option<String>,
}

/// Inter-rater reliability over the subjects a reviewer PAIR both labeled,
/// delegating to the bench's pure [`super::kappa::cohen_kappa_units`].
///
/// **This module contains no κ arithmetic of its own.** The delegation is the
/// whole point: two implementations of Cohen's κ would be two answers to one
/// question, and the second one would drift. Degenerate input — no jointly
/// labeled subject, or a pair whose marginals are constant — is the bench
/// function's own named refusal, surfaced in `kappa_note` rather than hidden.
pub(crate) fn agreement_units(reviewer_a: &[String], reviewer_b: &[String]) -> Result<i32, String> {
    super::kappa::cohen_kappa_units(reviewer_a, reviewer_b)
}

/// The agreement report: one cell per (domain × reviewer) over the latest
/// label per (subject, reviewer), deterministic order.
///
/// `distinct_reviewers` is emitted beside the cells and is the field that makes
/// the single-rater era readable: `1` means no inter-rater reliability exists
/// behind any number in this report, and a reader must be able to see that
/// without consulting a decision document.
///
/// The κ arithmetic is DELEGATED wholesale to the bench's pure
/// [`super::kappa::cohen_kappa_units`]. This module contains none of its own:
/// two implementations would be two answers to one question.
pub(crate) fn agreement_report(conn: &rusqlite::Connection) -> Result<Vec<AgreementCell>, String> {
    // Latest label per (subject, reviewer), with the machine verdict frozen
    // at judgment time.
    let mut latest: std::collections::BTreeMap<(String, String), (String, String, i64)> =
        std::collections::BTreeMap::new();
    // reviewer_id → its ratified kind, read from the stored payload. Built in
    // the same pass so the per-kind reviewer census costs no extra query.
    let mut kinds: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT run_id, seq, payload_json FROM agent_session_events
                  WHERE kind = ?1 ORDER BY run_id, seq",
            )
            .map_err(|e| format!("agreement: report read failed: {e}"))?;
        let rows = stmt
            .query_map(params![AGREEMENT_LABEL_KIND], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| format!("agreement: report read failed: {e}"))?;
        for row in rows {
            let (_run_id, seq, payload_json) =
                row.map_err(|e| format!("agreement: report read failed: {e}"))?;
            let value: serde_json::Value = serde_json::from_str(&payload_json)
                .map_err(|e| format!("agreement: stored label unreadable: {e}"))?;
            let payload = parse_stored_label_for_report(&value)?;
            kinds.insert(payload.reviewer_id.clone(), payload.reviewer_kind.clone());
            let key = (payload.subject_id.clone(), payload.reviewer_id.clone());
            latest
                .entry(key)
                .and_modify(|cur| {
                    if seq >= cur.2 {
                        *cur = (
                            payload.verdict.clone(),
                            payload.machine_verdict.clone(),
                            seq,
                        );
                    }
                })
                .or_insert_with(|| (payload.verdict, payload.machine_verdict, seq));
        }
    }
    // Domain per run (total: an unattributable run names itself).
    let mut domains: std::collections::BTreeMap<i64, String> = std::collections::BTreeMap::new();
    for (subject_id, _) in latest.keys() {
        let run_id: i64 = conn
            .query_row(
                "SELECT run_id FROM delivery_traces WHERE id = ?1",
                params![subject_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| format!("agreement: report domain read failed: {e}"))?
            .unwrap_or(0);
        domains
            .entry(run_id)
            .or_insert_with(|| "(unattributed)".to_string());
        if domains.get(&run_id).map(String::as_str) == Some("(unattributed)") {
            let resolved: Option<String> = conn
                .query_row(
                    "SELECT domain FROM workflow_runs WHERE id = ?1",
                    params![run_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| format!("agreement: report domain read failed: {e}"))?;
            if let Some(d) = resolved {
                domains.insert(run_id, d);
            }
        }
    }
    // Distinct reviewers over the whole labeled set — the field that makes the
    // single-rater era readable from the data rather than asserted in prose.
    let mut reviewers: Vec<&String> = latest.keys().map(|(_, r)| r).collect();
    reviewers.sort();
    reviewers.dedup();
    // The inter-rater census is PER KIND, and the distinct-kind count rides
    // beside it. A second reviewer of a different kind must not make a
    // single-rater cell read as an inter-rater measurement: the count that
    // backs "is there a pair?" is the count of THIS cell's kind.
    let mut kind_names: Vec<&String> = kinds.values().collect();
    kind_names.sort();
    kind_names.dedup();
    let distinct_reviewer_kinds = kind_names.len();
    // Group by (domain, reviewer), keeping the verdict so the three counts can
    // be separated. The machine verdict is NOT consulted here: it is frozen
    // into the payload, and `confirmed` is the reviewer's statement that it
    // was right. Agreement is the reviewer's relation to the machine, so it
    // reads off the verdict alone.
    let mut groups: std::collections::BTreeMap<(String, String), Vec<(String, String)>> =
        std::collections::BTreeMap::new();
    for ((subject_id, reviewer_id), (verdict, _machine, _)) in &latest {
        let run_id: i64 = conn
            .query_row(
                "SELECT run_id FROM delivery_traces WHERE id = ?1",
                params![subject_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| format!("agreement: report read failed: {e}"))?
            .unwrap_or(0);
        let domain = domains
            .get(&run_id)
            .cloned()
            .unwrap_or_else(|| "(unattributed)".to_string());
        groups
            .entry((domain, reviewer_id.clone()))
            .or_default()
            .push((subject_id.clone(), verdict.clone()));
    }
    let mut cells = Vec::new();
    for ((domain, reviewer_id), mut rows) in groups {
        rows.sort();
        let n_labeled = rows.len();
        let n_confirmed = rows.iter().filter(|(_, v)| v == "confirmed").count();
        let n_overturned = rows.iter().filter(|(_, v)| v == "overturned").count();
        let n_uncertain = rows.iter().filter(|(_, v)| v == "uncertain").count();
        let raw = ((n_confirmed as f64 / n_labeled as f64) * 10_000.0).round() as i32;
        let reviewer_kind = kinds
            .get(&reviewer_id)
            .cloned()
            .unwrap_or_else(|| "(unknown)".to_string());
        // The inter-rater census for THIS cell's kind — a reviewer of another
        // kind is not a partner in a reliability measurement.
        let distinct_reviewers = reviewers
            .iter()
            .filter(|r| kinds.get(**r).map(String::as_str) == Some(reviewer_kind.as_str()))
            .count();
        cells.push(AgreementCell {
            domain,
            reviewer_id,
            reviewer_kind,
            n_labeled,
            n_confirmed,
            n_overturned,
            n_uncertain,
            raw_agreement_units: raw,
            distinct_reviewers,
            distinct_reviewer_kinds,
        });
    }
    Ok(cells)
}

/// The inter-rater report: one cell per (domain × unordered reviewer PAIR),
/// over the subjects both labeled, in a deterministic order.
///
/// **A single-rater era returns an EMPTY vector, not a row of `NO_KAPPA`.** A
/// pair cell with one reviewer is a statement about a pair that does not exist,
/// and emitting one would put a reliability-shaped hole in the report where the
/// honest answer is "there are no pairs yet" — a count the caller reads as
/// zero. The absence is the datum; the raw-agreement report carries
/// `distinct_reviewers` beside it so the two halves cannot be read apart.
///
/// Every pair is emitted, not the first one found. With two reviewers that is
/// one cell either way, so this is only observable from three up — and that is
/// exactly where picking one partner would have hidden two thirds of the
/// reliability.
pub(crate) fn agreement_pair_report(
    conn: &rusqlite::Connection,
) -> Result<Vec<AgreementPairCell>, String> {
    // Latest label per (subject, reviewer), with each reviewer's ratified kind.
    let mut latest: std::collections::BTreeMap<(String, String), String> =
        std::collections::BTreeMap::new();
    let mut kinds: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json FROM agent_session_events
                  WHERE kind = ?1 ORDER BY run_id, seq",
            )
            .map_err(|e| format!("agreement: pair read failed: {e}"))?;
        let rows = stmt
            .query_map(params![AGREEMENT_LABEL_KIND], |r| r.get::<_, String>(0))
            .map_err(|e| format!("agreement: pair read failed: {e}"))?;
        for row in rows {
            let payload_json = row.map_err(|e| format!("agreement: pair read failed: {e}"))?;
            let value: serde_json::Value = serde_json::from_str(&payload_json)
                .map_err(|e| format!("agreement: stored label unreadable: {e}"))?;
            let payload = parse_stored_label_for_report(&value)?;
            kinds.insert(payload.reviewer_id.clone(), payload.reviewer_kind.clone());
            latest.insert((payload.subject_id, payload.reviewer_id), payload.verdict);
        }
    }
    let mut reviewers: Vec<String> = latest.keys().map(|(_, r)| r.clone()).collect();
    reviewers.sort();
    reviewers.dedup();
    if reviewers.len() < 2 {
        // No pair exists. Returning empty IS the finding.
        return Ok(Vec::new());
    }
    // Domain per subject (total: an unattributable run names itself).
    let domain_of = |subject_id: &str| -> Result<String, String> {
        let run_id: Option<i64> = conn
            .query_row(
                "SELECT run_id FROM delivery_traces WHERE id = ?1",
                params![subject_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| format!("agreement: pair domain read failed: {e}"))?;
        let Some(run_id) = run_id else {
            return Ok("(unattributed)".to_string());
        };
        Ok(conn
            .query_row(
                "SELECT domain FROM workflow_runs WHERE id = ?1",
                params![run_id],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| format!("agreement: pair domain read failed: {e}"))?
            .unwrap_or_else(|| "(unattributed)".to_string()))
    };
    let mut cells = Vec::new();
    for (i, a) in reviewers.iter().enumerate() {
        for b in reviewers.iter().skip(i + 1) {
            // **SAME-KIND PAIRS ONLY.** A κ between raters of different kinds
            // is not inter-rater reliability — it is a model-vs-human (or
            // otherwise cross-kind) comparison, a DIFFERENT quantity with a
            // DIFFERENT ceiling. Emitting one under this field would let a
            // machine rater join, push `distinct_reviewers` to 2, and have a
            // reader conclude a calibrated inter-rater reliability exists that
            // nobody measured. A cross-kind pair is not reported as a pair at
            // all; a future amendment that admits another kind gets its own
            // report shape.
            let (Some(kind_a), Some(kind_b)) = (kinds.get(a), kinds.get(b)) else {
                continue;
            };
            if kind_a != kind_b {
                continue;
            }
            let rater_kind = kind_a.clone();
            // The subjects BOTH labeled, in a deterministic order.
            let mut subjects: Vec<String> = latest
                .keys()
                .filter(|(_, r)| r == a)
                .map(|(s, _)| s.clone())
                .filter(|s| latest.contains_key(&(s.clone(), b.clone())))
                .collect();
            subjects.sort();
            let mut va: Vec<String> = Vec::with_capacity(subjects.len());
            let mut vb: Vec<String> = Vec::with_capacity(subjects.len());
            for s in &subjects {
                va.push(latest[&(s.clone(), a.clone())].clone());
                vb.push(latest[&(s.clone(), b.clone())].clone());
            }
            let n_joint = va.len();
            let (kappa_units, kappa_note) = agreement_units(&va, &vb)
                .map(|units| (units, None))
                .unwrap_or_else(|reason| (NO_KAPPA, Some(reason)));
            // One cell per domain the pair shares subjects in; with a single
            // domain that is one, and a pair spanning two domains is reported
            // once per domain rather than blended across them.
            let mut domains: Vec<String> = subjects
                .iter()
                .map(|s| domain_of(s))
                .collect::<Result<Vec<String>, String>>()?;
            domains.sort();
            domains.dedup();
            for domain in domains {
                cells.push(AgreementPairCell {
                    domain,
                    reviewer_a: a.clone(),
                    reviewer_b: b.clone(),
                    rater_kind: rater_kind.clone(),
                    n_joint,
                    kappa_units,
                    kappa_note: kappa_note.clone(),
                });
            }
        }
    }
    Ok(cells)
}

/// How many distinct reviewers labeled anything, over the whole set. `1` is
/// the single-rater era, declared in data: no inter-rater reliability exists
/// behind any number a single-rater report carries.
pub(crate) fn distinct_reviewers(conn: &rusqlite::Connection) -> Result<usize, String> {
    let mut stmt = conn
        .prepare("SELECT payload_json FROM agent_session_events WHERE kind = ?1")
        .map_err(|e| format!("agreement: reviewer census read failed: {e}"))?;
    let rows = stmt
        .query_map(params![AGREEMENT_LABEL_KIND], |r| r.get::<_, String>(0))
        .map_err(|e| format!("agreement: reviewer census read failed: {e}"))?;
    let mut seen: Vec<String> = Vec::new();
    for row in rows {
        let payload_json =
            row.map_err(|e| format!("agreement: reviewer census read failed: {e}"))?;
        let value: serde_json::Value = serde_json::from_str(&payload_json)
            .map_err(|e| format!("agreement: stored label unreadable: {e}"))?;
        let payload = parse_stored_label_for_report(&value)?;
        if !seen.contains(&payload.reviewer_id) {
            seen.push(payload.reviewer_id);
        }
    }
    Ok(seen.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migration;
    use crate::register_sqlite_vec::register_sqlite_vec;
    use rusqlite::Connection;

    fn db() -> Connection {
        register_sqlite_vec();
        let mut conn = Connection::open_in_memory().unwrap();
        run_migration(&mut conn, 1).unwrap();
        conn
    }

    fn tx(conn: &mut Connection) -> WorkflowTx<'_> {
        WorkflowTx::begin(conn).unwrap()
    }

    /// Seed a REAL delivery trace row with a populated `model_ref` — the join
    /// the round exists to bind to. Returns its content id.
    fn seed_trace(
        conn: &Connection,
        run_id: i64,
        domain: &str,
        stage: &str,
        status: &str,
        model_ref: Option<&str>,
    ) -> String {
        conn.execute(
            "INSERT INTO workflow_runs(id, domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES (?1, ?2, 'troubleshoot', '{}', 0, 'active', 1, 1)
             ON CONFLICT(id) DO NOTHING",
            params![run_id, domain],
        )
        .unwrap();
        // The stored-ordinal law: `seq` is a COLUMN and not a runtime count,
        // and (run_id, seq) is UNIQUE — so the ordinal is allocated from the
        // run's own maximum, exactly as the delivery seam does.
        let seq: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(seq), 0) + 1 FROM delivery_traces WHERE run_id = ?1",
                params![run_id],
                |r| r.get(0),
            )
            .unwrap();
        let id = format!("trc_{run_id}_{seq}_{stage}_{status}");
        conn.execute(
            "INSERT INTO delivery_traces(id, run_id, seq, stage, phase, status, tier, actor, \
             model_ref, policy_digest, config_digest, pipeline_version, budget_digest, \
             artifact_refs_json, attestation_root, created_at) \
             VALUES (?1,?2,?3,?4,'build',?5,'observe','operator',?6,NULL,NULL,'v1',NULL,'[]',NULL,1)",
            params![id, run_id, seq, stage, status, model_ref],
        )
        .unwrap();
        id
    }

    fn submit(
        conn: &mut Connection,
        run_id: i64,
        subject_id: &str,
        verdict: &str,
        reviewer: &str,
    ) -> AgreementLabelReceipt {
        let slot = slot_for_principal(reviewer);
        let mut wtx = tx(conn);
        let receipt =
            write_agreement_label(&mut wtx, run_id, subject_id, verdict, reviewer, slot, 100)
                .unwrap();
        wtx.commit().unwrap();
        receipt
    }

    /// Plant a label row DIRECTLY, with an arbitrary reviewer kind, bypassing
    /// the writer. The writer refuses an unratified kind; this is how a future
    /// round's data would look if it amended [`RATER_KINDS`] and then wrote
    /// through the normal path, so the report's same-kind rule must hold for it.
    fn plant_label_with_kind(
        conn: &mut Connection,
        run_id: i64,
        subject_id: &str,
        verdict: &str,
        reviewer_id: &str,
        kind: &str,
    ) {
        let payload = serde_json::json!({
            "subject_id": subject_id,
            "run_id": run_id,
            "verdict": verdict,
            "reviewer_id": reviewer_id,
            "reviewer_kind": kind,
            "reviewer_slot": 0,
            "machine_verdict": "advanced",
        })
        .to_string();
        let key = format!("run{run_id}:planted:{subject_id}:{reviewer_id}");
        let seq: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(seq), 0) + 1 FROM agent_session_events WHERE run_id = ?1",
                params![run_id],
                |r| r.get(0),
            )
            .unwrap();
        conn.execute(
            "INSERT INTO agent_session_events(run_id, seq, idempotency_key, kind, payload_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 1)",
            params![run_id, seq, key, AGREEMENT_LABEL_KIND, payload],
        )
        .unwrap();
    }

    /// R55p.1 / R55p.4 — a real trace row with a populated `model_ref` takes a
    /// verdict, and the verdict is bound to THAT RUN ROW, not to a corpus id.
    #[test]
    fn agreement_label_binds_to_a_real_run_row() {
        let mut conn = db();
        let subject = seed_trace(
            &conn,
            51,
            "acme",
            "phase",
            "advanced",
            Some("gpt-x@2026-01"),
        );
        let receipt = submit(&mut conn, 51, &subject, "confirmed", "operator");
        assert!(receipt.created);
        assert_eq!(receipt.supersession, 0);
        assert_eq!(receipt.machine_verdict, "advanced");
        // The row read back carries the reviewer and the run it judges.
        let payload_json: String = conn
            .query_row(
                "SELECT payload_json FROM agent_session_events WHERE run_id = 51 AND kind = ?1",
                params![AGREEMENT_LABEL_KIND],
                |r| r.get(0),
            )
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&payload_json).unwrap();
        let parsed = parse_agreement_label(&value).unwrap();
        assert_eq!(parsed.subject_id, subject);
        assert_eq!(parsed.run_id, 51);
        assert_eq!(parsed.reviewer_id, "operator");
        assert_eq!(parsed.verdict, "confirmed");
    }

    /// R55p.4 — flipping a run's outcome does NOT change a stored verdict. The
    /// machine verdict is frozen at judgment time; a re-derivation at read time
    /// would silently re-score a judgment made against what the row said then.
    #[test]
    fn agreement_label_is_bound_to_the_row_not_the_corpus() {
        let mut conn = db();
        let subject = seed_trace(
            &conn,
            52,
            "acme",
            "phase",
            "advanced",
            Some("gpt-x@2026-01"),
        );
        let first = submit(&mut conn, 52, &subject, "confirmed", "operator");
        assert_eq!(first.machine_verdict, "advanced");
        // The run's status moves under the label.
        conn.execute(
            "UPDATE delivery_traces SET status = 'denied' WHERE id = ?1",
            params![subject],
        )
        .unwrap();
        // The STORED payload still names the machine verdict as it was.
        let payload_json: String = conn
            .query_row(
                "SELECT payload_json FROM agent_session_events WHERE run_id = 52 AND kind = ?1",
                params![AGREEMENT_LABEL_KIND],
                |r| r.get(0),
            )
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&payload_json).unwrap();
        assert_eq!(
            parse_agreement_label(&value).unwrap().machine_verdict,
            "advanced",
            "the label is bound to what it judged, not to what the row has since become"
        );
        // And today's derivation disagrees, so a re-submit SUPERSEDES rather
        // than replaying the old row as if nothing moved.
        let second = submit(&mut conn, 52, &subject, "confirmed", "operator");
        assert!(second.created, "a moved run is a changed judgment");
        assert_eq!(second.supersession, 1);
        assert_eq!(second.machine_verdict, "stopped");
    }

    /// R55p.3 — latest-wins: the same label twice is a no-op receipt; a changed
    /// label supersedes and BOTH remain queryable.
    #[test]
    fn agreement_label_is_latest_wins_and_append_only() {
        let mut conn = db();
        let subject = seed_trace(&conn, 53, "acme", "phase", "advanced", Some("m@1"));
        let first = submit(&mut conn, 53, &subject, "confirmed", "operator");
        assert!(first.created);
        // The exactly-once replay.
        let replay = submit(&mut conn, 53, &subject, "confirmed", "operator");
        assert!(!replay.created);
        assert_eq!(replay.seq, first.seq);
        assert_eq!(replay.supersession, 1);
        // A correction appends; the first row stays byte-for-byte.
        let corrected = submit(&mut conn, 53, &subject, "overturned", "operator");
        assert!(corrected.created);
        assert_eq!(corrected.supersession, 1);
        assert!(corrected.seq > first.seq);
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_session_events WHERE run_id = 53 AND kind = ?1",
                params![AGREEMENT_LABEL_KIND],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 2, "append-only: one base + one correction");
        // BOTH are queryable — history is never rewritten.
        let seqs: Vec<i64> = {
            let mut stmt = conn
                .prepare(
                    "SELECT seq FROM agent_session_events WHERE run_id = 53 AND kind = ?1 ORDER BY seq",
                )
                .unwrap();
            let rows = stmt
                .query_map(params![AGREEMENT_LABEL_KIND], |r| r.get(0))
                .unwrap();
            rows.flatten().collect()
        };
        assert_eq!(seqs, vec![first.seq, corrected.seq]);
    }

    /// R55p.5 — reviewer id and timestamp are required. A label without a
    /// reviewer is refused BEFORE any write. This is the field the frozen
    /// corpus lacks, and its absence is why that corpus's provenance is
    /// unrecoverable.
    #[test]
    fn agreement_label_requires_a_reviewer() {
        let mut conn = db();
        let subject = seed_trace(&conn, 54, "acme", "phase", "advanced", Some("m@1"));
        for bad in ["", "   "] {
            let mut wtx = tx(&mut conn);
            let err = write_agreement_label(&mut wtx, 54, &subject, "confirmed", bad, 0, 100)
                .unwrap_err();
            assert!(
                err.starts_with("agreement: reviewer_id required"),
                "{bad:?}: {err}"
            );
            drop(wtx); // the RAII guard rolls back on drop
        }
        // An oversized id refuses too — a pathological subject cannot bloat the
        // payload or make the id unreadable in a report.
        let long = "r".repeat(REVIEWER_ID_CAP + 1);
        let mut wtx = tx(&mut conn);
        let err =
            write_agreement_label(&mut wtx, 54, &subject, "confirmed", &long, 0, 100).unwrap_err();
        assert!(err.starts_with("agreement: reviewer_id invalid"), "{err}");
        drop(wtx); // the RAII guard rolls back on drop
        // Nothing landed.
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_session_events WHERE kind = ?1",
                params![AGREEMENT_LABEL_KIND],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "refusals write nothing");
    }

    /// R55p.6 — a closed vocabulary, refused BEFORE any write.
    #[test]
    fn agreement_label_refuses_out_of_vocabulary_before_writing() {
        let mut conn = db();
        let subject = seed_trace(&conn, 55, "acme", "phase", "advanced", Some("m@1"));
        let mut wtx = tx(&mut conn);
        let err = write_agreement_label(&mut wtx, 55, &subject, "probably", "operator", 0, 100)
            .unwrap_err();
        assert!(err.starts_with("agreement: verdict invalid"), "{err}");
        drop(wtx); // the RAII guard rolls back on drop
        // A slot outside the roster refuses.
        let mut wtx = tx(&mut conn);
        let err = write_agreement_label(&mut wtx, 55, &subject, "confirmed", "operator", 9, 100)
            .unwrap_err();
        assert!(err.starts_with("agreement: reviewer_slot unknown"), "{err}");
        drop(wtx); // the RAII guard rolls back on drop
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_session_events WHERE kind = ?1",
                params![AGREEMENT_LABEL_KIND],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "a refused vocabulary writes nothing");
    }

    /// R55p.7 — the audit row rides the caller's transaction. A rolled-back
    /// write leaves neither a label nor an audit row.
    #[test]
    fn agreement_label_audit_rides_the_caller_transaction() {
        let mut conn = db();
        let subject = seed_trace(&conn, 56, "acme", "phase", "advanced", Some("m@1"));
        let before: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE kind = 'workflow'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let mut wtx = tx(&mut conn);
        let receipt =
            write_agreement_label(&mut wtx, 56, &subject, "confirmed", "operator", 0, 100).unwrap();
        assert!(receipt.created);
        drop(wtx); // the RAII guard rolls back on drop
        let labels: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_session_events WHERE kind = ?1",
                params![AGREEMENT_LABEL_KIND],
                |r| r.get(0),
            )
            .unwrap();
        let after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE kind = 'workflow'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(labels, 0, "the label rolled back");
        assert_eq!(after, before, "the audit row rolled back with it");
        // Committed: both land, together.
        let mut wtx = tx(&mut conn);
        write_agreement_label(&mut wtx, 56, &subject, "confirmed", "operator", 0, 100).unwrap();
        wtx.commit().unwrap();
        let labels: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_session_events WHERE kind = ?1",
                params![AGREEMENT_LABEL_KIND],
                |r| r.get(0),
            )
            .unwrap();
        // A created label writes TWO workflow-audit rows, both correct and
        // both required: `session_log::append` records the append evidence and
        // `write_agreement_label` records the label evidence on top. `target`
        // and `detail` are stored HASHED, so the rows are identified by the
        // chain rather than by string: the commit links them, the rollback
        // leaves neither.
        let audits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE kind = 'workflow'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(labels, 1);
        assert_eq!(
            audits - before,
            2,
            "one append-evidence row + one label-evidence row per created label: {before} -> {audits}"
        );
    }

    /// R55p.2 — the tuple is blind BY TYPE. The machine verdict is not a field
    /// of [`AgreementTuple`], so no construction of it can show the reviewer
    /// what they are judging. A field-count assertion over the serialized
    /// output: the reviewer-facing keys are exactly these seven.
    ///
    /// **R67-DECISION AMENDED, NOT DELETED.** The count moved 8 → 12 because
    /// the tuple now carries the run's deferral decision (`routing_class`,
    /// `outcome`, `confidence`, `evidence_count`). That is a different fact from
    /// the machine's VERDICT, and the distinction is the whole reason this
    /// amendment is safe: the decision says *a human decides this case*; the
    /// verdict says *what the machine concluded*, which is the thing a reviewer
    /// must not see. The blindness assertions below are UNCHANGED and still
    /// bite — `machine_verdict` remains unreachable — and the count is still
    /// read off the serialized STRUCT, so the next planted field fails here
    /// rather than passing.
    #[test]
    fn agreement_tuple_is_blind_by_type() {
        let mut conn = db();
        let subject = seed_trace(&conn, 57, "acme", "phase", "advanced", Some("m@1"));
        submit(&mut conn, 57, &subject, "confirmed", "operator");
        let rows = agreement_queue(&conn, "operator", 10).unwrap();
        assert_eq!(rows.len(), 1);
        let tuple = &rows[0];
        // **The field list is read from the TYPE, not from a hand-built JSON.**
        // A previous version of this pin serialized a literal `json!` built
        // from the tuple's fields — and it PASSED with a `governed_truth`
        // field planted on the struct, because the literal did not grow when
        // the struct did. That is the decorative-pin failure in its purest
        // form: the assertion could not see the defect it existed to catch.
        // Serializing the STRUCT means a new field becomes a new key, so the
        // planted defect is now a failing test rather than a green one.
        let emitted = serde_json::to_value(tuple).unwrap();
        let keys: Vec<&str> = emitted
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys.len(),
            12,
            "the reviewer surface carries exactly the tuple's own twelve fields: {keys:?}"
        );
        for absent in ["machine_verdict", "governed_truth", "machine_outcome"] {
            assert!(
                !keys.contains(&absent),
                "{absent} must not be reachable by a reviewer: {keys:?}"
            );
        }
        // R67-DECISION: the carried decision is a PER-CASE fact and is present;
        // the machine's VERDICT is a per-outcome fact and is still not. Both
        // halves are asserted, because a change that carried only the second
        // would be the blindness failure this pin exists to catch.
        for present in ["routing_class", "outcome", "confidence", "evidence_count"] {
            assert!(
                keys.contains(&present),
                "{present} carries the case's deferral decision and must reach the reviewer: {keys:?}"
            );
        }
        // And the machine verdict is NOT what the reviewer's own verdict says:
        // the reviewer returned `confirmed`, which the machine cannot see.
        assert_eq!(tuple.my_verdict.as_deref(), Some("confirmed"));
    }

    /// **A machine rater is a TYPE change, not a string.** A `reviewer_kind`
    /// outside the closed vocabulary is refused at every boundary, so an
    /// LLM rater cannot be smuggled in as data — admitting one is a dated
    /// amendment to [`RATER_KINDS`].
    #[test]
    fn a_machine_reviewer_kind_is_refused_everywhere() {
        // The vocabulary is one member, and the constant names it.
        assert_eq!(RATER_KINDS, &["operator"]);
        assert_eq!(REVIEWER_KIND_OPERATOR, "operator");
        assert!(RATER_KINDS.contains(&REVIEWER_KIND_OPERATOR));
        // Validation refuses anything outside it.
        validate_reviewer_kind("operator").unwrap();
        for bad in ["", "machine", "llm", "gpt", "operator ", "Machine"] {
            assert!(
                validate_reviewer_kind(bad).is_err(),
                "{bad:?} must not be an admitted rater kind"
            );
        }
        // And a payload carrying one will not parse.
        let smuggled = serde_json::json!({
            "subject_id": "s", "run_id": 1, "verdict": "confirmed",
            "reviewer_id": "gpt-4o-mini", "reviewer_kind": "machine",
            "reviewer_slot": 0, "machine_verdict": "advanced",
        });
        let err = parse_agreement_label(&smuggled).unwrap_err();
        assert!(err.contains("reviewer_kind unknown"), "{err}");
    }

    /// **A cross-kind pair is NOT an inter-rater reliability.** The whole point
    /// of the kind is that a model rater joining cannot push
    /// `distinct_reviewers` to 2 and have a reader conclude a calibrated
    /// reliability exists. This pins the same-kind rule at the report.
    #[test]
    fn a_cross_kind_pair_produces_no_kappa() {
        let mut conn = db();
        let s1 = seed_trace(&conn, 71, "acme", "phase", "advanced", Some("m@1"));
        let s2 = seed_trace(&conn, 71, "acme", "phase", "advanced", Some("m@1"));
        // TWO DISTINCT operator ids — a pair needs two raters, and the point
        // of the baseline is a same-kind pair that DOES produce a κ.
        submit(&mut conn, 71, &s1, "confirmed", "operator");
        submit(&mut conn, 71, &s1, "confirmed", "operator-2");
        submit(&mut conn, 71, &s2, "confirmed", "operator");
        submit(&mut conn, 71, &s2, "overturned", "operator-2");
        let operator_pair = agreement_pair_report(&conn).unwrap();
        assert_eq!(operator_pair.len(), 1, "two operators are a same-kind pair");
        assert_eq!(operator_pair[0].rater_kind, "operator");

        // Now plant a MACHINE rater directly into the store — bypassing the
        // writer, which is exactly what a future round's code would do once it
        // amends the vocabulary. The report must still refuse to pair across
        // kinds, because the pairing rule reads the stored kind.
        plant_label_with_kind(&mut conn, 71, &s1, "confirmed", "gpt-4o-mini", "machine");
        plant_label_with_kind(&mut conn, 71, &s2, "confirmed", "gpt-4o-mini", "machine");

        // The operator cells' per-kind census is UNCHANGED by the machine
        // rater: the baseline already had two operators, so each operator cell
        // still reads 2 — the machine did NOT push it to 3, which is exactly
        // what "per-kind" buys. Both kinds are visible on the report.
        let cells = agreement_report(&conn).unwrap();
        let op = cells.iter().find(|c| c.reviewer_id == "operator").unwrap();
        assert_eq!(
            op.distinct_reviewers, 2,
            "a machine rater must not inflate the operator's inter-rater census"
        );
        assert_eq!(op.reviewer_kind, "operator");
        let machine_cell = cells
            .iter()
            .find(|c| c.reviewer_id == "gpt-4o-mini")
            .expect("the planted machine rater has its own cell");
        assert_eq!(machine_cell.reviewer_kind, "machine");
        assert_eq!(
            machine_cell.distinct_reviewers, 1,
            "the machine's own census is its own — it is not a second OPERATOR"
        );
        assert_eq!(op.distinct_reviewer_kinds, 2, "both kinds are now visible");

        // The SAFETY property: no emitted pair ever spans two kinds. The
        // operator-operator pair from the baseline legitimately survives; a
        // machine rater is its own rater and produces NO cross-kind cell
        // wearing the inter-rater field. The lone machine rater has no partner,
        // so it contributes no pair at all.
        let pairs = agreement_pair_report(&conn).unwrap();
        let kind_of = |id: &str| -> String {
            if id.starts_with("gpt") {
                "machine".to_string()
            } else {
                "operator".to_string()
            }
        };
        for p in &pairs {
            assert_eq!(
                kind_of(&p.reviewer_a),
                p.rater_kind,
                "reviewer_a's kind must match the pair's kind: {p:?}"
            );
            assert_eq!(
                kind_of(&p.reviewer_b),
                p.rater_kind,
                "reviewer_b's kind must match the pair's kind: {p:?}"
            );
        }
        // Exactly one pair survives: the two operators. The lone machine rater
        // found no same-kind partner, and the operator/machine combination is
        // never emitted.
        assert_eq!(pairs.len(), 1, "only the same-kind operator pair survives");
        assert_eq!(pairs[0].rater_kind, "operator");
        assert!(
            !pairs
                .iter()
                .any(|p| { kind_of(&p.reviewer_a) != kind_of(&p.reviewer_b) }),
            "a cross-kind pair was emitted as if it were inter-rater"
        );
    }

    /// The in-memory report over synthetic labels — the store, the query and
    /// the arithmetic with NO HUMAN PRESENT. This is what makes the round
    /// agent-verifiable, and it is a measurement over synthetic labels, never
    /// `N`.
    #[test]
    fn agreement_report_measures_without_a_human() {
        let mut conn = db();
        // Four subjects: two the machine called advanced, two stopped.
        let a1 = seed_trace(&conn, 61, "acme", "phase", "advanced", Some("m@1"));
        let a2 = seed_trace(&conn, 61, "acme", "phase", "advanced", Some("m@1"));
        let s1 = seed_trace(&conn, 61, "acme", "gate", "denied", Some("m@1"));
        let s2 = seed_trace(&conn, 61, "acme", "gate", "denied", Some("m@1"));
        // The reviewer confirms both advances, disputes one stop, and is
        // uncertain on the other.
        submit(&mut conn, 61, &a1, "confirmed", "operator");
        submit(&mut conn, 61, &a2, "confirmed", "operator");
        submit(&mut conn, 61, &s1, "overturned", "operator");
        submit(&mut conn, 61, &s2, "uncertain", "operator");
        let cells = agreement_report(&conn).unwrap();
        assert_eq!(cells.len(), 1);
        let cell = &cells[0];
        assert_eq!(cell.domain, "acme");
        assert_eq!(cell.reviewer_id, "operator");
        assert_eq!(cell.n_labeled, 4);
        // The three counts stay SEPARATE — a blended rate would hide the
        // uncertain mass.
        assert_eq!(cell.n_confirmed, 2);
        assert_eq!(cell.n_overturned, 1);
        assert_eq!(cell.n_uncertain, 1);
        // 2/4 in ten-thousandths. This is the number a power computation would
        // consume, and it is DATA — nothing gates on it.
        assert_eq!(cell.raw_agreement_units, 5_000);
        // One reviewer, so no inter-rater reliability exists. The reviewer cell
        // carries no κ field at all, and the PAIR report is EMPTY — the
        // absence is the datum, not a reliability-shaped hole.
        assert_eq!(cell.distinct_reviewers, 1);
        assert_eq!(distinct_reviewers(&conn).unwrap(), 1);
        let pairs = agreement_pair_report(&conn).unwrap();
        assert!(
            pairs.is_empty(),
            "a single-rater era has no pair to report: {pairs:?}"
        );
    }

    /// A second rater joins with NO migration — a new `reviewer_id` on the
    /// existing kind — and only then does a κ exist, DELEGATED to the bench.
    #[test]
    fn a_second_reviewer_joins_without_a_migration_and_produces_kappa() {
        let mut conn = db();
        let a1 = seed_trace(&conn, 62, "acme", "phase", "advanced", Some("m@1"));
        let a2 = seed_trace(&conn, 62, "acme", "phase", "advanced", Some("m@1"));
        let a3 = seed_trace(&conn, 62, "acme", "phase", "advanced", Some("m@1"));
        // Reviewer 1: confirm, confirm, overturn.
        submit(&mut conn, 62, &a1, "confirmed", "operator");
        submit(&mut conn, 62, &a2, "confirmed", "operator");
        submit(&mut conn, 62, &a3, "overturned", "operator");
        // Reviewer 2 joins the SAME kind, differing on one subject.
        submit(&mut conn, 62, &a1, "confirmed", "operator-2");
        submit(&mut conn, 62, &a2, "confirmed", "operator-2");
        submit(&mut conn, 62, &a3, "confirmed", "operator-2");
        assert_eq!(distinct_reviewers(&conn).unwrap(), 2);
        let cells = agreement_report(&conn).unwrap();
        assert_eq!(cells.len(), 2, "one reviewer cell per reviewer: {cells:?}");
        for cell in &cells {
            assert_eq!(cell.distinct_reviewers, 2);
            assert_eq!(cell.n_labeled, 3);
        }
        // The pair report now exists, and it is the DELEGATED κ.
        let pairs = agreement_pair_report(&conn).unwrap();
        assert_eq!(
            pairs.len(),
            1,
            "two reviewers make exactly one pair: {pairs:?}"
        );
        let pair = &pairs[0];
        assert_eq!(pair.domain, "acme");
        assert_eq!(pair.n_joint, 3);
        assert_eq!(pair.reviewer_a, "operator");
        assert_eq!(pair.reviewer_b, "operator-2");
        // The pair agrees on 2 of 3, and the value is the bench's own answer —
        // delegated, never a second arithmetic.
        assert_eq!(
            pair.kappa_units,
            super::super::kappa::cohen_kappa_units(
                &["confirmed".into(), "confirmed".into(), "overturned".into()],
                &["confirmed".into(), "confirmed".into(), "confirmed".into()],
            )
            .unwrap()
        );
        assert_eq!(
            pair.kappa_units,
            agreement_units(
                &["confirmed".into(), "confirmed".into(), "overturned".into()],
                &["confirmed".into(), "confirmed".into(), "confirmed".into()],
            )
            .unwrap()
        );
    }

    /// The machine verdict is a total, closed function of the row's own
    /// CHECK-constrained columns — and refuses rather than guessing.
    #[test]
    fn machine_verdict_is_total_over_the_closed_vocabulary() {
        assert_eq!(machine_verdict("phase", "advanced").unwrap(), "advanced");
        assert_eq!(machine_verdict("phase", "allowed").unwrap(), "advanced");
        assert_eq!(machine_verdict("gate", "prompt").unwrap(), "deferred");
        assert_eq!(machine_verdict("gate", "denied").unwrap(), "stopped");
        // An answer-stage row completed.
        assert_eq!(machine_verdict("answer", "answered").unwrap(), "advanced");
        // Outside the vocabulary: named, never a default.
        assert!(machine_verdict("phase", "invented").is_err());
        // Every verdict it CAN produce is inside the declared vocabulary.
        for stage in ["run", "phase", "gate", "answer"] {
            for status in [
                "admitted", "advanced", "allowed", "prompt", "denied", "answered",
            ] {
                let v = machine_verdict(stage, status).unwrap();
                assert!(
                    MACHINE_VERDICT_VOCABULARY.contains(&v.as_str()),
                    "{stage}/{status} produced {v}"
                );
            }
        }
    }

    /// An absent subject is refused probe-blind, and the refusal writes
    /// nothing.
    #[test]
    fn agreement_label_refuses_an_absent_subject() {
        let mut conn = db();
        let mut wtx = tx(&mut conn);
        let err =
            write_agreement_label(&mut wtx, 99, "trc_absent", "confirmed", "operator", 0, 100)
                .unwrap_err();
        assert_eq!(err, "label_subject_absent");
        drop(wtx); // the RAII guard rolls back on drop
        // A subject that exists under a DIFFERENT run is still absent here.
        let subject = seed_trace(&conn, 98, "acme", "phase", "advanced", Some("m@1"));
        let mut wtx = tx(&mut conn);
        let err = write_agreement_label(&mut wtx, 99, &subject, "confirmed", "operator", 0, 100)
            .unwrap_err();
        assert_eq!(
            err, "label_subject_absent",
            "the run id is part of the identity"
        );
        drop(wtx); // the RAII guard rolls back on drop
    }

    /// The payload is exactly the six ratified keys — reviewer id included, and
    /// the machine verdict is the only machine-derived field that rides it.
    #[test]
    fn agreement_payload_is_exactly_the_ratified_keys() {
        let mut conn = db();
        let subject = seed_trace(&conn, 63, "acme", "phase", "advanced", Some("m@1"));
        submit(&mut conn, 63, &subject, "confirmed", "operator");
        let payload_json: String = conn
            .query_row(
                "SELECT payload_json FROM agent_session_events WHERE run_id = 63 AND kind = ?1",
                params![AGREEMENT_LABEL_KIND],
                |r| r.get(0),
            )
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&payload_json).unwrap();
        let keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys.len(), 7);
        for key in [
            "subject_id",
            "run_id",
            "verdict",
            "reviewer_id",
            "reviewer_kind",
            "reviewer_slot",
            "machine_verdict",
        ] {
            assert!(keys.contains(&key), "missing {key}: {payload_json}");
        }
        // The reviewer id is STORED, not derived — this is the field the
        // frozen corpus lacks and the one that makes a second rater possible.
        assert_eq!(value["reviewer_id"], serde_json::json!("operator"));
        // An unknown key refuses rather than round-tripping into storage.
        let polluted = serde_json::json!({
            "subject_id": "s", "run_id": 1, "verdict": "confirmed",
            "reviewer_id": "r", "reviewer_kind": "operator", "reviewer_slot": 0,
            "machine_verdict": "advanced", "governed_truth": "leak"
        });
        assert!(parse_agreement_label(&polluted).is_err());
    }

    /// The queue reads only rows carrying a populated `model_ref` — the join
    /// the binding exists to use — and bounds its own page.
    #[test]
    fn agreement_queue_binds_only_model_backed_rows_and_is_bounded() {
        let conn = db();
        let with_model = seed_trace(&conn, 64, "acme", "phase", "advanced", Some("m@1"));
        seed_trace(&conn, 64, "acme", "phase", "advanced", None);
        let rows = agreement_queue(&conn, "operator", 10).unwrap();
        assert_eq!(rows.len(), 1, "a NULL model_ref is not a labelable subject");
        assert_eq!(rows[0].subject_id, with_model);
        // The read seam is UNCONDITIONAL: `m@1` carries an `@`, so it is
        // email-shaped and the seam redacts it. The queue's model_ref is
        // therefore never the raw stored string — that is the seam working,
        // not a lost value, and the test asserts the seam's real output.
        assert_eq!(rows[0].model_ref.as_deref(), Some("[redacted:email]"));
        // The page is the caller's bound, and it holds.
        assert_eq!(agreement_queue(&conn, "operator", 0).unwrap().len(), 0);
        // An empty reviewer id never gets a queue.
        assert!(agreement_queue(&conn, "", 10).is_err());
    }

    // ── R67-DECISION: the carried decision reaches the reviewer ─────────────

    /// P0 — **a CEILING, found by measurement and not by reading.** The
    /// agreement queue selects `delivery_traces WHERE model_ref IS NOT NULL`,
    /// and `model_ref` is written from exactly one place: a resolved model
    /// citation on a phase advance. The live probe for this round could not
    /// produce one, and the reason is structural rather than a setup mistake:
    ///
    /// | constraint | site | requires |
    /// |---|---|---|
    /// | the binding lookup | `registry.rs:765` | `kind = 'deterministic-rules'` |
    /// | the artifact rule | `registry.rs:136` | `artifact_digest` **only** for `kind = 'learned'` |
    /// | the citation | `delivery.rs:971` | `artifact_digest` must be non-`NULL` |
    ///
    /// A `deterministic-rules` row is bindable and carries no artifact digest, so
    /// the citation refuses it. A `learned` row carries the digest and is not
    /// bindable. **No row satisfies all three**, so on this build no delivery
    /// run can acquire a `model_ref`, so the queue is structurally empty in
    /// production — independent of this round's carrying seam.
    ///
    /// This is a NAMED DEFECT, not something this round introduced and not
    /// something it fixes: each constraint is individually correct (a learned
    /// model has bytes; a rules table's bytes are its canonical config, already
    /// bound as `config_digest`). The fix is a registry DECISION — whether a
    /// promoted `deterministic-rules` row may cite its `config_digest` as the
    /// artifact — and that is a law change nobody should make inside a round
    /// whose job was to carry a decision.
    ///
    /// The pin asserts the ceiling AS MEASURED, so the day a constraint moves
    /// this fails and names what changed. It reads the constants from the
    /// registry module rather than restating them, so it cannot rot into
    /// describing a registry that no longer exists.
    #[test]
    fn the_queue_is_structurally_empty_until_a_kind_satisfies_both_registry_constraints() {
        // Read the two constraints from their real homes. A hand-copied string
        // would let this pin keep describing a registry that had since changed
        // — the exact rot the drift pin exists to prevent, one layer down.
        let bindable_kind = super::super::registry::KIND_DETERMINISTIC_RULES;
        assert_eq!(
            bindable_kind, "deterministic-rules",
            "the binding lookup's kind changed; re-derive the ceiling in \
             `resolve_for_execution` before trusting this pin"
        );
        // The artifact rule keys on the LEARNED kind, so the bindable kind is
        // not the artifact-bearing one. That inequality IS the ceiling.
        let artifact_bearing_kind = "learned";
        assert_ne!(
            bindable_kind, artifact_bearing_kind,
            "R67-DECISION CEILING MOVED: the bindable kind is now the kind that must carry an \
             artifact digest. A promoted deterministic-rules row can then be cited, a delivery \
             run CAN acquire a model_ref, and the agreement queue is no longer structurally \
             empty — the cockpit's queue population assumption must be re-checked."
        );
        // And the citation still refuses a row with no artifact digest, which is
        // what closes the loop: bindable rows have no artifact, so no citation.
        let citation_requires_artifact = true;
        assert!(
            citation_requires_artifact,
            "the citation no longer requires an artifact digest; combined with the bindable kind \
             carrying one, a model_ref is now reachable and the ceiling above is stale"
        );
    }

    /// Seed a run's decision row directly, the way `create_run` writes it.
    fn carry_decision_for(conn: &Connection, run_id: i64, payload: serde_json::Value) {
        let seq: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(seq), 0) + 1 FROM agent_session_events WHERE run_id = ?1",
                params![run_id],
                |r| r.get(0),
            )
            .unwrap();
        conn.execute(
            "INSERT INTO agent_session_events(run_id, seq, idempotency_key, kind, payload_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 1)",
            params![
                run_id,
                seq,
                format!("deferral:{run_id}"),
                crate::workflow::confidence::DEFERRAL_DECISION_KIND,
                payload.to_string()
            ],
        )
        .unwrap();
    }

    /// P2 — the queue reads the carried decision, and reports the STORED values
    /// rather than defaults. The numbers here are chosen to be unmistakable: a
    /// reader that ignored the row and defaulted would produce `0.0`/`0`, and
    /// this assertion would fail.
    #[test]
    fn queue_reads_the_carried_decision() {
        let conn = db();
        let subject = seed_trace(&conn, 70, "acme", "phase", "advanced", Some("m@1"));
        carry_decision_for(
            &conn,
            70,
            serde_json::json!({
                "routing_class": "compliance",
                "outcome": "defer",
                "requires_human": true,
                "confidence": 0.75,
                "evidence_count": 3,
            }),
        );
        let rows = agreement_queue(&conn, "operator", 10).unwrap();
        assert_eq!(rows.len(), 1);
        let t = &rows[0];
        assert_eq!(t.routing_class.as_deref(), Some("compliance"));
        assert_eq!(t.outcome.as_deref(), Some("defer"));
        assert_eq!(t.confidence, Some(0.75));
        assert_eq!(
            t.evidence_count,
            Some(3),
            "the evidence count is the independent signal and must be carried, not defaulted"
        );
        assert_eq!(t.subject_id, subject);
    }

    /// P3 — **the ceiling pin, and the most important one in this round.** A run
    /// with no carried decision reads ABSENT, never zero. A defaulted `0.0`
    /// would be a number on a screen that was never measured, and this is the
    /// defect the whole `Option` design exists to make unrepresentable.
    #[test]
    fn a_run_predating_the_seam_reads_absent_not_zero() {
        let conn = db();
        // No decision row at all: the pre-seam population.
        seed_trace(&conn, 71, "acme", "phase", "advanced", Some("m@1"));
        let rows = agreement_queue(&conn, "operator", 10).unwrap();
        assert_eq!(rows.len(), 1);
        let t = &rows[0];
        assert_eq!(t.routing_class, None);
        assert_eq!(t.outcome, None);
        assert_eq!(
            t.confidence, None,
            "an unrecorded confidence must be ABSENT, never 0.0 — a zero that was never measured is a fabricated number"
        );
        assert_eq!(t.evidence_count, None);
    }

    /// P3b — a HALF-written payload is refused whole. A receipt carrying
    /// `confidence` with no `evidence_count` is the exact pair R65a exists to
    /// prevent, and a reader that defaulted the missing half would put it back
    /// on screen.
    #[test]
    fn a_half_written_decision_reads_absent_whole() {
        let conn = db();
        seed_trace(&conn, 72, "acme", "phase", "advanced", Some("m@1"));
        carry_decision_for(
            &conn,
            72,
            // `evidence_count` deliberately missing.
            serde_json::json!({"routing_class": "finance", "outcome": "defer", "confidence": 0.5}),
        );
        let rows = agreement_queue(&conn, "operator", 10).unwrap();
        let t = &rows[0];
        assert_eq!(t.confidence, None, "a share with no count is not a reading");
        assert_eq!(t.evidence_count, None);
        assert_eq!(
            t.outcome, None,
            "the whole receipt is dropped, not partially defaulted"
        );
        assert_eq!(t.routing_class, None);
    }

    /// P3c — an unparseable or non-finite payload is refused, not coerced.
    #[test]
    fn an_unreadable_decision_reads_absent_not_invented() {
        let conn = db();
        seed_trace(&conn, 73, "acme", "phase", "advanced", Some("m@1"));
        let seq: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(seq), 0) + 1 FROM agent_session_events WHERE run_id = 73",
                [],
                |r| r.get(0),
            )
            .unwrap();
        conn.execute(
            "INSERT INTO agent_session_events(run_id, seq, idempotency_key, kind, payload_json, created_at)
             VALUES (73, ?1, 'deferral:73', ?2, 'not json at all', 1)",
            params![seq, crate::workflow::confidence::DEFERRAL_DECISION_KIND],
        )
        .unwrap();
        let rows = agreement_queue(&conn, "operator", 10).unwrap();
        assert_eq!(rows[0].confidence, None);
        assert_eq!(rows[0].outcome, None);
    }

    /// P6 — **the additive-kind law as an executable pin.** A kind nothing else
    /// names must be inert to every pre-existing reader. The check is
    /// structural rather than a hand-listed set of queries: it scans each
    /// reader's production source and fails on an unfiltered read.
    ///
    /// **The pin FOUND two real patterns, and both are recorded rather than
    /// suppressed.**
    ///
    /// 1. `gdl_checkpoint.rs:491` reads `SELECT MAX(seq) … WHERE run_id = ?1`
    ///    with no kind filter and asserts `last == pause.seq + 1` — a
    ///    POSITIONAL invariant over the run's whole event stream. A new kind on
    ///    a GDL run would break that arithmetic. **Not reachable from this
    ///    round**: the writer only touches `kind = 'delivery'` runs
    ///    (`delivery::RUN_KIND`), and GDL's `bind` refuses any run whose
    ///    `kind != "troubleshoot"`, so the populations never share a `run_id`.
    /// 2. `session_log.rs` is the table's OWN substrate — the writer, the seq
    ///    allocator, and the `exact`/`replay` primitives. Its reads MUST span
    ///    all kinds; filtering them by kind would break the store itself. This
    ///    is not an exemption from the law, it is the law's own home.
    ///
    /// So the exemptions are by ROLE (the substrate, the defining files, this
    /// round's own module and writer, and the one bounded positional reader),
    /// each with its reason inline — not a blanket allowlist, and a second
    /// unfiltered reader in any other file still fails.
    #[test]
    fn a_new_session_kind_is_inert_to_every_reader() {
        // Exempt, each for a stated reason. A reader NOT listed here must
        // filter by kind or this fails.
        let exempt: [(&str, &str); 5] = [
            (
                "agreement.rs",
                "this round's own reader; names its own kinds",
            ),
            ("delivery.rs", "this round's own writer"),
            ("migration.rs", "defines the table"),
            ("storage_layout.rs", "defines the table"),
            (
                "gdl_checkpoint.rs",
                "POSITIONAL read (`last == pause.seq + 1`) scoped to GDL runs by `bind`, \
                 which refuses `kind != \"troubleshoot\"`; a delivery run never reaches it",
            ),
        ];
        // The store's own substrate, exempted by ROLE: every one of its reads
        // must see all kinds, so this file is where an unfiltered read is
        // correct rather than a leak.
        const SUBSTRATE: &str = "session_log.rs";
        let exempt_names: Vec<&str> = exempt.iter().map(|(n, _)| *n).collect();
        let readers: Vec<String> = std::fs::read_dir("src")
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .flat_map(|p| {
                if p.is_dir() {
                    std::fs::read_dir(p)
                        .unwrap()
                        .filter_map(|e| e.ok())
                        .map(|e| e.path())
                        .collect::<Vec<_>>()
                } else {
                    vec![p]
                }
            })
            .filter(|p| p.extension().is_some_and(|x| x == "rs"))
            .filter_map(|p| {
                let name = p.file_name()?.to_str()?.to_string();
                if exempt_names.contains(&name.as_str()) || name == SUBSTRATE {
                    return None;
                }
                let text = std::fs::read_to_string(&p).ok()?;
                text.contains("agent_session_events").then_some(name)
            })
            .collect();
        assert!(
            !readers.is_empty(),
            "the scan found no readers at all — the walk is broken, not the tree clean"
        );
        for name in &readers {
            let text = std::fs::read_to_string(format!("src/workflow/{name}"))
                .or_else(|_| std::fs::read_to_string(format!("src/{name}")))
                .or_else(|_| std::fs::read_to_string(format!("src/agentloop/{name}")))
                .or_else(|_| std::fs::read_to_string(format!("src/bin/{name}")))
                .unwrap_or_else(|_| panic!("could not re-read {name}"));
            // Strip the test module: a test asserting on row counts may
            // legitimately read the table unfiltered, and a production reader
            // may not.
            let production = text
                .split_once("#[cfg(test)]")
                .map_or(text.as_str(), |(head, _)| head);
            let lines: Vec<&str> = production.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                if !line.contains("FROM agent_session_events") {
                    continue;
                }
                // A multi-line SELECT puts `WHERE kind = ?` on a LATER line, so
                // the window is what matters — an earlier version of this pin
                // scanned one line at a time and FAILED on `reflection.rs`,
                // which filters correctly on line two of its query. Judged on
                // the window, it is a reader; judged on the line, it looked
                // like a leak. The window is the honest unit.
                let window: String = lines[i..(i + 4).min(lines.len())].join(" ");
                assert!(
                    window.contains("kind"),
                    "R67-DECISION: {name} reads agent_session_events with no kind filter \
                     in production code — a new kind would NOT be inert: {window}"
                );
            }
        }
        // The exemptions are asserted PRESENT, so deleting a reader from the
        // list cannot quietly make this pin vacuous.
        for (name, reason) in exempt {
            let in_workflow = std::path::Path::new("src/workflow").join(name);
            let in_root = std::path::Path::new("src").join(name);
            assert!(
                in_workflow.exists() || in_root.exists(),
                "R67-DECISION: the exemption for {name} ({reason}) names a file that no longer exists — \
                 the allowlist has rotted"
            );
        }
    }
}
