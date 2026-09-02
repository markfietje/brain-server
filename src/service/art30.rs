//! The Art.30 register's data reads: categories, connector recipients,
//! DSAR history, and the chunk-lifecycle counts.
//!
//! OWNS the register's read story; the register JSON itself (purposes,
//! legal bases, transfer sections, provenance fields) is the handler's
//! wire shape. The per-site error postures are the contract, kept
//! verbatim:
//! - `node_kind_counts`: FAILS the request on a storage error (the
//!   categories section is the register's core — absence would
//!   misrepresent the processing);
//! - `connector_recipients` / `dsar_history`: BEST-EFFORT (a failed
//!   section reads as absent — `if let Ok` posture at the caller);
//! - `lifecycle_counts`: FAIL-OPEN per count (`unwrap_or(0)` — a degraded
//!   DB reads zero, never a failed register).
//!
//! Read-only aggregate: `&Connection` in, no tx, no audit owed.

use rusqlite::Connection;

/// One (kind, count) category row.
pub(crate) type KindCount = (String, i64);

/// One registered connector: (kind, instance).
pub(crate) type ConnectorRow = (String, String);

/// The knowledge categories: per-node-kind counts, kind-ordered. Errors
/// propagate — the caller fails the request.
pub(crate) fn node_kind_counts(conn: &Connection) -> rusqlite::Result<Vec<KindCount>> {
    let mut stmt = conn.prepare(
        "SELECT node_kind, COUNT(*) FROM knowledge GROUP BY node_kind ORDER BY node_kind",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
    Ok(rows.flatten().collect())
}

/// The registered connectors, kind-ordered. The caller treats errors as
/// "no recipients section".
pub(crate) fn connector_recipients(conn: &Connection) -> rusqlite::Result<Vec<ConnectorRow>> {
    let mut stmt = conn.prepare("SELECT kind, instance FROM connectors ORDER BY kind")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    Ok(rows.flatten().collect())
}

/// The DSAR exercise history: (action, status, count), action-ordered. The
/// caller treats errors as "no history section".
pub(crate) fn dsar_history(conn: &Connection) -> rusqlite::Result<Vec<(String, String, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT action, status, COUNT(*) FROM dsar_requests GROUP BY action, status ORDER BY action",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;
    Ok(rows.flatten().collect())
}

/// The chunk-lifecycle summary: (live, superseded, tombstoned). Each count
/// is fail-open (0 on a read error) — the documented register posture.
pub(crate) fn lifecycle_counts(conn: &Connection) -> (i64, i64, i64) {
    let live: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM knowledge WHERE valid_to IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let superseded: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM knowledge WHERE valid_to IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let tombstoned: i64 = conn
        .query_row("SELECT COUNT(*) FROM tombstones", [], |r| r.get(0))
        .unwrap_or(0);
    (live, superseded, tombstoned)
}
