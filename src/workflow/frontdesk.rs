//! Frontdesk terminal discipline — the Order-of-Care amendments made
//! structural, computed from the run's lineage ONLY (no surveys, no
//! sentiment models): the close gate (a case closes on customer
//! confirmation or the documented three-attempt exception — never on
//! silence) and the customer-effort proxy (repeats × channel switches ×
//! handovers × re-asks). Deterministic, bounded inputs, fail-closed on
//! ambiguity.

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
/// The documented consent-absent exception: this many logged attempts and
/// no objection lets a case close without confirmation (the peak-end rule's
/// escape hatch, fully audited).
pub(crate) const CLOSE_ATTEMPT_EXCEPTION_MIN: usize = 3;

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
///
/// Score = repeats×2 + switches×1 + handovers×3 — a repeat costs more than
/// a switch, an unowned handover costs most.
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
    repeats * 2 + switches + handovers * 3
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
