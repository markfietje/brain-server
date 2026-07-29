//! Token revocation + refresh-chain reuse detection (v1.2.0 "AuthN" M2).
//!
//! Two tables, one cognitive model:
//!   - `revoked_tokens` — the `(jti, iss)` denylist. Logout adds a row here;
//!     every authenticated request checks it.
//!   - `refresh_chains` — the family tracker. Each refresh-token family has
//!     a chain id; the current (most-recently-issued) token is the only one
//!     that may legitimately be presented to `/auth/refresh`. Presenting an
//!     older one is reuse → revoke the whole family (OWASP pattern).
//!
//! Schema is created by `migration::run_migration` (additive — only adds
//! tables that don't exist). Lookups are SQL, parameterized (no injection
//! surface). Negative lookups are cached in-process for 60s (revocation is
//! eventually consistent by design — a revoked token lives up to 60s past
//! the revoke call before every replica sees it; acceptable for v1.2 single-
//! instance, documented as a ceiling).
//!
//! ponytail ceiling: the 60s negative cache + the per-instance denylist mean
//! a revoke propagates in ≤60s on this instance, and never across instances
//! without a shared backing store. Distributed revocation (Redis pub/sub,
//! PostgreSQL LISTEN/NOTIFY) is the v2.1 upgrade path.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

/// Cache TTL for negative jti lookups. 60s = the documented eventual-
/// consistency bound. `/auth/logout` purges the cache entry it just wrote so
/// the logout is visible on the next request from the same connection.
pub const NEG_CACHE_TTL_SECS: u64 = 60;

/// Purge cadence for `revoked_tokens` rows past their `exp`. Runs every 5 min
/// from a background task in main.rs. Keeps the table bounded by active
/// token lifetime, not by total tokens ever issued.
pub const PURGE_INTERVAL_SECS: u64 = 300;

/// In-process negative-lookup cache. Keyed by `(jti, iss)`. A present entry
/// means "we checked recently and it was NOT revoked" — re-checking within
/// the TTL is a cache hit (skip the SQL). An absent entry means "check SQL".
///
/// Positive lookups (token IS revoked) bypass the cache entirely and hit SQL
/// every time — a revoked token must be caught immediately, not eventually.
#[derive(Default)]
pub struct RevocationCache {
    /// `(jti, iss)` → when the negative lookup was performed.
    negatives: Mutex<HashMap<(String, String), CacheEntry>>,
}

#[derive(Clone, Copy)]
struct CacheEntry {
    checked_at: SystemTime,
}

impl RevocationCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Is `(jti, iss)` revoked? Checks the negative cache first; on miss,
    /// falls through to SQL. `conn` is a pooled connection. The cache is
    /// per-process; on a fresh start every lookup misses and goes to SQL
    /// (which is fast — indexed PK lookup).
    pub fn is_revoked(
        &self,
        conn: &Connection,
        jti: &str,
        iss: &str,
    ) -> Result<bool, rusqlite::Error> {
        let key = (jti.to_string(), iss.to_string());
        // Fast path: negative cache hit.
        if let Ok(g) = self.negatives.lock() {
            if let Some(entry) = g.get(&key) {
                if SystemTime::now()
                    .duration_since(entry.checked_at)
                    .map(|d| d < Duration::from_secs(NEG_CACHE_TTL_SECS))
                    .unwrap_or(false)
                {
                    return Ok(false);
                }
            }
        }
        // Slow path: SQL lookup. Parameterized — jti/iss come from a verified
        // JWT but we treat them as untrusted at the storage layer too. Use
        // `SELECT EXISTS(...)` so a missing row is a clean `false` (not an
        // error) — `query_row` with `LIMIT 1` returns QueryReturnedNoRows
        // which we'd have to map; EXISTS always returns exactly one row.
        let revoked: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM revoked_tokens WHERE jti = ?1 AND iss = ?2)",
            rusqlite::params![jti, iss],
            |r| r.get(0),
        )?;
        if !revoked {
            // Cache the negative result.
            if let Ok(mut g) = self.negatives.lock() {
                g.insert(
                    key,
                    CacheEntry {
                        checked_at: SystemTime::now(),
                    },
                );
            }
        }
        Ok(revoked)
    }

    /// Invalidate the negative cache for `(jti, iss)`. Called after a revoke
    /// so the very next request from the same process sees the new state.
    pub fn invalidate(&self, jti: &str, iss: &str) {
        if let Ok(mut g) = self.negatives.lock() {
            g.remove(&(jti.to_string(), iss.to_string()));
        }
    }

    /// Drop every expired negative entry. Called by the purge task; keeps the
    /// cache bounded by `NEG_CACHE_TTL_SECS * peak_qps` entries. Cheap because
    /// it's a single mutex lock + retain.
    pub fn purge_negatives(&self) {
        let now = SystemTime::now();
        if let Ok(mut g) = self.negatives.lock() {
            g.retain(|_, entry| {
                now.duration_since(entry.checked_at)
                    .map(|d| d < Duration::from_secs(NEG_CACHE_TTL_SECS))
                    .unwrap_or(false)
            });
        }
    }
}

/// Revoke a single access token by `(jti, iss)`. Idempotent: revoking an
/// already-revoked token is a no-op (PK conflict → ignore). `expires_at` is
/// the token's `exp` claim — we keep the row until then so the purge job
/// can drop it once it's meaningless (the token would be expired anyway).
pub fn revoke(
    conn: &Connection,
    jti: &str,
    iss: &str,
    sub: Option<&str>,
    expires_at: u64,
    revoked_by: Option<&str>,
    reason: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT OR IGNORE INTO revoked_tokens
            (jti, iss, sub, expires_at, revoked_at, revoked_by, reason)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            jti,
            iss,
            sub,
            expires_at as i64,
            now_unix() as i64,
            revoked_by,
            reason,
        ],
    )?;
    Ok(())
}

/// Delete every revoked row past its `exp`. Safe to run any time — rows past
/// `exp` are tokens that would be rejected anyway; dropping them is purely
/// housekeeping. Returns the number of rows purged.
pub fn purge_expired(conn: &Connection) -> Result<usize, rusqlite::Error> {
    let now = now_unix() as i64;
    let n = conn.execute("DELETE FROM revoked_tokens WHERE expires_at < ?1", [now])?;
    Ok(n)
}

/// Revoke every token in a refresh chain (the OWASP refresh-reuse pattern).
/// Used when `/auth/refresh` detects reuse: the entire family is burned and
/// the legitimate user must re-authenticate. Returns the chain id burned.
///
/// Implementation note: the `refresh_chains` table tracks only `current_jti`
/// (the latest token). Prior tokens in the chain were already revoked by
/// `rotate_chain` at each rotation step. So burning the chain = revoke the
/// current jti + mark the chain burned so any future presentation is rejected.
pub fn revoke_chain(
    conn: &Connection,
    chain_id: &str,
    iss: &str,
    revoked_by: Option<&str>,
    reason: &str,
) -> Result<usize, rusqlite::Error> {
    let now = now_unix() as i64;
    // Far-future exp so the revoke row survives until the chain is fully dead.
    let far_future = now + 10 * 365 * 24 * 3600;
    // Revoke the current jti (the only live token in the chain).
    let n = conn.execute(
        "INSERT OR IGNORE INTO revoked_tokens (jti, iss, sub, expires_at, revoked_at, revoked_by, reason)
         SELECT current_jti, iss, NULL, ?3, ?4, ?5, ?6 FROM refresh_chains
         WHERE chain_id = ?1 AND iss = ?2 AND state = 'active'
         ORDER BY id DESC LIMIT 1",
        rusqlite::params![chain_id, iss, far_future, now, revoked_by, reason],
    )?;
    // Mark the chain as burned so `/auth/refresh` rejects any future presentation.
    conn.execute(
        "UPDATE refresh_chains SET state = 'burned', burned_at = ?3
         WHERE chain_id = ?1 AND iss = ?2",
        rusqlite::params![chain_id, iss, now],
    )?;
    Ok(n)
}

/// Record a refresh-token presentation: track which jti is "current" for the
/// family. If `current != presented`, that's reuse → caller revokes the chain.
/// Returns `Ok(())` if this is the current token, `Err(ReuseDetected)` if not.
pub fn record_refresh_use(
    conn: &Connection,
    chain_id: &str,
    iss: &str,
    presented_jti: &str,
) -> Result<(), RefreshError> {
    // Load the chain state.
    let row: Option<(String, String, i64)> = conn
        .query_row(
            "SELECT chain_id, current_jti, burned_at FROM refresh_chains
             WHERE chain_id = ?1 AND iss = ?2
             ORDER BY id DESC LIMIT 1",
            rusqlite::params![chain_id, iss],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                ))
            },
        )
        .ok();
    match row {
        None => {
            // No prior chain record — first use. Insert.
            conn.execute(
                "INSERT INTO refresh_chains (chain_id, iss, current_jti, state, first_seen)
                 VALUES (?1, ?2, ?3, 'active', ?4)",
                rusqlite::params![chain_id, iss, presented_jti, now_unix() as i64],
            )?;
            Ok(())
        }
        Some((_, current_jti, burned_at)) => {
            if burned_at > 0 {
                // Chain already burned — any presentation is reuse.
                return Err(RefreshError::ChainBurned);
            }
            if current_jti == presented_jti {
                // Current token — legitimate use.
                Ok(())
            } else {
                // Stale token presented — reuse detected. Caller must revoke.
                Err(RefreshError::ReuseDetected)
            }
        }
    }
}

/// Rotate the chain's current jti to a new refresh token. Called after a
/// successful `/auth/refresh` — the old refresh token is revoked and the new
/// one becomes "current".
pub fn rotate_chain(
    conn: &Connection,
    chain_id: &str,
    iss: &str,
    new_jti: &str,
    old_jti: &str,
    old_expires_at: u64,
) -> Result<(), rusqlite::Error> {
    // Revoke the old refresh token.
    revoke(
        conn,
        old_jti,
        iss,
        None,
        old_expires_at,
        Some("rotation"),
        "rotation",
    )?;
    // Update the chain's current pointer. We also insert a fresh row so the
    // history is auditable (append-only; the latest row wins per the query
    // above's ORDER BY id DESC).
    conn.execute(
        "INSERT INTO refresh_chains (chain_id, iss, current_jti, state, first_seen)
         VALUES (?1, ?2, ?3, 'active', ?4)",
        rusqlite::params![chain_id, iss, new_jti, now_unix() as i64],
    )?;
    Ok(())
}

#[derive(Debug, PartialEq)]
pub enum RefreshError {
    /// A refresh token from a non-current position in the chain was presented.
    /// The caller MUST revoke the entire chain (OWASP pattern).
    ReuseDetected,
    /// The chain was already burned by a prior reuse detection.
    ChainBurned,
    Rusqlite(rusqlite::Error),
}

impl From<rusqlite::Error> for RefreshError {
    fn from(e: rusqlite::Error) -> Self {
        RefreshError::Rusqlite(e)
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE revoked_tokens (
                jti TEXT NOT NULL,
                iss TEXT NOT NULL,
                sub TEXT,
                expires_at INTEGER NOT NULL,
                revoked_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                revoked_by TEXT,
                reason TEXT,
                PRIMARY KEY (jti, iss)
             );
             CREATE INDEX idx_revoked_expires ON revoked_tokens(expires_at);
             CREATE TABLE refresh_chains (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                chain_id TEXT NOT NULL,
                iss TEXT NOT NULL,
                current_jti TEXT NOT NULL,
                state TEXT NOT NULL DEFAULT 'active',
                first_seen INTEGER NOT NULL,
                burned_at INTEGER
             );
             CREATE INDEX idx_refresh_chain ON refresh_chains(chain_id, iss);",
        )
        .unwrap();
        conn
    }

    #[test]
    fn revoke_then_lookup_finds_it() {
        let conn = mem_db();
        let cache = RevocationCache::new();
        assert!(!cache.is_revoked(&conn, "jti-1", "iss").unwrap());
        revoke(&conn, "jti-1", "iss", Some("user"), 9999, None, "logout").unwrap();
        cache.invalidate("jti-1", "iss");
        assert!(cache.is_revoked(&conn, "jti-1", "iss").unwrap());
    }

    #[test]
    fn negative_lookup_caches_within_ttl() {
        let conn = mem_db();
        let cache = RevocationCache::new();
        // First lookup: SQL miss → cache.
        assert!(!cache.is_revoked(&conn, "jti-2", "iss").unwrap());
        // Revoke behind the cache's back — the cached negative must hold.
        revoke(&conn, "jti-2", "iss", None, 9999, None, "test").unwrap();
        // No invalidate → cached result wins.
        assert!(!cache.is_revoked(&conn, "jti-2", "iss").unwrap());
        // After invalidation, SQL truth is visible.
        cache.invalidate("jti-2", "iss");
        assert!(cache.is_revoked(&conn, "jti-2", "iss").unwrap());
    }

    #[test]
    fn revoke_is_idempotent() {
        let conn = mem_db();
        revoke(&conn, "jti-3", "iss", None, 9999, None, "first").unwrap();
        // Second revoke must not error (INSERT OR IGNORE).
        revoke(&conn, "jti-3", "iss", None, 9999, None, "second").unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM revoked_tokens WHERE jti = 'jti-3'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "idempotent revoke must not duplicate rows");
    }

    #[test]
    fn purge_drops_only_expired_rows() {
        let conn = mem_db();
        let now = now_unix() as i64;
        // One expired, one live.
        conn.execute(
            "INSERT INTO revoked_tokens (jti, iss, expires_at) VALUES ('old', 'iss', ?1)",
            [now - 100],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO revoked_tokens (jti, iss, expires_at) VALUES ('live', 'iss', ?1)",
            [now + 3600],
        )
        .unwrap();
        let purged = purge_expired(&conn).unwrap();
        assert_eq!(purged, 1);
        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM revoked_tokens", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 1);
    }

    #[test]
    fn refresh_reuse_detected_when_stale_presented() {
        let conn = mem_db();
        // First use establishes the chain.
        record_refresh_use(&conn, "chain-1", "iss", "rt-old").unwrap();
        // Rotation: new token becomes current.
        rotate_chain(&conn, "chain-1", "iss", "rt-new", "rt-old", 9999).unwrap();
        // Presenting the old token again = reuse.
        let err = record_refresh_use(&conn, "chain-1", "iss", "rt-old").unwrap_err();
        assert_eq!(err, RefreshError::ReuseDetected);
    }

    #[test]
    fn refresh_chain_revoke_burns_everything() {
        let conn = mem_db();
        record_refresh_use(&conn, "chain-2", "iss", "rt-a").unwrap();
        rotate_chain(&conn, "chain-2", "iss", "rt-b", "rt-a", 9999).unwrap();
        rotate_chain(&conn, "chain-2", "iss", "rt-c", "rt-b", 9999).unwrap();
        let n = revoke_chain(&conn, "chain-2", "iss", None, "reuse").unwrap();
        // Burns the current jti (rt-c) and marks the chain. Prior tokens
        // (rt-a, rt-b) were already revoked by rotate_chain at each step.
        assert_eq!(
            n, 1,
            "the current jti is revoked; prior tokens were revoked on rotation"
        );
        // Chain is now burned.
        let err = record_refresh_use(&conn, "chain-2", "iss", "rt-c").unwrap_err();
        assert_eq!(err, RefreshError::ChainBurned);
    }
}
