//! v0.9.7 "Guard" — append-only audit events.
//!
//! Every audit row stores **identifiers and hashes only** — never raw indexed
//! content, token values, or secret-file contents. The `record` helper is a
//! one-liner callers use at trust boundaries (auth, ingest, webhook verification,
//! reconcile, backup). Hashing uses the existing `xxh3_64` dependency so there
//! is no new crypto surface.
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
//!   detail_hash TEXT        -- xxh3 of a short detail string (no secrets)
//! );
//! ```

use rusqlite::{params, Connection};
use serde::Serialize;
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

/// Append one audit event. Best-effort: audit must never fail the primary
/// action, so errors are swallowed (logged at debug). Callers pass already-
/// hashed identifiers where the value is sensitive.
pub fn record(
    conn: &Connection,
    kind: AuditKind,
    actor: &str,
    target: &str,
    status: AuditStatus,
    detail: &str,
) {
    let _ = conn.execute(
        "INSERT INTO audit_events(kind, actor, target_hash, status, detail_hash)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            kind.as_str(),
            actor,
            hash(target),
            status.as_str(),
            hash(detail),
        ],
    );
}

/// Read recent audit events (operator diagnostics only). Bounded by `limit`.
pub fn recent(
    conn: &Connection,
    kind: Option<&str>,
    limit: usize,
) -> rusqlite::Result<Vec<AuditRow>> {
    let sql = match kind {
        Some(_) => {
            "SELECT id, ts, kind, actor, target_hash, status, detail_hash \
                    FROM audit_events WHERE kind = ?1 \
                    ORDER BY id DESC LIMIT ?2"
        }
        None => {
            "SELECT id, ts, kind, actor, target_hash, status, detail_hash \
                 FROM audit_events ORDER BY id DESC LIMIT ?1"
        }
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = match kind {
        Some(k) => stmt
            .query_map(params![k, limit as i64], row_mapper)?
            .filter_map(|r| r.ok())
            .collect(),
        None => stmt
            .query_map(params![limit as i64], row_mapper)?
            .filter_map(|r| r.ok())
            .collect(),
    };
    Ok(rows)
}

fn row_mapper(row: &rusqlite::Row<'_>) -> rusqlite::Result<AuditRow> {
    Ok(AuditRow {
        id: row.get(0)?,
        ts: row.get(1)?,
        kind: row.get(2)?,
        actor: row.get(3)?,
        target_hash: row.get(4)?,
        status: row.get(5)?,
        detail_hash: row.get(6)?,
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
               detail_hash TEXT);",
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
}
