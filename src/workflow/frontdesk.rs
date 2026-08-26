//! Frontdesk terminal discipline — the Order-of-Care amendments made
//! structural, computed from the run's lineage ONLY (no surveys, no
//! sentiment models): the close gate (a case closes on customer
//! confirmation or the documented three-attempt exception — never on
//! silence) and the customer-effort proxy (repeats × channel switches ×
//! handovers × re-asks). Deterministic, bounded inputs, fail-closed on
//! ambiguity.

use rusqlite::Connection;

/// One lineage event as the gate reads it: the outbox topic plus the
/// channel it rode (empty for channel-less events).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LineageEvent {
    pub(crate) topic: String,
    pub(crate) channel: String,
}

impl LineageEvent {
    pub(crate) fn new(topic: &str, channel: &str) -> Self {
        Self {
            topic: topic.to_string(),
            channel: channel.to_string(),
        }
    }
}

/// The topics the close gate recognizes.
pub(crate) const TOPIC_CUSTOMER_CONFIRMATION: &str = "customer/confirmation";
pub(crate) const TOPIC_CLOSE_ATTEMPT: &str = "outreach/close_attempt";
/// The re-ask event: the customer's issue arrived AGAIN
/// because the first answer didn't land. One topic, three sources:
/// `crm_merge` | `marked` | `derived` — pinned by tests.
pub(crate) const TOPIC_REASK: &str = "case/reask";
/// The documented consent-absent exception: this many logged attempts and
/// no objection lets a case close without confirmation (the peak-end rule's
/// escape hatch, fully audited).
pub(crate) const CLOSE_ATTEMPT_EXCEPTION_MIN: usize = 3;

/// The legal re-ask sources — a closed vocabulary, validated at the door.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReaskSource {
    CrmMerge,
    Marked,
    Derived,
}

impl ReaskSource {
    pub(crate) fn parse(s: &str) -> Option<Self> {
        match s {
            "crm_merge" => Some(ReaskSource::CrmMerge),
            "marked" => Some(ReaskSource::Marked),
            "derived" => Some(ReaskSource::Derived),
            _ => None,
        }
    }

    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            ReaskSource::CrmMerge => "crm_merge",
            ReaskSource::Marked => "marked",
            ReaskSource::Derived => "derived",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CloseDecision {
    /// A confirmation event exists on the lineage.
    Confirmed,
    /// No confirmation, but the consent-absent exception is documented.
    ClosedByAttempts,
    /// Neither leg holds: the case stays open. Silence never certifies.
    RemainOpen,
}

/// The confirm-gate: terminal `closed` requires a customer-confirmation
/// event on the lineage, OR the documented consent-absent exception (at
/// least [`CLOSE_ATTEMPT_EXCEPTION_MIN`] logged outreach attempts).
pub(crate) fn evaluate_close(events: &[LineageEvent]) -> CloseDecision {
    if events
        .iter()
        .any(|e| e.topic == TOPIC_CUSTOMER_CONFIRMATION)
    {
        return CloseDecision::Confirmed;
    }
    let attempts = events
        .iter()
        .filter(|e| e.topic == TOPIC_CLOSE_ATTEMPT)
        .count();
    if attempts >= CLOSE_ATTEMPT_EXCEPTION_MIN {
        return CloseDecision::ClosedByAttempts;
    }
    CloseDecision::RemainOpen
}

/// The deterministic customer-effort proxy (CES without surveys): every
/// dimension is counted from lineage shape alone.
///
/// - `repeats`: ask-events beyond the first per distinct ask subject
///   (`ask/<subject>` topics)
/// - `channel_switches`: adjacent lineage events on different channels
/// - `handovers`: recorded handover events
/// - `re_asks` (Keystone): `case/reask` events — the customer's issue
///   arrived again because the first answer didn't land. Weighted like a
///   repeat (×2).
///
/// Score = repeats×2 + switches×1 + handovers×3 + re_asks×2.
pub(crate) fn effort_proxy(events: &[LineageEvent]) -> i64 {
    use std::collections::BTreeSet;
    let mut subjects_asked = BTreeSet::new();
    let mut repeats = 0i64;
    for e in events {
        if let Some(subject) = e.topic.strip_prefix("ask/")
            && !subjects_asked.insert(subject)
        {
            repeats += 1;
        }
    }
    let mut switches = 0i64;
    for pair in events.windows(2) {
        if !pair[0].channel.is_empty()
            && !pair[1].channel.is_empty()
            && pair[0].channel != pair[1].channel
        {
            switches += 1;
        }
    }
    let handovers = events.iter().filter(|e| e.topic == "handover").count() as i64;
    let re_asks = re_asks(events);
    repeats * 2 + switches + handovers * 3 + re_asks * 2
}

/// Count `case/reask` events on the lineage (the effort proxy's missing
/// input, now emitted from three deterministic sources).
pub(crate) fn re_asks(events: &[LineageEvent]) -> i64 {
    events.iter().filter(|e| e.topic == TOPIC_REASK).count() as i64
}

/// Write one re-ask lineage event on the run: exactly-once by idempotency
/// key (`reask:{source}:{detail_digest}`), payload carries ids/digests only
/// (never content), audited inside the caller's tx via append_lineage.
pub(crate) fn record_reask(
    conn: &Connection,
    run_id: i64,
    source: ReaskSource,
    detail_digest: &str,
    now: i64,
) -> rusqlite::Result<i64> {
    crate::workflow::outbox::append_lineage(
        conn,
        run_id,
        TOPIC_REASK,
        &serde_json::json!({
            "source": source.as_str(),
            "detail_digest": detail_digest,
            "ts": now,
        })
        .to_string(),
        &format!("reask:{}:{detail_digest}", source.as_str()),
        now,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    #[test]
    fn close_requires_confirmation_or_three_attempt_exception() {
        // Silence is never certified: no events → open.
        assert_eq!(evaluate_close(&[]), CloseDecision::RemainOpen);
        // A single confirmation closes.
        assert_eq!(
            evaluate_close(&[LineageEvent::new(TOPIC_CUSTOMER_CONFIRMATION, "email")]),
            CloseDecision::Confirmed
        );
        // Two attempts are not the exception; three are.
        let two = [
            LineageEvent::new(TOPIC_CLOSE_ATTEMPT, "email"),
            LineageEvent::new(TOPIC_CLOSE_ATTEMPT, "sms"),
        ];
        assert_eq!(evaluate_close(&two), CloseDecision::RemainOpen);
        let mut three = two.to_vec();
        three.push(LineageEvent::new(TOPIC_CLOSE_ATTEMPT, "call"));
        assert_eq!(evaluate_close(&three), CloseDecision::ClosedByAttempts);
        // Confirmation outranks attempts wherever it appears.
        let mut with_confirm = three.clone();
        with_confirm.push(LineageEvent::new(TOPIC_CUSTOMER_CONFIRMATION, "web"));
        assert_eq!(evaluate_close(&with_confirm), CloseDecision::Confirmed);
    }

    #[test]
    fn effort_proxy_computes_from_lineage_only_no_surveys() {
        // A clean single-channel, single-ask run scores zero.
        let clean = [LineageEvent::new("ask/status", "web")];
        assert_eq!(effort_proxy(&clean), 0);
        // Repeated asks of the same subject count as repeats (×2); a NEW
        // subject does not.
        let repeated = [
            LineageEvent::new("ask/status", "web"),
            LineageEvent::new("ask/status", "web"),
            LineageEvent::new("ask/billing", "web"),
            LineageEvent::new("ask/billing", "web"),
        ];
        let base = effort_proxy(&repeated);
        assert_eq!(base, 4, "two repeat-asks × 2");
        // Channel switches add one per adjacency change.
        let switched = [
            LineageEvent::new("ask/status", "web"),
            LineageEvent::new("ask/status", "sms"),
        ];
        assert_eq!(effort_proxy(&switched), 3, "repeat ×2 + one switch");
        // Handovers cost most (×3 each).
        let handed = [
            LineageEvent::new("ask/status", "web"),
            LineageEvent::new("handover", ""),
        ];
        assert_eq!(effort_proxy(&handed), 3);
        // Channel-less events never fabricate switches.
        assert_eq!(effort_proxy(&[LineageEvent::new("handover", "")]), 3);
    }

    // ── Keystone M3: the re-ask event.

    fn lineage_db() -> Connection {
        let conn = Connection::open_in_memory().expect("open");
        conn.execute_batch(
            "CREATE TABLE outbox(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                run_id INTEGER NOT NULL, topic TEXT NOT NULL, payload_json TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending', idempotency_key TEXT UNIQUE,
                created_at INTEGER NOT NULL, parent_id INTEGER, delivered_at INTEGER);",
        )
        .expect("outbox");
        conn
    }

    #[test]
    fn marked_reask_writes_lineage_event_and_counts() {
        let conn = lineage_db();
        let id = record_reask(&conn, 9, ReaskSource::Marked, "note:77", 5_000).expect("record");
        assert!(id > 0);
        let (topic, payload): (String, String) = conn
            .query_row(
                "SELECT topic, payload_json FROM outbox WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("event");
        assert_eq!(topic, TOPIC_REASK);
        assert!(payload.contains("\"source\":\"marked\""));
        assert!(payload.contains("note:77"), "the digest names its evidence");
        assert!(
            !payload.contains("content"),
            "no content rides a re-ask payload"
        );
        // Exactly-once: the same digest+source replay is a no-op.
        record_reask(&conn, 9, ReaskSource::Marked, "note:77", 6_000).expect("replay");
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM outbox WHERE run_id = 9", [], |r| {
                r.get(0)
            })
            .expect("count");
        assert_eq!(n, 1, "first write wins");
        // The proxy counts it — twice, like a repeat.
        let events = vec![
            LineageEvent::new(TOPIC_REASK, ""),
            LineageEvent::new(TOPIC_REASK, ""),
        ];
        assert_eq!(re_asks(&events), 2);
        assert_eq!(effort_proxy(&events), 4, "each re-ask weighs ×2");
        // Unknown sources refuse at the door — the closed vocabulary.
        assert!(ReaskSource::parse("vibes").is_none());
        for legal in ["crm_merge", "marked", "derived"] {
            assert!(ReaskSource::parse(legal).is_some());
        }
    }
}
