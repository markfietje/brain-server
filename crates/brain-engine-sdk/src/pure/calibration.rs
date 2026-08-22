//! Calibration: the scorer measured against agreed truth. Cohen's κ in
//! integer ten-thousandths, the weekly/monthly cadence gates, and the
//! `CalibrationRecord` that rides the audit chain. Only baseline deltas are
//! ever recorded — no industry lift numbers.

use crate::pure::qa_score::SCALE;

/// The scorer contract version a calibration record certifies.
pub const SCORER_VERSION: &str = "1";

/// One week, the REPORT cadence.
pub const WEEK_SECS: i64 = 7 * 86400;

/// Sentinel κ for "no human calibration has happened yet".
pub const NO_KAPPA: i32 = -1;

/// A calibration record as it lands on the audit chain (kind `workflow`).
/// `human_agreement_kappa_units` is [`NO_KAPPA`] until a human signs;
/// `uplift_vs_baseline_units` is OUR delta vs our own last recorded score —
/// never an external comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CalibrationRecord {
    pub scorer_version: &'static str,
    pub human_agreement_kappa_units: i32,
    pub uplift_vs_baseline_units: i32,
    pub reviewer_id: String,
}

impl CalibrationRecord {
    /// Build a record for the current scorer version (the crate pins the
    /// version so a host can never certify a stale one by accident).
    pub fn new(
        human_agreement_kappa_units: i32,
        uplift_vs_baseline_units: i32,
        reviewer_id: &str,
    ) -> Self {
        Self {
            scorer_version: SCORER_VERSION,
            human_agreement_kappa_units,
            uplift_vs_baseline_units,
            reviewer_id: reviewer_id.to_string(),
        }
    }

    /// Serialize to the audit-row detail string (stable field order).
    pub fn detail(&self) -> String {
        format!(
            "calibration{{scorer_version:{},human_agreement_kappa_units:{},uplift_vs_baseline_units:{},reviewer_id:{}}}",
            self.scorer_version,
            self.human_agreement_kappa_units,
            self.uplift_vs_baseline_units,
            self.reviewer_id
        )
    }
}

/// Cohen's κ over paired binary verdicts (human vs machine), ten-thousandths.
/// `None` when inputs are empty, unequal length, or κ is undefined
/// (denominator 0 — perfect disagreement with balanced marginals).
pub fn kappa_units(human: &[bool], machine: &[bool]) -> Option<i32> {
    if human.is_empty() || human.len() != machine.len() {
        return None;
    }
    let n = human.len();
    let agreements = human.iter().zip(machine).filter(|(h, m)| h == m).count();
    let observed = agreements as f64 / n as f64;
    let h_yes = human.iter().filter(|b| **b).count() as f64 / n as f64;
    let m_yes = machine.iter().filter(|b| **b).count() as f64 / n as f64;
    let expected = h_yes * m_yes + (1.0 - h_yes) * (1.0 - m_yes);
    if expected >= 1.0 {
        return None;
    }
    let k = (observed - expected) / (1.0 - expected);
    Some((k * SCALE as f64).round() as i32)
}

/// Whether a weekly calibration REPORT is due: none recorded yet, or the last
/// one is older than one week.
pub fn week_due(last_report_at: Option<i64>, now: i64) -> bool {
    match last_report_at {
        None => true,
        Some(t) => now - t >= WEEK_SECS,
    }
}

/// Calendar month index (UTC) from a unix timestamp — the monthly signed-gate
/// granularity.
pub fn month_index(unix_secs: i64) -> i64 {
    unix_secs.div_euclid(2_629_800) // ~30.44 days; monotonic month proxy
}

/// Whether the monthly human-signed gate blocks: no signature this calendar
/// month yet.
pub fn month_due(last_signed_month: Option<i64>, now_month: i64) -> bool {
    match last_signed_month {
        None => true,
        Some(m) => m < now_month,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kappa_perfect_and_known_values() {
        // Perfect agreement → SCALE.
        assert_eq!(
            kappa_units(&[true, false, true], &[true, false, true]),
            Some(SCALE)
        );
        // Classic 2x2: observed .5, expected .5 → κ 0.
        let h = [true, true, false, false];
        let m = [true, false, true, false];
        assert_eq!(kappa_units(&h, &m), Some(0));
        // Empty or ragged input refused.
        assert_eq!(kappa_units(&[], &[]), None);
        assert_eq!(kappa_units(&[true], &[true, false]), None);
    }

    #[test]
    fn record_detail_is_stable_and_sentinel_aware() {
        let r = CalibrationRecord {
            scorer_version: SCORER_VERSION,
            human_agreement_kappa_units: NO_KAPPA,
            uplift_vs_baseline_units: -120,
            reviewer_id: "dpo-7".into(),
        };
        assert_eq!(
            r.detail(),
            "calibration{scorer_version:1,human_agreement_kappa_units:-1,uplift_vs_baseline_units:-120,reviewer_id:dpo-7}"
        );
    }

    #[test]
    fn cadence_gates() {
        assert!(week_due(None, 0));
        assert!(!week_due(Some(100), 100 + WEEK_SECS - 1));
        assert!(week_due(Some(100), 100 + WEEK_SECS));
        // Monthly: same month fine, next month due.
        let m0 = month_index(1_700_000_000);
        assert!(!month_due(Some(m0), m0));
        assert!(month_due(Some(m0), m0 + 1));
        assert!(month_due(None, m0));
        // Month index is monotonic.
        assert!(month_index(1_760_000_000) > m0);
    }
}
