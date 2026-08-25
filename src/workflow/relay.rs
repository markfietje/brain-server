//! The one-click handover (Relay): offer / accept / decline over the
//! I-PASS packet the Lineage release already assembles. Storage projections
//! plus pure predicates — no engine logic; the state stays engine-opaque.
//!
//! Laws this module encodes:
//! - An OFFER refuses unless the packet is complete (the five gate
//!   predicates); the refusal carries the MISSING list — the machine
//!   coaches the protocol, the human fixes the packet.
//! - Offer/accept/decline are LINEAGE events (parent-linked outbox rows)
//!   audited in the same transaction as their state change.
//! - ACCEPT transfers ownership by CAS (state `owner`), never touches the
//!   SLA clock, and answers with the resume-at checkpoint.
//! - DECLINE requires a screened reason — an audited refusal beats a
//!   silent bounce.

use crate::audit::AuditStatus;
use rusqlite::{Connection, OptionalExtension, params};

use super::audit_write;
use super::frontdoor::DISPUTE_REASON;
use super::outbox;

/// Closed vocabulary for an offer's lifecycle.
pub const OFFERED: &str = "offered";
pub const ACCEPTED: &str = "accepted";
pub const DECLINED: &str = "declined";

/// The lineage topic every handover event rides.
pub const TOPIC: &str = "workflow/handover";

/// Identity bounds mirror crew presence (`principal` ≤ 256).
pub const MAX_PRINCIPAL_LEN: usize = 256;

/// One packet-completeness predicate: the five questions the receiving team
/// must be able to answer from the I-PASS fields, as facts.
pub struct PacketFacts {
    pub pending_question: Option<String>,
    pub sla_deadline: Option<i64>,
    pub now: i64,
    pub has_current_step: bool,
    pub has_evidence: bool,
    pub escalation_honored: bool,
}

/// The missing list for an incomplete packet. Empty = complete = offerable.
/// Each entry names the unanswered question, not a field index — the list is
/// the coaching surface.
pub fn packet_missing(f: &PacketFacts) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if f.pending_question
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
    {
        missing.push("situation: no open question recorded");
    }
    match f.sla_deadline {
        None => missing.push("safety: no SLA deadline stamped"),
        Some(d) if d <= f.now => missing.push("safety: SLA already breached"),
        Some(_) => {}
    }
    if !f.has_current_step {
        missing.push("action: no current step or event recorded");
    }
    if !f.has_evidence {
        missing.push("situation: no evidence linked (no checkpoint)");
    }
    if !f.escalation_honored {
        missing.push("safety: escalation state unresolved");
    }
    missing
}

#[derive(Debug)]
pub enum OfferError {
    Missing(String),
    Database(String),
}

impl From<rusqlite::Error> for OfferError {
    fn from(e: rusqlite::Error) -> Self {
        OfferError::Database(e.to_string())
    }
}

pub struct OfferDraft<'a> {
    pub domain: &'a str,
    pub run_id: i64,
    pub from_principal: &'a str,
    pub to_principal: &'a str,
    pub overlap_minutes: i64,
    pub sla_deadline: i64,
    pub now: i64,
}

/// The audit detail for an escalation-to-dispute (ISO 10003 handover of a
/// complaint). The complaints register IS the audit chain — this row is it.
pub const DISPUTE_AUDIT_DETAIL: &str = "handover/dispute";

/// Insert an offer row + its lineage event + its audit row. The caller owns
/// the surrounding transaction; nothing here commits. Idempotent by
/// `(run_id, offered-state)` key so a retried POST cannot double-offer.
pub fn insert_offer(conn: &Connection, draft: &OfferDraft<'_>) -> Result<(i64, bool), OfferError> {
    insert_offer_event(conn, draft, "offer", "handover/offer")
}

/// Escalate a complaint to dispute: the SAME documented handover machinery
/// (offer row + lineage event) audited with reason `handover/dispute` in the
/// caller's transaction — the dispute register entry.
pub fn record_dispute_escalation(
    conn: &Connection,
    draft: &OfferDraft<'_>,
) -> Result<(i64, bool), OfferError> {
    insert_offer_event(conn, draft, DISPUTE_REASON, DISPUTE_AUDIT_DETAIL)
}

fn insert_offer_event(
    conn: &Connection,
    draft: &OfferDraft<'_>,
    action: &str,
    audit_detail: &str,
) -> Result<(i64, bool), OfferError> {
    let open: Option<i64> = conn
        .query_row(
            "SELECT id FROM handover_offers
              WHERE run_id = ?1 AND state = ?2 ORDER BY id DESC LIMIT 1",
            params![draft.run_id, OFFERED],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(existing) = open {
        return Ok((existing, false));
    }
    conn.execute(
        "INSERT INTO handover_offers(domain, run_id, from_principal, to_principal,
             state, reason, overlap_minutes, sla_deadline, created_at, decided_at)
         VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7, ?8, NULL)",
        params![
            draft.domain,
            draft.run_id,
            draft.from_principal,
            draft.to_principal,
            OFFERED,
            draft.overlap_minutes,
            draft.sla_deadline,
            draft.now
        ],
    )?;
    let id = conn.last_insert_rowid();
    emit_event(
        conn,
        draft.run_id,
        &format!("handover:{id}:{action}"),
        &serde_json::json!({
            "action": action,
            "offer_id": id,
            "to": draft.to_principal,
        })
        .to_string(),
        draft.now,
    )?;
    audit_write(
        conn,
        draft.run_id,
        &format!("offer:{id}"),
        AuditStatus::Ok,
        audit_detail,
    );
    Ok((id, true))
}

/// Resolve an open offer as accepted or declined. Declines REQUIRE a reason.
/// Returns the offer id when a row actually moved (`false` = replayed
/// decision — idempotent receipt). The caller owns the tx.
pub fn decide_offer(
    conn: &Connection,
    run_id: i64,
    offer_id: i64,
    accept: bool,
    reason: Option<&str>,
    now: i64,
) -> Result<bool, OfferError> {
    let state: Option<String> = conn
        .query_row(
            "SELECT state FROM handover_offers WHERE id = ?1 AND run_id = ?2",
            params![offer_id, run_id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(state) = state else {
        return Err(OfferError::Missing(format!("offer {offer_id} not found")));
    };
    if state != OFFERED {
        return Ok(false);
    }
    if !accept && reason.map(str::trim).unwrap_or("").is_empty() {
        return Err(OfferError::Missing("decline requires a reason".into()));
    }
    let new_state = if accept { ACCEPTED } else { DECLINED };
    conn.execute(
        "UPDATE handover_offers SET state = ?1, reason = ?2, decided_at = ?3
          WHERE id = ?4 AND state = 'offered'",
        params![new_state, reason, now, offer_id],
    )?;
    emit_event(
        conn,
        run_id,
        &format!("handover:{offer_id}:{new_state}"),
        &serde_json::json!({
            "action": new_state,
            "offer_id": offer_id,
            "reason": reason,
        })
        .to_string(),
        now,
    )?;
    audit_write(
        conn,
        run_id,
        &format!("offer:{offer_id}"),
        AuditStatus::Ok,
        &format!("handover/{new_state}"),
    );
    Ok(true)
}

fn emit_event(
    conn: &Connection,
    run_id: i64,
    idempotency_key: &str,
    payload_json: &str,
    now: i64,
) -> Result<(), OfferError> {
    outbox::append_lineage(conn, run_id, TOPIC, payload_json, idempotency_key, now)
        .map_err(|e| OfferError::Database(e.to_string()))?;
    Ok(())
}

pub struct BoardRow {
    pub run_id: i64,
    pub owner: Option<String>,
    pub sla_deadline: i64,
    pub remaining_secs: i64,
    pub in_overlap: bool,
}

/// The handover-due board: every active run's SLA remaining, ranked
/// soonest-first, flagged when `now` sits inside the ring boundary's
/// derived overlap window (the Watchbill handover moment). Pure read-time
/// arithmetic — no scheduler daemon. A run with a corrupt `state_json` is
/// SKIPPED loudly (warn + counted in the returned tuple), never silently
/// folded into the P3 fallback — a poisoned row must not quietly distort
/// the ranking the handover decision reads.
pub fn board(
    conn: &Connection,
    domain: &str,
    now: i64,
) -> Result<(Vec<BoardRow>, usize), OfferError> {
    use super::shifts;
    let all = shifts::list_shifts(conn, domain).map_err(|e| OfferError::Missing(e.to_string()))?;
    let view = shifts::ring_view(&all, domain, now);
    let in_overlap = view.in_overlap;
    let mut stmt = conn.prepare(
        "SELECT id, state_json, status, created_at FROM workflow_runs
          WHERE domain = ?1 AND status = 'active' ORDER BY id LIMIT 500",
    )?;
    let it = stmt.query_map(params![domain], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
        ))
    })?;
    let mut rows = Vec::new();
    let mut corrupt = 0usize;
    for r in it {
        let (id, state_json, _status, created_at) = r?;
        let st: serde_json::Value = match serde_json::from_str(&state_json) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    run = id,
                    "handover board skipped a run with corrupt state_json: {e}"
                );
                corrupt += 1;
                continue;
            }
        };
        // SLA envelope mirrors the handoff read: recorded deadline wins,
        // else the P3 policy stamp at run-open time.
        let deadline = st
            .get("sla_deadline")
            .and_then(|v| v.as_i64())
            .unwrap_or_else(|| created_at + brain_engine_sdk::policy::Priority::P3.ttl_secs());
        rows.push(BoardRow {
            run_id: id,
            owner: st.get("owner").and_then(|v| v.as_str()).map(str::to_string),
            sla_deadline: deadline,
            remaining_secs: deadline - now,
            in_overlap,
        });
    }
    rows.sort_by_key(|b| b.remaining_secs);
    Ok((rows, corrupt))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::state::cas_update;
    use crate::workflow::tx::WorkflowTx;
    use brain_server::migration::run_migration;
    use brain_server::register_sqlite_vec::register_sqlite_vec;
    use rusqlite::Connection;

    fn db() -> Connection {
        register_sqlite_vec();
        let mut conn = Connection::open_in_memory().unwrap();
        run_migration(&mut conn, 1).unwrap();
        conn.execute(
            "INSERT INTO workflow_runs(domain, kind, state_json, status, created_at, updated_at)
             VALUES ('acme', 'interview', '{}', 'active', 1000, 1000)",
            [],
        )
        .unwrap();
        conn
    }

    fn complete_facts() -> PacketFacts {
        PacketFacts {
            pending_question: Some("which NL group?".into()),
            sla_deadline: Some(2000),
            now: 1000,
            has_current_step: true,
            has_evidence: true,
            escalation_honored: true,
        }
    }

    /// offer_refuses_incomplete_packet_with_missing_list
    #[test]
    fn offer_refuses_incomplete_packet_with_missing_list() {
        let base = complete_facts();
        assert!(
            packet_missing(&base).is_empty(),
            "complete packet offers clean"
        );

        let mut f = base.clone_owned();
        f.pending_question = None;
        f.has_evidence = false;
        f.escalation_honored = false;
        f.sla_deadline = None;
        let m = packet_missing(&f);
        assert_eq!(m.len(), 4);
        assert!(m.iter().any(|x| x.contains("open question")));
        assert!(m.iter().any(|x| x.contains("evidence")));
        assert!(m.iter().any(|x| x.contains("escalation")));
        assert!(m.iter().any(|x| x.contains("SLA")));

        let mut breached = base.clone_owned();
        breached.now = 3000;
        let m2 = packet_missing(&breached);
        assert!(
            m2.iter().any(|x| x.contains("breached")),
            "a breached SLA is not offerable"
        );
    }

    impl PacketFacts {
        fn clone_owned(&self) -> PacketFacts {
            PacketFacts {
                pending_question: self.pending_question.clone(),
                sla_deadline: self.sla_deadline,
                now: self.now,
                has_current_step: self.has_current_step,
                has_evidence: self.has_evidence,
                escalation_honored: self.escalation_honored,
            }
        }
    }

    /// complaint_escalation_is_audited_as_dispute
    #[test]
    fn complaint_escalation_is_audited_as_dispute() {
        let mut conn = db();
        let draft = OfferDraft {
            domain: "acme",
            run_id: 1,
            from_principal: "manila-op",
            to_principal: "complaints-team",
            overlap_minutes: 30,
            sla_deadline: 2000,
            now: 1100,
        };
        let mut tx = WorkflowTx::begin(&mut conn).unwrap();
        let (offer_id, created) = record_dispute_escalation(tx.tx(), &draft).unwrap();
        assert!(created);
        // Retried escalation cannot double-register the dispute.
        let (again, created2) = record_dispute_escalation(tx.tx(), &draft).unwrap();
        assert!(!created2 && again == offer_id);
        tx.commit().unwrap();

        // The dispute IS an audited handover: exactly one audit row with the
        // documented `handover/dispute` reason, chained in the audit trail.
        let details: Vec<String> = {
            let mut stmt = conn
                .prepare(
                    "SELECT detail_hash FROM audit_events
                      WHERE kind='workflow' AND target_hash = ?1 ORDER BY id",
                )
                .unwrap();
            let it = stmt
                .query_map([crate::audit::hash("offer:1")], |r| r.get::<_, String>(0))
                .unwrap();
            it.filter_map(Result::ok).collect()
        };
        assert_eq!(details.len(), 1, "the dispute registers exactly once");
        assert_eq!(details[0], crate::audit::hash(DISPUTE_AUDIT_DETAIL));

        // And it is a lineage event like any other handover.
        assert!(outbox::verify_outbox_lineage(&conn, 1).unwrap());
    }

    /// offer_accept_decline_are_lineage_events_audited_once
    #[test]
    fn offer_accept_decline_are_lineage_events_audited_once() {
        let mut conn = db();
        let draft = OfferDraft {
            domain: "acme",
            run_id: 1,
            from_principal: "manila-op",
            to_principal: "ams-op",
            overlap_minutes: 30,
            sla_deadline: 2000,
            now: 1100,
        };
        let mut tx = WorkflowTx::begin(&mut conn).unwrap();
        let (offer_id, created) = insert_offer(tx.tx(), &draft).unwrap();
        assert!(created);
        // Retried POST → idempotent receipt, no second row.
        let (again, created2) = insert_offer(tx.tx(), &draft).unwrap();
        assert!(!created2 && again == offer_id);

        assert!(decide_offer(tx.tx(), 1, offer_id, false, None, 1200).is_err());
        decide_offer(tx.tx(), 1, offer_id, false, Some("shift ends first"), 1200).unwrap();
        // A decided offer cannot re-decide.
        assert!(!decide_offer(tx.tx(), 1, offer_id, true, None, 1300).unwrap());
        tx.commit().unwrap();

        let topics: Vec<String> = {
            let mut stmt = conn
                .prepare(
                    "SELECT topic || ':' || payload_json FROM outbox WHERE run_id=1 ORDER BY id",
                )
                .unwrap();
            let it = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
            it.filter_map(Result::ok).collect()
        };
        assert!(
            topics
                .iter()
                .any(|t| t.starts_with("workflow/handover:") && t.contains("\"offer\""))
        );
        assert!(
            topics
                .iter()
                .any(|t| t.contains("\"declined\"") && t.contains("shift ends first"))
        );
        assert!(!topics.iter().any(|t| t.contains("\"accepted\"")));
        assert!(outbox::verify_outbox_lineage(&conn, 1).unwrap());

        let audits: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT target_hash FROM audit_events WHERE kind='workflow' AND target_hash = ?1 ORDER BY id")
                .unwrap();
            let it = stmt
                .query_map([crate::audit::hash("offer:1")], |r| r.get::<_, String>(0))
                .unwrap();
            it.filter_map(Result::ok).collect()
        };
        assert_eq!(audits.len(), 2, "offer + decline audited exactly once each");
    }

    /// accept_transfers_owner_without_sla_reset
    #[test]
    fn accept_transfers_owner_without_sla_reset() {
        let mut conn = db();
        conn.execute(
            "UPDATE workflow_runs SET state_json='{\"owner\":\"manila\",\"sla_deadline\":5555}',
             state_revision=7 WHERE id=1",
            [],
        )
        .unwrap();
        let draft = OfferDraft {
            domain: "acme",
            run_id: 1,
            from_principal: "manila-op",
            to_principal: "ams-op",
            overlap_minutes: 30,
            sla_deadline: 5555,
            now: 1100,
        };
        let mut tx = WorkflowTx::begin(&mut conn).unwrap();
        let (offer_id, _) = insert_offer(tx.tx(), &draft).unwrap();

        // CAS transfer: read current state+revision, rewrite owner only.
        let (js, rev): (String, i64) = tx
            .tx()
            .query_row(
                "SELECT state_json, state_revision FROM workflow_runs WHERE id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        let mut st: serde_json::Value = serde_json::from_str(&js).unwrap();
        st["owner"] = serde_json::json!("ams-op");
        cas_update(tx.tx(), 1, rev, &st.to_string(), "active", 1200).unwrap();
        decide_offer(tx.tx(), 1, offer_id, true, None, 1200).unwrap();
        tx.commit().unwrap();

        let (owner, sla): (String, i64) = conn
            .query_row(
                "SELECT json_extract(state_json,'$.owner'),
                        json_extract(state_json,'$.sla_deadline') FROM workflow_runs WHERE id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(owner, "ams-op", "ownership moved to the acceptor");
        assert_eq!(sla, 5555, "the SLA clock was never touched");
        assert!(
            decide_offer(&conn, 1, offer_id, false, Some("late"), 1400).is_ok_and(|moved| !moved)
        );
    }

    /// handover_board_ranks_by_sla_remaining_at_boundary
    #[test]
    fn handover_board_ranks_by_sla_remaining_at_boundary() {
        let conn = db();
        for (id, deadline, created) in [(2i64, 9000i64, 1000i64), (3, 4000, 1000)] {
            conn.execute(
                "INSERT INTO workflow_runs(id, domain, kind, state_json, status, created_at, updated_at)
                 VALUES (?1,'acme','interview',?2,'active',?3,?3)",
                rusqlite::params![
                    id,
                    format!("{{\"owner\":\"p{id}\",\"sla_deadline\":{deadline}}}"),
                    created
                ],
            )
            .unwrap();
        }
        // Manila owns [0,7200) with a 60-min handover budget; Amsterdam
        // follows at 6900 → the derived overlap window is [6900, 6960).
        conn.execute(
            "INSERT INTO shifts(domain, site, tz, start_epoch, end_epoch, overlap_minutes, roster_json, created_at)
             VALUES ('acme','manila','Asia/Manila',0,7200,60,'[]',0)",
            [],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO shifts(domain, site, tz, start_epoch, end_epoch, overlap_minutes, roster_json, created_at)
             VALUES ('acme','amsterdam','Europe/Amsterdam',6900,99999,0,'[]',0)",
            [],
        )
        .unwrap();

        // now=7000 sits inside manila's final overlap window [6900, 7200).
        let (rows, corrupt) = board(&conn, "acme", 7000).unwrap();
        assert_eq!(corrupt, 0, "a healthy corpus reports no skipped rows");
        assert_eq!(rows.len(), 3);
        assert!(
            rows.windows(2)
                .all(|w| w[0].remaining_secs <= w[1].remaining_secs),
            "ranked soonest-first"
        );
        assert!(
            rows.iter().all(|r| r.in_overlap),
            "boundary flags the whole board"
        );
        assert_eq!(
            rows[0].run_id, 3,
            "run 3 holds the tightest envelope (4000)"
        );
        assert_eq!(rows[1].run_id, 2, "run 2 next (9000)");
        assert_eq!(
            rows[2].run_id, 1,
            "run 1 falls back to P3-from-created (260200)"
        );

        // Off-boundary: the flag drops but the ranking law holds.
        let (rows_off, _corrupt) = board(&conn, "acme", 10_000).unwrap();
        assert!(rows_off.iter().all(|r| !r.in_overlap));
    }

    /// board_skips_corrupt_state_loudly_never_silently
    #[test]
    fn board_skips_corrupt_state_loudly_never_silently() {
        let conn = db();
        conn.execute(
            "INSERT INTO workflow_runs(id, domain, kind, state_json, status, created_at, updated_at)
             VALUES (9,'acme','interview','{not json','active',1000,1000)",
            [],
        )
        .unwrap();
        let (rows, corrupt) = board(&conn, "acme", 2000).unwrap();
        assert_eq!(corrupt, 1, "the poisoned row is counted, never absorbed");
        assert!(
            rows.iter().all(|r| r.run_id != 9),
            "a corrupt-state run cannot distort the ranking it is skipped"
        );
    }
}
