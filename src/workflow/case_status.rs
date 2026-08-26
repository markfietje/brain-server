//! The unguessable public case-status ref: one live
//! HMAC-derived token per run naming the static `status/{ref}.json`
//! artifact that `brain kb build --with-case-status` emits.
//!
//! Laws this module encodes:
//! - **The ref ships by human hands.** brain never sends it anywhere: the
//!   closing note or a CRM ticket field carries it. brain-server stays
//!   loopback — there is no public route, ever.
//! - **Rotation kills the old ref** (`salt_version` is the rotation
//!   counter); **revocation stays dead** — a revoked run refuses a fresh
//!   mint loudly instead of silently resurrecting its public page.
//! - Every action is audited INSIDE the caller's transaction
//!   (`record_tenant`, SAVEPOINT-nested): the action and its evidence
//!   commit or roll back together.
//!
//! The salt comes through the standard secret ladder
//! (`BRAIN_CASE_STATUS_KEY_FILE` → `BRAIN_CASE_STATUS_KEY`, mode-checked
//! 0600); an unreadable salt fails closed.

use crate::audit::{AuditKind, AuditStatus, record_tenant};
use hmac::{Hmac, KeyInit, Mac};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::Sha256;

/// Audit-detail prefixes (writer and reader share the exact formats).
pub(crate) const AUDIT_MINT: &str = "workflow/case-status/mint";
pub(crate) const AUDIT_ROTATE: &str = "workflow/case-status/rotate";
pub(crate) const AUDIT_REVOKE: &str = "workflow/case-status/revoke";

/// The ref is base32(HMAC-SHA256(...))[..26] — 130 bits of entropy.
pub(crate) const REF_LEN: usize = 26;

#[derive(Debug)]
pub(crate) enum CaseStatusError {
    NotFound(String),
    Revoked(String),
    Salt(String),
    Database(String),
}

impl std::fmt::Display for CaseStatusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CaseStatusError::NotFound(m)
            | CaseStatusError::Revoked(m)
            | CaseStatusError::Salt(m)
            | CaseStatusError::Database(m) => write!(f, "{m}"),
        }
    }
}

impl From<rusqlite::Error> for CaseStatusError {
    fn from(e: rusqlite::Error) -> Self {
        CaseStatusError::Database(e.to_string())
    }
}

impl From<crate::secrets::SecretError> for CaseStatusError {
    fn from(e: crate::secrets::SecretError) -> Self {
        let msg = e.to_string();
        CaseStatusError::Salt(msg)
    }
}

/// RFC4648 base32 (A–Z2–7, unpadded). Small, dependency-free, deterministic;
/// pinned against RFC test vectors below.
pub(crate) fn base32(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::with_capacity((bytes.len() * 8).div_ceil(5));
    for chunk in bytes.chunks(5) {
        let mut buf = [0u8; 5];
        buf[..chunk.len()].copy_from_slice(chunk);
        let bits = u64::from(buf[0]) << 32
            | u64::from(buf[1]) << 24
            | u64::from(buf[2]) << 16
            | u64::from(buf[3]) << 8
            | u64::from(buf[4]);
        let take = (chunk.len() * 8).div_ceil(5);
        for i in 0..take {
            out.push(ALPHABET[(bits >> (35 - i * 5)) as usize & 0x1f] as char);
        }
    }
    out
}

/// Derive the ref for a run at a given rotation version. Pure; the ONLY
/// place the ref formula lives.
pub(crate) fn derive_ref(salt: &[u8], run_id: i64, salt_version: i64) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(salt).expect("HMAC accepts keys of any length");
    mac.update(format!("{run_id}:{salt_version}").as_bytes());
    let digest = mac.finalize().into_bytes();
    base32(&digest)[..REF_LEN].to_string()
}

/// Resolve the status-ref salt through the secret ladder (fail closed).
pub(crate) fn ref_salt() -> Result<Vec<u8>, CaseStatusError> {
    Ok(crate::secrets::resolve("case_status")?.into_bytes())
}

struct RefRow {
    r: Option<String>,
    salt_version: i64,
    revoked_at: Option<i64>,
}

fn load_row(conn: &Connection, run_id: i64) -> Result<Option<RefRow>, CaseStatusError> {
    conn.query_row(
        "SELECT ref, salt_version, revoked_at FROM case_status_refs WHERE run_id = ?1",
        params![run_id],
        |r| {
            Ok(RefRow {
                r: r.get(0)?,
                salt_version: r.get::<_, Option<i64>>(1)?.unwrap_or(1),
                revoked_at: r.get(2)?,
            })
        },
    )
    .optional()
    .map_err(CaseStatusError::from)
}

/// Mint the run's status ref. Idempotent-per-run: an existing live ref is
/// returned unchanged; a REVOKED ref refuses loudly (fail closed —
/// resurrection is a deliberate operator decision we do not make silently).
pub(crate) fn mint(conn: &Connection, run_id: i64, now: i64) -> Result<String, CaseStatusError> {
    if load_row(conn, run_id)?.is_none() {
        let salt = ref_salt()?;
        let r = derive_ref(&salt, run_id, 1);
        conn.execute(
            "INSERT INTO case_status_refs(run_id, ref, salt_version, minted_at)
             VALUES (?1, ?2, 1, ?3)",
            params![run_id, r, now],
        )?;
        record_tenant(
            conn,
            AuditKind::Workflow,
            "system",
            &format!("run/{run_id}/status-ref"),
            AuditStatus::Ok,
            &format!("{AUDIT_MINT}:v1"),
            "global",
        );
        return Ok(r);
    }
    let row = load_row(conn, run_id)?.expect("row checked above");
    if let Some(revoked) = row.revoked_at {
        return Err(CaseStatusError::Revoked(format!(
            "run {run_id}: status ref revoked at {revoked}; mint refuses"
        )));
    }
    Ok(row.r.expect("live row carries its ref"))
}

/// Rotate: bump the rotation counter so the old ref dies and issue the new
/// one. The row stays live; the previous token simply stops resolving.
pub(crate) fn rotate(conn: &Connection, run_id: i64, now: i64) -> Result<String, CaseStatusError> {
    let Some(row) = load_row(conn, run_id)? else {
        return Err(CaseStatusError::NotFound(format!(
            "run {run_id}: no status ref to rotate; mint first"
        )));
    };
    if row.revoked_at.is_some() {
        return Err(CaseStatusError::Revoked(format!(
            "run {run_id}: status ref revoked; rotate refuses"
        )));
    }
    let next_version = row.salt_version + 1;
    let salt = ref_salt()?;
    let r = derive_ref(&salt, run_id, next_version);
    conn.execute(
        "UPDATE case_status_refs SET ref = ?2, salt_version = ?3, rotated_at = ?4
         WHERE run_id = ?1",
        params![run_id, r, next_version, now],
    )?;
    record_tenant(
        conn,
        AuditKind::Workflow,
        "system",
        &format!("run/{run_id}/status-ref"),
        AuditStatus::Ok,
        &format!("{AUDIT_ROTATE}:v{next_version}"),
        "global",
    );
    Ok(r)
}

/// Revoke: remove the ref from the next build. Idempotent; the ref stays
/// dead — mint refuses afterwards.
pub(crate) fn revoke(conn: &Connection, run_id: i64, now: i64) -> Result<(), CaseStatusError> {
    let changed = conn.execute(
        "UPDATE case_status_refs SET revoked_at = ?2 WHERE run_id = ?1 AND revoked_at IS NULL",
        params![run_id, now],
    )?;
    if changed > 0 {
        record_tenant(
            conn,
            AuditKind::Workflow,
            "system",
            &format!("run/{run_id}/status-ref"),
            AuditStatus::Ok,
            &format!("{AUDIT_REVOKE}:{now}"),
            "global",
        );
    }
    Ok(())
}

/// The live refs a build consumes: never revoked. Static artifact = these
/// are the only runs whose status pages exist.
pub(crate) fn live_refs(conn: &Connection) -> Result<Vec<(i64, String)>, CaseStatusError> {
    let mut stmt = conn.prepare(
        "SELECT run_id, ref FROM case_status_refs
          WHERE revoked_at IS NULL ORDER BY run_id",
    )?;
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// DSAR sweep support: PURGE every ref row for the given runs (the subject
/// is being erased; the refs die with them). Returns the number purged.
pub(crate) fn purge_for_runs(conn: &Connection, run_ids: &[i64]) -> Result<usize, CaseStatusError> {
    let mut n = 0;
    for id in run_ids {
        n += conn.execute(
            "DELETE FROM case_status_refs WHERE run_id = ?1",
            params![id],
        )?;
    }
    Ok(n)
}

/// Legal-hold support: freeze means REVOKE (the page must not keep
/// advertising state about a held matter) without purging the row — the
/// evidence stays for the hold, the public surface goes dark.
pub(crate) fn revoke_for_runs(
    conn: &Connection,
    run_ids: &[i64],
    now: i64,
) -> Result<usize, CaseStatusError> {
    let mut n = 0;
    for id in run_ids {
        let before = load_row(conn, *id)?;
        revoke(conn, *id, now)?;
        if before.is_some_and(|r| r.revoked_at.is_none()) {
            n += 1;
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test-only env guard: sets/removes salt vars, restoring on drop.
    struct SaltGuard(Vec<(&'static str, Option<String>)>);
    impl SaltGuard {
        fn set(var: &'static str, val: &str) -> Self {
            // SAFETY (test-only): single-threaded env mutation in this module's tests.
            unsafe { std::env::set_var(var, val) };
            SaltGuard(vec![(var, None)])
        }
        fn remove_all(vars: &[&'static str]) -> Self {
            let mut held = Vec::new();
            for var in vars {
                held.push((*var, std::env::var(var).ok()));
                // SAFETY (test-only): single-threaded env mutation in this module's tests.
                unsafe { std::env::remove_var(var) };
            }
            SaltGuard(held)
        }
    }
    impl Drop for SaltGuard {
        fn drop(&mut self) {
            for (var, prev) in &self.0 {
                match prev {
                    Some(v) => {
                        // SAFETY (test-only): single-threaded env mutation in this module's tests.
                        unsafe { std::env::set_var(var, v) };
                    }
                    None => {
                        // SAFETY (test-only): single-threaded env mutation in this module's tests.
                        unsafe { std::env::remove_var(var) };
                    }
                }
            }
        }
    }

    /// Env vars are process-global: every salt-touching test holds this
    /// lock (poison-tolerantly) for its whole body.
    static SALT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn conn() -> (Connection, SaltGuard, std::sync::MutexGuard<'static, ()>) {
        let conn = Connection::open_in_memory().expect("open");
        conn.execute_batch(
            "CREATE TABLE case_status_refs(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                run_id INTEGER NOT NULL UNIQUE,
                ref TEXT NOT NULL UNIQUE,
                salt_version INTEGER NOT NULL DEFAULT 1,
                minted_at INTEGER NOT NULL,
                rotated_at INTEGER,
                revoked_at INTEGER);",
        )
        .expect("create table");
        let lock = SALT_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let guard = SaltGuard::set("BRAIN_CASE_STATUS_KEY", "test-salt-key");
        (conn, guard, lock)
    }

    #[test]
    fn status_ref_is_unguessable_and_rotation_kills_old_ref() {
        let (conn, _guard, _lock) = conn();
        let first = mint(&conn, 42, 1_000).expect("mint");
        assert_eq!(first.len(), REF_LEN, "130 bits, base32-trimmed");
        assert!(
            first
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()),
            "base32 alphabet only: {first}"
        );
        // Idempotent per run: the same ref comes back, nothing new minted.
        let again = mint(&conn, 42, 1_100).expect("remint");
        assert_eq!(first, again);
        // A different run derives a different ref (unguessable across runs).
        let other = mint(&conn, 43, 1_000).expect("mint");
        assert_ne!(first, other);
        // Rotation issues a NEW ref and kills the old one.
        let rotated = rotate(&conn, 42, 2_000).expect("rotate");
        assert_ne!(first, rotated);
        let stored: String = conn
            .query_row(
                "SELECT ref FROM case_status_refs WHERE run_id=42",
                [],
                |r| r.get::<_, String>(0),
            )
            .expect("read");
        assert_eq!(rotated, stored);
        // The old ref no longer resolves anywhere: the live set has only the
        // current tokens.
        let live = live_refs(&conn).expect("live");
        assert_eq!(live, vec![(42, rotated.clone()), (43, other)]);
    }

    #[test]
    fn revoke_removes_page_from_next_build_and_stays_dead() {
        let (conn, _guard, _lock) = conn();
        let r = mint(&conn, 7, 1_000).expect("mint");
        assert_eq!(live_refs(&conn).expect("live").len(), 1);
        revoke(&conn, 7, 2_000).expect("revoke");
        // The next build sees nothing for this run.
        assert!(live_refs(&conn).expect("live").is_empty());
        // Re-mint refuses loudly — a revoked page does not resurrect.
        let err = mint(&conn, 7, 3_000).expect_err("revoked refuses");
        assert!(matches!(err, CaseStatusError::Revoked(_)));
        assert_eq!(r.len(), REF_LEN);
        // Revoke is idempotent.
        revoke(&conn, 7, 4_000).expect("idempotent");
    }

    #[test]
    fn dsar_sweep_and_legal_hold_revoke_refs() {
        let (conn, _guard, _lock) = conn();
        let _a = mint(&conn, 1, 1_000).expect("mint");
        let _b = mint(&conn, 2, 1_000).expect("mint");
        let _c = mint(&conn, 3, 1_000).expect("mint");
        // DSAR erasure PURGES the refs of swept runs.
        let purged = purge_for_runs(&conn, &[1]).expect("purge");
        assert_eq!(purged, 1);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM case_status_refs", [], |r| r.get(0))
            .expect("count");
        assert_eq!(count, 2, "the swept run's ref row is gone");
        // Legal hold REVOKES without purging (evidence stays, page goes dark).
        let frozen = revoke_for_runs(&conn, &[2, 99], 2_000).expect("freeze");
        assert_eq!(frozen, 1, "only the live ref counts as frozen");
        let revoked: Option<i64> = conn
            .query_row(
                "SELECT revoked_at FROM case_status_refs WHERE run_id=2",
                [],
                |r| r.get::<_, Option<i64>>(0),
            )
            .expect("read");
        assert_eq!(revoked, Some(2_000));
    }

    #[test]
    fn missing_salt_fails_closed() {
        let conn = Connection::open_in_memory().expect("open");
        conn.execute_batch(
            "CREATE TABLE case_status_refs(id INTEGER PRIMARY KEY AUTOINCREMENT, run_id INTEGER NOT NULL UNIQUE, ref TEXT NOT NULL UNIQUE, salt_version INTEGER NOT NULL DEFAULT 1, minted_at INTEGER NOT NULL, rotated_at INTEGER, revoked_at INTEGER);",
        )
        .expect("create table");
        let _lock = SALT_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _guard =
            SaltGuard::remove_all(&["BRAIN_CASE_STATUS_KEY_FILE", "BRAIN_CASE_STATUS_KEY"]);
        let err = mint(&conn, 5, 1_000).expect_err("no salt must refuse");
        assert!(
            matches!(err, CaseStatusError::Salt(_)),
            "fail closed: {err}"
        );
    }

    #[test]
    fn base32_matches_rfc4648_vectors() {
        assert_eq!(base32(b""), "");
        assert_eq!(base32(b"f"), "MY");
        assert_eq!(base32(b"fo"), "MZXQ");
        assert_eq!(base32(b"foo"), "MZXW6");
        assert_eq!(base32(b"foobar"), "MZXW6YTBOI");
    }
}
