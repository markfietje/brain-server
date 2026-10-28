//! `brain plugin uninstall` — the REGISTRATION act, and the erasure
//! boundary. The other half of the pack lifecycle (plan 28 §4's re-install
//! law completed): the installer imported content through doors that audit;
//! the uninstaller removes the REGISTRATION and leaves the imported DATA
//! exactly where it is.
//!
//! The law, in order:
//!
//! 1. **UNINSTALL IS A REGISTRATION ACT, NEVER AN ERASURE ACT.** The pack
//!    leaves the upgrade path (the register row is MARKED, never deleted —
//!    history is kept); the procedures and rules rows it imported remain,
//!    still governed by the ordinary retention/hold/DSAR machinery.
//!    Bulk-deleting content on uninstall would cross the erasure law
//!    (content may be entangled with live decisions' audit hashes); the
//!    operator erases data through the doors that exist (`brain
//!    source-delete`, `brain shred`) when they choose.
//! 2. **REFUSE-WHILE-LIVE, fail-closed** ([`installed_pack_is_live`]): the
//!    act is refused while anything the pack imported is still live — a
//!    roster principal granted a role the pack upserted, a profile the
//!    pack bound still bound to its domain. The refusal NAMES what is live
//!    and the door that clears it (`brain principal revoke`, the profile
//!    unbind). The C2 installer's transaction map (`install.rs`'s module
//!    doc: profile → roles → procedures → rules) is the checklist reversed.
//! 3. **HISTORY IS KEPT, NOT DELETED.** The register row gains `removed_at`
//!    (additive, presence-guarded, the A-1 replay pattern); after
//!    uninstall the re-install law resets — the SAME version may be
//!    installed again, because the C2 refusal guarded a tampered RE-OFFER
//!    riding a LIVE registration, and with the registration removed a
//!    fresh install is a fresh decision (its bytes are hash-checked
//!    against the signature anyway).
//! 4. **THE DRY-RUN LAW IS THE INSTALLER'S, INHERITED.** Nothing writes
//!    without `--yes`; one `Plugin` audit row lands on the real act. NO
//!    HTTP route — the mint refusal's posture, third time.

use rusqlite::Connection;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packs::install;

    fn db() -> Connection {
        crate::register_sqlite_vec::register_sqlite_vec();
        let mut conn = Connection::open_in_memory().expect("db");
        crate::migration::run_migration(&mut conn, 512).expect("migrate fixture");
        conn
    }

    /// U-1 — the A-1 replay pattern (the `principals.manages` precedent):
    /// a DB that already carries a register row gains the column without
    /// losing the row, and re-running the migration changes nothing (the
    /// presence-guarded ALTER is idempotent — the column lands exactly
    /// once). This is what the migrate-rehearse copy does on every
    /// rehearsal: VACUUM INTO carries the column, `run_migration` lands it
    /// on DBs that predate it.
    #[test]
    fn the_removed_at_migration_is_replay_safe_and_keeps_rows() {
        let mut conn = db();
        conn.execute(
            "INSERT INTO installed_packs(pack_id, version, kinds, manifest_hash,
                 archive_hash, file_count, import_audit_id, imported_at)
             VALUES ('support-pack', '0.1.0', 'domain,rules', 'm', 'a', 3, NULL, 1_000)",
            [],
        )
        .expect("the C2-shaped register row");
        // THE REPLAY: the same DB migrated again — every boot, and the
        // rehearse tool's copy phase.
        crate::migration::run_migration(&mut conn, 512).expect("a second migration run");
        crate::migration::run_migration(&mut conn, 512).expect("a third migration run");
        let columns: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('installed_packs')
                 WHERE name = 'removed_at'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(columns, 1, "the column lands exactly once");
        let (version, removed_at): (String, Option<i64>) = conn
            .query_row(
                "SELECT version, removed_at FROM installed_packs WHERE pack_id = 'support-pack'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("the row survived the replay");
        assert_eq!(version, "0.1.0", "a rehearsal forgets no register row");
        assert_eq!(
            removed_at, None,
            "NULL = a live registration: every pre-uninstall row reads live"
        );
        let _ = install::content_hash(b"the register module is the pin's home");
    }
}
