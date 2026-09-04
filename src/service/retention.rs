//! The retention core (the Foundation Line exemplar) — the per-kind
//! retention-policy aggregate, moved verbatim out of the govern handler.
//!
//! OWNS (this aggregate's complete storage story):
//! - the persisted overrides in `retention_policy` (read + upsert);
//! - the per-kind `knowledge` counts (the coverage view an operator reads
//!   before changing a policy);
//! - the retention-schedule report rows (domain × kind: ttl, count,
//!   expiring-window count);
//! - the evidence audit row a policy override owes — written INSIDE the
//!   caller's transaction (SAVEPOINT-nested via `audit::record`'s
//!   autocommit probe), so the override and its evidence commit or roll
//!   back together. Pre-move, that audit rode a SECOND pooled connection
//!   after the write had already committed — a crash between them left the
//!   override permanently unevidenced. That gap is what this move closes.
//!
//! FK-children map: `retention_policy` rows have NO dependents — nothing in
//! the schema references them, and this aggregate has no delete path
//! (overrides are overwritten by upsert, never removed; a future "clear"
//! is a governed act, not a silent delete).
//!
//! Bounds: `days` must be an integer in [1, 36500] and the kind non-empty —
//! asserted HERE so every future caller inherits the fence (the fence holds
//! of the FUNCTION, not call-site discipline). The wire route pre-validates
//! with the identical 400 outcome, so the probe-blind error vocabulary is
//! unchanged.
//!
//! Wire-shape ceiling (honest): report rows stay the legacy
//! `serde_json::Value` maps, built with the exact json! literal the handler
//! used pre-move — the
//! [`retention_report_rows_match_legacy_byte_for_byte`] pin outranks the
//! domain-type aspiration; typing them is a follow-up, NOT part of this
//! move.
//!
//! Non-goal: nothing here runs autonomously. Retention is applied at query
//! time by the retriever, never by a sweeper.

use rusqlite::{Connection, params};
use std::collections::BTreeMap;

use crate::audit::{AuditKind, AuditStatus};

/// The report window — rows whose effective expiry falls inside the next
/// [`REPORT_WINDOW_DAYS`] days are counted as "expiring soon".
pub const REPORT_WINDOW_DAYS: i64 = 30;

/// The override fence: a policy must be at least one day and at most 100
/// years. Identical to the bound the handler has always enforced; held here
/// so the storage boundary refuses what the API boundary misses.
pub const MIN_POLICY_DAYS: i64 = 1;
pub const MAX_POLICY_DAYS: i64 = 36_500;

/// Typed service error (the ServiceError convention: one enum per module).
/// `Database` carries the rusqlite text VERBATIM — the handler maps it onto
/// the route's frozen internal-error body byte-for-byte.
#[derive(Debug)]
pub enum RetentionError {
    /// A query failed; the rusqlite message travels unchanged.
    Database(String),
    /// `days` outside [1, 36500] — the storage-boundary fence; unreachable
    /// over the wire (the handler pre-validates with the identical 400).
    InvalidDays(i64),
    /// An empty kind — same fence posture.
    EmptyKind,
}

impl std::fmt::Display for RetentionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RetentionError::Database(e) => write!(f, "database error: {e}"),
            RetentionError::InvalidDays(d) => write!(
                f,
                "retention days must be an integer in [{MIN_POLICY_DAYS}, {MAX_POLICY_DAYS}] (got {d})"
            ),
            RetentionError::EmptyKind => write!(f, "retention kind must be a non-empty name"),
        }
    }
}

impl From<rusqlite::Error> for RetentionError {
    fn from(e: rusqlite::Error) -> Self {
        RetentionError::Database(e.to_string())
    }
}

/// The persisted overrides (kind → days), ordered by kind — the merge input
/// the effective policy is built from (code defaults + these).
pub fn effective_overrides(conn: &Connection) -> Result<Vec<(String, i64)>, RetentionError> {
    let mut stmt = conn.prepare("SELECT kind, days FROM retention_policy ORDER BY kind")?;
    let overridden: Vec<(String, i64)> = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
        .flatten()
        .collect();
    Ok(overridden)
}

/// Per-kind knowledge counts (the coverage view). Best-effort READ, exactly
/// the pre-move posture (`if let Ok` in the handler): a failed count query
/// logs loud and yields an empty map rather than failing the whole policy
/// view — the counts are advisory, the policy is the answer.
pub fn kind_counts(conn: &Connection) -> BTreeMap<String, i64> {
    let mut counts = BTreeMap::new();
    match conn.prepare("SELECT node_kind, COUNT(*) FROM knowledge GROUP BY node_kind") {
        Ok(mut cs) => match cs.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
        {
            Ok(rows) => {
                for row in rows.flatten() {
                    counts.insert(row.0, row.1);
                }
            }
            Err(e) => tracing::warn!("retention kind counts unreadable: {e}"),
        },
        Err(e) => tracing::warn!("retention kind counts unreadable: {e}"),
    }
    counts
}

/// Upsert the overrides AND write their evidence audit row inside the
/// CALLER'S transaction. Returns the total affected row count (the number
/// the receipt and the audit target carry). Any failure — including a
/// mid-loop failure — rolls the WHOLE set back (pre-move, each upsert
/// autocommitted on its own and a mid-loop crash could persist a partial
/// policy; that error-path hardening is the one intended behavior delta of
/// this move, on the failure path only).
///
/// The audit vocabulary is the legacy one, unchanged:
/// kind `reconcile`, actor `api`, target `retention:{n}`, status `ok`,
/// detail `retention_set`.
pub fn set_overrides(
    tx: &rusqlite::Transaction<'_>,
    entries: &[(String, i64)],
    now_unix: i64,
) -> Result<usize, RetentionError> {
    // The fence first: refuse the whole set before touching a row.
    for (kind, days) in entries {
        if kind.is_empty() {
            return Err(RetentionError::EmptyKind);
        }
        if !(*days >= MIN_POLICY_DAYS && *days <= MAX_POLICY_DAYS) {
            return Err(RetentionError::InvalidDays(*days));
        }
    }
    let mut affected = 0usize;
    for (kind, days) in entries {
        affected += tx.execute(
            "INSERT INTO retention_policy(kind, days, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(kind) DO UPDATE SET days = excluded.days, updated_at = excluded.updated_at",
            params![kind, days, now_unix],
        )?;
    }
    crate::audit::record(
        tx,
        AuditKind::Reconcile,
        "api",
        &format!("retention:{affected}"),
        AuditStatus::Ok,
        "retention_set",
    );
    Ok(affected)
}

/// One domain's retention schedule rows — the pure core of the report.
/// `policy` is the EFFECTIVE kind→days map for this domain (the caller merges
/// a bound profile's retention block over the server-wide map already). A
/// domain × kind row exists for every knowledge kind present OR every policy
/// kind (whichever is larger) — the schedule reports coverage even at zero
/// rows. `ttl_days` is `None` = the kind never decays by kind-default (a
/// per-chunk `expires_at` still counts toward expiry). `expiring_30d` counts
/// rows whose *effective* expiry (explicit `expires_at`, else kind-default
/// from `created_at`) falls inside the next 30 days.
pub fn report_rows(
    conn: &Connection,
    now_unix: i64,
    domain: &str,
    policy: &BTreeMap<String, i64>,
) -> Result<Vec<serde_json::Value>, RetentionError> {
    let mut present: Vec<String> = {
        let mut stmt =
            conn.prepare("SELECT DISTINCT node_kind FROM knowledge ORDER BY node_kind")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.flatten().collect()
    };
    for k in policy.keys() {
        if !present.contains(k) {
            present.push(k.clone());
        }
    }
    present.sort();
    let cutoff = now_unix + REPORT_WINDOW_DAYS * 86_400;
    let mut rows = Vec::with_capacity(present.len());
    for kind in present {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM knowledge WHERE node_kind = ?1",
            params![kind],
            |r| r.get(0),
        )?;
        let ttl = policy.get(&kind).copied();
        let expiring_30d: i64 = match ttl {
            Some(days) => {
                let mut stmt = conn.prepare(
                    "SELECT COUNT(*) FROM knowledge
                      WHERE node_kind = ?1 AND (
                        expires_at IS NOT NULL AND expires_at < ?2
                     OR expires_at IS NULL AND created_at IS NOT NULL
                        AND unixepoch(COALESCE(created_at,'1970-01-01 00:00:00'))
                            < ?2 - ?3 * 86400
                      )",
                )?;
                stmt.query_row(params![kind, cutoff, days], |r| r.get(0))?
            }
            None => conn.query_row(
                "SELECT COUNT(*) FROM knowledge
                      WHERE node_kind = ?1 AND expires_at IS NOT NULL AND expires_at < ?2",
                params![kind, cutoff],
                |r| r.get(0),
            )?,
        };
        rows.push(serde_json::json!({
            "domain": domain,
            "kind": kind,
            "ttl_days": ttl,
            "count": count,
            "expiring_30d": expiring_30d,
        }));
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migration;
    use crate::register_sqlite_vec::register_sqlite_vec;
    use crate::workflow::tx::WorkflowTx;

    fn db() -> Connection {
        register_sqlite_vec();
        let mut conn = Connection::open_in_memory().unwrap();
        run_migration(&mut conn, 1).unwrap();
        conn
    }

    /// Moved with its function from the govern handler (repointed: the call
    /// path changed, the assertions did not). Plan Verification 4: the
    /// retention report reflects the configured per-kind TTL + counts + the
    /// 30-day-expiring window. A kind with a policy reports expiring rows
    /// via created_at; a kind with no policy counts only explicit
    /// `expires_at`, and the schedule still reports the policy kind even at
    /// zero rows.
    #[test]
    fn retention_report_matches_policy() -> rusqlite::Result<()> {
        let conn = rusqlite::Connection::open_in_memory()?;
        conn.execute_batch(
            "CREATE TABLE knowledge(
                id INTEGER PRIMARY KEY,
                node_kind TEXT NOT NULL,
                expires_at INTEGER,
                created_at TEXT
             );
             INSERT INTO knowledge(node_kind, created_at) VALUES ('fact', '2020-01-01 00:00:00');
             INSERT INTO knowledge(node_kind, created_at) VALUES ('episodic', '2026-08-01 00:00:00');
             INSERT INTO knowledge(node_kind, expires_at) VALUES ('episodic', 1000000);",
        )?;
        let now = 1_800_000_000_i64; // fixed "today"
        let policy: std::collections::BTreeMap<String, i64> =
            [("fact".to_string(), 2555), ("episodic".to_string(), 90)]
                .into_iter()
                .collect();
        let rows = report_rows(&conn, now, "finance", &policy).expect("report for a known policy");

        let by_kind: std::collections::HashMap<&str, &serde_json::Value> = rows
            .iter()
            .map(|r| (r["kind"].as_str().expect("row has a kind"), r))
            .collect();
        assert_eq!(by_kind.len(), 2, "fact + episodic reported");
        // fact: policy TTL 2555d, 1 row, created 2020 → long expired → expiring.
        let fact = by_kind["fact"];
        assert_eq!(fact["ttl_days"], 2555);
        assert_eq!(fact["count"], 1);
        assert_eq!(
            fact["expiring_30d"], 1,
            "2020 fact expires within the window"
        );
        // episodic: policy TTL 90d; 1 explicit-expiry row + 1 created 2026-08.
        let ep = by_kind["episodic"];
        assert_eq!(ep["ttl_days"], 90);
        assert_eq!(ep["count"], 2);
        assert_eq!(ep["expiring_30d"], 2, "both episodic rows expiring");

        // A kind with no policy counts only explicit expires_at.
        let bare: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
        let rows = report_rows(&conn, now, "global", &bare).expect("report on a bare policy");
        let by_kind: std::collections::HashMap<&str, &serde_json::Value> = rows
            .iter()
            .map(|r| (r["kind"].as_str().expect("row has a kind"), r))
            .collect();
        assert_eq!(by_kind["fact"]["ttl_days"], serde_json::Value::Null);
        assert_eq!(
            by_kind["fact"]["expiring_30d"], 0,
            "no explicit expires_at on the fact"
        );

        // Coverage at zero rows: a policy kind absent from the data still ships.
        let policy: std::collections::BTreeMap<String, i64> =
            [("decision".to_string(), 365)].into_iter().collect();
        let rows =
            report_rows(&conn, now, "global", &policy).expect("report for the decision policy");
        let decision = rows
            .iter()
            .find(|r| r["kind"] == "decision")
            .expect("decision kind is reported");
        assert_eq!(decision["count"], 0);
        assert_eq!(decision["ttl_days"], 365);
        Ok(())
    }

    /// Foundation-Line pin: on this seeded fixture the moved core's
    /// serialized output is the byte sequence captured from the PRE-move
    /// handler function (v1.28.46) — the move changed the address, not one
    /// byte of the wire.
    #[test]
    fn retention_report_rows_match_legacy_byte_for_byte() {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            "CREATE TABLE knowledge(
                id INTEGER PRIMARY KEY,
                node_kind TEXT NOT NULL,
                expires_at INTEGER,
                created_at TEXT
             );
             INSERT INTO knowledge(node_kind, created_at) VALUES ('fact', '2020-01-01 00:00:00');
             INSERT INTO knowledge(node_kind, created_at) VALUES ('episodic', '2026-08-01 00:00:00');
             INSERT INTO knowledge(node_kind, expires_at) VALUES ('episodic', 1000000);",
        )
        .expect("seed fixture");
        let now = 1_800_000_000_i64;
        let policy: std::collections::BTreeMap<String, i64> =
            [("fact".to_string(), 2555), ("episodic".to_string(), 90)]
                .into_iter()
                .collect();
        let rows =
            report_rows(&conn, now, "finance", &policy).expect("report rows for the fixture");
        let wire = serde_json::to_string(&rows).expect("rows serialize");
        assert_eq!(
            wire,
            "[{\"count\":2,\"domain\":\"finance\",\"expiring_30d\":2,\"kind\":\"episodic\",\"ttl_days\":90},{\"count\":1,\"domain\":\"finance\",\"expiring_30d\":1,\"kind\":\"fact\",\"ttl_days\":2555}]",
            "the moved core must reproduce the pre-move output byte-for-byte"
        );
    }

    /// The audit-per-write law, proven: the override and its evidence audit
    /// row are visible INSIDE the same uncommitted transaction (the audit
    /// SAVEPOINT-nests — pre-move it rode a second pooled connection after
    /// the write had already committed) and both persist through commit.
    #[test]
    fn retention_override_audits_inside_the_tx() {
        let mut conn = db();
        let mut tx = WorkflowTx::begin(&mut conn).unwrap();
        let n = set_overrides(tx.tx(), &[("notes".to_string(), 90)], 1_700_000_000).unwrap();
        assert_eq!(n, 1, "one upserted policy row");

        let in_tx = tx.tx();
        let override_rows: i64 = in_tx
            .query_row("SELECT COUNT(*) FROM retention_policy", [], |r| r.get(0))
            .unwrap();
        assert_eq!(override_rows, 1, "the override rides the same tx");
        let audit_rows: i64 = in_tx
            .query_row(
                "SELECT COUNT(*) FROM audit_events
                  WHERE kind = 'reconcile' AND actor = 'api'
                    AND target_hash = ?1 AND detail_hash = ?2 AND status = 'ok'",
                params![
                    crate::audit::hash("retention:1"),
                    crate::audit::hash("retention_set")
                ],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            audit_rows, 1,
            "the evidence audit is written INSIDE the same uncommitted tx"
        );
        tx.commit().unwrap();

        let overrides = effective_overrides(&conn).unwrap();
        assert_eq!(overrides, vec![("notes".to_string(), 90)]);
    }

    /// The rollback twin: an uncommitted transition leaves NEITHER the
    /// override NOR its evidence — the audit cannot outlive the write it
    /// evidences (`audit_rolls_back_with_the_transition`, retention leg).
    #[test]
    fn retention_override_rolls_back_with_its_audit() {
        let mut conn = db();
        {
            let mut tx = WorkflowTx::begin(&mut conn).unwrap();
            set_overrides(tx.tx(), &[("notes".to_string(), 90)], 1_700_000_000).unwrap();
            drop(tx); // no commit → the guard rolls EVERYTHING back
        }
        let override_rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM retention_policy", [], |r| r.get(0))
            .unwrap();
        let audit_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE detail_hash = ?1",
                params![crate::audit::hash("retention_set")],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(override_rows, 0, "the rolled-back override is gone");
        assert_eq!(
            audit_rows, 0,
            "the evidence rolls back WITH the mutation — no unevidenced write survives"
        );
    }

    /// The storage-boundary fence: out-of-bound days and an empty kind are
    /// refused BEFORE any row is touched — a future caller inherits the
    /// bound even if it skips the handler's pre-validation.
    #[test]
    fn retention_set_refuses_out_of_bound_entries() {
        let mut conn = db();
        let mut tx = WorkflowTx::begin(&mut conn).unwrap();
        let err = set_overrides(tx.tx(), &[("notes".to_string(), 0)], 1_700_000_000).unwrap_err();
        assert!(matches!(err, RetentionError::InvalidDays(0)), "{err}");
        let err = set_overrides(
            tx.tx(),
            &[("notes".to_string(), MAX_POLICY_DAYS + 1)],
            1_700_000_000,
        )
        .unwrap_err();
        assert!(matches!(err, RetentionError::InvalidDays(36_501)), "{err}");
        let err = set_overrides(tx.tx(), &[("".to_string(), 30)], 1_700_000_000).unwrap_err();
        assert!(matches!(err, RetentionError::EmptyKind), "{err}");
        tx.commit().unwrap();
        let override_rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM retention_policy", [], |r| r.get(0))
            .unwrap();
        assert_eq!(override_rows, 0, "a refused set writes nothing");
    }

    /// The coverage read executes on a migrated schema and yields the empty
    /// map on an empty store (best-effort read posture, no panic path).
    #[test]
    fn kind_counts_on_an_empty_store_is_an_empty_map() {
        let conn = db();
        let counts = kind_counts(&conn);
        assert!(counts.is_empty());
    }
}
