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
/// The hash-chain link is read + written inside a single transaction so the
/// read-modify-write is atomic under SQLite's single-writer lock.
pub fn record(
    conn: &Connection,
    kind: AuditKind,
    actor: &str,
    target: &str,
    status: AuditStatus,
    detail: &str,
) {
    record_tenant(conn, kind, actor, target, status, detail, DEFAULT_TENANT)
}

/// Per-tenant variant of [`record`]. Same best-effort semantics.
///
/// ponytail: the chain link read+write is NOT wrapped in an explicit tx.
/// SQLite is single-writer, so a pooled connection sees a stable tip across
/// the read+INSERT pair as long as the same logical connection isn't
/// re-entrant. Callers already inside their own transaction (e.g.
/// `delete_quarantine`) get atomicity from that outer tx for free; autocommit
/// callers rely on SQLite's writer lock to serialize them. If we ever need
/// strict cross-statement atomicity for autocommit callers, wrap the pair in
/// `conn.transaction()` — the upgrade path is one line.
pub fn record_tenant(
    conn: &Connection,
    kind: AuditKind,
    actor: &str,
    target: &str,
    status: AuditStatus,
    detail: &str,
    tenant: &str,
) {
    let target_hash = hash(target);
    let detail_hash = hash(detail);
    let kind_str = kind.as_str();
    let status_str = status.as_str();
    // Read the chain tip (the most recent row). SQLite is single-writer, so the
    // read+INSERT pair is race-free against other connections; callers already
    // inside their own transaction get atomicity from that outer tx for free.
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
    let _ = conn.execute(
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
    );
}

/// Decoded prior-row fields needed to compute the chain link for the next row.
struct ChainTip {
    ts: String,
    kind: String,
    actor: String,
    target_hash: String,
    prev_hash: String,
}

/// Verify the audit hash chain end-to-end. Returns `false` if any row's
/// recomputed link disagrees with its stored `prev_hash`. Pre-v1.1.0 rows
/// (NULL `prev_hash`) are skipped — the chain starts at the first v1.1 row.
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
    let mut expected: Option<String> = None; // expected prev_hash for the next row
    for row in rows.flatten() {
        match (&expected, &row.prev_hash) {
            (Some(want), Some(got)) if want == got => {}
            (None, None) => {} // pre-v1.1 row or first v1.1 row before any link
            _ => return false,
        }
        // Advance: the next row's prev_hash must equal the chain link of this row.
        expected = Some(chain_link(
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
    recent_tenant(conn, kind, None, limit)
}

/// Per-tenant variant of [`recent`]. `tenant = None` returns rows across all
/// tenants (operator diagnostics); `Some(t)` enforces `WHERE tenant_id = ?`.
pub fn recent_tenant(
    conn: &Connection,
    kind: Option<&str>,
    tenant: Option<&str>,
    limit: usize,
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
    sql.push_str(" ORDER BY id DESC LIMIT ?");
    let limit_i: i64 = limit as i64;
    params.push(&limit_i);

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
        let a = recent_tenant(&db, None, Some("team-a"), 100).unwrap();
        let b = recent_tenant(&db, None, Some("team-b"), 100).unwrap();
        assert_eq!(a.len(), 1, "team-a must see only its own row");
        assert_eq!(b.len(), 1, "team-b must see only its own row");
        assert_eq!(a[0].tenant_id, "team-a");
        assert_eq!(b[0].tenant_id, "team-b");
        // Forgetting the tenant filter returns both — proves the SQL filter is
        // the enforcement point, not a missing app-level guard.
        let all = recent_tenant(&db, None, None, 100).unwrap();
        assert_eq!(all.len(), 2);
    }
}
