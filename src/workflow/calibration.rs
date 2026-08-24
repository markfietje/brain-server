//! Calibration persistence: weekly REPORT + monthly human-signed gate, both
//! riding the workflow audit chain. The audit row carries the hashed record;
//! `schema_meta` stamps the cadence/baseline state so staleness is checkable
//! without trusting any client input.

use brain_engine_sdk::calibration::{
    CalibrationRecord, NO_KAPPA, month_due, month_index, week_due,
};
use rusqlite::{Connection, OptionalExtension};

/// Meta keys — one owner, written only through this module.
const KEY_LAST_REPORT_AT: &str = "calibration_last_report_at";
const KEY_LAST_SIGNED_MONTH: &str = "calibration_last_signed_month";
const KEY_BASELINE_UNITS: &str = "calibration_baseline_units";
const KEY_LAST_KAPPA_UNITS: &str = "calibration_last_kappa_units";

fn meta_get(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM schema_meta WHERE key = ?1", [key], |r| {
        r.get::<_, String>(0)
    })
    .optional()
    .ok()
    .flatten()
}

fn meta_set(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO schema_meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )
}

pub(crate) fn last_report_at(conn: &Connection) -> Option<i64> {
    meta_get(conn, KEY_LAST_REPORT_AT)?.parse().ok()
}

pub(crate) fn last_baseline_units(conn: &Connection) -> Option<i32> {
    meta_get(conn, KEY_BASELINE_UNITS)?.parse().ok()
}

pub(crate) fn last_kappa_units(conn: &Connection) -> i32 {
    meta_get(conn, KEY_LAST_KAPPA_UNITS)
        .and_then(|v| v.parse().ok())
        .unwrap_or(NO_KAPPA)
}

/// Whether a weekly calibration REPORT is due right now.
pub(crate) fn report_due(conn: &Connection, now: i64) -> bool {
    week_due(last_report_at(conn), now)
}

/// Whether the monthly human-signed gate blocks a sign-off attempt.
pub(crate) fn signature_blocked(conn: &Connection, now: i64) -> bool {
    !month_due(
        meta_get(conn, KEY_LAST_SIGNED_MONTH).and_then(|v| v.parse().ok()),
        month_index(now),
    )
}

/// Emit a calibration REPORT (machine-generated, reviewer empty): the audit
/// row lands first (best-effort chain), then the cadence/baseline stamps.
/// Stamps are transactional with each other; a failed stamp bumps nothing and
/// the next call re-reports (idempotent at the gate).
pub(crate) fn record_report(
    conn: &Connection,
    score_units: i32,
    now: i64,
    kcs_summary: &str,
) -> Result<(), rusqlite::Error> {
    let baseline = last_baseline_units(conn);
    // Baseline absent → uplift 0 (the first report anchors, never scores).
    let uplift = baseline.map_or(0, |b| score_units - b);
    let record = CalibrationRecord::new(last_kappa_units(conn), uplift, "");
    let detail = if kcs_summary.is_empty() {
        record.detail()
    } else {
        format!("{} {kcs_summary}", record.detail())
    };
    super::audit_write_global(
        conn,
        "calibration/report",
        crate::audit::AuditStatus::Ok,
        &detail,
    );
    meta_set(conn, KEY_LAST_REPORT_AT, &now.to_string())?;
    if baseline.is_none() {
        meta_set(conn, KEY_BASELINE_UNITS, &score_units.to_string())?;
    }
    Ok(())
}

/// Record a human-signed monthly calibration (the drift gate). Caller has
/// already enforced Admin/DPO authorization; `reviewer_id` is bounded there.
pub(crate) fn record_signed(
    conn: &Connection,
    kappa_units: i32,
    score_units: i32,
    reviewer_id: &str,
    now: i64,
) -> Result<(), rusqlite::Error> {
    let uplift = match last_baseline_units(conn) {
        Some(b) => score_units - b,
        None => 0,
    };
    let record = CalibrationRecord::new(kappa_units, uplift, reviewer_id);
    super::audit_write_global(
        conn,
        "calibration/sign",
        crate::audit::AuditStatus::Ok,
        &record.detail(),
    );
    meta_set(conn, KEY_LAST_SIGNED_MONTH, &month_index(now).to_string())?;
    meta_set(conn, KEY_LAST_KAPPA_UNITS, &kappa_units.to_string())?;
    // A human signature re-anchors the baseline: deltas stay OUR-vs-US.
    meta_set(conn, KEY_BASELINE_UNITS, &score_units.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::verify_chain;
    use brain_server::migration::run_migration;

    fn db() -> Connection {
        crate::register_sqlite_vec();
        let mut conn = Connection::open_in_memory().unwrap();
        run_migration(&mut conn, 1).unwrap();
        conn
    }

    #[test]
    fn report_due_then_stamped_for_a_week() {
        let conn = db();
        assert!(report_due(&conn, 1000));
        record_report(&conn, 9000, 1000, "").unwrap();
        assert!(!report_due(&conn, 1000 + 7 * 86400 - 1));
        assert!(report_due(&conn, 1000 + 7 * 86400));
        // First report anchors the baseline; uplift 0.
        assert_eq!(last_baseline_units(&conn), Some(9000));
    }

    #[test]
    fn signed_gate_blocks_within_month_and_anchors_kappa() {
        let conn = db();
        record_report(&conn, 8000, 1000, "").unwrap();
        assert!(!signature_blocked(&conn, 2000));
        record_signed(&conn, 8500, 8200, "dpo-1", 2000).unwrap();
        assert!(signature_blocked(&conn, 3000));
        assert_eq!(last_kappa_units(&conn), 8500);
        // Signature re-anchored the baseline to the signed score.
        assert_eq!(last_baseline_units(&conn), Some(8200));
        // A later report carries the human κ, not the sentinel.
        record_report(&conn, 8300, 1000 + 8 * 86400, "").unwrap();
        assert!(verify_chain(&conn), "chain stays green");
    }

    #[test]
    fn records_land_on_the_audit_chain_with_their_detail() {
        let conn = db();
        record_report(&conn, 9000, 100, "").unwrap();
        record_signed(&conn, 8000, 9100, "dpo-9", 200).unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE kind='workflow'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 2, "both records audited");
        // The chain stores hashes (target_hash/detail_hash), never raw text.
        let hashes: (bool, bool) = conn
            .query_row(
                "SELECT COUNT(*) = 2, COUNT(detail_hash) = 2 FROM audit_events WHERE kind='workflow'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert!(hashes.0 && hashes.1, "two rows, both with detail hashes");
        assert!(verify_chain(&conn));
    }
}
