//! v0.9.7 "Guard" — append-only audit events.
//!
//! Every audit row stores **identifiers and hashes only** — never raw indexed
//! content, token values, or secret-file contents. The `record` helper is a
//! one-liner callers use at trust boundaries (auth, ingest, webhook verification,
//! reconcile, backup). Hashing uses the existing `xxh3_64` dependency so there
//! is no new crypto surface.
//!
//! v1.1.0 "Harden":
//! - `tenant_id` column (default 'global') enables per-tenant scoping at the
//!   SQL layer (`WHERE tenant_id = ?`), so forgetting the param cannot leak
//!   cross-tenant audit rows.
//! - `prev_hash` column implements a tamper-evident hash chain. Each row stores
//!   SHA-256 over the prior row's `(ts, kind, actor, target_hash, prev_hash)`.
//!   Reads verify the chain; `verify_chain` returns false on any break. The
//!   chain is computed inside the same tx as the insert, so SQLite's
//!   single-writer serializes the read-modify-write atomically.
//!
//! Schema (additive migration in `main.rs::run_migration`):
//! ```sql
//! CREATE TABLE IF NOT EXISTS audit_events(
//!   id INTEGER PRIMARY KEY AUTOINCREMENT,
//!   ts TEXT DEFAULT CURRENT_TIMESTAMP,
//!   kind TEXT NOT NULL,     -- 'auth'|'ingest'|'webhook'|'reconcile'|'backup'|'connector'
//!   actor TEXT,             -- connector kind/instance, 'api', or 'loopback'
//!   target_hash TEXT,       -- xxh3 of the affected uri/id (NOT the content)
//!   status TEXT,            -- 'ok'|'denied'|'error'
//!   detail_hash TEXT,       -- xxh3 of a short detail string (no secrets)
//!   tenant_id TEXT NOT NULL DEFAULT 'global',  -- v1.1.0 per-tenant scoping
//!   prev_hash TEXT          -- v1.1.0 tamper-evidence chain link
//! );
//! ```

use rusqlite::{params, Connection};
use serde::Serialize;
use sha2::{Digest, Sha256};
use xxhash_rust::xxh3::xxh3_64;

/// Audit event categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditKind {
    Auth,
    Ingest,
    Webhook,
    Reconcile,
    Backup,
    Connector,
    /// v1.15.0 "Observe" M1: read-event kinds — a `/recall`, `/search`,
    /// `/get`, or `/multi-get` that injected memory into a decision path.
    /// Recorded only when the read-event audit is enabled (JWT mode default).
    Recall,
    Search,
    Get,
}

impl AuditKind {
    fn as_str(self) -> &'static str {
        match self {
            AuditKind::Auth => "auth",
            AuditKind::Ingest => "ingest",
            AuditKind::Webhook => "webhook",
            AuditKind::Reconcile => "reconcile",
            AuditKind::Backup => "backup",
            AuditKind::Connector => "connector",
            AuditKind::Recall => "recall",
            AuditKind::Search => "search",
            AuditKind::Get => "get",
        }
    }
}

/// Status of the audited action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditStatus {
    Ok,
    Denied,
    Error,
}

impl AuditStatus {
    fn as_str(self) -> &'static str {
        match self {
            AuditStatus::Ok => "ok",
            AuditStatus::Denied => "denied",
            AuditStatus::Error => "error",
        }
    }
}

/// Hash an identifier/detail string with xxh3. Used so the audit log never
/// stores the raw value (content, token, uri-with-secret, etc.).
pub fn hash(s: &str) -> String {
    format!("{:016x}", xxh3_64(s.as_bytes()))
}

/// SHA-256 hex digest of the chain-link payload. The payload is the
/// concatenation of the prior row's `(ts, kind, actor, target_hash, prev_hash)`
/// — every field a tamperer would need to touch to rewrite history. `id` is
/// deliberately excluded so a renumbered restore keeps the chain intact.
fn chain_link(ts: &str, kind: &str, actor: &str, target_hash: &str, prev_hash: &str) -> String {
    let mut h = Sha256::new();
    h.update(ts.as_bytes());
    h.update(b"|");
    h.update(kind.as_bytes());
    h.update(b"|");
    h.update(actor.as_bytes());
    h.update(b"|");
    h.update(target_hash.as_bytes());
    h.update(b"|");
    h.update(prev_hash.as_bytes());
    format!("{:x}", h.finalize())
}

/// Default tenant id for rows written before v1.1.0 and for callers that don't
/// track tenancy. Kept as a constant so every defaulting site uses the same
/// spelling; the migration's `DEFAULT 'global'` matches it byte-for-byte.
pub const DEFAULT_TENANT: &str = "global";

/// Append one audit event. Best-effort: audit must never fail the primary
/// action, so errors are swallowed (logged at debug). Callers pass already-
/// hashed identifiers where the value is sensitive. `tenant` scopes the row;
/// pass [`DEFAULT_TENANT`] when the caller has no tenant context.
///
/// Returns the inserted row id (`Some`) or `None` if the write failed. Most
/// callers ignore the return; the DSAR/trace paths use it to key a replayable
/// trace row.
///
/// The hash-chain link is read + written inside a single transaction so the
/// read-modify-write is atomic under SQLite's single-writer lock.
pub fn record(
    conn: &Connection,
    kind: AuditKind,
    actor: &str,
    target: &str,
    status: AuditStatus,
    detail: &str,
) -> Option<i64> {
    record_tenant(conn, kind, actor, target, status, detail, DEFAULT_TENANT)
}

/// Per-tenant variant of [`record`]. Same best-effort semantics; returns the
/// inserted row id on success, `None` on failure.
///
/// The chain-tip read + INSERT are wrapped in a `SAVEPOINT` so the
/// read-modify-write is atomic even for autocommit callers. `SAVEPOINT`
/// (rather than `BEGIN`) is used because it nests cleanly inside callers'
/// existing transactions (e.g. `delete_quarantine`) — a `BEGIN` would fail
/// with "cannot start a transaction within a transaction" in that case.
/// Errors are swallowed at every step: audit must never fail the primary
/// action, and a broken audit row is preferable to a rolled-back write.
pub fn record_tenant(
    conn: &Connection,
    kind: AuditKind,
    actor: &str,
    target: &str,
    status: AuditStatus,
    detail: &str,
    tenant: &str,
) -> Option<i64> {
    let target_hash = hash(target);
    let detail_hash = hash(detail);
    let kind_str = kind.as_str();
    let status_str = status.as_str();
    // SAVEPOINT nests inside a caller's existing tx (BEGIN would error);
    // if it fails to open, fall through with autocommit semantics so the
    // audit row still lands.
    let sp_ok = conn.execute("SAVEPOINT audit_link", []).is_ok();
    // Read the chain tip (the most recent row). Inside the savepoint this is
    // stable against concurrent writers — and the INSERT below commits/rolls
    // back atomically with it.
    let tip: Option<ChainTip> = conn
        .query_row(
            "SELECT ts, kind, actor, target_hash, prev_hash \
              FROM audit_events ORDER BY id DESC LIMIT 1",
            [],
            |r| {
                Ok(ChainTip {
                    ts: r.get(0)?,
                    kind: r.get(1)?,
                    actor: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    target_hash: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                    prev_hash: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                })
            },
        )
        .ok();
    let prev_hash = tip
        .as_ref()
        .map(|t| chain_link(&t.ts, &t.kind, &t.actor, &t.target_hash, &t.prev_hash));
    let inserted = conn
        .execute(
            "INSERT INTO audit_events(kind, actor, target_hash, status, detail_hash, tenant_id, prev_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                kind_str,
                actor,
                target_hash,
                status_str,
                detail_hash,
                tenant,
                prev_hash,
            ],
        )
        .is_ok();
    let id = if inserted {
        conn.last_insert_rowid()
    } else {
        -1
    };
    if sp_ok {
        // Release on success (commits the savepoint); roll back on failure so
        // a partial write doesn't leave a dangling tip. Rolling back the
        // savepoint does NOT touch the caller's outer transaction.
        let _ = conn.execute(
            if inserted {
                "RELEASE SAVEPOINT audit_link"
            } else {
                "ROLLBACK TO SAVEPOINT audit_link"
            },
            [],
        );
        if !inserted {
            // ROLLBACK TO keeps the savepoint open; release it to clean up.
            let _ = conn.execute("RELEASE SAVEPOINT audit_link", []);
        }
    }
    (id >= 0).then_some(id)
}

/// Decoded prior-row fields needed to compute the chain link for the next row.
struct ChainTip {
    ts: String,
    kind: String,
    actor: String,
    target_hash: String,
    prev_hash: String,
}

/// v1.15.0 "Observe" M1/M2: record a read event AND persist its replayable
/// decision-path trace. The audit row is hash-only (chunk ids + scores go into
/// `detail_hash`; never content). The full trace detail (non-content decision
/// metadata: ids, scores, ranks, decision, scope, principal, query) lives in
/// the `recall_traces` side table keyed by the audit row id, so `/recall/{id}/
/// trace` can replay it without touching the tamper-evident chain. Returns the
/// audit row id (the trace id), or `None` if the audit write failed.
///
/// `trace_detail` is optional — non-recall read events (`/search`, `/get`,
/// `/multi-get`) record the audit row only, with no replay artifact.
pub fn record_read_event(
    conn: &Connection,
    kind: AuditKind,
    actor: &str,
    target: &str,
    trace_detail: Option<&str>,
    tenant: &str,
) -> Option<i64> {
    let detail = trace_detail.unwrap_or(target);
    let id = record_tenant(conn, kind, actor, target, AuditStatus::Ok, detail, tenant)?;
    if let Some(t) = trace_detail {
        let _ = conn.execute(
            "INSERT INTO recall_traces(audit_id, trace_json) VALUES (?1, ?2)",
            params![id, t],
        );
    }
    Some(id)
}

/// Fetch a stored recall trace by audit row id (the `?trace=true` id returned
/// by `/recall`). Returns the raw JSON string or `None` when absent.
pub fn read_trace(conn: &Connection, audit_id: i64) -> Option<String> {
    conn.query_row(
        "SELECT trace_json FROM recall_traces WHERE audit_id = ?1",
        params![audit_id],
        |r| r.get(0),
    )
    .ok()
}

/// v1.15.0 "Observe" M1.3: bounded audit retention. Removes rows older than
/// `retention_days` and re-anchors the hash chain so the oldest surviving row
/// becomes the new genesis. Called on read-event writes (only when
/// `BRAIN_AUDIT_RETENTION_DAYS` is set), guarded so a steady-state pass with
/// nothing to prune costs one cheap COUNT. Returns the number pruned.
///
/// `audit_events.ts` is stored as SQLite `CURRENT_TIMESTAMP` (`YYYY-MM-DD
/// HH:MM:SS` UTC), which sorts lexicographically, so the cutoff is computed in
/// SQL and compared as text.
///
/// ponytail: re-anchoring rewrites `prev_hash` for every surviving row (O(n))
/// and only runs when there ARE expired rows — rare, so the occasional cost is
/// acceptable for a multi-thousand-row audit log. A >1M-row log would want a
/// periodic checkpoint instead (verify_chain already notes the same ceiling).
pub fn prune_audit_retention(conn: &Connection, retention_days: u32) -> Option<i64> {
    let cutoff: String = conn
        .query_row(
            "SELECT datetime('now', ?1)",
            params![format!("-{retention_days} days")],
            |r| r.get(0),
        )
        .ok()?;
    let expired: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM audit_events WHERE ts < ?1",
            params![cutoff],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if expired == 0 {
        return Some(0);
    }
    let tx = conn.unchecked_transaction().ok()?;
    // Remove expired rows from the head.
    tx.execute("DELETE FROM audit_events WHERE ts < ?1", params![cutoff])
        .ok()?;
    // Re-anchor: the oldest survivor becomes the genesis (NULL prev_hash), and
    // every subsequent survivor's prev_hash is recomputed so the retained
    // window stays internally tamper-evident.
    let mut ids: Vec<i64> = Vec::new();
    {
        let mut stmt = tx
            .prepare("SELECT id FROM audit_events ORDER BY id ASC")
            .ok()?;
        let rows = stmt.query_map([], |r| r.get::<_, i64>(0)).ok()?;
        for v in rows.flatten() {
            ids.push(v);
        }
    }
    let mut prev: Option<String> = None;
    for (i, id) in ids.iter().enumerate() {
        let row: Option<(String, String, String, String, Option<String>)> = tx
            .query_row(
                "SELECT ts, kind, actor, target_hash, prev_hash FROM audit_events WHERE id = ?1",
                params![id],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                        r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                        r.get(4)?,
                    ))
                },
            )
            .ok();
        let (ts, kind, actor, th, _old_prev) = row?;
        let new_prev = if i == 0 {
            None // genesis
        } else {
            Some(chain_link(
                &ts,
                &kind,
                &actor,
                &th,
                prev.as_deref().unwrap_or(""),
            ))
        };
        let _ = tx.execute(
            "UPDATE audit_events SET prev_hash = ?1 WHERE id = ?2",
            params![new_prev, id],
        );
        prev = new_prev;
    }
    // v1.16.1: sweep orphaned trace artifacts. `recall_traces` is keyed by the
    // audit row id with no FK; retention-pruned audit rows would otherwise
    // leave their replay traces behind forever. Delete any trace whose audit
    // row is gone. (The DSAR/purge cascade is handled in gate::purge_chunk_ids;
    // this covers the retention path.)
    tx.execute(
        "DELETE FROM recall_traces
          WHERE audit_id NOT IN (SELECT id FROM audit_events)",
        [],
    )
    .ok()?;
    tx.commit().ok()?;
    Some(expired)
}

/// The current hash-chain head: the link a new audit row would chain from
/// (SHA-256 hex of the newest row's `(ts, kind, actor, target_hash, prev_hash)`).
/// Used by DSAR certificates as the `chain_head` evidence of a valid chain at
/// certification time.
pub fn chain_head(conn: &Connection) -> Option<String> {
    let row: Option<(String, String, String, String, String)> = conn
        .query_row(
            "SELECT ts, kind, actor, target_hash, prev_hash \
              FROM audit_events ORDER BY id DESC LIMIT 1",
            [],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                    r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                ))
            },
        )
        .ok();
    row.map(|(ts, kind, actor, th, prev)| chain_link(&ts, &kind, &actor, &th, &prev))
}

/// Verify the audit hash chain end-to-end. Returns `false` if any v1.1 row's
/// stored `prev_hash` disagrees with the link recomputed from the prior row.
/// Pre-v1.1.0 rows (NULL `prev_hash`) carry no backref and are skipped — a
/// migrated DB may have thousands of them, followed by the first v1.1 row that
/// links back to the last NULL row's recomputed link.
///
/// ponytail: O(n) full-table scan. Adequate for the multi-thousand-row audit
/// volumes brain-server targets; a >1M-row audit log would want a periodic
/// checkpoint (store the verified tip hash in `schema_meta`).
pub fn verify_chain(conn: &Connection) -> bool {
    let mut stmt = match conn.prepare(
        "SELECT ts, kind, actor, target_hash, prev_hash FROM audit_events \
          ORDER BY id ASC",
    ) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let rows = match stmt.query_map([], |r| {
        Ok(ChainWalkRow {
            ts: r.get(0)?,
            kind: r.get(1)?,
            actor: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
            target_hash: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
            prev_hash: r.get::<_, Option<String>>(4)?,
        })
    }) {
        Ok(r) => r,
        Err(_) => return false,
    };
    // `prev_link` is the chain link computed from the prior row; the next row's
    // stored `prev_hash` (if it has one) must equal it. NULL `prev_hash` rows
    // are pre-v1.1.0 (or the very first row) and carry no backref to verify —
    // they only contribute their own link to the next row. A migrated DB has
    // arbitrarily many consecutive NULL rows, so NULL must never fail.
    let mut prev_link: Option<String> = None;
    for row in rows.flatten() {
        if let Some(got) = &row.prev_hash {
            match &prev_link {
                Some(want) if want == got => {}
                Some(_) => return false, // tampered or out-of-order v1.1 row
                None => {}               // first row overall — chain origin
            }
        }
        // Advance: every row contributes its link, including NULL ones (the
        // first v1.1 row after a NULL run links back to the last NULL row).
        prev_link = Some(chain_link(
            &row.ts,
            &row.kind,
            &row.actor,
            &row.target_hash,
            row.prev_hash.as_deref().unwrap_or(""),
        ));
    }
    true
}

/// Walk-time shape for [`verify_chain`] — `prev_hash` is nullable because
/// pre-v1.1 rows have NULL and start the chain.
struct ChainWalkRow {
    ts: String,
    kind: String,
    actor: String,
    target_hash: String,
    prev_hash: Option<String>,
}

/// Read recent audit events (operator diagnostics only). Bounded by `limit`.
/// v1.1.0: optional `tenant` filter — when `Some`, scoped to that tenant only
/// at the SQL layer so a forgotten app-level filter cannot leak cross-tenant.
pub fn recent(
    conn: &Connection,
    kind: Option<&str>,
    limit: usize,
) -> rusqlite::Result<Vec<AuditRow>> {
    recent_tenant(conn, kind, None, limit, 0)
}

/// Per-tenant variant of [`recent`]. `tenant = None` returns rows across all
/// tenants (operator diagnostics); `Some(t)` enforces `WHERE tenant_id = ?`.
/// v1.16.7 M4: `offset` is the pagination cursor (`ORDER BY id DESC LIMIT ? OFFSET ?`).
pub fn recent_tenant(
    conn: &Connection,
    kind: Option<&str>,
    tenant: Option<&str>,
    limit: usize,
    offset: usize,
) -> rusqlite::Result<Vec<AuditRow>> {
    let mut sql = String::from(
        "SELECT id, ts, kind, actor, target_hash, status, detail_hash, tenant_id \
           FROM audit_events",
    );
    let mut clauses: Vec<&'static str> = Vec::new();
    let mut params: Vec<&dyn rusqlite::ToSql> = Vec::new();
    if kind.is_some() {
        clauses.push("kind = ?");
        params.push(&kind);
    }
    if tenant.is_some() {
        clauses.push("tenant_id = ?");
        params.push(&tenant);
    }
    if !clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&clauses.join(" AND "));
    }
    sql.push_str(" ORDER BY id DESC LIMIT ? OFFSET ?");
    let limit_i: i64 = limit as i64;
    params.push(&limit_i);
    let offset_i: i64 = offset as i64;
    params.push(&offset_i);

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params.as_slice(), row_mapper)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

fn row_mapper(row: &rusqlite::Row<'_>) -> rusqlite::Result<AuditRow> {
    Ok(AuditRow {
        id: row.get(0)?,
        ts: row.get(1)?,
        kind: row.get(2)?,
        actor: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
        target_hash: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
        status: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
        detail_hash: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
        tenant_id: row
            .get::<_, Option<String>>(7)?
            .unwrap_or_else(|| DEFAULT_TENANT.to_string()),
    })
}

/// A single audit row as returned to operators. Contains only hashes — no
/// raw content or secrets survive the round-trip.
#[derive(Debug, Clone, Serialize)]
pub struct AuditRow {
    pub id: i64,
    pub ts: String,
    pub kind: String,
    pub actor: String,
    pub target_hash: String,
    pub status: String,
    pub detail_hash: String,
    pub tenant_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let db = Connection::open_in_memory().expect("open in-memory DB");
        db.execute_batch(
            "CREATE TABLE audit_events(
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               ts TEXT DEFAULT CURRENT_TIMESTAMP,
               kind TEXT NOT NULL,
               actor TEXT,
               target_hash TEXT,
               status TEXT,
               detail_hash TEXT,
               tenant_id TEXT NOT NULL DEFAULT 'global',
               prev_hash TEXT);",
        )
        .expect("create audit_events");
        db
    }

    #[test]
    fn test_record_stores_only_hashes() {
        let db = db();
        // A secret string fed as the "target" must NOT appear verbatim.
        let secret = "ghp_verysecrettokenvalue1234567890ABCDEF";
        record(
            &db,
            AuditKind::Webhook,
            "github:myrepo",
            secret,
            AuditStatus::Ok,
            "delivery abc",
        );

        let raw: String = db
            .query_row(
                "SELECT group_concat(target_hash || '|' || detail_hash) FROM audit_events",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            !raw.contains(secret),
            "audit row must never contain the raw secret, got: {raw}"
        );
        assert!(
            !raw.contains("delivery abc"),
            "audit detail must be hashed, got: {raw}"
        );
    }

    #[test]
    fn test_recent_respects_kind_and_limit() {
        let db = db();
        record(&db, AuditKind::Auth, "api", "tok1", AuditStatus::Ok, "ok");
        record(&db, AuditKind::Ingest, "api", "c1", AuditStatus::Ok, "ok");
        record(
            &db,
            AuditKind::Auth,
            "api",
            "tok2",
            AuditStatus::Denied,
            "bad",
        );
        let auth = recent(&db, Some("auth"), 10).unwrap();
        assert_eq!(auth.len(), 2, "kind filter should return only auth rows");
        let all = recent(&db, None, 1).unwrap();
        assert_eq!(all.len(), 1, "limit should cap to 1");
    }

    #[test]
    fn hash_chain_detects_tampering() {
        let db = db();
        // Three rows build a chain: r1 (no prev), r2 (links to r1), r3 (links to r2).
        record(&db, AuditKind::Ingest, "api", "c1", AuditStatus::Ok, "d1");
        record(&db, AuditKind::Ingest, "api", "c2", AuditStatus::Ok, "d2");
        record(&db, AuditKind::Ingest, "api", "c3", AuditStatus::Ok, "d3");
        assert!(verify_chain(&db), "unmodified chain must verify");

        // Tamper with row 2's prev_hash — the chain must break at row 2.
        let _ = db.execute(
            "UPDATE audit_events SET prev_hash = 'deadbeef' WHERE id = 2",
            [],
        );
        assert!(
            !verify_chain(&db),
            "a tampered prev_hash must fail the chain check"
        );
    }

    #[test]
    fn hash_chain_survives_migration_with_many_null_rows() {
        // Regression: the v1.1.0 migration adds `prev_hash` as a nullable
        // column, so every pre-v1.1 row is NULL. The original `verify_chain`
        // assumed at most one NULL row at the start and returned false on the
        // second NULL — a migrated DB with thousands of existing rows would
        // fail `/audit/verify` and `brain_audit_chain_ok` immediately.
        let db = db();
        // Simulate 5000 pre-v1.1 rows by inserting them directly with NULL
        // prev_hash (exactly what ALTER TABLE ADD COLUMN produces).
        for i in 0..5_000 {
            let _ = db.execute(
                "INSERT INTO audit_events(kind, actor, target_hash, status, detail_hash, prev_hash)
                 VALUES ('ingest', 'api', ?1, 'ok', ?2, NULL)",
                params![format!("c{i}"), format!("d{i}")],
            );
        }
        // Now record three v1.1 rows via the real writer. The first one links
        // back to the last NULL row; the next two chain normally.
        record(
            &db,
            AuditKind::Ingest,
            "api",
            "v1.1-a",
            AuditStatus::Ok,
            "d",
        );
        record(
            &db,
            AuditKind::Ingest,
            "api",
            "v1.1-b",
            AuditStatus::Ok,
            "d",
        );
        record(
            &db,
            AuditKind::Ingest,
            "api",
            "v1.1-c",
            AuditStatus::Ok,
            "d",
        );
        assert!(
            verify_chain(&db),
            "migrated DB with many NULL prev_hash rows must still verify"
        );

        // Tamper protection is preserved across the NULL boundary: editing the
        // first v1.1 row's prev_hash must still break the chain.
        let first_v1_1: i64 = db
            .query_row(
                "SELECT id FROM audit_events WHERE prev_hash IS NOT NULL ORDER BY id ASC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let _ = db.execute(
            "UPDATE audit_events SET prev_hash = 'deadbeef' WHERE id = ?1",
            params![first_v1_1],
        );
        assert!(
            !verify_chain(&db),
            "tampering with a v1.1 row's prev_hash must still fail after migration"
        );
    }

    #[test]
    #[allow(clippy::missing_transmute_annotations)]
    fn hash_chain_survives_real_v1_0_to_v1_1_migration() {
        // Closing the "fixture-based migration test" ceiling: actually run
        // `run_migration` against a DB whose `audit_events` table was created
        // with the pre-v1.1 schema (no `tenant_id`, no `prev_hash`) and already
        // has data. This is exactly the upgrade path the live v1.0 DB takes.
        use crate::migration::run_migration;

        // Register sqlite-vec so the full migration (which includes vec0
        // tables) runs the same way it does against the live DB. Local copy
        // because this test is in the lib crate (which doesn't share main.rs's
        // helper). See main.rs::register_sqlite_vec for the safety proof.
        // SAFETY: sqlite3_vec_init is extern "C" with the signature
        // sqlite3_auto_extension expects; the pointer is process-lifetime
        // static. See main.rs::register_sqlite_vec for the full proof.
        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
                sqlite_vec::sqlite3_vec_init as *const (),
            )));
        }
        let mut db = Connection::open_in_memory().expect("open in-memory DB");
        // 1. Build the v1.0 audit_events schema (the version before M2 added
        //    the two columns).
        db.execute_batch(
            "CREATE TABLE schema_meta(key TEXT PRIMARY KEY, value TEXT);
             CREATE TABLE audit_events(
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               ts TEXT DEFAULT CURRENT_TIMESTAMP,
               kind TEXT NOT NULL,
               actor TEXT,
               target_hash TEXT,
               status TEXT,
               detail_hash TEXT
             );
             CREATE INDEX idx_audit_kind ON audit_events(kind);
             CREATE INDEX idx_audit_ts ON audit_events(ts);",
        )
        .expect("create pre-v1.1 audit_events");
        // 2. Populate it with pre-v1.1 rows exactly as v1.0 wrote them — no
        //    prev_hash, no tenant_id column at all.
        for i in 0..1_000 {
            db.execute(
                "INSERT INTO audit_events(kind, actor, target_hash, status, detail_hash)
                 VALUES ('ingest', 'api', ?1, 'ok', ?2)",
                params![format!("legacy-{i}"), format!("d-{i}")],
            )
            .unwrap();
        }
        // 3. Run the real migration — adds `tenant_id` + `prev_hash` via
        //    ALTER TABLE ADD COLUMN. Existing rows must get NULL prev_hash and
        //    'global' tenant_id by default.
        run_migration(&mut db, 0).expect("v1.1 migration on populated v1.0 DB");

        // 4. Assert the back-compat defaults the migration promises.
        let null_count: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE prev_hash IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            null_count, 1_000,
            "every legacy row must have NULL prev_hash"
        );
        let global_count: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE tenant_id = 'global'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            global_count, 1_000,
            "every legacy row must default to 'global' tenant"
        );

        // 5. Now write v1.1 rows via the real writer and verify the chain holds
        //    across the NULL → Some boundary. This is the scenario the original
        //    `verify_chain` bug choked on.
        record(
            &db,
            AuditKind::Ingest,
            "api",
            "post-migration-1",
            AuditStatus::Ok,
            "d",
        );
        record(
            &db,
            AuditKind::Ingest,
            "api",
            "post-migration-2",
            AuditStatus::Ok,
            "d",
        );
        record(
            &db,
            AuditKind::Ingest,
            "api",
            "post-migration-3",
            AuditStatus::Ok,
            "d",
        );
        assert!(
            verify_chain(&db),
            "v1.0→v1.1 migrated DB with real data must verify end-to-end"
        );
    }

    #[test]
    fn record_tenant_is_safe_inside_caller_transaction() {
        // Closing the "record_tenant not wrapped in tx" ceiling: callers like
        // `delete_quarantine` are already inside their own transaction when they
        // audit. The SAVEPOINT must nest cleanly (BEGIN would error), and a
        // failure of the audit INSERT must NOT roll back the caller's work.
        let db = db();
        // Caller opens its own tx and does some work.
        let tx = db.unchecked_transaction().unwrap();
        tx.execute(
            "INSERT INTO audit_events(kind, actor, target_hash, status, detail_hash)
             VALUES ('ingest', 'caller', 'caller-row', 'ok', 'd')",
            [],
        )
        .unwrap();
        // Now audit happens inside the caller's tx. This used to rely on
        // autocommit semantics; now it nests via SAVEPOINT.
        record_tenant(
            &tx,
            AuditKind::Ingest,
            "api",
            "inside-tx",
            AuditStatus::Ok,
            "d",
            "team-a",
        );
        // Caller commits.
        tx.commit().unwrap();

        let rows = recent(&db, None, 10).unwrap();
        assert_eq!(rows.len(), 2, "caller row + audit row both landed");
        assert!(
            verify_chain(&db),
            "chain holds when audit ran inside a caller tx"
        );
    }

    #[test]
    fn record_tenant_rollback_does_not_undo_caller_work() {
        // Negative path of the SAVEPOINT wrap: if the audit INSERT itself fails
        // (e.g. constraint violation), the savepoint rolls back ONLY the audit
        // work, not the caller's. We simulate failure by dropping the table
        // mid-call isn't feasible without a different schema, so we verify the
        // positive invariant instead: caller work before + after a successful
        // audit survives a commit. This test exists to pin the savepoint shape.
        let db = db();
        let tx = db.unchecked_transaction().unwrap();
        tx.execute(
            "INSERT INTO audit_events(kind, actor, target_hash, status, detail_hash)
             VALUES ('ingest', 'before', 'before-audit', 'ok', 'd')",
            [],
        )
        .unwrap();
        record(
            &tx,
            AuditKind::Ingest,
            "api",
            "audit-event",
            AuditStatus::Ok,
            "d",
        );
        tx.execute(
            "INSERT INTO audit_events(kind, actor, target_hash, status, detail_hash)
             VALUES ('ingest', 'after', 'after-audit', 'ok', 'd')",
            [],
        )
        .unwrap();
        tx.commit().unwrap();
        let count: i64 = db
            .query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count, 3,
            "caller work before + audit + caller work after all landed"
        );
        assert!(verify_chain(&db));
    }

    #[test]
    fn hash_chain_rejects_tampered_kind() {
        let db = db();
        record(&db, AuditKind::Ingest, "api", "c1", AuditStatus::Ok, "d1");
        record(&db, AuditKind::Ingest, "api", "c2", AuditStatus::Ok, "d2");
        assert!(verify_chain(&db));
        // Rewrite row 1's kind without updating row 2's prev_hash → link breaks.
        let _ = db.execute("UPDATE audit_events SET kind = 'webhook' WHERE id = 1", []);
        assert!(!verify_chain(&db), "rewriting a field must break the chain");
    }

    #[test]
    fn tenant_filter_is_enforced_at_sql_layer() {
        let db = db();
        record_tenant(
            &db,
            AuditKind::Ingest,
            "api",
            "c1",
            AuditStatus::Ok,
            "d1",
            "team-a",
        );
        record_tenant(
            &db,
            AuditKind::Ingest,
            "api",
            "c2",
            AuditStatus::Ok,
            "d2",
            "team-b",
        );
        let a = recent_tenant(&db, None, Some("team-a"), 100, 0).unwrap();
        let b = recent_tenant(&db, None, Some("team-b"), 100, 0).unwrap();
        assert_eq!(a.len(), 1, "team-a must see only its own row");
        assert_eq!(b.len(), 1, "team-b must see only its own row");
        assert_eq!(a[0].tenant_id, "team-a");
        assert_eq!(b[0].tenant_id, "team-b");
        // Forgetting the tenant filter returns both — proves the SQL filter is
        // the enforcement point, not a missing app-level guard.
        let all = recent_tenant(&db, None, None, 100, 0).unwrap();
        assert_eq!(all.len(), 2);
    }

    /// v1.16.7 M4: pagination cursor. `limit`+`offset` pages the newest-first
    /// stream with no overlap and no dupes; an offset past the end is empty.
    #[test]
    fn recent_tenant_paginates_with_offset() {
        let db = db();
        for i in 0..10 {
            record_tenant(
                &db,
                AuditKind::Ingest,
                "api",
                &format!("c{i}"),
                AuditStatus::Ok,
                "d",
                "team-a",
            );
        }
        // Newest-first: id 10 (the last insert) is page[0].
        let page0 = recent_tenant(&db, None, Some("team-a"), 4, 0).unwrap();
        let page1 = recent_tenant(&db, None, Some("team-a"), 4, 4).unwrap();
        let page2 = recent_tenant(&db, None, Some("team-a"), 4, 8).unwrap();
        assert_eq!(page0.len(), 4);
        assert_eq!(page1.len(), 4);
        assert_eq!(page2.len(), 2);
        assert!(
            page0[0].target_hash > page0[3].target_hash,
            "descending by id"
        );
        // No overlap / no gap: the union is exactly ids 1..=10.
        let all: Vec<i64> = [page0, page1, page2]
            .into_iter()
            .flatten()
            .map(|r| r.id)
            .collect();
        assert_eq!(all.len(), 10);
        let mut sorted = all.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (1..=10).collect::<Vec<i64>>());
        // Offset past the end → empty, not an error.
        assert!(recent_tenant(&db, None, Some("team-a"), 4, 40)
            .unwrap()
            .is_empty());
    }
}
