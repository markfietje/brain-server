//! Consent-first outreach as a service core: the hashed-subject consent
//! registry written ONLY through approved HITL proposals, campaigns as
//! HITL proposals whose recipients carry per-recipient consent proof,
//! the post-close follow-up scheduled by policy interval and gated on
//! consent, and the deterministic retention-cohort view.
//!
//! Laws this module encodes:
//! - **No consent, no send — a gate, not a warning.** Every recipient
//!   without an in-force grant is EXCLUDED from its campaign and counted;
//!   a follow-up without consent refuses loudly. No send engine exists
//!   anywhere in this server: approved campaigns EXPORT for CRM-side
//!   execution, nothing more.
//! - Subjects live here HASHED (`audit::hash`) — raw identifiers never
//!   touch the registry. Rows die with their subject on a DSAR sweep.
//! - Registry writes happen inside the caller's transaction with their
//!   audit row (`record_tenant`, SAVEPOINT-nested) — grant and evidence
//!   commit or roll back together.

use crate::audit::{AuditKind, AuditStatus, record_tenant};
use brain_engine_sdk::pure::consent::{self, Channel, ConsentDecision, Purpose};
use rusqlite::{Connection, OptionalExtension, params};

/// Proposal kinds owned by this module.
pub(crate) const KIND_CONSENT: &str = "outreach_consent";
pub(crate) const KIND_CAMPAIGN: &str = "outreach_campaign";
pub(crate) const KIND_FOLLOWUP: &str = "outreach_followup";

/// The lineage topic outreach events ride.
pub(crate) const TOPIC_OUTREACH: &str = "workflow/outreach";

/// Audit-detail prefixes (writer and reader share the exact formats).
pub(crate) const AUDIT_CONSENT: &str = "outreach/consent";
pub(crate) const AUDIT_CAMPAIGN: &str = "outreach/campaign";

/// Bounds law: pinned by tests below.
pub(crate) const MAX_RECIPIENTS: usize = 1_000;
pub(crate) const MAX_RAW_SUBJECT_LEN: usize = 512;
pub(crate) const MAX_TEMPLATE_ID_LEN: usize = 256;
pub(crate) const COHORT_LIMIT: usize = 200;

#[derive(Debug)]
pub(crate) enum OutreachError {
    NotFound(String),
    Invalid(String),
    Database(String),
}

impl std::fmt::Display for OutreachError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutreachError::NotFound(m) | OutreachError::Invalid(m) | OutreachError::Database(m) => {
                write!(f, "{m}")
            }
        }
    }
}

impl From<rusqlite::Error> for OutreachError {
    fn from(e: rusqlite::Error) -> Self {
        OutreachError::Database(e.to_string())
    }
}

/// Hash a raw subject identifier at the door. The registry never sees the
/// raw value; DSAR sweeps erase by hashing the sweep subject the same way.
pub(crate) fn hash_subject(raw: &str) -> String {
    crate::audit::hash(raw.trim())
}

fn validate_subject_hash(hash: &str) -> Result<(), OutreachError> {
    if hash.is_empty() || hash.len() > crate::workflow::relay::MAX_PRINCIPAL_LEN {
        return Err(OutreachError::Invalid("subject_hash unbounded".into()));
    }
    Ok(())
}

/// Domain-scoped audit row for registry writes (no run binds a consent row;
/// the tenant IS the domain).
fn audit_domain(conn: &Connection, domain: &str, target: &str, detail: &str) {
    record_tenant(
        conn,
        AuditKind::Workflow,
        super::ACTOR,
        target,
        AuditStatus::Ok,
        detail,
        domain,
    );
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConsentProof {
    pub(crate) decision: ConsentDecision,
    pub(crate) granted_at: Option<i64>,
    pub(crate) expires_at: Option<i64>,
    pub(crate) provenance: String,
}

impl ConsentProof {
    fn to_json(&self, decision: ConsentDecision) -> serde_json::Value {
        serde_json::json!({
            "decision": match decision {
                ConsentDecision::Granted => "granted",
                ConsentDecision::Absent => "absent",
                ConsentDecision::Revoked => "revoked",
                ConsentDecision::Expired => "expired",
            },
            "granted_at": self.granted_at,
            "expires_at": self.expires_at,
            "provenance": self.provenance,
        })
    }
}

/// The stored consent row for one (domain, subject, channel, purpose)
/// triple, or `None` when absent. The verdict is computed by the SDK's
/// pure gate — the row supplies facts, never conclusions.
pub(crate) fn consent_proof(
    conn: &Connection,
    domain: &str,
    subject_hash: &str,
    channel: Channel,
    purpose: Purpose,
    now: i64,
) -> Result<Option<ConsentProof>, OutreachError> {
    let row: Option<(i64, Option<i64>, Option<i64>, String)> = conn
        .query_row(
            "SELECT granted_at, expires_at, revoked_at, provenance FROM consent_registry
              WHERE domain = ?1 AND subject_hash = ?2 AND channel = ?3 AND purpose = ?4",
            params![domain, subject_hash, channel.as_str(), purpose.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    Ok(row.map(
        |(granted_at, expires_at, revoked_at, provenance)| ConsentProof {
            decision: consent::consent_decision(now, Some(granted_at), expires_at, revoked_at),
            granted_at: Some(granted_at),
            expires_at,
            provenance,
        },
    ))
}

/// One consent decision to write (the ONLY writer is the approved-proposal
/// path).
pub(crate) struct ConsentWrite<'a> {
    pub(crate) domain: &'a str,
    pub(crate) subject_hash: &'a str,
    pub(crate) channel: Channel,
    pub(crate) purpose: Purpose,
    pub(crate) grant: bool,
    pub(crate) provenance: &'a str,
    pub(crate) now: i64,
    pub(crate) expires_at: Option<i64>,
}

/// Write one consent decision — reached exclusively from the
/// approved-proposal path. Grant/revoke lands with its audit row in the
/// caller's transaction.
pub(crate) fn record_consent(conn: &Connection, w: &ConsentWrite<'_>) -> Result<(), OutreachError> {
    let now = w.now;
    let expires_at = w.expires_at;
    let domain = w.domain;
    let subject_hash = w.subject_hash;
    if domain.trim().is_empty() || domain.len() > 128 {
        return Err(OutreachError::Invalid("domain unbounded".into()));
    }
    validate_subject_hash(subject_hash)?;
    let (status, granted_at, revoked_at) = if w.grant {
        ("granted", now, None)
    } else {
        ("revoked", now, Some(now))
    };
    conn.execute(
        "INSERT INTO consent_registry(domain, subject_hash, channel, purpose, status,
                                      provenance, granted_at, expires_at, revoked_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(domain, subject_hash, channel, purpose) DO UPDATE SET
           status = excluded.status,
           provenance = excluded.provenance,
           granted_at = excluded.granted_at,
           expires_at = excluded.expires_at,
           revoked_at = excluded.revoked_at,
           updated_at = excluded.updated_at",
        params![
            domain,
            subject_hash,
            w.channel.as_str(),
            w.purpose.as_str(),
            status,
            w.provenance,
            granted_at,
            expires_at,
            revoked_at,
            now,
        ],
    )?;
    audit_domain(
        conn,
        domain,
        &format!(
            "consent:{subject_hash}:{}:{}",
            w.channel.as_str(),
            w.purpose.as_str()
        ),
        &format!("{AUDIT_CONSENT} {status} via {}", w.provenance),
    );
    Ok(())
}

/// The approve-path side effect for an `outreach_consent` proposal (called
/// from the gate branch INSIDE its transaction). The proposal content names
/// domain, subject_hash, channel, purpose, action grant|revoke, and an
/// optional expiry.
pub(crate) fn apply_consent_proposal(
    conn: &Connection,
    proposal_id: i64,
    content: &serde_json::Value,
    now: i64,
) -> Result<(), OutreachError> {
    let domain = content
        .get("domain")
        .and_then(|v| v.as_str())
        .ok_or_else(|| OutreachError::Invalid("consent packet missing domain".into()))?;
    let subject_hash = content
        .get("subject_hash")
        .and_then(|v| v.as_str())
        .ok_or_else(|| OutreachError::Invalid("consent packet missing subject_hash".into()))?;
    let channel = Channel::parse(
        content
            .get("channel")
            .and_then(|v| v.as_str())
            .unwrap_or_default(),
    )
    .ok_or_else(|| OutreachError::Invalid("consent packet has unknown channel".into()))?;
    let purpose = Purpose::parse(
        content
            .get("purpose")
            .and_then(|v| v.as_str())
            .unwrap_or_default(),
    )
    .ok_or_else(|| OutreachError::Invalid("consent packet has unknown purpose".into()))?;
    let grant = match content.get("action").and_then(|v| v.as_str()) {
        Some("grant") => true,
        Some("revoke") => false,
        _ => {
            return Err(OutreachError::Invalid(
                "consent packet action must be grant|revoke".into(),
            ));
        }
    };
    let expires_at = content.get("expires_at").and_then(|v| v.as_i64());
    let provenance = format!("proposal:{proposal_id}");
    record_consent(
        conn,
        &ConsentWrite {
            domain,
            subject_hash,
            channel,
            purpose,
            grant,
            provenance: &provenance,
            now,
            expires_at,
        },
    )
}

pub(crate) struct CampaignDraft<'a> {
    pub(crate) domain: &'a str,
    pub(crate) channel: Channel,
    pub(crate) purpose: Purpose,
    /// The KB template reference (Beacon-published templates reuse the
    /// public sanitize path at export render time).
    pub(crate) template_id: &'a str,
    /// RAW subject identifiers — hashed here before anything touches disk.
    pub(crate) audience: &'a [String],
    pub(crate) proposed_by: &'a str,
}

#[derive(Debug)]
pub(crate) struct CampaignOutcome {
    pub(crate) proposal_id: i64,
    /// Recipients WITH their consent proof — every included row can show
    /// exactly which grant covers the contact.
    pub(crate) included: usize,
    pub(crate) excluded: Vec<(String, &'static str)>,
}

/// Propose a campaign: the deterministic consent gate runs per recipient at
/// proposal time. Only recipients holding an in-force grant ride the
/// proposal, each with its proof attached; everyone else is excluded with
/// the reason visible. An audience producing ZERO eligible recipients
/// refuses loudly — a campaign nobody may receive is never filed.
pub(crate) fn propose_campaign(
    conn: &Connection,
    draft: &CampaignDraft<'_>,
    now: i64,
) -> Result<CampaignOutcome, OutreachError> {
    if draft.audience.len() > MAX_RECIPIENTS {
        return Err(OutreachError::Invalid(format!(
            "audience exceeds the {}-recipient bound",
            MAX_RECIPIENTS
        )));
    }
    if draft.template_id.trim().is_empty() || draft.template_id.len() > MAX_TEMPLATE_ID_LEN {
        return Err(OutreachError::Invalid("template_id unbounded".into()));
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut recipients = Vec::new();
    let mut excluded = Vec::new();
    for raw in draft.audience {
        if raw.len() > MAX_RAW_SUBJECT_LEN {
            return Err(OutreachError::Invalid("audience entry unbounded".into()));
        }
        let hash = hash_subject(raw);
        if !seen.insert(hash.clone()) {
            continue;
        }
        let proof = consent_proof(conn, draft.domain, &hash, draft.channel, draft.purpose, now)?;
        match proof {
            Some(p) if p.decision == ConsentDecision::Granted => {
                recipients.push(serde_json::json!({
                    "subject_hash": hash,
                    "consent": p.to_json(p.decision),
                }));
            }
            Some(p) => {
                excluded.push((hash, reason_of(p.decision)));
            }
            None => excluded.push((hash, reason_of(ConsentDecision::Absent))),
        }
    }
    if recipients.is_empty() {
        return Err(OutreachError::Invalid(
            "no recipient holds in-force consent for this channel+purpose — campaign refused"
                .into(),
        ));
    }
    let content = serde_json::json!({
        "domain": draft.domain,
        "channel": draft.channel.as_str(),
        "purpose": draft.purpose.as_str(),
        "template_id": draft.template_id,
        "proposed_by": draft.proposed_by,
        "recipients": recipients,
        "excluded": excluded.iter().map(|(h, r)| serde_json::json!({
            "subject_hash": h, "reason": r,
        })).collect::<Vec<_>>(),
        "execution": "export-for-crm-only — brain decides and records; the CRM/telco system sends",
    });
    conn.execute(
        "INSERT INTO proposals(kind, content, source, authority, observed_at,
                               novelty, conflict_with, salience, status, created_at)
         VALUES (?1, ?2, 'agent', NULL, NULL, 1.0, NULL, 0.6, 'pending', ?3)",
        params![KIND_CAMPAIGN, content.to_string(), now],
    )?;
    let proposal_id = conn.last_insert_rowid();
    audit_domain(
        conn,
        draft.domain,
        &format!("proposal:{proposal_id}"),
        &format!(
            "{AUDIT_CAMPAIGN}/proposed recipients:{} excluded:{}",
            recipients.len(),
            excluded.len()
        ),
    );
    Ok(CampaignOutcome {
        proposal_id,
        included: recipients.len(),
        excluded,
    })
}

fn reason_of(d: ConsentDecision) -> &'static str {
    match d {
        ConsentDecision::Granted => "granted",
        ConsentDecision::Absent => "absent",
        ConsentDecision::Revoked => "revoked",
        ConsentDecision::Expired => "expired",
    }
}

/// The export packet for an APPROVED campaign — the artifact an operator or
/// CRM connector feed executes outside this server. A pending or rejected
/// campaign exports NOTHING (404 at the seam).
pub(crate) fn campaign_packet(
    conn: &Connection,
    proposal_id: i64,
) -> Result<serde_json::Value, OutreachError> {
    let (status, content): (String, String) = conn
        .query_row(
            "SELECT status, content FROM proposals WHERE id = ?1 AND kind = ?2",
            params![proposal_id, KIND_CAMPAIGN],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| {
            OutreachError::NotFound(format!("campaign proposal {proposal_id} not found"))
        })?;
    if status != "approved" {
        return Err(OutreachError::NotFound(format!(
            "campaign {proposal_id} is '{status}' — only approved campaigns export"
        )));
    }
    let v: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| OutreachError::Invalid(format!("corrupt campaign packet: {e}")))?;
    Ok(serde_json::json!({
        "proposal_id": proposal_id,
        "channel": v.get("channel"),
        "purpose": v.get("purpose"),
        "template_id": v.get("template_id"),
        "recipients": v.get("recipients"),
        "excluded": v.get("excluded"),
        "execution": v.get("execution"),
    }))
}

const FOLLOWUP_KINDS: &str = "('complaint')";

/// Schedule the Order-of-Care post-close follow-up for a CLOSED complaint
/// run whose state carries `subject_hash`: due at the policy interval after
/// close, gated on an IN-FORCE care_followup consent for the email channel.
/// Anything else refuses loudly — a follow-up nobody consented to is never
/// even proposed.
pub(crate) fn schedule_followup(
    conn: &Connection,
    run_id: i64,
    actor: &str,
    now: i64,
) -> Result<serde_json::Value, OutreachError> {
    let kind: String = conn
        .query_row(
            "SELECT kind FROM workflow_runs WHERE id = ?1",
            params![run_id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| OutreachError::NotFound(format!("run {run_id} not found")))?;
    if !FOLLOWUP_KINDS.contains(kind.as_str()) {
        return Err(OutreachError::Invalid(format!(
            "run {run_id} is a '{kind}' run — follow-ups schedule on complaint runs"
        )));
    }
    let state_json: String = conn.query_row(
        "SELECT state_json FROM workflow_runs WHERE id = ?1",
        params![run_id],
        |r| r.get(0),
    )?;
    let state: serde_json::Value = serde_json::from_str(&state_json)
        .map_err(|e| OutreachError::Invalid(format!("corrupt run state: {e}")))?;
    let raw_subject = state
        .get("subject")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            OutreachError::Invalid(
                "run state carries no subject — consent cannot be checked, follow-up refused"
                    .into(),
            )
        })?;
    if raw_subject.len() > MAX_RAW_SUBJECT_LEN {
        return Err(OutreachError::Invalid("run subject unbounded".into()));
    }
    let subject_hash = hash_subject(raw_subject);
    // Close time = the creation stamp of the newest lifecycle event that
    // moved the complaint to `closed`.
    let closed_at: Option<i64> = conn
        .query_row(
            "SELECT created_at FROM outbox
              WHERE run_id = ?1 AND topic = 'workflow/complaint'
                AND payload_json LIKE '%\"to\":\"closed\"%'
              ORDER BY id DESC LIMIT 1",
            params![run_id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(closed_at) = closed_at else {
        return Err(OutreachError::Invalid(format!(
            "run {run_id} has not reached `closed` — no follow-up before the peak ends"
        )));
    };
    let interval = consent::DEFAULT_FOLLOWUP_INTERVAL_SECS;
    let domain: String = conn.query_row(
        "SELECT domain FROM workflow_runs WHERE id = ?1",
        params![run_id],
        |r| r.get(0),
    )?;
    let proof = consent_proof(
        conn,
        &domain,
        &subject_hash,
        Channel::Email,
        Purpose::CareFollowup,
        now,
    )?;
    match proof.as_ref().map(|p| p.decision) {
        Some(ConsentDecision::Granted) => {}
        other => {
            return Err(OutreachError::Invalid(format!(
                "care_followup consent is {:?} — no consent, no send",
                other.map(reason_of).unwrap_or("absent")
            )));
        }
    }
    let due_at = closed_at.saturating_add(interval);
    let content = serde_json::json!({
        "run_id": run_id,
        "domain": domain,
        "subject_hash": subject_hash,
        "channel": Channel::Email.as_str(),
        "purpose": Purpose::CareFollowup.as_str(),
        "due_at": due_at,
        "interval_secs": interval,
        "consent": proof.map(|p| p.to_json(ConsentDecision::Granted)),
        "requested_by": actor,
        "execution": "export-for-crm-only — brain decides and records; the CRM/telco system sends",
    });
    conn.execute(
        "INSERT INTO proposals(kind, content, source, authority, observed_at,
                               novelty, conflict_with, salience, status, created_at)
         VALUES (?1, ?2, 'agent', NULL, NULL, 1.0, NULL, 0.5, 'pending', ?3)",
        params![KIND_FOLLOWUP, content.to_string(), now],
    )?;
    let proposal_id = conn.last_insert_rowid();
    super::outbox::append_lineage(
        conn,
        run_id,
        TOPIC_OUTREACH,
        &serde_json::json!({
            "event": "followup_scheduled",
            "due_at": due_at,
            "actor": actor,
        })
        .to_string(),
        &format!("outreach:{run_id}:followup:{now}"),
        now,
    )?;
    super::audit_write(
        conn,
        run_id,
        &format!("proposal:{proposal_id}"),
        AuditStatus::Ok,
        &format!("outreach/followup due:{due_at}"),
    );
    Ok(serde_json::json!({
        "proposal_id": proposal_id,
        "due_at": due_at,
        "status": "pending",
    }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RetentionRow {
    pub(crate) run_id: i64,
    pub(crate) kind: String,
    pub(crate) subject_hash: String,
    pub(crate) contract_expires_at: Option<i64>,
    pub(crate) created_at: i64,
    pub(crate) signals: Vec<&'static str>,
    pub(crate) retention_consent: bool,
}

/// The retention cohort: a DETERMINISTIC query over lineage facts only —
/// contract-expiry window, complaint history, and recorded repeat contact.
/// Retention itself stays a human strategy: this view makes the cohort and
/// each member's consent state visible; it proposes nothing by itself.
/// Same input state → byte-identical output, always.
pub(crate) fn retention_cohort(
    conn: &Connection,
    now: i64,
    window_secs: i64,
    limit: usize,
) -> Result<Vec<RetentionRow>, OutreachError> {
    if window_secs < 0 {
        return Err(OutreachError::Invalid("window inverted".into()));
    }
    let limit = limit.min(COHORT_LIMIT);
    let mut stmt = conn.prepare(
        "SELECT id, kind, domain, created_at,
                json_extract(state_json, '$.subject'),
                json_extract(state_json, '$.contract_expires_at'),
                json_extract(state_json, '$.repeat_contact')
          FROM workflow_runs
          WHERE kind IN ('complaint', 'return', 'warranty_claim', 'repair_field')
          ORDER BY id DESC LIMIT 2000",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, Option<i64>>(5)?,
            r.get::<_, Option<rusqlite::types::Value>>(6)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (run_id, kind, domain, created_at, subject, expires_at, repeat) = row?;
        let repeat_flag = match repeat {
            Some(rusqlite::types::Value::Integer(1)) => true,
            Some(rusqlite::types::Value::Text(t)) => t == "true" || t == "1",
            _ => false,
        };
        let Some(subject) = subject else {
            continue;
        };
        let mut signals = Vec::new();
        if expires_at.is_some_and(|exp| exp > now && exp <= now.saturating_add(window_secs)) {
            signals.push("contract_expiring");
        }
        if kind == "complaint" {
            signals.push("complaint_history");
        }
        if repeat_flag {
            signals.push("repeat_contact");
        }
        if signals.is_empty() {
            continue;
        }
        let hash = hash_subject(&subject);
        let consented = consent_proof(
            conn,
            &domain,
            &hash,
            Channel::Email,
            Purpose::Retention,
            now,
        )?
        .is_some_and(|p| p.decision == ConsentDecision::Granted);
        out.push(RetentionRow {
            run_id,
            kind,
            subject_hash: hash,
            contract_expires_at: expires_at,
            created_at,
            signals,
            retention_consent: consented,
        });
        if out.len() >= limit {
            break;
        }
    }
    Ok(out)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VocMeasures {
    pub(crate) contacts_total: i64,
    pub(crate) complaints_total: i64,
    /// Complaints per thousand contacts, in hundredths (per-mille × 100):
    /// `complaints * 100_000 / max(contacts, 1)` — zero contacts score 0,
    /// never perfection.
    pub(crate) complaints_per_thousand_units: i64,
}

/// ISO 10004 VoC as DATA, not surveys-built-here: the CSAT/DSAT instruments
/// stay CRM-side (ingested via Bridges); this derives what the lineage
/// already knows — contact volume and the complaint-per-thousand-contacts
/// ratio the dictionary formulas pin.
pub(crate) fn voc_measures(conn: &Connection) -> Result<VocMeasures, OutreachError> {
    let contacts_total: i64 =
        conn.query_row("SELECT COUNT(*) FROM workflow_runs", [], |r| r.get(0))?;
    let complaints_total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM workflow_runs WHERE kind = 'complaint'",
        [],
        |r| r.get(0),
    )?;
    let units = if contacts_total == 0 {
        0
    } else {
        complaints_total * 100_000 / contacts_total
    };
    Ok(VocMeasures {
        contacts_total,
        complaints_total,
        complaints_per_thousand_units: units,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use brain_server::migration::run_migration;
    use brain_server::register_sqlite_vec::register_sqlite_vec;
    use rusqlite::Connection;

    fn db() -> Connection {
        register_sqlite_vec();
        let mut conn = Connection::open_in_memory().unwrap();
        run_migration(&mut conn, 1).unwrap();
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('acme', 'complaint', '{\"subject\":\"jane@example.com\"}', 0, 'active', 1, 1)",
            [],
        )
        .unwrap();
        conn
    }

    fn grant(
        conn: &Connection,
        subject: &str,
        channel: Channel,
        purpose: Purpose,
        now: i64,
    ) -> i64 {
        let content = serde_json::json!({
            "domain": "acme", "subject_hash": hash_subject(subject),
            "channel": channel.as_str(), "purpose": purpose.as_str(),
            "action": "grant",
        });
        conn.execute(
            "INSERT INTO proposals(kind, content, source, authority, observed_at,
                                   novelty, conflict_with, salience, status, created_at)
             VALUES (?1, ?2, 'agent', NULL, NULL, 1.0, NULL, 0.5, 'pending', 1)",
            params![KIND_CONSENT, content.to_string()],
        )
        .unwrap();
        let id = conn.last_insert_rowid();
        apply_consent_proposal(conn, id, &content, now).unwrap();
        id
    }

    /// consent_registry_is_dsar_erasable — grants land via approved
    /// proposals, revocation wins, and the DSAR sweep takes the rows by
    /// re-hashing the sweep subject.
    #[test]
    fn consent_registry_is_dsar_erasable() {
        let mut conn = db();
        grant(
            &conn,
            "jane@example.com",
            Channel::Email,
            Purpose::Retention,
            10,
        );
        grant(
            &conn,
            "jane@example.com",
            Channel::Email,
            Purpose::CareFollowup,
            11,
        );
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM consent_registry", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 2);
        assert!(
            crate::audit::verify_chain(&conn),
            "registry writes stay on the chain"
        );

        // Revocation through the same proposal path flips the verdict.
        let content = serde_json::json!({
            "domain": "acme", "subject_hash": hash_subject("jane@example.com"),
            "channel": "email", "purpose": "retention", "action": "revoke",
        });
        apply_consent_proposal(&conn, 999, &content, 20).unwrap();
        let proof = consent_proof(
            &conn,
            "acme",
            &hash_subject("jane@example.com"),
            Channel::Email,
            Purpose::Retention,
            30,
        )
        .unwrap()
        .unwrap();
        assert_eq!(proof.decision, ConsentDecision::Revoked);

        // The DSAR sweep erases every row of the subject (exact-hash arm),
        // while other subjects' rows survive.
        conn.execute(
            "INSERT INTO consent_registry(domain, subject_hash, channel, purpose, status, provenance, granted_at, updated_at)
             VALUES ('acme', ?, 'email', 'retention', 'granted', 'x', 5, 5)",
            params![hash_subject("bob@example.com")],
        )
        .unwrap();
        let tx = conn.transaction().unwrap();
        let rep = crate::service::dsar::sweep::sweep_subject(&tx, "jane@example.com").unwrap();
        tx.commit().unwrap();
        assert_eq!(rep.consent_rows, 2);
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM consent_registry", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1, "only bob's row survives the sweep");
    }

    /// no_consent_no_send_is_a_gate_not_warning — campaign-side.
    #[test]
    fn no_consent_no_send_is_a_gate_not_warning_campaign() {
        let conn = db();
        // One consented, one absent, one expired, one revoked.
        grant(
            &conn,
            "ok@example.com",
            Channel::Email,
            Purpose::Retention,
            10,
        );
        grant(
            &conn,
            "stale@example.com",
            Channel::Email,
            Purpose::Retention,
            10,
        );
        conn.execute(
            "UPDATE consent_registry SET expires_at = 50 WHERE granted_at = 10 AND provenance = 'proposal:1'",
            [],
        )
        .unwrap();
        grant(
            &conn,
            "gone@example.com",
            Channel::Email,
            Purpose::Retention,
            10,
        );
        conn.execute(
            "UPDATE consent_registry SET revoked_at = 20 WHERE provenance = 'proposal:3'",
            [],
        )
        .unwrap();
        let outcome = propose_campaign(
            &conn,
            &CampaignDraft {
                domain: "acme",
                channel: Channel::Email,
                purpose: Purpose::Retention,
                template_id: "tmpl-retention-q3",
                audience: &[
                    "ok@example.com".into(),
                    "absent@example.com".into(),
                    "stale@example.com".into(),
                    "gone@example.com".into(),
                    "ok@example.com".into(),
                ],
                proposed_by: "t1-agent",
            },
            100,
        )
        .unwrap();
        assert_eq!(outcome.included, 1, "only the in-force grant rides");
        assert_eq!(
            outcome.excluded.len(),
            3,
            "duplicate collapses; absent/expired/revoked all excluded"
        );
        let content: String = conn
            .query_row(
                "SELECT content FROM proposals WHERE id = ?1",
                params![outcome.proposal_id],
                |r| r.get(0),
            )
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["recipients"].as_array().unwrap().len(), 1);
        let reasons: Vec<&str> = v["excluded"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["reason"].as_str().unwrap())
            .collect();
        assert!(reasons.contains(&"absent"));
        assert!(reasons.contains(&"expired"));
        assert!(reasons.contains(&"revoked"));
        // Zero eligible recipients refuses loudly — no empty campaign ships.
        let err = propose_campaign(
            &conn,
            &CampaignDraft {
                domain: "acme",
                channel: Channel::Call,
                purpose: Purpose::Retention,
                template_id: "tmpl-x",
                audience: &["nobody@example.com".into()],
                proposed_by: "t1-agent",
            },
            100,
        )
        .unwrap_err();
        assert!(matches!(err, OutreachError::Invalid(_)));
        // Audience over the bound denies before any lookup.
        let big: Vec<String> = (0..=MAX_RECIPIENTS).map(|i| format!("s{i}@x")).collect();
        let err = propose_campaign(
            &conn,
            &CampaignDraft {
                domain: "acme",
                channel: Channel::Email,
                purpose: Purpose::Retention,
                template_id: "tmpl-x",
                audience: &big,
                proposed_by: "t1-agent",
            },
            100,
        )
        .unwrap_err();
        assert!(matches!(err, OutreachError::Invalid(_)));
    }

    /// campaign_recipients_carry_consent_proof — and only APPROVED
    /// campaigns export.
    #[test]
    fn campaign_recipients_carry_consent_proof() {
        let conn = db();
        grant(
            &conn,
            "ok@example.com",
            Channel::Sms,
            Purpose::RecallNotice,
            10,
        );
        let outcome = propose_campaign(
            &conn,
            &CampaignDraft {
                domain: "acme",
                channel: Channel::Sms,
                purpose: Purpose::RecallNotice,
                template_id: "tmpl-recall-batch-7",
                audience: &["ok@example.com".into()],
                proposed_by: "dpo",
            },
            20,
        )
        .unwrap();
        let content: String = conn
            .query_row(
                "SELECT content FROM proposals WHERE id = ?1",
                params![outcome.proposal_id],
                |r| r.get(0),
            )
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        let rcpt = &v["recipients"][0];
        assert_eq!(rcpt["consent"]["decision"], "granted");
        assert_eq!(rcpt["consent"]["granted_at"], 10);
        assert!(
            rcpt["consent"]["provenance"]
                .as_str()
                .unwrap()
                .starts_with("proposal:")
        );
        // Pending campaigns export NOTHING.
        assert!(matches!(
            campaign_packet(&conn, outcome.proposal_id),
            Err(OutreachError::NotFound(_))
        ));
        conn.execute(
            "UPDATE proposals SET status = 'approved' WHERE id = ?1",
            params![outcome.proposal_id],
        )
        .unwrap();
        let packet = campaign_packet(&conn, outcome.proposal_id).unwrap();
        assert_eq!(packet["template_id"], "tmpl-recall-batch-7");
        assert_eq!(packet["recipients"].as_array().unwrap().len(), 1);
        assert!(matches!(
            campaign_packet(&conn, 99_999),
            Err(OutreachError::NotFound(_))
        ));
    }

    /// followup_scheduled_by_policy_and_consent_gated — service leg.
    #[test]
    fn followup_scheduled_by_policy_and_consent_gated() {
        let conn = db();
        conn.execute(
            "UPDATE workflow_runs SET state_json = '{\"subject\":\"jane@example.com\"}' WHERE id = 1",
            [],
        )
        .unwrap();
        super::super::outbox::append_lineage(
            &conn,
            1,
            "workflow/complaint",
            r#"{"from":"remedy_approved","to":"closed","actor":"agent"}"#,
            "lc-closed-1",
            100,
        )
        .unwrap();

        // Without consent: loud refusal, nothing written.
        let err = schedule_followup(&conn, 1, "agent", 200).unwrap_err();
        assert!(matches!(err, OutreachError::Invalid(ref m) if m.contains("no consent")));
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM proposals WHERE kind = 'outreach_followup'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0, "the gate refuses BEFORE any proposal exists");

        // With an in-force care_followup grant: scheduled at policy interval.
        grant(
            &conn,
            "jane@example.com",
            Channel::Email,
            Purpose::CareFollowup,
            110,
        );
        let out = schedule_followup(&conn, 1, "agent", 200).unwrap();
        let iv = consent::DEFAULT_FOLLOWUP_INTERVAL_SECS;
        assert_eq!(out["due_at"], 100 + iv, "due = close + policy interval");
        assert_eq!(out["status"], "pending");
        // The lineage event rode the run's chain.
        assert!(super::super::outbox::verify_outbox_lineage(&conn, 1).unwrap());
        assert!(crate::audit::verify_chain(&conn));

        // Not-yet-closed runs refuse; non-complaint runs refuse; missing
        // subjects refuse.
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('acme', 'interview', '{}', 0, 'active', 2, 2)",
            [],
        )
        .unwrap();
        assert!(matches!(
            schedule_followup(&conn, 2, "agent", 300),
            Err(OutreachError::Invalid(_))
        ));
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('acme', 'complaint', '{}', 0, 'active', 3, 3)",
            [],
        )
        .unwrap();
        assert!(matches!(
            schedule_followup(&conn, 3, "agent", 300),
            Err(OutreachError::Invalid(_))
        ));
        assert!(matches!(
            schedule_followup(&conn, 4_096, "agent", 300),
            Err(OutreachError::NotFound(_))
        ));
    }

    /// retention_cohort_is_deterministic_query — same state, identical
    /// output; signals derive from lineage facts only; consent state is
    /// visible per member.
    #[test]
    fn retention_cohort_is_deterministic_query() {
        let conn = db();
        // jane: complaint history + expiring contract + repeat flag.
        conn.execute(
            "UPDATE workflow_runs SET state_json = '{\"subject\":\"jane@example.com\",\"contract_expires_at\":150,\"repeat_contact\":true}' WHERE id = 1",
            [],
        )
        .unwrap();
        // bob: nothing signal-bearing — filtered out.
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('acme', 'return', '{\"subject\":\"bob@example.com\",\"contract_expires_at\":900000}', 0, 'active', 2, 2)",
            [],
        )
        .unwrap();
        grant(
            &conn,
            "jane@example.com",
            Channel::Email,
            Purpose::Retention,
            10,
        );

        let a = retention_cohort(&conn, 100, 200, COHORT_LIMIT).unwrap();
        let b = retention_cohort(&conn, 100, 200, COHORT_LIMIT).unwrap();
        assert_eq!(a, b, "deterministic: same state, identical output");
        assert_eq!(a.len(), 1, "signal-less members never surface");
        let row = &a[0];
        assert_eq!(row.run_id, 1);
        assert!(row.signals.contains(&"contract_expiring"));
        assert!(row.signals.contains(&"complaint_history"));
        assert!(row.signals.contains(&"repeat_contact"));
        assert!(row.retention_consent);

        // Outside the expiry window the contract signal drops off.
        let c = retention_cohort(&conn, 100, 49, COHORT_LIMIT).unwrap();
        assert!(!c[0].signals.contains(&"contract_expiring"));
        // Inverted windows deny.
        assert!(matches!(
            retention_cohort(&conn, 100, -1, COHORT_LIMIT),
            Err(OutreachError::Invalid(_))
        ));
    }

    /// voc_fields_have_dictionary_formulas — service leg: the ratio derives
    /// from lineage counts alone; zero contacts score 0.
    #[test]
    fn voc_complaint_ratio_derives_from_lineage_counts() {
        let conn = db();
        let m = voc_measures(&conn).unwrap();
        assert_eq!(m.contacts_total, 1);
        assert_eq!(m.complaints_total, 1);
        assert_eq!(m.complaints_per_thousand_units, 100_000);
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('acme', 'interview', '{}', 0, 'active', 2, 2)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('acme', 'interview', '{}', 0, 'active', 3, 3)",
            [],
        )
        .unwrap();
        let m = voc_measures(&conn).unwrap();
        assert_eq!(m.contacts_total, 3);
        assert_eq!(m.complaints_per_thousand_units, 33_333);
        let empty_db = {
            register_sqlite_vec();
            let mut c = Connection::open_in_memory().unwrap();
            run_migration(&mut c, 1).unwrap();
            c
        };
        let m = voc_measures(&empty_db).unwrap();
        assert_eq!(
            m.complaints_per_thousand_units, 0,
            "zero contacts score 0, never dressed up as perfection"
        );
    }
}
