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
pub(crate) const REMEDY_APPROVED_AUDIT_DETAIL_PREFIX: &str = "complaint/remedy/approved";

/// Proposal kinds owned by this module.
pub(crate) const KIND_REMEDY: &str = "complaint_remedy";
pub(crate) const KIND_RCA: &str = "complaint_rca";

/// The lineage topic every complaint lifecycle event rides.
pub(crate) const TOPIC_COMPLAINT: &str = "workflow/complaint";

/// The knowledge `source` values this module reads (DPO/team-maintained
/// through the ordinary governed knowledge write path — screened at write,
/// sanitized at read).
pub(crate) const SOURCE_CONDUCT_CLAUSE: &str = "code_of_conduct";
pub(crate) const SOURCE_ADR_BODY: &str = "adr_body";

/// Read bound for the ledger scan (the bounds law; pinned by test below).
pub(crate) const LEDGER_SCAN_LIMIT: i64 = 1_000;

#[derive(Debug)]
pub(crate) enum ComplaintError {
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
pub(crate) fn current_state(
    conn: &Connection,
    run_id: i64,
) -> Result<ComplaintState, ComplaintError> {
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
    to.map_or(Ok(ComplaintState::Received), |t| {
        ComplaintState::parse(&t)
            .ok_or_else(|| ComplaintError::Invalid(format!("corrupt lifecycle state '{t}'")))
    })
}

/// Advance the lifecycle one legal step. Fails closed on any transition the
/// closed SDK table does not name, or on a non-complaint run. Lineage event
/// + audit row land in the caller's transaction; nothing here commits.
pub(crate) fn transition(
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
    // The confirm-gate (the doctrine amendment, wired into the lifecycle):
    // terminal closure requires a customer confirmation on the lineage or
    // the documented three-attempt exception. Silence never certifies.
    if to == ComplaintState::Closed {
        let decision = super::frontdesk::evaluate_close(&close_gate_events(conn, run_id)?);
        if decision == super::frontdesk::CloseDecision::RemainOpen {
            return Err(ComplaintError::Invalid(
                "closure requires customer confirmation or the documented \
                 three-attempt exception — complaint stays open"
                    .into(),
            ));
        }
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
pub(crate) struct ConductClause {
    pub(crate) clause_id: String,
    pub(crate) excerpt: String,
    pub(crate) excludes: Vec<RemedyKind>,
    pub(crate) max_goodwill_cents: Option<i64>,
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
pub(crate) fn conduct_clause(
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
pub(crate) enum CodeConflict {
    /// The published clause names this remedy kind as excluded.
    ExcludedByClause,
    /// A goodwill payment above the clause's published ceiling.
    OverGoodwillCeiling,
}

pub(crate) struct RemedyDraft<'a> {
    pub(crate) run_id: i64,
    pub(crate) kind: RemedyKind,
    pub(crate) amount_cents: i64,
    /// The cited ISO 10001 clause id (KB title under `code_of_conduct`).
    pub(crate) code_clause_id: &'a str,
    /// Support tier 1..=4 — the approval matrix column.
    pub(crate) tier: u8,
    pub(crate) proposed_by: &'a str,
}

#[derive(Debug)]
pub(crate) struct RemedyProposal {
    pub(crate) proposal_id: i64,
    pub(crate) legal_basis: &'static str,
    pub(crate) conflicts: Vec<CodeConflict>,
}

/// Propose a remedy: validate citations, compute the deterministic conflict
/// flags, insert ONE pending HITL proposal audited in the caller's tx.
/// Financial remedies REQUIRE their code-clause citation; explanation-only
/// may ride without one (nothing is promised beyond the response itself).
pub(crate) fn propose_remedy(
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
pub(crate) enum RemedyApproval {
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
pub(crate) fn apply_remedy_approval(
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
pub(crate) fn adr_packet(
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

/// The published complaints policy's `knowledge.source` value — owned by
/// the KB module (`crate::kb::COMPLAINT_POLICY_SOURCE`), re-exported here
/// for the lifecycle core's readers.
///
/// The alert-bus topic for a missed acknowledgment deadline. Rides the
/// `workflow/*` coordinate space, so the existing event worker drains it.
pub(crate) const TOPIC_ACK_OVERDUE: &str = "workflow/complaint/ack_overdue";

/// Read bound for one ack sweep (the bounds law).
pub(crate) const ACK_SWEEP_LIMIT: i64 = 500;

/// The acknowledgment deadline for a complaint born at `created_at` —
/// the SDK's ISO 10002 clock (one hour), the single owner of the number.
pub(crate) fn ack_deadline(created_at: i64) -> i64 {
    created_at + brain_engine_sdk::policy::COMPLAINT_ACK_SECS
}

/// Acknowledge the complaint: the legal lifecycle step with its dedicated
/// audit marker (`complaint/ack`) so the register can measure attainment.
pub(crate) fn acknowledge(
    conn: &Connection,
    run_id: i64,
    actor: &str,
    now: i64,
) -> Result<(ComplaintState, ComplaintState), ComplaintError> {
    let actor = actor.trim();
    if actor.is_empty() || actor.len() > super::relay::MAX_PRINCIPAL_LEN {
        return Err(ComplaintError::Invalid("actor unbounded".into()));
    }
    let res = transition(conn, run_id, ComplaintState::Acknowledged, actor, now)?;
    super::audit_write(
        conn,
        run_id,
        &format!("run:{run_id}"),
        AuditStatus::Ok,
        &format!("complaint/ack at:{now}"),
    );
    Ok(res)
}

/// One overdue-ack sweep over ACTIVE complaint runs past their deadline
/// whose lineage shows no acknowledgment yet: exactly one alert event per
/// run per lifetime (idempotency key), each audited INSIDE the caller's
/// transaction, bounded by [`ACK_SWEEP_LIMIT`]. Returns the alerted ids.
pub(crate) fn ack_sweep(conn: &Connection, now: i64) -> Result<Vec<i64>, ComplaintError> {
    // Overdue iff the ack clock has run out: created_at + ACK_SECS <= now.
    let cutoff = now - brain_engine_sdk::policy::COMPLAINT_ACK_SECS;
    let mut stmt = conn.prepare(
        "SELECT r.id FROM workflow_runs r
          WHERE r.kind = 'complaint' AND r.status = 'active' AND r.created_at <= ?1
          ORDER BY r.id LIMIT ?2",
    )?;
    let ids = stmt
        .query_map(params![cutoff, ACK_SWEEP_LIMIT], |r| r.get::<_, i64>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let mut alerted = Vec::new();
    for id in ids {
        if current_state(conn, id)? != ComplaintState::Received {
            continue;
        }
        let payload = serde_json::json!({
            "run_id": id,
            "deadline": ack_deadline(now),
            "signal": "ack_overdue",
        })
        .to_string();
        let parent: Option<i64> = conn
            .query_row(
                "SELECT MAX(id) FROM outbox WHERE run_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        let (inserted, _) = super::outbox::enqueue_child(
            conn,
            id,
            parent,
            TOPIC_ACK_OVERDUE,
            &payload,
            &format!("ack_overdue:{id}"),
            now,
        )?;
        if inserted {
            // enqueue_child already wrote the audit row inside the caller's
            // tx (a replayed enqueue is deliberately not audited).
            alerted.push(id);
        }
    }
    Ok(alerted)
}

/// The confirm-gate inputs read straight off the run's lineage: the topics
/// frontdesk::evaluate_close arbitrates on.
fn close_gate_events(
    conn: &Connection,
    run_id: i64,
) -> Result<Vec<super::frontdesk::LineageEvent>, ComplaintError> {
    use super::frontdesk::LineageEvent;
    let mut stmt = conn.prepare(
        "SELECT topic FROM outbox
          WHERE run_id = ?1 AND topic IN (?2, ?3)
          ORDER BY id",
    )?;
    let rows = stmt
        .query_map(
            params![
                run_id,
                super::frontdesk::TOPIC_CUSTOMER_CONFIRMATION,
                super::frontdesk::TOPIC_CLOSE_ATTEMPT
            ],
            |r| r.get::<_, String>(0),
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .map(|t| LineageEvent::new(&t, ""))
        .collect())
}

#[derive(Debug, Default)]
pub(crate) struct MonthlyReport {
    pub(crate) total: i64,
    /// Count per terminal lifecycle state (the disposition mix).
    pub(crate) by_state: Vec<(String, i64)>,
    pub(crate) ack_in_sla: i64,
    pub(crate) ack_total: i64,
    pub(crate) adr_referrals: i64,
}

impl MonthlyReport {
    /// The register extract as the JSON payload the signed calibration row
    /// carries (deterministic field order for a stable audit detail).
    pub(crate) fn to_json_value(&self) -> serde_json::Value {
        let by_state: serde_json::Map<String, serde_json::Value> = self
            .by_state
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::json!(v)))
            .collect();
        serde_json::json!({
            "total": self.total,
            "by_state": by_state,
            "ack_in_sla": self.ack_in_sla,
            "ack_total": self.ack_total,
            "adr_referrals": self.adr_referrals,
        })
    }
}

/// The monthly complaints register extract (ISO 10002 continual-improvement
/// stage): counts by terminal disposition, acknowledgment-SLA attainment,
/// and ADR referrals — computed deterministically from the audit-chain
/// lineage over [from, to]. This is what the signed monthly calibration
/// row carries; there is no parallel complaint database.
pub(crate) fn monthly_report(
    conn: &Connection,
    from: i64,
    to: i64,
) -> Result<serde_json::Value, ComplaintError> {
    if to < from {
        return Err(ComplaintError::Invalid("window inverted".into()));
    }
    let mut stmt = conn.prepare(
        "SELECT id, created_at FROM workflow_runs
          WHERE kind = 'complaint' AND created_at BETWEEN ?1 AND ?2
          ORDER BY id LIMIT ?3",
    )?;
    let runs = stmt
        .query_map(params![from, to, LEDGER_SCAN_LIMIT], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut report = MonthlyReport {
        total: 0,
        by_state: Vec::new(),
        ack_in_sla: 0,
        ack_total: 0,
        adr_referrals: 0,
    };
    for (run_id, created_at) in runs {
        report.total += 1;
        // Terminal disposition = the furthest state reached on the lineage.
        let mut reached: Option<ComplaintState> = None;
        {
            let mut stmt = conn.prepare(
                "SELECT payload_json FROM outbox
                  WHERE run_id = ?1 AND topic = ?2 ORDER BY id",
            )?;
            let payloads = stmt
                .query_map(params![run_id, TOPIC_COMPLAINT], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            for p in payloads {
                if let Some(to) = serde_json::from_str::<serde_json::Value>(&p)
                    .ok()
                    .and_then(|v| v.get("to").and_then(|t| t.as_str()).map(String::from))
                    && let Some(s) = ComplaintState::parse(&to)
                {
                    reached = Some(s);
                }
            }
        }
        if let Some(s) = reached {
            let name = s.as_str().to_string();
            if let Some((_, count)) = report.by_state.iter_mut().find(|(k, _)| *k == name) {
                *count += 1;
            } else {
                report.by_state.push((name, 1));
            }
            if s == ComplaintState::AdrReferred {
                report.adr_referrals += 1;
            }
        }
        // Ack-SLA attainment from the same lineage events.
        let acked_at: Option<i64> = {
            let mut stmt = conn.prepare(
                "SELECT created_at FROM outbox
                  WHERE run_id = ?1 AND topic = ?2 AND payload_json LIKE '%\"to\":\"acknowledged\"%'
                  ORDER BY id LIMIT 1",
            )?;
            stmt.query_row(params![run_id, TOPIC_COMPLAINT], |r| r.get(0))
                .optional()?
        };
        if let Some(at) = acked_at {
            report.ack_total += 1;
            if at <= ack_deadline(created_at) {
                report.ack_in_sla += 1;
            }
        }
    }
    Ok(report.to_json_value())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GoodwillEntry {
    pub(crate) proposal_id: i64,
    pub(crate) run_id: i64,
    pub(crate) kind: String,
    pub(crate) amount_cents: i64,
    pub(crate) created_at: i64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct GoodwillLedger {
    pub(crate) entries: Vec<GoodwillEntry>,
    pub(crate) total_cents: i64,
    /// Entries whose audit row could NOT be found were excluded BEFORE the
    /// aggregate — the count is the honest gap signal, never folded away.
    pub(crate) unaudited_excluded: usize,
}

/// The goodwill ledger: aggregate over APPROVED remedy proposals in the
/// window whose workflow audit row actually references them. An approved
/// remedy without its audit row is excluded and counted — absence is
/// surfaced, never silently aggregated. Bounded by [`LEDGER_SCAN_LIMIT`].
pub(crate) fn goodwill_ledger(
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

    // ── Ack SLA, confirm-gate closure, and the monthly register.

    /// ack_deadline_alerts_and_audits
    #[test]
    fn ack_deadline_alerts_and_audits() {
        let conn = db();
        // Born at t=1; the ack clock is one hour.
        assert_eq!(
            ack_deadline(1),
            1 + brain_engine_sdk::policy::COMPLAINT_ACK_SECS
        );
        // Before the deadline: no alerts.
        assert!(ack_sweep(&conn, 100).unwrap().is_empty());
        // Past the deadline with NO ack event on the lineage: exactly one
        // alert per overdue run, audited, riding the alert bus topic space.
        let alerted = ack_sweep(&conn, 4_000).unwrap();
        assert_eq!(alerted, vec![1]);
        let (topic, payload): (String, String) = conn
            .query_row(
                "SELECT topic, payload_json FROM outbox WHERE idempotency_key = 'ack_overdue:1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert!(
            topic.starts_with("workflow/"),
            "the alert bus drains workflow/*"
        );
        assert!(topic.contains("ack_overdue"));
        assert!(!payload.is_empty());
        // The alert is audited in the same tx posture (one audit row).
        let audits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE kind='workflow'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(audits, 1);
        // Idempotent: a second sweep does not re-alert.
        assert!(ack_sweep(&conn, 5_000).unwrap().is_empty());
        // An acknowledged complaint never alerts again.
        acknowledge(&conn, 1, "agent-9", 6_000).unwrap();
        assert_eq!(
            current_state(&conn, 1).unwrap(),
            ComplaintState::Acknowledged
        );
        // A non-complaint run past any clock is ignored by the sweep.
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('acme', 'interview', '{}', 0, 'active', 1, 1)",
            [],
        )
        .unwrap();
        assert!(ack_sweep(&conn, 99_000).unwrap().is_empty());
        assert!(crate::audit::verify_chain(&conn));
        assert!(crate::workflow::outbox::verify_outbox_lineage(&conn, 1).unwrap());
    }

    /// complaint_closure_requires_confirm_gate
    #[test]
    fn complaint_closure_requires_confirm_gate() {
        use crate::workflow::frontdesk::TOPIC_CLOSE_ATTEMPT;
        let conn = db();
        walk_to_remedy_approved(&conn, 1);
        // Silence never certifies: closure without confirmation refuses.
        let err = transition(&conn, 1, ComplaintState::Closed, "agent", 900).unwrap_err();
        assert!(matches!(err, ComplaintError::Invalid(m) if m.contains("confirmation")));
        // Two attempts are not the exception either.
        for t in [910i64, 920] {
            crate::workflow::outbox::append_lineage(
                &conn,
                1,
                TOPIC_CLOSE_ATTEMPT,
                "{\"channel\":\"email\"}",
                &format!("attempt:{t}"),
                t,
            )
            .unwrap();
        }
        assert!(matches!(
            transition(&conn, 1, ComplaintState::Closed, "agent", 930),
            Err(ComplaintError::Invalid(_))
        ));
        // Three documented attempts satisfy the consent-absent exception.
        crate::workflow::outbox::append_lineage(
            &conn,
            1,
            TOPIC_CLOSE_ATTEMPT,
            "{\"channel\":\"call\"}",
            "attempt:940",
            940,
        )
        .unwrap();
        transition(&conn, 1, ComplaintState::Closed, "agent", 950).unwrap();
        assert_eq!(current_state(&conn, 1).unwrap(), ComplaintState::Closed);
        // And a fresh run closes on a customer confirmation alone.
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('acme', 'complaint', '{}', 0, 'active', 2, 2)",
            [],
        )
        .unwrap();
        walk_to_remedy_approved(&conn, 2);
        crate::workflow::outbox::append_lineage(
            &conn,
            2,
            crate::workflow::frontdesk::TOPIC_CUSTOMER_CONFIRMATION,
            "{}",
            "confirm:2",
            1_500,
        )
        .unwrap();
        transition(&conn, 2, ComplaintState::Closed, "agent", 1_600).unwrap();
        assert!(crate::audit::verify_chain(&conn));
    }

    /// Walk run `run_id` received → remedy_approved through legal steps.
    fn walk_to_remedy_approved(conn: &Connection, run_id: i64) {
        transition(conn, run_id, ComplaintState::Acknowledged, "agent", 10).unwrap();
        transition(conn, run_id, ComplaintState::Investigated, "agent", 20).unwrap();
        transition(conn, run_id, ComplaintState::RemedyProposed, "agent", 30).unwrap();
        transition(conn, run_id, ComplaintState::RemedyApproved, "agent", 40).unwrap();
    }

    /// complaint_register_report_joins_monthly_calibration
    #[test]
    fn complaint_register_report_joins_monthly_calibration() {
        let conn = db();
        // Run 1: acknowledged within the hour, confirmed by the customer,
        // closed.
        walk_to_remedy_approved(&conn, 1);
        crate::workflow::outbox::append_lineage(
            &conn,
            1,
            crate::workflow::frontdesk::TOPIC_CUSTOMER_CONFIRMATION,
            "{}",
            "confirm:1",
            45,
        )
        .unwrap();
        transition(&conn, 1, ComplaintState::Closed, "agent", 50).unwrap();
        // Run 2: acknowledged LATE (past the ack deadline), ADR-referred.
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, state_revision, status, created_at, updated_at)
             VALUES ('acme', 'complaint', '{}', 0, 'active', 100, 100)",
            [],
        )
        .unwrap();
        transition(&conn, 2, ComplaintState::Acknowledged, "agent", 100 + 7_200).unwrap();
        transition(&conn, 2, ComplaintState::Investigated, "agent", 100 + 8_000).unwrap();
        transition(
            &conn,
            2,
            ComplaintState::RemedyProposed,
            "agent",
            100 + 9_000,
        )
        .unwrap();
        transition(
            &conn,
            2,
            ComplaintState::RemedyApproved,
            "agent",
            100 + 9_500,
        )
        .unwrap();
        crate::workflow::outbox::append_lineage(
            &conn,
            2,
            crate::workflow::frontdesk::TOPIC_CUSTOMER_CONFIRMATION,
            "{}",
            "confirm:2",
            100 + 9_550,
        )
        .unwrap();
        transition(&conn, 2, ComplaintState::Closed, "agent", 100 + 9_600).unwrap();
        transition(&conn, 2, ComplaintState::AdrReferred, "agent", 100 + 9_700).unwrap();

        let report = monthly_report(&conn, 0, 200_000).unwrap();
        assert_eq!(report["total"], 2);
        // Dispositions from the register's terminal states.
        assert_eq!(report["by_state"]["closed"], 1);
        assert_eq!(report["by_state"]["adr_referred"], 1);
        // Ack-SLA attainment: run 1 in time, run 2 late → 1/2.
        assert_eq!(report["ack_in_sla"], 1);
        assert_eq!(report["ack_total"], 2);
        assert_eq!(report["adr_referrals"], 1);

        // The signed monthly calibration carries the register extract in its
        // SAME audited row (joined, not parallel machinery).
        crate::workflow::calibration::record_signed(
            &conn,
            8_000,
            8_100,
            "dpo-1",
            300_000,
            &serde_json::to_string(&report).unwrap(),
        )
        .unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE kind='workflow' AND target_hash = ?1",
                [crate::audit::hash("calibration/sign")],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "one signature row carries both payloads");
        // Inverted windows deny loudly.
        assert!(matches!(
            monthly_report(&conn, 100, 0),
            Err(ComplaintError::Invalid(_))
        ));
        assert!(crate::audit::verify_chain(&conn));
    }

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

    fn draft(run_id: i64, kind: RemedyKind, amount: i64, clause: &str) -> RemedyDraft<'_> {
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
