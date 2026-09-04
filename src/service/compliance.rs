//! The compliance-pack evidence storage: Art.14 oversight evidence, the
//! evidence-inventory counts, and the GDPR Art.30 RoPA register.
//!
//! OWNS the compliance-pack aggregates' storage story:
//! - `record_oversight`: the oversight-evidence write — one signed decision
//!   ledger row + one `oversight_evidence` row linked by hash. Best-effort
//!   BY CONTRACT (returns Option; evidence must never fail the primary
//!   action it evidences) — do not "upgrade" this to a hard error during a
//!   move;
//! - `evidence_counts`: the six per-connection table counts behind the
//!   inventory checker, `.unwrap_or(-1)` per table (an unreadable table is
//!   `absent`, never a 500 — the documented fail-open posture). The
//!   max-across-domains merge stays handler-side (pool orchestration);
//! - `ropa_rows`: the RoPA register read. The rows are the legacy
//!   `serde_json::Value` maps — the byte-for-byte wire shape outranks the
//!   domain-type aspiration (the retention-report ceiling);
//! - `ropa_upsert_tx`: the register write — UPDATE (rows-affected IS the
//!   404 signal) or INSERT, plus its audit row INSIDE the caller's tx
//!   (evidence commits with the write).
//!
//! Feature-gated with the handler module (`compliance-pack`); the wire caps
//! (`capped`), the SHA-256 predicate, the export/evaluation-record ledger
//! writes, and every gate stay handler-side.

use std::fmt;

use rusqlite::{Connection, params};

/// A storage failure. `Database`'s Display carries the exact pre-move
/// message; the handler wraps it in `HandlerError::internal` unchanged.
/// `NotFound` maps to the route's frozen probe-blind 404.
#[derive(Debug)]
pub(crate) enum ComplianceError {
    Database(String),
    NotFound(String),
}

impl fmt::Display for ComplianceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ComplianceError::Database(m) => f.write_str(m),
            ComplianceError::NotFound(m) => f.write_str(m),
        }
    }
}

impl From<rusqlite::Error> for ComplianceError {
    fn from(e: rusqlite::Error) -> Self {
        ComplianceError::Database(e.to_string())
    }
}

/// Append one oversight-evidence row linked to a fresh decision record.
/// Best-effort like the audit chain: evidence must never fail the primary
/// action. `basis` is the snapshot hash of what the reviewer saw (the
/// review digest — never raw content); `outcome` ∈ accept|modify|override.
pub(crate) fn record_oversight(
    conn: &rusqlite::Connection,
    reviewer_id: &str,
    basis: &str,
    outcome: &str,
    authority: &str,
    proposal_id: Option<i64>,
    domain: &str,
) -> Option<i64> {
    let decision = crate::audit::decision::record_decision(
        conn,
        &crate::audit::decision::DecisionInput {
            actor_id: reviewer_id,
            role: authority,
            policy_version: env!("CARGO_PKG_VERSION"),
            prompt_class: "review",
            tool: "oversight",
            model_id: "",
            outcome,
        },
    )?;
    let decision_hash = decision.hash.clone();
    conn.execute(
        "INSERT INTO oversight_evidence(reviewer_id, reviewed_at, basis, outcome, authority, decision_hash, proposal_id, domain)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            reviewer_id,
            chrono::Utc::now().timestamp(),
            basis,
            outcome,
            authority,
            decision_hash,
            proposal_id,
            domain
        ],
    )
    .ok()?;
    Some(conn.last_insert_rowid())
}

/// The evidence-inventory counts across one connection's schema.
#[derive(Default)]
pub(crate) struct InventoryCounts {
    pub(crate) decisions: i64,
    pub(crate) oversight: i64,
    pub(crate) dsar: i64,
    pub(crate) incidents: i64,
    pub(crate) transfers: i64,
    pub(crate) ropa: i64,
}

/// The six table counts behind the inventory checker. Per-table
/// `.unwrap_or(-1)`: an unreadable table reads as `absent` (-1), never a
/// failed request — the documented posture, verbatim.
pub(crate) fn evidence_counts(conn: &Connection) -> InventoryCounts {
    let mut counts = InventoryCounts::default();
    let n = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap_or(-1) };
    counts.decisions = counts
        .decisions
        .max(n("SELECT COUNT(*) FROM decision_records"));
    counts.oversight = counts
        .oversight
        .max(n("SELECT COUNT(*) FROM oversight_evidence"));
    counts.dsar = counts.dsar.max(n("SELECT COUNT(*) FROM dsar_requests"));
    counts.incidents = counts.incidents.max(n("SELECT COUNT(*) FROM breaches"));
    counts.transfers = counts.transfers.max(n("SELECT COUNT(*) FROM transfers"));
    counts.ropa = counts.ropa.max(n("SELECT COUNT(*) FROM ropa_registry"));
    counts
}

/// The RoPA register, id order. Legacy JSON map rows — byte-for-byte wire
/// ceiling (do not type these during a move).
pub(crate) fn ropa_rows(conn: &Connection) -> Result<Vec<serde_json::Value>, ComplianceError> {
    let mut stmt = conn
        .prepare(
            "SELECT id, activity, controller, processor, categories, recipients,
                    lawful_basis, retention_days, security_measures, transfers, updated_at
             FROM ropa_registry ORDER BY id",
        )
        .map_err(ComplianceError::from)?;
    let rows = stmt
        .query_map([], |r| {
            Ok(serde_json::json!({
                "id": r.get::<_, i64>(0)?,
                "activity": r.get::<_, String>(1)?,
                "controller": r.get::<_, String>(2)?,
                "processor": r.get::<_, String>(3)?,
                "categories": r.get::<_, String>(4)?,
                "recipients": r.get::<_, String>(5)?,
                "lawful_basis": r.get::<_, String>(6)?,
                "retention_days": r.get::<_, Option<i64>>(7)?,
                "security_measures": r.get::<_, String>(8)?,
                "transfers": r.get::<_, String>(9)?,
                "updated_at": r.get::<_, i64>(10)?,
            }))
        })
        .map_err(ComplianceError::from)?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(ComplianceError::from)
}

/// One Art.30 register entry (the handler's parsed request body — plain
/// data, no transport types).
#[derive(Debug, serde::Deserialize)]
pub struct RopaInput {
    pub activity: String,
    pub controller: String,
    pub processor: String,
    #[serde(default)]
    pub categories: String,
    #[serde(default)]
    pub recipients: String,
    pub lawful_basis: String,
    #[serde(default)]
    pub retention_days: Option<i64>,
    #[serde(default)]
    pub security_measures: String,
    #[serde(default)]
    pub transfers: String,
}

/// The RoPA register write, inside the CALLER'S tx: UPDATE when `id` is
/// given (the rows-affected count IS the 404 signal) else INSERT, then the
/// audit row in the SAME tx. The caller owns begin/commit; the field caps
/// stay at the handler.
pub(crate) fn ropa_upsert_tx(
    tx: &rusqlite::Transaction<'_>,
    id: Option<i64>,
    body: &RopaInput,
    now: i64,
    actor: &str,
) -> Result<i64, ComplianceError> {
    let rid = match id {
        Some(rid) => {
            let n = tx
                .execute(
                    "UPDATE ropa_registry SET activity=?2, controller=?3, processor=?4,
                        categories=?5, recipients=?6, lawful_basis=?7, retention_days=?8,
                        security_measures=?9, transfers=?10, updated_at=?11 WHERE id=?1",
                    params![
                        rid,
                        body.activity,
                        body.controller,
                        body.processor,
                        body.categories,
                        body.recipients,
                        body.lawful_basis,
                        body.retention_days,
                        body.security_measures,
                        body.transfers,
                        now
                    ],
                )
                .map_err(ComplianceError::from)?;
            if n == 0 {
                return Err(ComplianceError::NotFound(format!(
                    "no RoPA activity with id {rid}"
                )));
            }
            rid
        }
        None => {
            tx.execute(
                "INSERT INTO ropa_registry(activity, controller, processor, categories,
                      recipients, lawful_basis, retention_days, security_measures,
                      transfers, created_at, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10)",
                params![
                    body.activity,
                    body.controller,
                    body.processor,
                    body.categories,
                    body.recipients,
                    body.lawful_basis,
                    body.retention_days,
                    body.security_measures,
                    body.transfers,
                    now
                ],
            )
            .map_err(ComplianceError::from)?;
            tx.last_insert_rowid()
        }
    };
    crate::audit::record(
        tx,
        crate::audit::AuditKind::Client,
        actor,
        &format!("ropa:{rid}"),
        crate::audit::AuditStatus::Ok,
        "ropa_upserted",
    );
    Ok(rid)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The decision signing key resolves once per process from the env; the
    /// oversight pins install the same fixed seed under the crate-wide
    /// decision lock so signatures verify deterministically.
    fn ensure_test_key() -> std::sync::MutexGuard<'static, ()> {
        let _g = crate::audit::decision::decision_test_lock();
        crate::audit::decision::install_test_signing_key([7u8; 32]);
        _g
    }

    fn db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::audit::decision::DDL).unwrap();
        conn.execute_batch(
            "CREATE TABLE oversight_evidence(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                reviewer_id TEXT NOT NULL,
                reviewed_at INTEGER NOT NULL,
                basis TEXT NOT NULL,
                outcome TEXT NOT NULL,
                authority TEXT NOT NULL DEFAULT '',
                decision_hash TEXT,
                proposal_id INTEGER,
                domain TEXT NOT NULL DEFAULT 'global'
             );",
        )
        .unwrap();
        conn
    }

    #[test]
    fn oversight_links_a_signed_decision_record() {
        let _key = ensure_test_key();
        let conn = db();
        let id = record_oversight(
            &conn,
            "dpo-1",
            "digest-abc",
            "accept",
            "approve",
            Some(7),
            "global",
        )
        .unwrap();
        assert_eq!(id, 1);
        let (hash, outcome): (String, String) = conn
            .query_row(
                "SELECT decision_hash, outcome FROM oversight_evidence WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(outcome, "accept");
        // the linked decision record exists and carries the same hash
        let stored: String = conn
            .query_row(
                "SELECT hash FROM decision_records WHERE hash = ?1",
                rusqlite::params![hash],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, hash);
        let _g = crate::audit::decision::decision_test_lock();
        assert!(crate::audit::decision::verify_decisions(&conn).unwrap());
    }

    /// The tamper-evidence twin: flipping one bit of a stored signature must
    /// fail full-chain verification. Exercises the same signed chain that
    /// [`record_oversight`] writes (moved here with the oversight core;
    /// assertions unchanged from the handler-side pin it was).
    #[test]
    fn tampered_signature_fails_verification() {
        let _key = ensure_test_key();
        let conn = db();
        record_oversight(
            &conn,
            "dpo-1",
            "d",
            "override",
            "reject",
            Some(9),
            "acme-us",
        );
        let n = conn
            .execute(
                "UPDATE decision_records SET sig = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'",
                [],
            )
            .unwrap();
        assert_eq!(n, 1);
        let _g = crate::audit::decision::decision_test_lock();
        assert!(!crate::audit::decision::verify_decisions(&conn).unwrap());
    }

    #[test]
    fn ropa_upsert_audits_inside_its_tx_and_404s_on_missing_id() {
        crate::register_sqlite_vec::register_sqlite_vec();
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::migration::run_migration(&mut conn, 0)
            .unwrap_or_else(|e| panic!("in-memory migration must succeed: {e}"));
        let body = RopaInput {
            activity: "payroll".into(),
            controller: "acme".into(),
            processor: "acme hr".into(),
            categories: String::new(),
            recipients: String::new(),
            lawful_basis: "contract".into(),
            retention_days: Some(3650),
            security_measures: String::new(),
            transfers: String::new(),
        };
        let tx = conn.transaction().unwrap();
        let rid = ropa_upsert_tx(&tx, None, &body, 1_000, "dpo-1").unwrap();
        tx.commit().unwrap();
        // The audit row rode the SAME tx: the register entry and its
        // evidence exist together.
        let audited: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE target = ?1",
                rusqlite::params![format!("ropa:{rid}")],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(audited, 1, "the upsert is evidenced in-tx");
        // Update on a missing id is the typed NotFound (404 at the handler).
        let tx = conn.transaction().unwrap();
        let err = ropa_upsert_tx(&tx, Some(999), &body, 2_000, "dpo-1").unwrap_err();
        tx.rollback().unwrap();
        assert!(matches!(err, ComplianceError::NotFound(_)));
    }
}
