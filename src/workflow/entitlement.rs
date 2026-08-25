//! The entitlement registry's arithmetic core: warranty/contract state lives
//! as governed knowledge rows (`memory_kind='entitlement'`, proposal-created
//! like every knowledge write); this module owns the pure date/region
//! arithmetic those rows answer to — Directive 2019/771 coverage windows,
//! the 14-day withdrawal window with its exceptions table, and the
//! geographic rules that honor the `BRAIN_REGION` stamp. Deterministic and
//! testable by construction: no DB, no clock reads, no judgment.

use chrono::{Duration, NaiveDate};
use serde::Deserialize;

/// One governed entitlement row (the JSON payload of an
/// `memory_kind='entitlement'` knowledge chunk). Product/serial carry the
/// GPSR traceability spine; dates are civil `YYYY-MM-DD`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct EntitlementRecord {
    pub product: String,
    pub serial: String,
    /// Date of purchase / delivery of goods (`YYYY-MM-DD`).
    pub purchase_date: String,
    /// Contract SLA tier label, when a contract exists.
    #[serde(default)]
    pub contract_tier: Option<String>,
    /// Residency stamp copied from the source site; empty = unstamped.
    #[serde(default)]
    pub region: String,
}

impl EntitlementRecord {
    /// Parse one row's JSON payload. Malformed payloads deny loudly — a
    /// registry row that cannot be read never grants coverage.
    pub fn parse(json: &str) -> Result<Self, String> {
        let r: Self =
            serde_json::from_str(json).map_err(|e| format!("entitlement_row_invalid: {e}"))?;
        date(&r.purchase_date)
            .map_err(|e| format!("entitlement_row_invalid purchase_date: {e}"))?;
        Ok(r)
    }
}

fn date(s: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").map_err(|e| format!("bad date '{s}': {e}"))
}

/// The EU conformity guarantee baseline (Directive 2019/771): two years.
pub const CONFORMITY_BASE_DAYS: i64 = 2 * 365;

/// Coverage window under 2019/771: purchase date through purchase +
/// 730 days, extended by the member-state limitation extension where one
/// applies (several states extend beyond two years). Returns `(start, end)`
/// as inclusive civil dates.
pub fn coverage_window_771(
    purchase_date: &str,
    limitation_extension_days: i64,
) -> Result<(NaiveDate, NaiveDate), String> {
    let start = date(purchase_date)?;
    let end = start + Duration::days(CONFORMITY_BASE_DAYS + limitation_extension_days);
    Ok((start, end))
}

/// Whether `on` falls inside a 2019/771 coverage window.
pub fn covered_on_771(purchase_date: &str, on: &str, extension_days: i64) -> Result<bool, String> {
    let (start, end) = coverage_window_771(purchase_date, extension_days)?;
    let day = date(on)?;
    Ok(day >= start && day <= end)
}

/// Withdrawal exceptions (Directive 2011/83 art. 13 posture): the closed
/// table of reasons the standard 14-day right changes shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WithdrawalException {
    /// Standard distance-sale withdrawal: full 14 days.
    Standard,
    /// Custom-made / clearly-personalized goods: no withdrawal right.
    MadeToOrder,
    /// Sealed goods unsealed after delivery (hygiene/data): no withdrawal.
    SealedGoodsUnsealed,
    /// Separate deliveries: the window runs from the LAST delivery.
    MultipleDeliveries,
}

/// Compute the withdrawal window for an order. Returns `Some((start, end))`
/// or `None` where the exception removes the right entirely.
pub fn withdrawal_window(
    order_date: &str,
    last_delivery_date: Option<&str>,
    exception: WithdrawalException,
) -> Result<Option<(NaiveDate, NaiveDate)>, String> {
    match exception {
        WithdrawalException::MadeToOrder | WithdrawalException::SealedGoodsUnsealed => Ok(None),
        WithdrawalException::Standard => {
            let start = date(order_date)?;
            Ok(Some((start, start + Duration::days(14))))
        }
        WithdrawalException::MultipleDeliveries => {
            let d = last_delivery_date
                .ok_or_else(|| "multiple_deliveries requires last_delivery_date".to_string())?;
            let start = date(d)?;
            Ok(Some((start, start + Duration::days(14))))
        }
    }
}

/// Geographic rule: an entitlement row answers only inside its own region.
/// An unstamped server (`stamp` empty) sees unstamped rows; a stamped
/// server sees its own rows only — cross-region rows are FOREIGN (the same
/// residency law parcels obey), and fail CLOSED.
pub fn region_allows(record_region: &str, stamp: &str) -> bool {
    if stamp.is_empty() {
        return record_region.is_empty() || record_region == stamp;
    }
    record_region == stamp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entitlement_window_computes_771_extension() {
        // Baseline: two years of conformity coverage from purchase.
        let (s, e) = coverage_window_771("2026-01-10", 0).expect("window computes");
        assert_eq!(s.to_string(), "2026-01-10");
        assert_eq!(e.to_string(), "2028-01-10");
        // A member-state limitation extension lengthens the tail.
        let (_, ext) = coverage_window_771("2026-01-10", 365).expect("extension computes");
        assert_eq!(ext.to_string(), "2029-01-09");
        assert!(ext > e);
        // Day-inside / day-outside are exact boundaries.
        assert!(covered_on_771("2026-01-10", "2028-01-10", 0).expect("inside"));
        assert!(!covered_on_771("2026-01-10", "2028-01-11", 0).expect("outside"));
        // A row parses and carries its traceability spine.
        let row = EntitlementRecord::parse(
            r#"{"product":"laptop-x1","serial":"SN-001","purchase_date":"2026-01-10","region":"eu"}"#,
        )
        .expect("row parses");
        assert_eq!(row.serial, "SN-001");
        assert!(EntitlementRecord::parse("{not json").is_err());
        assert!(
            EntitlementRecord::parse(
                r#"{"product":"p","serial":"s","purchase_date":"2026-13-99"}"#
            )
            .is_err(),
            "a row with an impossible date never grants coverage"
        );
    }

    #[test]
    fn withdrawal_window_14_days_computes_with_exceptions_table() {
        use WithdrawalException::*;
        // Standard: 14 days from the order date.
        let w = withdrawal_window("2026-03-01", None, Standard).expect("standard");
        assert_eq!(w.expect("some").1.to_string(), "2026-03-15");
        // Made-to-order: the right does not exist at all.
        assert!(
            withdrawal_window("2026-03-01", None, MadeToOrder)
                .expect("computes")
                .is_none()
        );
        assert!(
            withdrawal_window("2026-03-01", None, SealedGoodsUnsealed)
                .expect("computes")
                .is_none()
        );
        // Separate deliveries: the clock starts at the LAST delivery.
        let multi = withdrawal_window("2026-03-01", Some("2026-03-20"), MultipleDeliveries)
            .expect("multi")
            .expect("some");
        assert_eq!(multi.0.to_string(), "2026-03-20");
        assert_eq!(multi.1.to_string(), "2026-04-03");
        assert!(withdrawal_window("2026-03-01", None, MultipleDeliveries).is_err());
    }

    #[test]
    fn region_rules_honor_stamp_fail_closed() {
        // Unstamped server: unstamped rows only.
        assert!(region_allows("", ""));
        assert!(!region_allows("eu", ""));
        // Stamped server: own rows in, every foreign row out.
        assert!(region_allows("eu", "eu"));
        assert!(!region_allows("us", "eu"));
        assert!(
            !region_allows("", "eu"),
            "unstamped row is foreign to a stamped site"
        );
    }
}
