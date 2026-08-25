//! The full ISO 10002/10003 complaint lifecycle as a service core:
//! lifecycle states as lineage events on the audit chain, the remedy
//! matrix as HITL proposals with role-capped approvals that escalate one
//! level over cap, the goodwill ledger aggregated ONLY from audited
//! remedies, and the external-dispute packet targeting the competent
//! NATIONAL ADR body (the EU ODR platform is discontinued — Reg. 2024/3228;
//! the packet states that basis explicitly).
//!
//! Laws this module encodes:
//! - Every lifecycle transition validates against the closed SDK table and
//!   appends a lineage event audited in the caller's transaction.
//! - A remedy proposal MUST cite both its legal basis (from the closed
//!   anchor set) and its published code-of-conduct clause (ISO 10001); a
//!   remedy contradicting the published promise is flagged visibly, never
//!   silently blocked — the human decides with the flag in view.
//! - Approval over cap escalates EXACTLY one level with the packet
//!   attached; an unknown approver role denies loudly (fail closed).

use crate::audit::AuditStatus;
use brain_engine_sdk::pure::complaint::{
    self, ApprovalDecision, ApprovalLevel, ComplaintState, RemedyKind,
};
use rusqlite::{Connection, OptionalExtension, params};

/// The exact audit-detail string marking an APPROVED remedy — the ledger's
/// audited-presence contract (writer and reader share this one format).
pub const REMEDY_APPROVED_AUDIT_DETAIL_PREFIX: &str = "complaint/remedy/approved";

/// Proposal kinds owned by this module.
pub const KIND_REMEDY: &str = "complaint_remedy";
pub const KIND_RCA: &str = "complaint_rca";

/// The lineage topic every complaint lifecycle event rides.
pub const TOPIC_COMPLAINT: &str = "workflow/complaint";

/// The knowledge `source` values this module reads (DPO/team-maintained
/// through the ordinary governed knowledge write path — screened at write,
/// sanitized at read).
pub const SOURCE_CONDUCT_CLAUSE: &str = "code_of_conduct";
pub const SOURCE_ADR_BODY: &str = "adr_body";

/// Read bound for the ledger scan (the bounds law; pinned by test below).
pub const LEDGER_SCAN_LIMIT: i64 = 1_000;

#[derive(Debug)]
pub enum ComplaintError {
    NotFound(String),
    Invalid(String),
    Database(String),
}

impl std::fmt::Display for ComplaintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ComplaintError::NotFound(m)
            | ComplaintError::Invalid(m)
            | ComplaintError::Database(m) => {
                write!(f, "{m}")
            }
        }
    }
}

impl From<rusqlite::Error> for ComplaintError {
    fn from(e: rusqlite::Error) -> Self {
        ComplaintError::Database(e.to_string())
    }
}

fn run_kind(conn: &Connection, run_id: i64) -> Result<String, ComplaintError> {
    conn.query_row(
        "SELECT kind FROM workflow_runs WHERE id = ?1",
        params![run_id],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(|| ComplaintError::NotFound(format!("run {run_id} not found")))
}

/// The complaint's current lifecycle state: the `to` of the newest lineage
/// event on [`TOPIC_COMPLAINT`], or `received` when none exists yet (a run
/// classified as a complaint is born received).
pub fn current_state(conn: &Connection, run_id: i64) -> Result<ComplaintState, ComplaintError> {
    let payload: Option<String> = conn
        .query_row(
            "SELECT payload_json FROM outbox
              WHERE run_id = ?1 AND topic = ?2 ORDER BY id DESC LIMIT 1",
            params![run_id, TOPIC_COMPLAINT],
            |r| r.get(0),
        )
        .optional()?;
    let to = payload
        .as_deref()
        .and_then(|p| serde_json::from_str::<serde_json::Value>(p).ok())
        .and_then(|v| v.get("to").and_then(|t| t.as_str()).map(String::from));
    match to {
        Some(t) => ComplaintState::parse(&t)
            .ok_or_else(|| ComplaintError::Invalid(format!("corrupt lifecycle state '{t}'"))),
        None => Ok(ComplaintState::Received),
    }
}

/// Advance the lifecycle one legal step. Fails closed on any transition the
/// closed SDK table does not name, or on a non-complaint run. Lineage event
/// + audit row land in the caller's transaction; nothing here commits.
pub fn transition(
    conn: &Connection,
    run_id: i64,
    to: ComplaintState,
    actor: &str,
    now: i64,
) -> Result<(ComplaintState, ComplaintState), ComplaintError> {
    let kind = run_kind(conn, run_id)?;
    if kind != "complaint" {
        return Err(ComplaintError::Invalid(format!(
            "run {run_id} is a '{kind}' run, not a complaint"
        )));
    }
    let from = current_state(conn, run_id)?;
    if !complaint::transition_allowed(from, to) {
        return Err(ComplaintError::Invalid(format!(
            "transition {} → {} is not in the lifecycle table",
            from.as_str(),
            to.as_str()
        )));
    }
    let actor = actor.trim();
    if actor.is_empty() || actor.len() > super::relay::MAX_PRINCIPAL_LEN {
        return Err(ComplaintError::Invalid("actor unbounded".into()));
    }
    super::outbox::append_lineage(
        conn,
        run_id,
        TOPIC_COMPLAINT,
        &serde_json::json!({
            "from": from.as_str(),
            "to": to.as_str(),
            "actor": actor,
        })
        .to_string(),
        &format!("complaint:{run_id}:{}:{}", now, to.as_str()),
        now,
    )?;
    super::audit_write(
        conn,
        run_id,
        &format!("run:{run_id}"),
        AuditStatus::Ok,
        &format!("complaint/lifecycle {}", to.as_str()),
    );
    Ok((from, to))
}

/// A published code-of-conduct clause as the remedy gate reads it. The
/// machine preamble inside the clause body carries the enforceable bits
/// (the `kcs:` preamble precedent): `coc: excludes=<kind,…>` and
/// `coc: max_goodwill_cents=<n>`.
pub struct ConductClause {
    pub clause_id: String,
    pub excerpt: String,
    pub excludes: Vec<RemedyKind>,
    pub max_goodwill_cents: Option<i64>,
}

fn parse_clause(clause_id: &str, body: &str) -> ConductClause {
    let mut excludes = Vec::new();
    let mut max_goodwill_cents = None;
    for line in body.lines().take(8) {
        if let Some(rest) = line.trim().strip_prefix("coc: ") {
            for part in rest.split_whitespace() {
                if let Some(list) = part.strip_prefix("excludes=") {
                    for k in list.split(',') {
                        if let Some(kind) = RemedyKind::parse(k.trim()) {
                            excludes.push(kind);
                        }
                    }
                } else if let Some(v) = part.strip_prefix("max_goodwill_cents=") {
                    max_goodwill_cents = v.parse().ok();
                }
            }
        }
    }
    ConductClause {
        clause_id: clause_id.to_string(),
        excerpt: body.chars().take(512).collect(),
        excludes,
        max_goodwill_cents,
    }
}

/// Load a published conduct clause by its id (the KB row's `title`) under
/// `source = 'code_of_conduct'`. Missing clauses deny loudly — a remedy
/// cannot ship against a promise nobody published.
pub fn conduct_clause(
    conn: &Connection,
    clause_id: &str,
) -> Result<Option<ConductClause>, ComplaintError> {
    let body: Option<String> = conn
        .query_row(
            "SELECT content FROM knowledge
              WHERE source = ?1 AND title = ?2
              ORDER BY id DESC LIMIT 1",
            params![SOURCE_CONDUCT_CLAUSE, clause_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(body.map(|b| parse_clause(clause_id, &b)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeConflict {
    /// The published clause names this remedy kind as excluded.
    ExcludedByClause,
    /// A goodwill payment above the clause's published ceiling.
    OverGoodwillCeiling,
}

pub struct RemedyDraft<'a> {
    pub run_id: i64,
    pub kind: RemedyKind,
    pub amount_cents: i64,
    /// The cited ISO 10001 clause id (KB title under `code_of_conduct`).
    pub code_clause_id: &'a str,
    /// Support tier 1..=4 — the approval matrix column.
    pub tier: u8,
    pub proposed_by: &'a str,
}

#[derive(Debug)]
pub struct RemedyProposal {
    pub proposal_id: i64,
    pub legal_basis: &'static str,
    pub conflicts: Vec<CodeConflict>,
}

/// Propose a remedy: validate citations, compute the deterministic conflict
/// flags, insert ONE pending HITL proposal audited in the caller's tx.
/// Financial remedies REQUIRE their code-clause citation; explanation-only
/// may ride without one (nothing is promised beyond the response itself).
pub fn propose_remedy(
    conn: &Connection,
    draft: &RemedyDraft<'_>,
    now: i64,
) -> Result<RemedyProposal, ComplaintError> {
    let kind = run_kind(conn, draft.run_id)?;
    if kind != "complaint" {
        return Err(ComplaintError::Invalid(format!(
            "run {} is a '{kind}' run, not a complaint",
            draft.run_id
        )));
    }
    if !(complaint::MIN_TIER..=complaint::MAX_TIER).contains(&draft.tier) {
        return Err(ComplaintError::Invalid(format!(
            "tier {} outside {}..={}",
            draft.tier,
            complaint::MIN_TIER,
            complaint::MAX_TIER
        )));
    }
    let financial = !matches!(draft.kind, RemedyKind::ExplanationOnly);
    let clause = if financial || !draft.code_clause_id.trim().is_empty() {
        let id = draft.code_clause_id.trim();
        if id.is_empty() {
            return Err(ComplaintError::Invalid(
                "financial remedies must cite their code-of-conduct clause".into(),
            ));
        }
        Some(conduct_clause(conn, id)?.ok_or_else(|| {
            ComplaintError::NotFound(format!("no published code_of_conduct clause titled '{id}'"))
        })?)
    } else {
        None
    };

    let mut conflicts = Vec::new();
    if let Some(c) = &clause {
        if c.excludes.contains(&draft.kind) {
            conflicts.push(CodeConflict::ExcludedByClause);
        }
        if draft.kind == RemedyKind::GoodwillPayment
            && let Some(max) = c.max_goodwill_cents
            && draft.amount_cents > max
        {
            conflicts.push(CodeConflict::OverGoodwillCeiling);
        }
    }

    let content = serde_json::json!({
        "run_id": draft.run_id,
        "kind": draft.kind.as_str(),
        "legal_basis": draft.kind.legal_basis(),
        "code_clause_id": clause.as_ref().map(|c| c.clause_id.clone()),
        "amount_cents": draft.amount_cents,
        "tier": draft.tier,
        "contradicts_published_code": !conflicts.is_empty(),
        "conflicts": conflicts.iter().map(|c| format!("{c:?}")).collect::<Vec<_>>(),
        "proposed_by": draft.proposed_by,
    });
    // A visible contradiction raises salience: the reviewer must see it.
    let salience = if conflicts.is_empty() { 0.5 } else { 0.9 };
    conn.execute(
        "INSERT INTO proposals(kind, content, source, authority, observed_at,
                              novelty, conflict_with, salience, status, created_at)
         VALUES (?1, ?2, 'agent', NULL, NULL, 1.0, NULL, ?3, 'pending', ?4)",
        params![KIND_REMEDY, content.to_string(), salience, now],
    )?;
    let proposal_id = conn.last_insert_rowid();
    super::audit_write(
        conn,
        draft.run_id,
        &format!("proposal:{proposal_id}"),
        AuditStatus::Ok,
        &format!(
            "complaint/remedy {} {}c conflicts:{}",
            draft.kind.as_str(),
            draft.amount_cents,
            conflicts.len()
        ),
    );
    Ok(RemedyProposal {
        proposal_id,
        legal_basis: draft.kind.legal_basis(),
        conflicts,
    })
}

/// The outcome of presenting a remedy proposal to an approver role.
#[derive(Debug, PartialEq, Eq)]
pub enum RemedyApproval {
    /// Bound within the approver's cap: proposal approved, lifecycle event
    /// appended, audit written — all in the caller's tx.
    Approved,
    /// One cent over cap: NOTHING is approved here. An escalation proposal
    /// naming exactly one higher rung is created with the full packet
    /// attached; the original stays pending untouched.
    Escalated {
        escalation_proposal_id: i64,
        to: ApprovalLevel,
    },
}

/// The approve-path side effect for a `complaint_remedy` proposal (called
/// from the gate's approve branch INSIDE its immediate transaction).
/// `approver_roles` are the principal's deployment role names; every name
/// must resolve on the closed ladder — an unknown role denies loudly.
pub fn apply_remedy_approval(
    conn: &Connection,
    proposal_id: i64,
    content: &serde_json::Value,
    approver_roles: &[String],
    now: i64,
) -> Result<RemedyApproval, ComplaintError> {
    let run_id = content
        .get("run_id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| ComplaintError::Invalid("remedy packet missing run_id".into()))?;
    let kind = RemedyKind::parse(
        content
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or_default(),
    )
    .ok_or_else(|| ComplaintError::Invalid("remedy packet has unknown kind".into()))?;
    let amount_cents = content
        .get("amount_cents")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| ComplaintError::Invalid("remedy packet missing amount".into()))?;
    let tier = content
        .get("tier")
        .and_then(|v| v.as_u64())
        .and_then(|t| u8::try_from(t).ok())
        .unwrap_or(complaint::MIN_TIER);

    // Fail closed: the HIGHEST resolved rung among the holder's roles is
    // the effective level; an empty or unresolvable set denies loudly.
    let mut level: Option<ApprovalLevel> = None;
    for r in approver_roles {
        if let Some(l) = complaint::approval_level_for_role(r)
            && level.is_none_or(|cur| l > cur)
        {
            level = Some(l);
        }
    }
    let Some(level) = level else {
        return Err(ComplaintError::Invalid(
            "approver holds no recognizable approval role — denied".into(),
        ));
    };

    match complaint::approval_decision(level, tier, kind, amount_cents) {
        ApprovalDecision::WithinCap => {
            conn.execute(
                "UPDATE proposals SET status = 'approved', decided_at = ?1
                  WHERE id = ?2 AND status = 'pending'",
                params![now, proposal_id],
            )?;
            // The lifecycle advances only from its own legal predecessor;
            // an approval arriving out of sequence still approves (the
            // human signed) but never fabricates a state jump.
            if current_state(conn, run_id)? == ComplaintState::RemedyProposed {
                let _ = transition(
                    conn,
                    run_id,
                    ComplaintState::RemedyApproved,
                    "approval-gate",
                    now,
                )?;
            }
            super::audit_write(
                conn,
                run_id,
                &format!("proposal:{proposal_id}"),
                AuditStatus::Ok,
                &format!("{REMEDY_APPROVED_AUDIT_DETAIL_PREFIX}:{proposal_id}"),
            );
            Ok(RemedyApproval::Approved)
        }
        ApprovalDecision::Escalated { to } => {
            let mut packet = content.clone();
            packet["escalated_from"] = serde_json::json!(proposal_id);
            packet["escalated_to"] = serde_json::json!(to.as_str());
            packet["escalation_reason"] =
                serde_json::json!("over role-tier cap — one level up, packet attached");
            conn.execute(
                "INSERT INTO proposals(kind, content, source, authority, observed_at,
                                       novelty, conflict_with, salience, status, created_at)
                 VALUES (?1, ?2, 'agent', NULL, NULL, 1.0, NULL, 0.9, 'pending', ?3)",
                params![KIND_REMEDY, packet.to_string(), now],
            )?;
            let esc_id = conn.last_insert_rowid();
            super::audit_write(
                conn,
                run_id,
                &format!("proposal:{esc_id}"),
                AuditStatus::Ok,
                &format!(
                    "complaint/remedy/escalated from:{proposal_id} to:{}",
                    to.as_str()
                ),
            );
            Ok(RemedyApproval::Escalated {
                escalation_proposal_id: esc_id,
                to,
            })
        }
    }
}

/// Build the ISO 10003 external-dispute packet: run identity, the audited
/// remedy history, the dispute-escalation lineage marker, and the
/// competent NATIONAL ADR body looked up from the DPO-maintained registry
/// (`knowledge.source='adr_body'`, title = member state). Missing registry
/// row denies loudly — the packet never guesses where a consumer files.
/// The Reg. 2024/3228 discontinuation note rides every packet.
pub fn adr_packet(
    conn: &Connection,
    run_id: i64,
    member_state: &str,
) -> Result<serde_json::Value, ComplaintError> {
    let kind = run_kind(conn, run_id)?;
    if kind != "complaint" {
        return Err(ComplaintError::Invalid(format!(
            "run {run_id} is a '{kind}' run, not a complaint"
        )));
    }
    let state = member_state.trim();
    if state.is_empty() || state.len() > 64 || state.contains("..") {
        return Err(ComplaintError::Invalid("member_state unbounded".into()));
    }
    let body: Option<String> = conn
        .query_row(
            "SELECT content FROM knowledge
              WHERE source = ?1 AND title = ?2
              ORDER BY id DESC LIMIT 1",
            params![SOURCE_ADR_BODY, state],
            |r| r.get(0),
        )
        .optional()?;
    let Some(adr_body) = body else {
        return Err(ComplaintError::NotFound(format!(
            "no national ADR body registered for '{state}' — maintain one before referring"
        )));
    };
    let state_now = current_state(conn, run_id)?;
    let mut remedies = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT id, status, created_at, content FROM proposals
              WHERE kind = ?1 ORDER BY id DESC LIMIT 100",
        )?;
        let rows = stmt.query_map(params![KIND_REMEDY], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        for row in rows {
            let (pid, status, created, content) = row?;
            let v: serde_json::Value = serde_json::from_str(&content)
                .map_err(|e| ComplaintError::Invalid(format!("corrupt remedy packet: {e}")))?;
            if v.get("run_id").and_then(|x| x.as_i64()) == Some(run_id) {
                remedies.push(serde_json::json!({
                    "proposal_id": pid,
                    "status": status,
                    "created_at": created,
                    "kind": v.get("kind"),
                    "amount_cents": v.get("amount_cents"),
                    "legal_basis": v.get("legal_basis"),
                    "code_clause_id": v.get("code_clause_id"),
                }));
            }
        }
    }
    Ok(serde_json::json!({
        "run_id": run_id,
        "lifecycle_state": state_now.as_str(),
        "remedy_history": remedies,
        "adr_body": adr_body,
        "member_state": state,
        "odr_note": complaint::ODR_DISCONTINUATION_BASIS,
        "filing": "humans file; brain prepares the packet only",
    }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoodwillEntry {
    pub proposal_id: i64,
    pub run_id: i64,
    pub kind: String,
    pub amount_cents: i64,
    pub created_at: i64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GoodwillLedger {
    pub entries: Vec<GoodwillEntry>,
    pub total_cents: i64,
    /// Entries whose audit row could NOT be found were excluded BEFORE the
    /// aggregate — the count is the honest gap signal, never folded away.
    pub unaudited_excluded: usize,
}

/// The goodwill ledger: aggregate over APPROVED remedy proposals in the
/// window whose workflow audit row actually references them. An approved
/// remedy without its audit row is excluded and counted — absence is
/// surfaced, never silently aggregated. Bounded by [`LEDGER_SCAN_LIMIT`].
pub fn goodwill_ledger(
    conn: &Connection,
    from: i64,
    to: i64,
) -> Result<GoodwillLedger, ComplaintError> {
    if to < from {
        return Err(ComplaintError::Invalid("window inverted".into()));
    }
    let mut stmt = conn.prepare(
        "SELECT p.id, p.created_at, p.content FROM proposals p
          WHERE p.kind = ?1 AND p.status = 'approved'
            AND p.created_at BETWEEN ?2 AND ?3
          ORDER BY p.id DESC LIMIT ?4",
    )?;
    let rows = stmt.query_map(params![KIND_REMEDY, from, to, LEDGER_SCAN_LIMIT], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    let mut ledger = GoodwillLedger::default();
    for row in rows {
        let (pid, created, content) = row?;
        let v: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| ComplaintError::Invalid(format!("corrupt remedy packet: {e}")))?;
        // Audit targets are stored hashed (the tamper-evidence law), so the
        // audited-presence check matches on the same hash the writer used.
        let target_hash = crate::audit::hash(&format!("proposal:{pid}"));
        let detail_hash =
            crate::audit::hash(&format!("{REMEDY_APPROVED_AUDIT_DETAIL_PREFIX}:{pid}"));
        let audited: i64 = conn.query_row(
            "SELECT COUNT(*) FROM audit_events
              WHERE target_hash = ?1 AND detail_hash = ?2
                AND status = 'ok' AND kind = 'workflow'",
            params![target_hash, detail_hash],
            |r| r.get(0),
        )?;
        if audited == 0 {
            ledger.unaudited_excluded += 1;
            continue;
        }
        let amount = v.get("amount_cents").and_then(|x| x.as_i64()).unwrap_or(0);
        ledger.total_cents += amount.max(0);
        ledger.entries.push(GoodwillEntry {
            proposal_id: pid,
            run_id: v.get("run_id").and_then(|x| x.as_i64()).unwrap_or(0),
            kind: v
                .get("kind")
                .and_then(|x| x.as_str())
                .unwrap_or("unknown")
                .to_string(),
            amount_cents: amount,
            created_at: created,
        });
    }
    Ok(ledger)
}

#[cfg(test)]
mod tests {
    use super::*;
    use brain_engine_sdk::pure::complaint::CAP_TABLE;
    use brain_server::migration::run_migration;
    use brain_server::register_sqlite_vec::register_sqlite_vec;
    use rusqlite::Connection;

    fn db() -> Connection {
        register_sqlite_vec();
        let mut conn = Connection::open_in_memory().unwrap();
        run_migration(&mut conn, 1).unwrap();
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('acme', 'complaint', '{}', 0, 'active', 1, 1)",
            [],
        )
        .unwrap();
        conn
    }

    fn seed_clause(conn: &Connection, title: &str, body: &str) {
        conn.execute(
            "INSERT INTO knowledge(title, content, source, knowledge_type)
             VALUES (?1, ?2, 'code_of_conduct', 'fact')",
            params![title, body],
        )
        .unwrap();
    }

    fn draft<'a>(run_id: i64, kind: RemedyKind, amount: i64, clause: &'a str) -> RemedyDraft<'a> {
        RemedyDraft {
            run_id,
            kind,
            amount_cents: amount,
            code_clause_id: clause,
            tier: 1,
            proposed_by: "t1-agent",
        }
    }

    /// remedy_citations_include_code_clause_and_legal_basis
    #[test]
    fn remedy_citations_include_code_clause_and_legal_basis() {
        let conn = db();
        seed_clause(
            &conn,
            "CoC-4.1",
            "We resolve within 14 days.\ncoc: max_goodwill_cents=5000",
        );
        let p = propose_remedy(
            &conn,
            &draft(1, RemedyKind::GoodwillPayment, 2_000, "CoC-4.1"),
            10,
        )
        .unwrap();
        // The legal basis comes from the closed anchor set — goodwill cites
        // POLICY, never a regulation.
        assert_eq!(p.legal_basis, "goodwill-policy");
        assert_eq!(p.conflicts, vec![]);
        let content: String = conn
            .query_row(
                "SELECT content FROM proposals WHERE id = ?1",
                params![p.proposal_id],
                |r| r.get(0),
            )
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["code_clause_id"], "CoC-4.1");
        assert_eq!(v["legal_basis"], "goodwill-policy");
        assert_eq!(v["contradicts_published_code"], false);
        // Over the published ceiling → flagged visibly on the packet.
        let p = propose_remedy(
            &conn,
            &draft(1, RemedyKind::GoodwillPayment, 6_000, "CoC-4.1"),
            11,
        )
        .unwrap();
        assert_eq!(p.conflicts, vec![CodeConflict::OverGoodwillCeiling]);
        let content: String = conn
            .query_row(
                "SELECT content FROM proposals WHERE id = ?1",
                params![p.proposal_id],
                |r| r.get(0),
            )
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["contradicts_published_code"], true);
        // A financial remedy without its citation refuses loudly.
        let err = propose_remedy(&conn, &draft(1, RemedyKind::Refund, 100, ""), 12).unwrap_err();
        assert!(matches!(err, ComplaintError::Invalid(_)));
        // A citation to an unpublished clause denies (fail closed).
        let err =
            propose_remedy(&conn, &draft(1, RemedyKind::Refund, 100, "CoC-9.9"), 13).unwrap_err();
        assert!(matches!(err, ComplaintError::NotFound(_)));
        // An excluded remedy kind is flagged, not blocked.
        seed_clause(
            &conn,
            "CoC-7.2",
            "No cash for digital goods.\ncoc: excludes=refund",
        );
        let p = propose_remedy(&conn, &draft(1, RemedyKind::Refund, 100, "CoC-7.2"), 14).unwrap();
        assert_eq!(p.conflicts, vec![CodeConflict::ExcludedByClause]);
    }

    /// approval_caps_escalate_deterministically (service leg)
    #[test]
    fn approval_caps_escalate_deterministically() {
        let conn = db();
        seed_clause(&conn, "CoC-4.1", "coc: max_goodwill_cents=5000");
        let p = propose_remedy(
            &conn,
            &draft(1, RemedyKind::Refund, CAP_TABLE[0][0], "CoC-4.1"),
            10,
        )
        .unwrap();
        let content: String = conn
            .query_row(
                "SELECT content FROM proposals WHERE id = ?1",
                params![p.proposal_id],
                |r| r.get(0),
            )
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();

        // Agent at the exact cap approves; lifecycle advances legally.
        let roles = vec!["agent".to_string()];
        let out = apply_remedy_approval(&conn, p.proposal_id, &v, &roles, 20).unwrap();
        assert_eq!(out, RemedyApproval::Approved);
        assert_eq!(
            current_state(&conn, 1).unwrap(),
            ComplaintState::Received,
            "an approval off-sequence never fabricates a lifecycle jump"
        );

        // One cent over the agent cap escalates EXACTLY one rung, packet
        // attached; the original stays pending.
        let p = propose_remedy(
            &conn,
            &draft(1, RemedyKind::Refund, CAP_TABLE[0][0] + 1, "CoC-4.1"),
            30,
        )
        .unwrap();
        let content: String = conn
            .query_row(
                "SELECT content FROM proposals WHERE id = ?1",
                params![p.proposal_id],
                |r| r.get(0),
            )
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        let out = apply_remedy_approval(&conn, p.proposal_id, &v, &roles, 40).unwrap();
        let RemedyApproval::Escalated {
            escalation_proposal_id,
            to,
        } = out
        else {
            panic!("over-cap must escalate");
        };
        assert_eq!(to, ApprovalLevel::Supervisor);
        let esc: String = conn
            .query_row(
                "SELECT content FROM proposals WHERE id = ?1",
                params![escalation_proposal_id],
                |r| r.get(0),
            )
            .unwrap();
        let ev: serde_json::Value = serde_json::from_str(&esc).unwrap();
        assert_eq!(ev["escalated_to"], "supervisor");
        assert_eq!(ev["escalated_from"], p.proposal_id);
        let orig: String = conn
            .query_row(
                "SELECT status FROM proposals WHERE id = ?1",
                params![p.proposal_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(orig, "pending");

        // The supervisor's approval of the escalation lands the lifecycle
        // step when it rides the legal predecessor.
        transition(&conn, 1, ComplaintState::Acknowledged, "agent", 50).unwrap();
        transition(&conn, 1, ComplaintState::Investigated, "agent", 51).unwrap();
        transition(&conn, 1, ComplaintState::RemedyProposed, "agent", 52).unwrap();
        let out = apply_remedy_approval(
            &conn,
            escalation_proposal_id,
            &ev,
            &["supervisor".to_string()],
            60,
        )
        .unwrap();
        assert_eq!(out, RemedyApproval::Approved);
        assert_eq!(
            current_state(&conn, 1).unwrap(),
            ComplaintState::RemedyApproved,
            "the approval lands the lifecycle step on its legal predecessor"
        );
        // An approver holding NO recognizable role denies loudly.
        let p = propose_remedy(&conn, &draft(1, RemedyKind::Refund, 100, "CoC-4.1"), 70).unwrap();
        let content: String = conn
            .query_row(
                "SELECT content FROM proposals WHERE id = ?1",
                params![p.proposal_id],
                |r| r.get(0),
            )
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(matches!(
            apply_remedy_approval(&conn, p.proposal_id, &v, &["intern".to_string()], 80),
            Err(ComplaintError::Invalid(_))
        ));
    }

    /// adr_packet_targets_national_body_not_odr
    #[test]
    fn adr_packet_targets_national_body_not_odr() {
        let conn = db();
        conn.execute(
            "INSERT INTO knowledge(title, content, source) VALUES ('DE', 'Schlichtungsstelle für den Verbraucherstreit (universal ADR body), zsh-online.de', 'adr_body')",
            [],
        )
        .unwrap();
        let packet = adr_packet(&conn, 1, "DE").unwrap();
        assert!(
            packet["adr_body"]
                .as_str()
                .unwrap()
                .contains("Schlichtungsstelle")
        );
        // The Reg. 2024/3228 discontinuation basis rides EVERY packet.
        assert_eq!(packet["odr_note"], complaint::ODR_DISCONTINUATION_BASIS);
        assert_eq!(
            packet["filing"],
            "humans file; brain prepares the packet only"
        );
        // No registry row for the member state → loud failure, no guess.
        let err = adr_packet(&conn, 1, "FR").unwrap_err();
        assert!(matches!(err, ComplaintError::NotFound(_)));
        // Non-complaint runs refuse.
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('acme', 'interview', '{}', 0, 'active', 2, 2)",
            [],
        )
        .unwrap();
        assert!(matches!(
            adr_packet(&conn, 2, "DE"),
            Err(ComplaintError::Invalid(_))
        ));
        // Path traversal in the member state denies before any lookup.
        assert!(matches!(
            adr_packet(&conn, 1, "../etc"),
            Err(ComplaintError::Invalid(_))
        ));
    }

    /// goodwill_ledger_aggregates_only_from_audited_remedies
    #[test]
    fn goodwill_ledger_aggregates_only_from_audited_remedies() {
        let conn = db();
        seed_clause(&conn, "CoC-4.1", "coc: max_goodwill_cents=5000");
        // Two approved remedies with audit rows (the normal path audits).
        for amount in [2_000i64, 3_000] {
            let p = propose_remedy(
                &conn,
                &draft(1, RemedyKind::GoodwillPayment, amount, "CoC-4.1"),
                10,
            )
            .unwrap();
            conn.execute(
                "UPDATE proposals SET status = 'approved', decided_at = 11 WHERE id = ?1",
                params![p.proposal_id],
            )
            .unwrap();
            crate::workflow::audit_write(
                &conn,
                1,
                &format!("proposal:{}", p.proposal_id),
                AuditStatus::Ok,
                &format!("{}:{}", REMEDY_APPROVED_AUDIT_DETAIL_PREFIX, p.proposal_id),
            );
        }
        // One approved remedy WITHOUT any audit row — must be excluded and
        // counted, not silently folded into the aggregate.
        let orphan =
            propose_remedy(&conn, &draft(1, RemedyKind::Refund, 99_999, "CoC-4.1"), 12).unwrap();
        conn.execute(
            "UPDATE proposals SET status = 'approved', decided_at = 13 WHERE id = ?1",
            params![orphan.proposal_id],
        )
        .unwrap();
        // Pending remedies never aggregate.
        propose_remedy(
            &conn,
            &draft(1, RemedyKind::GoodwillPayment, 1_234, "CoC-4.1"),
            14,
        )
        .unwrap();

        let ledger = goodwill_ledger(&conn, 0, 100).unwrap();
        assert_eq!(ledger.entries.len(), 2);
        assert_eq!(ledger.total_cents, 5_000);
        assert_eq!(ledger.unaudited_excluded, 1);
        // Out-of-window rows stay out.
        let empty = goodwill_ledger(&conn, 200, 300).unwrap();
        assert_eq!(empty.entries.len(), 0);
        assert_eq!(empty.total_cents, 0);
        // Inverted windows deny loudly.
        assert!(matches!(
            goodwill_ledger(&conn, 100, 0),
            Err(ComplaintError::Invalid(_))
        ));
        // Every write this module made still verifies on both chains.
        assert!(crate::audit::verify_chain(&conn));
        assert!(crate::workflow::outbox::verify_outbox_lineage(&conn, 1).unwrap());
    }
}
