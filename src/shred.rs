//! Physical shredder for deleted-content residue (the "Notary" verb pair).
//!
//! The DSAR certificate has always disclosed the honest posture: erasure is
//! LOGICAL — `secure_delete` defaults off, so freed page images, the WAL,
//! and freelist pages keep the purged bytes until the file happens to
//! rewrite them. Strict-posture domains already erase under
//! `secure_delete=ON` + a WAL TRUNCATE checkpoint (service::dsar), but
//! that only covers THAT run's deletes — pages freed by earlier,
//! pre-strict erasures (and by every other delete path) keep their byte
//! images in the freelist forever.
//!
//! This module is the operator-invoked one-shot closure:
//! `secure_delete=ON` (posture + readback), `wal_checkpoint(TRUNCATE)`
//! (drain the WAL), `VACUUM` (rebuild the file from live pages only —
//! stale freelist images are not copied), a second TRUNCATE checkpoint
//! (drain VACUUM's own WAL output), `integrity_check`, and one hash-chained
//! `forget` audit row evidencing the act. Every step's outcome is asserted
//! fail-closed — the verb never reports success on a posture it failed to
//! engage.
//!
//! It complements the logical purge; it does not replace it:
//!   brain client dsar --action purge --yes   (server: tombstones + certificate)
//!   brain shred                               (offline: drop the physical residue)
//!
//! ponytail: what this does NOT do —
//!  * no subject knowledge: residue is subject-independent (the rows are
//!    already gone; this drops orphaned page images wherever they sit);
//!  * no scheduling: operator-invoked only — no background worker, ever;
//!  * nothing OUTSIDE the SQL layer: filesystem copies, `<db>.bak`
//!    snapshots, standby follower chunks, and SSD wear-leveling keep
//!    whatever they captured (operator-level disposal, disclosed on every
//!    invocation);
//!  * not a quiet-period requirement: concurrent server writes merely
//!    serialize behind the write lock (busy_timeout), but the honest
//!    guidance is to run it in a quiet moment — VACUUM holds the writer.
//!
//! Run per domain DB (`--db PATH`, default the global DB) — the same
//! one-file-per-pool layout DSAR itself sweeps.

use rusqlite::Connection;

/// The completed shred's readback evidence — every field asserted, none
/// best-effort. `freelist_after == 0` is the residue claim's teeth: a fresh
/// VACUUM build has no free pages to hold stale images.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShredReport {
    pub pages_before: i64,
    pub pages_after: i64,
    pub freelist_before: i64,
    pub freelist_after: i64,
    /// `PRAGMA secure_delete` readback after the ON attempt (1 = on,
    /// 2 = fast; both overwrite freed content).
    pub secure_delete_readback: i64,
    pub integrity_ok: bool,
    /// The chain row id of the `forget` evidence (always written; a shred
    /// without evidence is an error, never a warning).
    pub audit_row: i64,
}

fn pragma_i64(conn: &Connection, sql: &str) -> Result<i64, String> {
    conn.query_row(sql, [], |r| r.get(0))
        .map_err(|e| format!("{sql}: {e}"))
}

/// Execute the shred. Fails closed at every step: a pragma that will not
/// engage, a vacuum that errors, a nonzero freelist readback, a failed
/// integrity check, or an unwritten evidence row each abort with an error
/// — the caller never sees a success that hid a step.
pub fn shred(conn: &mut Connection) -> Result<ShredReport, String> {
    // vec0 tables must resolve before VACUUM rebuilds the schema — the
    // process-wide auto-extension registration (idempotent, the standby
    // promote precedent).
    crate::register_sqlite_vec::register_sqlite_vec();
    let pages_before = pragma_i64(conn, "PRAGMA page_count")?;
    let freelist_before = pragma_i64(conn, "PRAGMA freelist_count")?;
    // 1. Posture: overwrite-on-delete for anything freed from here on, and
    //    the readback is part of the contract — a refused pragma is an
    //    error, not a degraded posture the report quietly tolerates.
    conn.execute_batch("PRAGMA secure_delete=ON;")
        .map_err(|e| format!("secure_delete=ON: {e}"))?;
    let secure_delete_readback = pragma_i64(conn, "PRAGMA secure_delete")?;
    if secure_delete_readback <= 0 {
        return Err(format!(
            "secure_delete readback = {secure_delete_readback} — the overwrite posture did \
             not engage; refusing to shred without it"
        ));
    }
    // 2. Drain the WAL into the main file (its page images are pre-shred
    //    residue-in-waiting; VACUUM below drops them with the rest).
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
        .map_err(|e| format!("wal_checkpoint(TRUNCATE) pre-vacuum: {e}"))?;
    // 3. Rebuild. VACUUM copies live pages only — freelist page images die
    //    with the old file. Cannot run inside a transaction; refuse loudly
    //    rather than erroring inside SQLite if a caller holds one.
    if !conn.is_autocommit() {
        return Err(
            "a transaction is open — VACUUM cannot run; shred must own the connection".to_string(),
        );
    }
    conn.execute_batch("VACUUM;")
        .map_err(|e| format!("VACUUM: {e}"))?;
    // 4. Drain VACUUM's own WAL output so the new pages land in the main
    //    file and the WAL truncates back to empty.
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
        .map_err(|e| format!("wal_checkpoint(TRUNCATE) post-vacuum: {e}"))?;
    // 5. Readbacks with teeth.
    let freelist_after = pragma_i64(conn, "PRAGMA freelist_count")?;
    if freelist_after != 0 {
        return Err(format!(
            "freelist_count = {freelist_after} after VACUUM — residue-bearing free pages \
             remain; refusing to certify"
        ));
    }
    let pages_after = pragma_i64(conn, "PRAGMA page_count")?;
    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .map_err(|e| format!("integrity_check: {e}"))?;
    let integrity_ok = integrity == "ok";
    if !integrity_ok {
        return Err(format!(
            "integrity_check: {integrity} — the shred did not land clean"
        ));
    }
    // 6. Evidence: the physical erasure act rides the hash chain like every
    //    other erasure (kind `forget`). A missing row is an ERROR — silence
    //    is never certified.
    let audit_row = crate::audit::record(
        conn,
        crate::audit::AuditKind::Forget,
        "operator",
        "storage:physical-shred",
        crate::audit::AuditStatus::Ok,
        &format!(
            "physical_shred: secure_delete={secure_delete_readback}; \
             wal_checkpoint(TRUNCATE) pre+post; VACUUM; pages {pages_before}->{pages_after}; \
             freelist {freelist_before}->{freelist_after}; integrity ok"
        ),
    )
    .ok_or_else(|| {
        "the shred's audit row failed to write — the act is NOT evidenced; treat this DB's \
         erasure claims as unproven until re-run"
            .to_string()
    })?;
    Ok(ShredReport {
        pages_before,
        pages_after,
        freelist_before,
        freelist_after,
        secure_delete_readback,
        integrity_ok,
        audit_row,
    })
}

#[cfg(test)]
mod pins {
    use super::*;

    /// Fresh real-schema WAL DB on a unique temp file — the same shape the
    /// live DB has (run_migration sets WAL + creates every table incl. the
    /// vec0 store). sqlite-vec registers process-wide first so VACUUM can
    /// rebuild the schema.
    fn shred_db(tag: &str) -> (std::path::PathBuf, Connection) {
        crate::register_sqlite_vec::register_sqlite_vec();
        let path =
            std::env::temp_dir().join(format!("brain-shred-{}-{}.db", tag, std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut conn = Connection::open(&path).expect("open temp db");
        crate::migration::run_migration(&mut conn, 0).expect("migrate temp db");
        (path, conn)
    }

    fn cleanup(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    /// The honest residue claim, made executable: a normal (pre-strict)
    /// delete leaves the marker greppable in the raw main/WAL bytes — and
    /// the shred removes it from BOTH while the live schema survives
    /// (integrity ok, freelist 0, chain verifiable).
    #[test]
    fn shred_removes_deleted_row_residue() {
        let marker = "SHREDMARKER-7f3a-never-again";
        let (path, conn) = shred_db("residue");
        conn.execute(
            "INSERT INTO knowledge (title, content, content_hash, domain) \
             VALUES ('t', ?1, 'x', 'global')",
            rusqlite::params![marker],
        )
        .expect("insert marker row");
        // A plain DELETE with secure_delete off — the historical-purge shape
        // whose residue this verb exists to drop.
        conn.execute("DELETE FROM knowledge WHERE 1=1", [])
            .expect("delete marker row");
        drop(conn);
        let pre_main = std::fs::read(&path).expect("read main pre-shred");
        let pre_wal = std::fs::read(path.with_extension("db-wal")).unwrap_or_default();
        assert!(
            pre_main
                .windows(marker.len())
                .any(|w| w == marker.as_bytes())
                || pre_wal
                    .windows(marker.len())
                    .any(|w| w == marker.as_bytes()),
            "fixture has no teeth: the deleted marker must be residue-greppable pre-shred"
        );
        let mut conn = Connection::open(&path).expect("reopen for shred");
        let report = shred(&mut conn).expect("shred");
        assert_eq!(report.freelist_after, 0, "freelist must read back zero");
        assert!(report.integrity_ok);
        assert!(report.pages_after > 0);
        drop(conn);
        let post_main = std::fs::read(&path).expect("read main post-shred");
        let post_wal = std::fs::read(path.with_extension("db-wal")).unwrap_or_default();
        assert!(
            !post_main
                .windows(marker.len())
                .any(|w| w == marker.as_bytes())
                && !post_wal
                    .windows(marker.len())
                    .any(|w| w == marker.as_bytes()),
            "the shred must remove the deleted marker's bytes from main AND wal"
        );
        cleanup(&path);
    }

    /// The shred is an EVIDENCED act: a `forget` row lands on the chain and
    /// the chain still verifies end-to-end afterwards.
    #[test]
    fn shred_writes_forget_evidence_and_keeps_chain_verifiable() {
        let (path, mut conn) = shred_db("evidence");
        let report = shred(&mut conn).expect("shred");
        let forget_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE kind = 'forget'",
                [],
                |r| r.get(0),
            )
            .expect("count forget rows");
        assert!(
            forget_rows >= 1,
            "the shred must leave forget-kind evidence (row {})",
            report.audit_row
        );
        assert!(
            crate::audit::verify_chain(&conn),
            "the chain must verify end-to-end after the shred and its evidence row"
        );
        cleanup(&path);
    }
}
