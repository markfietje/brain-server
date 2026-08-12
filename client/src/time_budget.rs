//! v1.20.15 "Clock" — the shared proposal-deadline clock core. One pure,
//! Dioxus-free implementation of the tier + format math, consumed by the
//! Review cards, the deep-link detail page, and /ops. The deadline itself is
//! **server-authoritative**: each `Proposal` carries `expires_at`
//! (`created_at + TTL`) plus the `warn_secs`/`critical_secs` SLA bands, so an
//! operator's `BRAIN_PROPOSAL_TTL_SECS` override is respected with no client
//! mirror to drift (the old per-panel client TTL mirror was the drift).

use std::time::{SystemTime, UNIX_EPOCH};

/// SLA tier for a remaining deadline, most urgent first. Mirrors the server's
/// `alert::Tier::from_remaining` boundaries; the client adds `Expired` as the
/// display tier for a lapsed deadline (the server's 400 `proposal_expired` is
/// its enforcement twin).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Critical,
    Warn,
    Ok,
    Expired,
}

/// `remaining <= 0 → Expired`, `< critical_secs → Critical`, `< warn_secs →
/// Warn`, else `Ok`. The bands come from the proposal (server-provided), so
/// the color follows an operator threshold override with no rebuild.
pub fn tier(remaining_secs: i64, warn_secs: i64, critical_secs: i64) -> Tier {
    if remaining_secs <= 0 {
        Tier::Expired
    } else if remaining_secs < critical_secs {
        Tier::Critical
    } else if remaining_secs < warn_secs {
        Tier::Warn
    } else {
        Tier::Ok
    }
}

/// `expires_at - now`, the absolute-deadline countdown the panels tick
/// (negative once past the deadline — that is the `Expired` tier).
pub fn remaining(expires_at: i64, now_unix: i64) -> i64 {
    expires_at.saturating_sub(now_unix)
}

pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Compact deadline label for the displayed clock: `Xd Yh` / `Xh Ym` / `Xm` /
/// `<5m` (sub-warn) / `expired`. ponytail: the `<5m` band is not parameterized
/// by the critical threshold — an operator override of `ALERT_CRITICAL_SECS`
/// shifts only the *tier color* (computed from server thresholds), never this
/// coarse display label.
pub fn format_remaining(remaining_secs: i64) -> String {
    if remaining_secs <= 0 {
        return "expired".into();
    }
    if remaining_secs < 5 * 60 {
        return "<5m".into();
    }
    let d = remaining_secs / 86400;
    let h = (remaining_secs % 86400) / 3600;
    let m = (remaining_secs % 3600) / 60;
    if d > 0 {
        format!("{d}d {h:02}h")
    } else if h > 0 {
        format!("{h}h {m:02}m")
    } else {
        format!("{m}m")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86400;

    #[test]
    fn tier_maps_budgets_and_expired() {
        assert_eq!(tier(-1, 3600, 300), Tier::Expired);
        assert_eq!(tier(0, 3600, 300), Tier::Expired);
        assert_eq!(tier(1, 3600, 300), Tier::Critical);
        assert_eq!(tier(299, 3600, 300), Tier::Critical);
        assert_eq!(tier(300, 3600, 300), Tier::Warn); // 5 min boundary → warn
        assert_eq!(tier(3599, 3600, 300), Tier::Warn);
        assert_eq!(tier(3600, 3600, 300), Tier::Ok); // 1 hr boundary → ok
        assert_eq!(tier(7 * DAY, 3600, 300), Tier::Ok);
        // The bands follow the server-provided thresholds, not constants.
        assert_eq!(tier(50, 600, 30), Tier::Warn);
        assert_eq!(tier(20, 600, 30), Tier::Critical);
    }

    #[test]
    fn format_remaining_labels() {
        assert_eq!(format_remaining(-5), "expired");
        assert_eq!(format_remaining(0), "expired");
        assert_eq!(format_remaining(59), "<5m");
        assert_eq!(format_remaining(300), "5m");
        assert_eq!(format_remaining(3599), "59m");
        assert_eq!(format_remaining(3661), "1h 01m");
        assert_eq!(format_remaining(2 * DAY + 4 * 3600), "2d 04h");
    }

    #[test]
    fn remaining_is_negative_past_deadline() {
        assert_eq!(remaining(1000, 900), 100);
        assert_eq!(remaining(1000, 1000), 0);
        assert_eq!(remaining(1000, 1100), -100);
    }
}
