//! Consent-first outreach as deterministic policy: the closed channel and
//! purpose vocabularies, the consent decision that fails CLOSED (absent,
//! revoked, or expired consent all deny — only an in-force grant passes),
//! and the post-close follow-up policy interval. Pure functions only —
//! no I/O, no clock.
//!
//! Mantra guardrails encoded here: outbound contact is consent-gated and
//! purpose-limited — the check is a GATE, not a warning; a revocation wins
//! over any later-dated grant window; a future-dated grant is not yet
//! consent.

/// The outbound channels brain may propose contact on. Closed vocabulary;
/// unknown strings deny.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Email,
    Sms,
    Call,
}

impl Channel {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "email" => Some(Self::Email),
            "sms" => Some(Self::Sms),
            "call" => Some(Self::Call),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Email => "email",
            Self::Sms => "sms",
            Self::Call => "call",
        }
    }
}

/// The care purposes outreach may serve. A grant is scoped to a purpose:
/// retention consent never covers recall notices and vice versa. Closed
/// vocabulary; unknown purposes deny.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// Post-close proactive check (the Order-of-Care follow-up).
    CareFollowup,
    /// Retention / satisfaction outreach (ISO 10004 VoC).
    Retention,
    /// GPSR safety-recall notification (duty, not marketing).
    RecallNotice,
}

impl Purpose {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "care_followup" => Some(Self::CareFollowup),
            "retention" => Some(Self::Retention),
            "recall_notice" => Some(Self::RecallNotice),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::CareFollowup => "care_followup",
            Self::Retention => "retention",
            Self::RecallNotice => "recall_notice",
        }
    }
}

/// The consent verdict for one (subject, channel, purpose) triple at a
/// point in time. Every non-Granted variant DENIES — the caller must treat
/// them identically: no send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentDecision {
    /// An in-force grant covers this contact.
    Granted,
    /// No registry row at all.
    Absent,
    /// The subject revoked — wins over every other fact.
    Revoked,
    /// The grant's expiry has passed (or sits exactly at `now`).
    Expired,
}

/// The deterministic consent gate. `granted_at` is when the subject granted;
/// `expires_at` the optional window end; `revoked_at` the optional revocation.
/// Laws:
/// - A revocation ALWAYS denies, even one dated before a re-grant stored in
///   the same row (rows carry the latest decision; a revoked row is revoked).
/// - `now < granted_at` is a future-dated row → [`ConsentDecision::Absent`]
///   (consent cannot precede its own granting).
/// - An expired grant is [`ConsentDecision::Expired`] — visible, still deny.
pub fn consent_decision(
    now: i64,
    granted_at: Option<i64>,
    expires_at: Option<i64>,
    revoked_at: Option<i64>,
) -> ConsentDecision {
    if revoked_at.is_some() {
        return ConsentDecision::Revoked;
    }
    let Some(granted_at) = granted_at else {
        return ConsentDecision::Absent;
    };
    if now < granted_at {
        return ConsentDecision::Absent;
    }
    if expires_at.is_some_and(|exp| now >= exp) {
        return ConsentDecision::Expired;
    }
    ConsentDecision::Granted
}

/// The default post-close follow-up policy interval: 7 days after close.
/// Domains override via policy rows; this constant is the fallback the
/// service core pins.
pub const DEFAULT_FOLLOWUP_INTERVAL_SECS: i64 = 7 * 86_400;

/// Whether the Order-of-Care follow-up is due: strictly after the policy
/// interval has elapsed since close. Negative intervals are nonsense and
/// are never due.
pub fn followup_due(closed_at: i64, now: i64, interval_secs: i64) -> bool {
    interval_secs >= 0 && now >= closed_at.saturating_add(interval_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_consent_no_send_is_a_gate_not_warning() {
        use ConsentDecision::*;
        // Absent, revoked, expired, future-dated: ALL deny identically.
        assert_eq!(consent_decision(100, None, None, None), Absent);
        assert_eq!(
            consent_decision(100, Some(50), None, Some(90)),
            Revoked,
            "revocation wins over an in-force window"
        );
        assert_eq!(
            consent_decision(100, Some(50), Some(100), None),
            Expired,
            "expiry AT now expires (inclusive)"
        );
        assert_eq!(consent_decision(99, Some(50), Some(101), None), Granted);
        // A grant dated in the future is not yet consent.
        assert_eq!(consent_decision(99, Some(100), None, None), Absent);
        // Determinism: same input, same verdict.
        let a = consent_decision(123, Some(1), Some(2), None);
        let b = consent_decision(123, Some(1), Some(2), None);
        assert_eq!(a, b);
    }

    #[test]
    fn channel_purpose_vocabularies_are_closed() {
        for c in [Channel::Email, Channel::Sms, Channel::Call] {
            assert_eq!(Channel::parse(c.as_str()), Some(c));
            assert!(!c.as_str().is_empty());
        }
        assert_eq!(Channel::parse("fax"), None);
        assert_eq!(Channel::parse(""), None);
        assert_eq!(Channel::parse("EMAIL"), Some(Channel::Email));
        for p in [
            Purpose::CareFollowup,
            Purpose::Retention,
            Purpose::RecallNotice,
        ] {
            assert_eq!(Purpose::parse(p.as_str()), Some(p));
            assert!(!p.as_str().is_empty());
        }
        assert_eq!(Purpose::parse("newsletter"), None);
    }

    #[test]
    fn followup_scheduled_by_policy_and_consent_gated_interval_arithmetic() {
        let iv = DEFAULT_FOLLOWUP_INTERVAL_SECS;
        assert!(followup_due(0, iv, iv), "due exactly at the interval");
        assert!(!followup_due(0, iv - 1, iv), "one second early is early");
        assert!(!followup_due(0, 10, -1), "negative intervals never fire");
        let late = i64::MAX / 2;
        assert!(followup_due(0, late, iv), "long-past closes stay due");
    }
}
