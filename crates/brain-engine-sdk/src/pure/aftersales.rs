//! Aftersales KPI formulas — integer ten-thousandths like the rest of the
//! scorer, defined once here and mirrored verbatim in docs/metrics.md.
//! FTFR mirrors the FCR repeat-window method on the repair-field cohort;
//! refund cycle time is the median of resolved return/warranty runs. No
//! invented CRM/telephony numbers: a metric whose cohort is empty scores 0
//! (documented absence), never a fabricated 100%.

use crate::pure::qa_score::SCALE;

/// The aftersales run kinds whose cohorts back the KPI set. The strings are
/// the workflow `kind` values the frontdoor intent table routes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AftersalesKind {
    Return,
    WarrantyClaim,
    RepairField,
}

impl AftersalesKind {
    pub fn from_kind(kind: &str) -> Option<Self> {
        match kind {
            "return" => Some(Self::Return),
            "warranty_claim" => Some(Self::WarrantyClaim),
            "repair_field" => Some(Self::RepairField),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Return => "return",
            Self::WarrantyClaim => "warranty_claim",
            Self::RepairField => "repair_field",
        }
    }
}

#[derive(Debug, Clone)]
pub struct AftersalesRun {
    pub kind: AftersalesKind,
    pub created_at: i64,
    /// Resolution timestamp when the run reached a terminal state.
    pub resolved_at: Option<i64>,
    /// A recurrence inside the FCR window marks this run unresolved
    /// (the same flag the scoreboard's FCR leg derives).
    pub repeat_within_window: bool,
    pub returnless: bool,
    pub fraud_flagged: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AftersalesKpis {
    pub return_rate_units: i32,
    pub warranty_claim_rate_units: i32,
    pub ftfr_units: i32,
    /// Median seconds between creation and resolution over resolved
    /// return/warranty runs; 0 when none resolved.
    pub refund_cycle_time_median_secs: i64,
    pub returnless_share_units: i32,
    pub aftersales_fraud_flag_rate_units: i32,
}

fn rate(part: usize, total: usize) -> i32 {
    if total == 0 {
        0
    } else {
        part as i32 * SCALE / total as i32
    }
}

fn median(values: &mut [i64]) -> i64 {
    values.sort_unstable();
    match values.len() {
        0 => 0,
        n if n % 2 == 1 => values[n / 2],
        n => (values[n / 2 - 1] + values[n / 2]) / 2,
    }
}

/// The KPI set over the aftersales cohort. FTFR = repair-field runs without
/// an in-window repeat over all repair-field runs — the FCR repeat-window
/// method applied to first-VISIT resolution.
pub fn aftersales_kpis(runs: &[AftersalesRun]) -> AftersalesKpis {
    let returns: Vec<_> = runs
        .iter()
        .filter(|r| r.kind == AftersalesKind::Return)
        .collect();
    let warranty: Vec<_> = runs
        .iter()
        .filter(|r| r.kind == AftersalesKind::WarrantyClaim)
        .collect();
    let repairs: Vec<_> = runs
        .iter()
        .filter(|r| r.kind == AftersalesKind::RepairField)
        .collect();
    let mut cycles: Vec<i64> = runs
        .iter()
        .filter(|r| {
            matches!(
                r.kind,
                AftersalesKind::Return | AftersalesKind::WarrantyClaim
            )
        })
        .filter_map(|r| r.resolved_at.map(|t| t - r.created_at))
        .filter(|d| *d >= 0)
        .collect();
    AftersalesKpis {
        return_rate_units: rate(returns.len(), runs.len()),
        warranty_claim_rate_units: rate(warranty.len(), runs.len()),
        ftfr_units: rate(
            repairs.iter().filter(|r| !r.repeat_within_window).count(),
            repairs.len(),
        ),
        refund_cycle_time_median_secs: median(&mut cycles),
        returnless_share_units: rate(
            returns.iter().filter(|r| r.returnless).count(),
            returns.len(),
        ),
        aftersales_fraud_flag_rate_units: rate(
            returns.iter().filter(|r| r.fraud_flagged).count(),
            returns.len(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(
        kind: AftersalesKind,
        created: i64,
        resolved: Option<i64>,
        repeat: bool,
    ) -> AftersalesRun {
        AftersalesRun {
            kind,
            created_at: created,
            resolved_at: resolved,
            repeat_within_window: repeat,
            returnless: false,
            fraud_flagged: false,
        }
    }

    #[test]
    fn ftfr_uses_repeat_window_method() {
        // The formula is FCR's repeat-window method restricted to the
        // repair-field cohort: no in-window repeat = first-visit resolved.
        let runs = [
            run(AftersalesKind::RepairField, 100, Some(200), false),
            run(AftersalesKind::RepairField, 300, Some(400), true),
            run(AftersalesKind::RepairField, 500, None, false),
            // Other worktypes never dilute FTFR — it is a repair metric.
            run(AftersalesKind::Return, 600, Some(700), true),
        ];
        let kpis = aftersales_kpis(&runs);
        assert_eq!(kpis.ftfr_units, 2 * SCALE / 3);
        assert_eq!(kpis.return_rate_units, SCALE / 4);
        assert_eq!(kpis.warranty_claim_rate_units, 0);
        assert_eq!(kpis.returnless_share_units, 0);
        // Refund cycle time is the MEDIAN of resolved return/warranty runs.
        let cyc = [
            run(AftersalesKind::Return, 0, Some(100), false),
            run(AftersalesKind::Return, 0, Some(300), false),
            run(AftersalesKind::WarrantyClaim, 0, Some(1_000), false),
        ];
        assert_eq!(aftersales_kpis(&cyc).refund_cycle_time_median_secs, 300);
        // Returnless share + fraud-flag rate ride the RETURN cohort only.
        let mut flagged = run(AftersalesKind::Return, 0, None, false);
        flagged.returnless = true;
        flagged.fraud_flagged = true;
        let cohort = vec![flagged, run(AftersalesKind::Return, 0, None, false)];
        let k = aftersales_kpis(&cohort);
        assert_eq!(k.returnless_share_units, SCALE / 2);
        assert_eq!(k.aftersales_fraud_flag_rate_units, SCALE / 2);
        // Empty cohort scores 0 across the board — absence is never dressed
        // up as perfection (the metrics-dictionary posture).
        let empty = aftersales_kpis(&[]);
        assert_eq!(
            empty,
            AftersalesKpis {
                return_rate_units: 0,
                warranty_claim_rate_units: 0,
                ftfr_units: 0,
                refund_cycle_time_median_secs: 0,
                returnless_share_units: 0,
                aftersales_fraud_flag_rate_units: 0,
            }
        );
    }
}
