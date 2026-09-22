//! The after-action reflection artifact and the disagreement corpus — the
//! labeled-corpus manufacturing instrument. Retrospective-only by law:
//! every value this module produces derives from AUDITED gate outcomes of
//! an already-sealed case, and every write is an additive session-log row
//! inside the caller's closing transaction. Nothing here feeds a gate, a
//! route, or a disposition; the capture can only add rows after the
//! outcome is sealed, never mutate one.
//!
//! De-identification posture: the raw case input never lands in a
//! reflection payload — tuples carry a SHA-256 content digest of the
//! input, kernel-named law strings, and bounded artifact excerpts. The
//! export read seam re-sanitizes and re-bounds every rendered field.

use rusqlite::params;
use serde::{Deserialize, Serialize};

/// The frozen holdout fraction (percent). Pinned: the split function maps
/// run ids deterministically, so exports are stable across versions and
/// the holdout partition is the same frozen set every consumer sees.
pub(crate) const REFLECTION_HOLDOUT_PCT: u32 = 20;

/// Additive session-log kinds. Every pre-existing reader filters by kind,
/// so these rows are inert to all of them (the additive-kind law).
pub(crate) const REFLECTION_ROW_KIND: &str = "reflection";
pub(crate) const REFLECTION_DISAGREEMENT_KIND: &str = "reflection_disagreement";

/// Render/capture bounds. The artifact excerpt is bounded at capture and
/// re-bounded at the export seam; the law-string field is shorter still.
pub(crate) const MODEL_PROPOSAL_CAP: usize = 4000;
pub(crate) const GOVERNED_TRUTH_CAP: usize = 1000;
pub(crate) const WOULD_DO_DIFFERENTLY_CAP: usize = 4000;
/// Deterministic cap per flag category: the first N tuples in session-log
/// order. A case cannot grow tuples without gates, and gates are bounded.
pub(crate) const MAX_TUPLES_PER_FLAG: usize = 64;

/// Kernel-authored markers inside `gdl_gate` error strings. The errors are
/// law constants minted by the gates, never model text, so matching on
/// them is deterministic and un-forgeable by an agent.
pub(crate) const MARKER_NO_SEARCH_HIT: &str = "L9";
pub(crate) const MARKER_RED_FLAG_LOCK: &str = "T10";
pub(crate) const MARKER_SECOND_VERIFICATION: &str = "second verification absent or";
pub(crate) const MARKER_ADVERSARIAL_CONTRADICTED: &str = "adversarial re-check contradicted";
pub(crate) const MARKER_OPEN_CONTRADICTIONS: &str = "open contradictions must be dispositioned";
const ADVERSARIAL_CONTRADICTED_FALLBACK: &str =
    "A4: adversarial re-check contradicted the confirmed hypothesis";

// ── the frozen holdout split ────────────────────────────────────────────────

/// Which half of the frozen split a run belongs to. Holdout is the same
/// frozen set every export and future consumer sees; rows carry their
/// partition so a train/holdout bleed is checkable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Partition {
    Train,
    Holdout,
}

impl Partition {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Partition::Train => "train",
            Partition::Holdout => "holdout",
        }
    }

    pub(crate) fn parse(s: &str) -> Option<Partition> {
        match s {
            "train" => Some(Partition::Train),
            "holdout" => Some(Partition::Holdout),
            _ => None,
        }
    }
}

/// Pure function of the run id over the pinned constant: the first eight
/// hex digits of the versioned digest fold to a number; a value under the
/// holdout percent is holdout. Stable across exports and versions.
pub(crate) fn partition_for_run(run_id: i64) -> Partition {
    let hex = crate::audit::hash(&format!("reflection-split-v1:{run_id}"));
    let mut acc = 0u32;
    for b in hex.bytes().take(8) {
        let d = match b {
            b'0'..=b'9' => (b - b'0') as u32,
            b'a'..=b'f' => (b - b'a' + 10) as u32,
            _ => 0,
        };
        acc = acc.wrapping_mul(16).wrapping_add(d);
    }
    if acc % 100 < REFLECTION_HOLDOUT_PCT {
        Partition::Holdout
    } else {
        Partition::Train
    }
}

// ── the artifact ────────────────────────────────────────────────────────────

/// One hard-negative tuple: what the machine proposed, what the governed
/// truth (the kernel-named gate law) required, and where. The input is a
/// content digest — raw case text never enters a payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct DisagreementTuple {
    pub input_digest: String,
    pub phase: String,
    pub model_proposal: String,
    pub governed_truth: String,
}

/// The flags block. Every tuple derives from audited governance events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ReflectionFlags {
    pub ood_flag: bool,
    pub abstain_events: Vec<DisagreementTuple>,
    pub c2_rejections: Vec<DisagreementTuple>,
    pub c3_falsifications: Vec<DisagreementTuple>,
    pub human_overrides: Vec<DisagreementTuple>,
    pub discordance: Vec<DisagreementTuple>,
}

/// The after-action reflection record. Append-only: captured once at the
/// close seam, never edited. `detached` is true by construction — the
/// parser refuses anything else, so no stored record can claim an
/// in-band voice. `would_do_differently` has no write-back surface this
/// round and lands `None`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ReflectionRecord {
    pub phasetrail: Vec<String>,
    pub flags: ReflectionFlags,
    pub would_do_differently: Option<String>,
    pub detected_at: i64,
    pub detached: bool,
}

/// The stored disagreement-row payload shape (the four-key contract).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct DisagreementPayload {
    pub input_digest: String,
    pub model_proposal: String,
    pub governed_truth: String,
    pub phase: String,
}

fn parse_string(v: &serde_json::Value, what: &str) -> Result<String, String> {
    v.as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("reflection: {what} must be a string"))
}

fn parse_tuples(v: &serde_json::Value, name: &str) -> Result<Vec<DisagreementTuple>, String> {
    let arr = v
        .as_array()
        .ok_or_else(|| format!("reflection: flags.{name} must be an array"))?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let obj = item
            .as_object()
            .ok_or("reflection: disagreement tuple must carry input_digest/model_proposal/governed_truth/phase")?;
        let missing = || {
            "reflection: disagreement tuple must carry input_digest/model_proposal/governed_truth/phase"
                .to_string()
        };
        out.push(DisagreementTuple {
            input_digest: parse_string(
                obj.get("input_digest").ok_or_else(missing)?,
                "tuple input_digest",
            )?,
            phase: parse_string(obj.get("phase").ok_or_else(missing)?, "tuple phase")?,
            model_proposal: parse_string(
                obj.get("model_proposal").ok_or_else(missing)?,
                "tuple model_proposal",
            )?,
            governed_truth: parse_string(
                obj.get("governed_truth").ok_or_else(missing)?,
                "tuple governed_truth",
            )?,
        });
    }
    Ok(out)
}

fn parse_reflection_flags(v: &serde_json::Value) -> Result<ReflectionFlags, String> {
    let obj = v.as_object().ok_or("reflection: flags must be an object")?;
    let ood_flag = obj
        .get("ood_flag")
        .ok_or("reflection: missing field flags.ood_flag")?
        .as_bool()
        .ok_or("reflection: flags.ood_flag must be a boolean")?;
    let abstain = obj
        .get("abstain_events")
        .ok_or("reflection: missing field flags.abstain_events")?;
    let c2 = obj
        .get("c2_rejections")
        .ok_or("reflection: missing field flags.c2_rejections")?;
    let c3 = obj
        .get("c3_falsifications")
        .ok_or("reflection: missing field flags.c3_falsifications")?;
    let human = obj
        .get("human_overrides")
        .ok_or("reflection: missing field flags.human_overrides")?;
    let discordance = obj
        .get("discordance")
        .ok_or("reflection: missing field flags.discordance")?;
    Ok(ReflectionFlags {
        ood_flag,
        abstain_events: parse_tuples(abstain, "abstain_events")?,
        c2_rejections: parse_tuples(c2, "c2_rejections")?,
        c3_falsifications: parse_tuples(c3, "c3_falsifications")?,
        human_overrides: parse_tuples(human, "human_overrides")?,
        discordance: parse_tuples(discordance, "discordance")?,
    })
}

/// The total parser. Any input yields the record or a named
/// `reflection: <field> <problem>` error — never a panic. This is the
/// function the fuzz seam drives.
pub(crate) fn parse_reflection_record(v: &serde_json::Value) -> Result<ReflectionRecord, String> {
    let obj = v.as_object().ok_or("reflection: not an object")?;
    let phasetrail = obj
        .get("phasetrail")
        .ok_or("reflection: missing field phasetrail")?;
    let arr = phasetrail
        .as_array()
        .ok_or("reflection: phasetrail must be an array of strings")?;
    let mut trail = Vec::with_capacity(arr.len());
    for p in arr {
        trail.push(parse_string(p, "phasetrail entry")?);
    }
    let flags = obj.get("flags").ok_or("reflection: missing field flags")?;
    let flags = parse_reflection_flags(flags)?;
    let would_do_differently = match obj.get("would_do_differently") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        Some(_) => return Err("reflection: would_do_differently must be a string or null".into()),
    };
    let detected_at = obj
        .get("detected_at")
        .ok_or("reflection: missing field detected_at")?
        .as_i64()
        .ok_or("reflection: detected_at must be an integer")?;
    let detached = obj
        .get("detached")
        .ok_or("reflection: missing field detached")?
        .as_bool()
        .ok_or("reflection: detached must be true")?;
    if !detached {
        return Err("reflection: detached must be true".into());
    }
    Ok(ReflectionRecord {
        phasetrail: trail,
        flags,
        would_do_differently,
        detected_at,
        detached,
    })
}

/// The total parser for a stored disagreement-row payload.
pub(crate) fn parse_disagreement_payload(
    v: &serde_json::Value,
) -> Result<DisagreementPayload, String> {
    let obj = v
        .as_object()
        .ok_or("reflection: disagreement tuple must carry input_digest/model_proposal/governed_truth/phase")?;
    let missing = || {
        "reflection: disagreement tuple must carry input_digest/model_proposal/governed_truth/phase"
            .to_string()
    };
    Ok(DisagreementPayload {
        input_digest: parse_string(obj.get("input_digest").ok_or_else(missing)?, "input_digest")?,
        model_proposal: parse_string(
            obj.get("model_proposal").ok_or_else(missing)?,
            "model_proposal",
        )?,
        governed_truth: parse_string(
            obj.get("governed_truth").ok_or_else(missing)?,
            "governed_truth",
        )?,
        phase: parse_string(obj.get("phase").ok_or_else(missing)?, "phase")?,
    })
}

// ── the derivation (pure, over audited rows only) ──────────────────────────

/// One recorded session-log row handed to the derivation. `payload_json`
/// is the verbatim stored payload of a kernel-written row.
#[derive(Debug, Clone)]
pub(crate) struct RecordedEvent {
    pub kind: String,
    pub payload_json: String,
}

fn bound(s: &str, cap: usize) -> String {
    s.char_indices()
        .nth(cap)
        .map_or_else(|| s.to_string(), |(idx, _)| s[..idx].to_string())
}

fn gate_errors(payload: &serde_json::Value) -> Vec<String> {
    payload
        .get("errors")
        .and_then(|e| e.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|e| e.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn gate_phase(payload: &serde_json::Value) -> Option<String> {
    payload.get("phase").and_then(|p| p.as_str()).map(|p| {
        // The gate rows store the serde variant name; the reflection
        // vocabulary is the lowercase phase name.
        match p {
            "Intake" => "intake",
            "Triage" => "triage",
            "Hypothesize" => "hypothesize",
            "Plan" => "plan",
            "Act" => "act",
            "Verify" => "verify",
            "Handoff" => "handoff",
            other => other,
        }
        .to_string()
    })
}

fn gate_verdict(payload: &serde_json::Value) -> Option<&str> {
    payload.get("verdict").and_then(|v| v.as_str())
}

fn gate_artifact(payload: &serde_json::Value) -> String {
    payload
        .get("artifact")
        .and_then(|a| a.as_str())
        .unwrap_or_default()
        .to_string()
}

/// The terminal tag on a gate row ("Routed" / "Escalated" / …), if any.
fn gate_terminal_tag(payload: &serde_json::Value) -> Option<&str> {
    payload
        .get("terminal")
        .and_then(|t| t.as_object())
        .and_then(|o| o.keys().next())
        .map(String::as_str)
}

fn gate_terminal_reason(payload: &serde_json::Value) -> String {
    payload
        .get("terminal")
        .and_then(|t| t.as_object())
        .and_then(|o| o.values().next())
        .and_then(|v| v.get("reason"))
        .and_then(|r| r.as_str())
        .unwrap_or_default()
        .to_string()
}

fn tuple(
    input_digest: &str,
    phase: &str,
    model_proposal: &str,
    governed_truth: &str,
) -> DisagreementTuple {
    DisagreementTuple {
        input_digest: input_digest.to_string(),
        phase: phase.to_string(),
        model_proposal: bound(model_proposal, MODEL_PROPOSAL_CAP),
        governed_truth: bound(governed_truth, GOVERNED_TRUTH_CAP),
    }
}

/// Derive the reflection record for a sealed case, from the case state and
/// the run's recorded rows only. Pure and deterministic: the same rows
/// derive the same record. Errors are kernel law strings; the adversarial
/// child's free-text detail is never copied into a tuple.
pub(crate) fn derive_reflection(
    case: &super::gdl::GdlCase,
    events: &[RecordedEvent],
    now: i64,
) -> ReflectionRecord {
    let input_digest = crate::audit::hash(&case.ticket);
    let mut phasetrail: Vec<String> = Vec::new();
    let mut abstain_events = Vec::new();
    let mut c2_rejections = Vec::new();
    let mut c3_falsifications = Vec::new();
    let mut discordance = Vec::new();
    let mut ood_flag = false;

    // Pass 1: the gate trail — the phasetrail, the abstain (route/defer/
    // escalate) records, and the marker-bearing gate failures.
    for e in events {
        if e.kind != "gdl_gate" {
            continue;
        }
        let Ok(p) = serde_json::from_str::<serde_json::Value>(&e.payload_json) else {
            continue;
        };
        let phase = gate_phase(&p).unwrap_or_default();
        if gate_verdict(&p) == Some("pass") && !phasetrail.contains(&phase) {
            phasetrail.push(phase.clone());
        }
        let errors = gate_errors(&p);
        let artifact = gate_artifact(&p);
        match gate_terminal_tag(&p) {
            Some("Routed") => {
                abstain_events.push(tuple(
                    &input_digest,
                    &phase,
                    &artifact,
                    &gate_terminal_reason(&p),
                ));
            }
            Some("Escalated") => {
                abstain_events.push(tuple(
                    &input_digest,
                    &phase,
                    &artifact,
                    "escalated: handed to a human with the bundle",
                ));
            }
            _ => {}
        }
        if errors
            .iter()
            .any(|e| e.contains(MARKER_SECOND_VERIFICATION))
        {
            c2_rejections.push(tuple(
                &input_digest,
                &phase,
                &artifact,
                errors
                    .iter()
                    .find(|e| e.contains(MARKER_SECOND_VERIFICATION))
                    .map(String::as_str)
                    .unwrap_or_default(),
            ));
        }
        if errors
            .iter()
            .any(|e| e.contains(MARKER_OPEN_CONTRADICTIONS))
        {
            discordance.push(tuple(
                &input_digest,
                &phase,
                &artifact,
                errors
                    .iter()
                    .find(|e| e.contains(MARKER_OPEN_CONTRADICTIONS))
                    .map(String::as_str)
                    .unwrap_or_default(),
            ));
        }
        if errors
            .iter()
            .any(|e| e.starts_with(MARKER_NO_SEARCH_HIT) || e.starts_with(MARKER_RED_FLAG_LOCK))
        {
            ood_flag = true;
        }
    }

    // Pass 2: the adversarial findings — the kernel-typed outcome label is
    // the trigger; the governed truth rides the matching A4 gate row.
    for e in events {
        if e.kind != "control:adversarial_recheck" {
            continue;
        }
        let Ok(p) = serde_json::from_str::<serde_json::Value>(&e.payload_json) else {
            continue;
        };
        if p.get("outcome").and_then(|o| o.as_str()) != Some("contradicted") {
            continue;
        }
        let matched = events.iter().rev().find_map(|g| {
            if g.kind != "gdl_gate" {
                return None;
            }
            let Ok(gp) = serde_json::from_str::<serde_json::Value>(&g.payload_json) else {
                return None;
            };
            gate_errors(&gp)
                .into_iter()
                .find(|err| err.contains(MARKER_ADVERSARIAL_CONTRADICTED))
                .map(|err| (gate_artifact(&gp), err))
        });
        c3_falsifications.push(matched.map_or_else(
            || {
                tuple(
                    &input_digest,
                    "verify",
                    "",
                    ADVERSARIAL_CONTRADICTED_FALLBACK,
                )
            },
            |(artifact, err)| tuple(&input_digest, "verify", &artifact, &err),
        ));
    }

    // Pass 3: the human overrides — the handoff lifecycle rows where a
    // human decision reference moved the machine's disposition. The
    // decision reference IS the ground-truth provenance.
    let mut human_overrides = Vec::new();
    for e in events {
        if e.kind != "handoff_lifecycle" {
            continue;
        }
        let Ok(p) = serde_json::from_str::<serde_json::Value>(&e.payload_json) else {
            continue;
        };
        if p.get("human_edited").and_then(|h| h.as_bool()) != Some(true) {
            continue;
        }
        human_overrides.push(tuple(
            &input_digest,
            "handoff",
            p.get("transition").and_then(|t| t.as_str()).unwrap_or(""),
            p.get("decision_ref").and_then(|d| d.as_str()).unwrap_or(""),
        ));
    }

    // The out-of-distribution marker: the sealed case itself confessed a
    // knowledge gap (unknown or deferred verdict, or an empty search hit
    // list), or the trail carries a kernel OOD lock marker.
    if let Some(t) = case.triage.as_ref()
        && (t.verdict == "unknown" || t.verdict == "defer" || t.search_hits.is_empty())
    {
        ood_flag = true;
    }

    // Deterministic caps: the first N tuples per category in row order.
    fn cap(v: Vec<DisagreementTuple>) -> Vec<DisagreementTuple> {
        v.into_iter().take(MAX_TUPLES_PER_FLAG).collect()
    }

    ReflectionRecord {
        phasetrail,
        flags: ReflectionFlags {
            ood_flag,
            abstain_events: cap(abstain_events),
            c2_rejections: cap(c2_rejections),
            c3_falsifications: cap(c3_falsifications),
            human_overrides: cap(human_overrides),
            discordance: cap(discordance),
        },
        would_do_differently: None,
        detected_at: now,
        detached: true,
    }
}

/// The hard-negative tuples a record carries, in row-write order.
pub(crate) fn disagreement_tuples(record: &ReflectionRecord) -> Vec<&DisagreementTuple> {
    let mut out = Vec::new();
    out.extend(record.flags.c2_rejections.iter());
    out.extend(record.flags.c3_falsifications.iter());
    out.extend(record.flags.human_overrides.iter());
    out.extend(record.flags.discordance.iter());
    out
}

// ── the capture primitive (inside the caller's transaction) ────────────────

/// Read the run's recorded governance rows through the caller's
/// transaction, in row order. Only the kinds the derivation consumes.
pub(crate) fn collect_events(
    conn: &rusqlite::Connection,
    run_id: i64,
) -> rusqlite::Result<Vec<RecordedEvent>> {
    let mut stmt = conn.prepare(
        "SELECT kind, payload_json FROM agent_session_events
          WHERE run_id = ?1
            AND kind IN ('gdl_gate', 'control:adversarial_recheck', 'handoff_lifecycle',
                         'control:soft_handoff', 'back_referral')
          ORDER BY seq",
    )?;
    let rows = stmt.query_map(params![run_id], |r| {
        Ok(RecordedEvent {
            kind: r.get(0)?,
            payload_json: r.get(1)?,
        })
    })?;
    rows.collect()
}

/// Derive + write, wired into the closing transaction (the capture path's
/// only production entry). Writes one `reflection` row, one
/// `reflection_disagreement` row per hard-negative tuple, and one
/// `reflection` audit row — all inside the caller's tx, atomic with
/// closure. Returns the disagreement-row count.
pub(crate) fn capture_on_resolve(
    tx: &mut super::tx::WorkflowTx<'_>,
    run_id: i64,
    case: &super::gdl::GdlCase,
    owner: &str,
    now: i64,
) -> rusqlite::Result<usize> {
    let events = collect_events(tx.tx(), run_id)?;
    let record = derive_reflection(case, &events, now);
    record_reflection(tx, run_id, owner, &record, now)
}

/// Write the record + its hard-negative rows + the audit row. Every write
/// is bounded so the capture cannot fail the close except on a real SQL
/// fault (which rolls the whole closure back — the atomicity law).
pub(crate) fn record_reflection(
    tx: &mut super::tx::WorkflowTx<'_>,
    run_id: i64,
    owner: &str,
    record: &ReflectionRecord,
    now: i64,
) -> rusqlite::Result<usize> {
    let payload = serde_json::to_string(record).map_err(|e| {
        rusqlite::Error::InvalidParameterName(format!("reflection: record serialize failed: {e}"))
    })?;
    super::session_log::append(
        tx.tx(),
        run_id,
        REFLECTION_ROW_KIND,
        &payload,
        &format!("run{run_id}:reflection"),
        now,
    )?;
    let tuples = disagreement_tuples(record);
    for (n, t) in tuples.iter().enumerate() {
        let payload = serde_json::to_string(&DisagreementPayload {
            input_digest: t.input_digest.clone(),
            model_proposal: t.model_proposal.clone(),
            governed_truth: t.governed_truth.clone(),
            phase: t.phase.clone(),
        })
        .map_err(|e| {
            rusqlite::Error::InvalidParameterName(format!(
                "reflection: tuple serialize failed: {e}"
            ))
        })?;
        super::session_log::append(
            tx.tx(),
            run_id,
            REFLECTION_DISAGREEMENT_KIND,
            &payload,
            &format!("run{run_id}:reflection_disagreement:{owner}:{n}"),
            now,
        )?;
    }
    let count = tuples.len();
    let justification = format!(
        "reflection captured after final closure: {count} disagreement row(s) \
         (c2={} c3={} human={} discordance={}) ood={}",
        record.flags.c2_rejections.len(),
        record.flags.c3_falsifications.len(),
        record.flags.human_overrides.len(),
        record.flags.discordance.len(),
        record.flags.ood_flag,
    );
    super::audit_write(
        tx.tx(),
        run_id,
        "reflection",
        crate::audit::AuditStatus::Ok,
        &justification,
    );
    Ok(count)
}

// ── the corpus read core (zero handler SQL — the handler delegates here) ───

/// One de-identified corpus entry as rendered for export.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CorpusEntry {
    pub run_id: i64,
    pub seq: i64,
    pub partition: Partition,
    pub record: ReflectionRecord,
    pub disagreements: Vec<DisagreementPayload>,
}

pub(crate) fn sanitize_seam(s: &str) -> String {
    // The corpus is a training export: PII masking is UNCONDITIONAL here
    // (the minimization posture) — the render runs as a synthetic
    // scope-less reader, so no caller's PII clearance can bypass it.
    let unprivileged: Option<crate::auth::Principal> = Some(crate::auth::policy::Principal {
        sub: String::new(),
        tenant: String::new(),
        scopes: vec![],
        jti: String::new(),
        roles: vec![],
        manages: vec![],
        kind: crate::auth::policy::PrincipalKind::Jwt,
    });
    crate::gate::sanitize_read(s, true, &unprivileged)
}

fn sanitize_tuple_mut(t: &mut DisagreementTuple) {
    t.model_proposal = bound(&sanitize_seam(&t.model_proposal), MODEL_PROPOSAL_CAP);
    t.governed_truth = bound(&sanitize_seam(&t.governed_truth), GOVERNED_TRUTH_CAP);
}

/// The bounded corpus page: the recorded `reflection` rows (optionally
/// since a timestamp, optionally one partition), in run/row order, each
/// rendered with its disagreement rows through the read-seam sanitizer.
/// Export rows carry their frozen partition so a bleed is checkable.
pub(crate) fn reflection_corpus(
    conn: &rusqlite::Connection,
    since: Option<i64>,
    limit: usize,
    partition: Option<Partition>,
) -> rusqlite::Result<Vec<CorpusEntry>> {
    let mut stmt = conn.prepare(
        "SELECT run_id, seq, payload_json FROM agent_session_events
          WHERE kind = ?1 AND (?2 IS NULL OR created_at >= ?2)
          ORDER BY run_id, seq",
    )?;
    let rows = stmt.query_map(params![REFLECTION_ROW_KIND, since], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (run_id, seq, payload_json) = row?;
        if out.len() >= limit {
            break;
        }
        let part = partition_for_run(run_id);
        if partition.is_some_and(|p| p != part) {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(&payload_json).map_err(|e| {
            rusqlite::Error::InvalidParameterName(format!(
                "reflection: stored record unreadable: {e}"
            ))
        })?;
        let mut record = parse_reflection_record(&value).map_err(|e| {
            rusqlite::Error::InvalidParameterName(format!("reflection: stored record invalid: {e}"))
        })?;
        for t in record.flags.abstain_events.iter_mut() {
            sanitize_tuple_mut(t);
        }
        for t in record.flags.c2_rejections.iter_mut() {
            sanitize_tuple_mut(t);
        }
        for t in record.flags.c3_falsifications.iter_mut() {
            sanitize_tuple_mut(t);
        }
        for t in record.flags.human_overrides.iter_mut() {
            sanitize_tuple_mut(t);
        }
        for t in record.flags.discordance.iter_mut() {
            sanitize_tuple_mut(t);
        }
        if let Some(w) = record.would_do_differently.as_mut() {
            *w = bound(&sanitize_seam(w), WOULD_DO_DIFFERENTLY_CAP);
        }
        let mut disagreements = Vec::new();
        let mut dstmt = conn.prepare(
            "SELECT payload_json FROM agent_session_events
              WHERE run_id = ?1 AND kind = ?2 ORDER BY seq",
        )?;
        let drows = dstmt.query_map(params![run_id, REFLECTION_DISAGREEMENT_KIND], |r| {
            r.get::<_, String>(0)
        })?;
        for d in drows {
            let dj: serde_json::Value = serde_json::from_str(&d?).map_err(|e| {
                rusqlite::Error::InvalidParameterName(format!(
                    "reflection: stored tuple unreadable: {e}"
                ))
            })?;
            let mut payload = parse_disagreement_payload(&dj).map_err(|e| {
                rusqlite::Error::InvalidParameterName(format!(
                    "reflection: stored tuple invalid: {e}"
                ))
            })?;
            payload.model_proposal =
                bound(&sanitize_seam(&payload.model_proposal), MODEL_PROPOSAL_CAP);
            payload.governed_truth =
                bound(&sanitize_seam(&payload.governed_truth), GOVERNED_TRUTH_CAP);
            disagreements.push(payload);
        }
        out.push(CorpusEntry {
            run_id,
            seq,
            partition: part,
            record,
            disagreements,
        });
    }
    Ok(out)
}

// ── the κ instrument lives in workflow::kappa: the promotion put the
//    pure fn in the domain core next to the assignment + label store it
//    serves, and the goldens moved with it ──────────────────────────────────

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

    fn seed_run(conn: &mut Connection, run_id: i64) {
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('acme', 'troubleshoot', '{}', 0, 'resolved', 1, 1)",
            [],
        )
        .unwrap();
        let _ = run_id;
    }

    fn record_fixture() -> ReflectionRecord {
        let t = |phase: &str| DisagreementTuple {
            input_digest: "d".into(),
            phase: phase.into(),
            model_proposal: "m".into(),
            governed_truth: "g".into(),
        };
        ReflectionRecord {
            phasetrail: vec!["intake".into(), "triage".into()],
            flags: ReflectionFlags {
                ood_flag: true,
                abstain_events: vec![t("triage")],
                c2_rejections: vec![t("verify")],
                c3_falsifications: vec![],
                human_overrides: vec![t("handoff")],
                discordance: vec![],
            },
            would_do_differently: None,
            detected_at: 7,
            detached: true,
        }
    }

    #[test]
    fn reflection_record_round_trips() {
        let r = record_fixture();
        let v = serde_json::to_value(&r).unwrap();
        let back = parse_reflection_record(&v).unwrap();
        assert_eq!(r, back);
        // And through the string form, as stored at rest.
        let s = serde_json::to_string(&r).unwrap();
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(parse_reflection_record(&v).unwrap(), r);
    }

    #[test]
    fn reflection_parser_total_on_malformed() {
        let cases: Vec<serde_json::Value> = vec![
            serde_json::Value::Null,
            serde_json::json!(7),
            serde_json::json!("soup"),
            serde_json::json!({}),
            serde_json::json!({"phasetrail": "intake"}),
            serde_json::json!({"phasetrail": [1]}),
            serde_json::json!({"phasetrail": []}),
            serde_json::json!({"phasetrail": [], "flags": 3}),
            serde_json::json!({"phasetrail": [], "flags": {}}),
            serde_json::json!({"phasetrail": [], "flags": {"ood_flag": "yes"}}),
            serde_json::json!({"phasetrail": [], "flags": {"ood_flag": true}}),
            serde_json::json!({"phasetrail": [], "flags": {"ood_flag": true, "abstain_events": [{}]}}),
            serde_json::json!({"phasetrail": [], "flags": {"ood_flag": true, "abstain_events": [], "c2_rejections": [], "c3_falsifications": [], "human_overrides": [], "discordance": []}}),
            serde_json::json!({"phasetrail": [], "flags": {"ood_flag": true, "abstain_events": [], "c2_rejections": [], "c3_falsifications": [], "human_overrides": [], "discordance": []}, "detected_at": "now"}),
            serde_json::json!({"phasetrail": [], "flags": {"ood_flag": true, "abstain_events": [], "c2_rejections": [], "c3_falsifications": [], "human_overrides": [], "discordance": []}, "detected_at": 1}),
            serde_json::json!({"phasetrail": [], "flags": {"ood_flag": true, "abstain_events": [], "c2_rejections": [], "c3_falsifications": [], "human_overrides": [], "discordance": []}, "detected_at": 1, "detached": false}),
            serde_json::json!({"phasetrail": [], "flags": {"ood_flag": true, "abstain_events": [], "c2_rejections": [], "c3_falsifications": [], "human_overrides": [], "discordance": []}, "detected_at": 1, "detached": true, "would_do_differently": 9}),
        ];
        for v in &cases {
            let out = parse_reflection_record(v);
            if let Ok(r) = out {
                // Only the fully-formed last-but-one shape may parse; every
                // parse must at minimum carry the detached-by-construction
                // law.
                assert!(r.detached);
            } else {
                let e = out.unwrap_err();
                assert!(e.starts_with("reflection: "), "unnamed error: {e}");
            }
        }
        // The canonical detached:false refusal is named, not generic.
        let v = serde_json::json!({"phasetrail": [], "flags": {"ood_flag": true, "abstain_events": [], "c2_rejections": [], "c3_falsifications": [], "human_overrides": [], "discordance": []}, "detected_at": 1, "detached": false});
        assert_eq!(
            parse_reflection_record(&v).unwrap_err(),
            "reflection: detached must be true"
        );
    }

    #[test]
    fn partition_covers_every_run() {
        // Totality: every run id maps to exactly one partition, the split
        // is deterministic, and the vocabulary is closed.
        for run_id in -500..500 {
            let p = partition_for_run(run_id);
            assert_eq!(p, partition_for_run(run_id));
            assert!(p.as_str() == "train" || p.as_str() == "holdout");
        }
        // The constant is pinned and the split exercises both sides.
        assert_eq!(REFLECTION_HOLDOUT_PCT, 20);
        let both: [bool; 2] = [
            (0..10_000).any(|r| partition_for_run(r) == Partition::Train),
            (0..10_000).any(|r| partition_for_run(r) == Partition::Holdout),
        ];
        assert!(both[0] && both[1], "the split must exercise both sides");
        // Roughly the pinned fraction: over 10k ids, holdout stays near 20%.
        let holdout = (0..10_000)
            .filter(|r| partition_for_run(*r) == Partition::Holdout)
            .count();
        assert!(
            (1_500..2_500).contains(&holdout),
            "holdout fraction drifted: {holdout}/10000"
        );
        Partition::parse("train").unwrap();
        Partition::parse("holdout").unwrap();
        assert!(Partition::parse("all").is_none());
    }

    #[test]
    fn derive_flags_map_only_from_audited_rows() {
        let mut case = super::super::gdl::GdlCase::fresh("the ticket text");
        case.triage = Some(serde_json::from_str::<super::super::gdl::TriageArtifact>(
            r#"{"priority":"P3","stabilized":false,"search_hits":["PB-1"],"verdict":"accept","acuity_band":"GREEN","esi_level":4,"care_setting":"in_person_primary","red_flag":{"worst_case":"w","ruled_out":true,"rule_out_basis":["b"],"first_would_miss_impact":"m"}}"#,
        ).unwrap());
        let gate = |phase: &str, verdict: &str, errors: &[&str], artifact: &str| {
            serde_json::json!({
                "version": 1, "phase": phase, "verdict": verdict, "attempt": 1,
                "errors": errors, "episode": "e", "exchange": null, "revision": 1,
                "artifact": artifact, "terminal": null
            })
            .to_string()
        };
        let events = vec![
            RecordedEvent {
                kind: "gdl_gate".into(),
                payload_json: gate("Verify", "fail", &["A6: second verification absent or inconsistent — must pass twice"], "{\"plan\":\"m\"}"),
            },
            RecordedEvent {
                kind: "control:adversarial_recheck".into(),
                payload_json: serde_json::json!({"outcome": "contradicted", "detail": "AGENT FREE TEXT MUST NOT LEAK"}).to_string(),
            },
            RecordedEvent {
                kind: "gdl_gate".into(),
                payload_json: gate("Verify", "fail", &["A4: adversarial re-check contradicted the confirmed hypothesis: x"], "{\"plan\":\"m\"}"),
            },
            RecordedEvent {
                kind: "handoff_lifecycle".into(),
                payload_json: serde_json::json!({"transition": "delivered", "human_edited": true, "decision_ref": "OP-77", "detail": {}}).to_string(),
            },
            RecordedEvent {
                kind: "gdl_gate".into(),
                payload_json: gate("Handoff", "fail", &["A4: open contradictions must be dispositioned via resolve_contradiction before resolution"], ""),
            },
        ];
        let r = derive_reflection(&case, &events, 42);
        assert!(r.detached);
        assert!(!r.flags.ood_flag, "a clean accept with hits is not OOD");
        assert_eq!(r.phasetrail, Vec::<String>::new(), "no pass rows, no trail");
        assert_eq!(r.flags.c2_rejections.len(), 1);
        assert_eq!(r.flags.c3_falsifications.len(), 1);
        assert_eq!(r.flags.human_overrides.len(), 1);
        assert_eq!(r.flags.human_overrides[0].governed_truth, "OP-77");
        assert_eq!(r.flags.discordance.len(), 1);
        assert!(r.would_do_differently.is_none());
        // The poisoning defense: the agent child's free text never lands.
        let serialized = serde_json::to_string(&r).unwrap();
        assert!(!serialized.contains("AGENT FREE TEXT MUST NOT LEAK"));
        // Every tuple digests the input; raw case text never appears.
        assert!(!serialized.contains("the ticket text"));
        for t in disagreement_tuples(&r) {
            assert_eq!(t.input_digest, crate::audit::hash("the ticket text"));
        }
        // An OOD marker flips the flag: unknown verdict on the sealed case.
        case.triage.as_mut().unwrap().verdict = "unknown".into();
        let r2 = derive_reflection(&case, &events, 42);
        assert!(r2.flags.ood_flag);
    }

    #[test]
    fn clean_resolve_writes_nothing_but_the_record() {
        let case = super::super::gdl::GdlCase::fresh("t");
        let r = derive_reflection(&case, &[], 1);
        assert!(disagreement_tuples(&r).is_empty());
        assert!(!r.flags.ood_flag);
    }

    #[test]
    fn bounds_hold_at_capture() {
        let long_artifact = "x".repeat(10_000);
        let mut case = super::super::gdl::GdlCase::fresh("t");
        case.triage = None;
        let events = vec![RecordedEvent {
            kind: "gdl_gate".into(),
            payload_json: serde_json::json!({
                "version": 1, "phase": "Verify", "verdict": "fail", "attempt": 1,
                "errors": ["A6: second verification absent or inconsistent"], "episode": "e",
                "exchange": null, "revision": 1, "artifact": long_artifact, "terminal": null
            })
            .to_string(),
        }];
        let r = derive_reflection(&case, &events, 1);
        assert_eq!(r.flags.c2_rejections.len(), 1);
        assert_eq!(
            r.flags.c2_rejections[0].model_proposal.len(),
            MODEL_PROPOSAL_CAP
        );
        assert_eq!(
            r.flags.c2_rejections[0].governed_truth.len(),
            GOVERNED_TRUTH_CAP.min("A6: second verification absent or inconsistent".len())
        );
    }

    #[test]
    fn record_and_corpus_laws_over_a_real_db() {
        let mut conn = db();
        seed_run(&mut conn, 1);
        let mut tx = super::super::tx::WorkflowTx::begin(&mut conn).unwrap();
        let case = super::super::gdl::GdlCase::fresh("t");
        let events = vec![RecordedEvent {
            kind: "handoff_lifecycle".into(),
            payload_json: serde_json::json!({"transition": "delivered", "human_edited": true, "decision_ref": "OP-1", "detail": {}}).to_string(),
        }];
        let record = derive_reflection(&case, &events, 5);
        let n = record_reflection(&mut tx, 1, "op", &record, 5).unwrap();
        assert_eq!(n, 1, "one disagreement tuple → one row");
        tx.commit().unwrap();

        let conn2 = conn;
        let page = reflection_corpus(&conn2, None, 500, None).unwrap();
        assert_eq!(page.len(), 1);
        let e = &page[0];
        assert_eq!(e.run_id, 1);
        assert_eq!(e.disagreements.len(), 1);
        assert_eq!(e.disagreements[0].phase, "handoff");
        assert_eq!(e.record, record, "the stored record renders verbatim");
        // Bounded page: limit is honored.
        assert_eq!(reflection_corpus(&conn2, None, 0, None).unwrap().len(), 0);
        // Partition filter is honored (both sides of the split exist somewhere).
        let train = reflection_corpus(&conn2, None, 500, Some(Partition::Train)).unwrap();
        let holdout = reflection_corpus(&conn2, None, 500, Some(Partition::Holdout)).unwrap();
        let expected = if partition_for_run(1) == Partition::Train {
            1
        } else {
            0
        };
        assert_eq!(train.len(), expected);
        assert_eq!(holdout.len(), 1 - expected);
        // Since filter excludes older rows.
        assert!(
            reflection_corpus(&conn2, Some(9), 500, None)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn fuzz_corpus_replays_reflection_parser() {
        let mut dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        dir.push("crates/brain-fuzz/corpus/reflection");
        let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("corpus dir: {e}"));
        let mut count = 0;
        for entry in entries.flatten() {
            let p = entry.path();
            if !p.is_file() {
                continue;
            }
            let bytes = std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            let text = String::from_utf8_lossy(&bytes);
            let v: serde_json::Value =
                serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
            match parse_reflection_record(&v) {
                Ok(r) => {
                    assert!(r.detached, "a parsed record is detached by construction");
                    let _ = serde_json::to_string(&r).unwrap();
                }
                Err(e) => assert!(e.starts_with("reflection: "), "unnamed error: {e}"),
            }
            count += 1;
        }
        assert!(count >= 8, "the reflection corpus must stay populated");
    }
}
