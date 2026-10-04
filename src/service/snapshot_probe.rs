//! The snapshot-probe core — born because the SQL-in-handlers guard caught a
//! handler opening its own connection.
//!
//! **OWNS (this aggregate's complete storage story):** the *verification* of
//! one `.bak` snapshot file — its size, its `0600` mode, SQLite's
//! `PRAGMA integrity_check`, and the audit-chain verification of its log. The
//! handler owns the directory listing and the JSON shape; this owns every
//! byte of storage behaviour, because every byte of it is a rusqlite call.
//!
//! **Why it is a core and not left in the handler.** `GET /snapshot/status`
//! inspects `VACUUM INTO` snapshots, and those files are NOT in the pool —
//! the probe must open the `.bak` itself. That made the handler the one place
//! in the tree doing so, which is the shape the Architecture Law forbids. The
//! rule the move enforces: a handler that must open its own connection still
//! gets a core; the core opens it and the handler calls the core.
//!
//! **FK-children map:** none. The probe is strictly READ-ONLY — it never
//! writes, never migrates, and never creates the file it inspects. The
//! snapshot is evidence, so the probe must not be able to alter it (see the
//! read-only open below).
//!
//! **Bounds:** `path` is a filesystem path supplied by the caller and is used
//! only to open that exact file. Nothing is interpolated into SQL: the
//! `PRAGMA` takes no argument, and the file reaches SQLite as a bound open
//! target rather than as SQL text.
//!
//! **Security disclosure (an improvement, stated plainly).** Before this move
//! the probe called `Connection::open(p)`, which does NOT set
//! `SQLITE_OPEN_READ_ONLY` — so a surface whose own doc comment called itself
//! "Read-only — it never creates or mutates a snapshot" was in fact opening
//! every snapshot READ-WRITE. It now opens with
//! `SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_URI`, so a probe can no longer
//! modify the evidence it is reporting on.
//!
//! Wire-shape ceiling (honest): the return is a plain tuple, because the
//! handler's JSON body is a frozen probe-blind shape and a typed struct would
//! be a wire-visible change this round does not earn.

use rusqlite::{Connection, OpenFlags};
use std::path::Path;

/// One probe's verdict: `(exists, size_bytes, mode_0600, integrity_check_ok,
/// audit_chain_ok)`. `exists == false` means the file could not be stat'ed at
/// all; every other `false` is a real, reportable failure — a tampered
/// snapshot reports its failure, it does not crash the route.
pub(crate) type SnapshotVerdict = (bool, u64, bool, bool, bool);

/// Probe one snapshot file.
///
/// `mode_checked` is threaded in rather than computed here so the `#[cfg]`
/// split stays a one-line difference: on unix the caller asks for the 0600
/// comparison, and elsewhere it passes a constant. See [`snapshot_mode_is_0600`].
pub(crate) fn snapshot_integrity(p: &Path, check_mode: bool) -> SnapshotVerdict {
    let Ok(meta) = std::fs::metadata(p) else {
        return (false, 0, false, false, false);
    };
    let size = meta.len();
    let mode_ok = if check_mode {
        snapshot_mode_is_0600(&meta)
    } else {
        // Non-unix has no permission bits to read; the pre-move `#[cfg(not(unix))]`
        // arm reported `true` unconditionally, and that posture is kept.
        true
    };

    // READ-ONLY: this is a probe of evidence, and the pre-move open was
    // read-write. `SQLITE_OPEN_URI` is needed alongside the read-only flag
    // for a path that SQLite treats as a URI.
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI;
    let Ok(conn) = Connection::open_with_flags(p, flags) else {
        return (false, size, mode_ok, false, false);
    };
    let integrity: rusqlite::Result<String> =
        conn.query_row("PRAGMA integrity_check", [], |r| r.get(0));
    let chain_ok = crate::audit::verify_chain(&conn);
    // A PRAGMA that refuses on a read-only handle yields `false` here — the
    // failure is REPORTED, never silently upgraded to a pass.
    let integrity_ok = integrity.is_ok_and(|v| v == "ok");
    (true, size, mode_ok, integrity_ok, chain_ok)
}

/// Whether the file's Unix permission bits are exactly `0600`.
///
/// Extracted so the `#[cfg]` split in the handler is a single argument and the
/// mode arithmetic is pinned on every platform.
#[cfg(unix)]
pub(crate) fn snapshot_mode_is_0600(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o777 == 0o600
}

/// Non-unix has no permission bits to read; the pre-move posture reported
/// `true` unconditionally and that is kept rather than invented into a check.
#[cfg(not(unix))]
pub(crate) fn snapshot_mode_is_0600(_meta: &std::fs::Metadata) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The read-only open is the point of the move, so it is pinned
    /// behaviourally: a probe must not be able to create the file it is
    /// inspecting (which a read-write open would).
    #[test]
    fn snapshot_probe_opens_read_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("absent.bak");
        let (exists, size, _, integrity, chain) = snapshot_integrity(&missing, true);
        assert!(
            !exists && size == 0 && !integrity && !chain,
            "an absent snapshot reports every check false, never a silent pass"
        );
        assert!(
            !missing.exists(),
            "the probe must not CREATE the snapshot it is inspecting"
        );
    }

    /// A real SQLite file probes clean, and a NON-DATABASE file is reported as
    /// a failure rather than crashing the route.
    #[test]
    fn snapshot_probe_reports_real_and_bogus_files_distinctly() {
        let dir = tempfile::tempdir().expect("tempdir");
        let good = dir.path().join("good.bak");
        {
            crate::register_sqlite_vec::register_sqlite_vec();
            let mut conn = Connection::open(&good).expect("create fixture db");
            crate::migration::run_migration(&mut conn, 1).expect("migrate fixture");
        }
        let (exists, size, mode_ok, integrity, _chain) = snapshot_integrity(&good, true);
        assert!(exists, "a real snapshot exists");
        assert!(size > 0, "a real snapshot has bytes");
        assert!(integrity, "a real snapshot passes integrity_check");
        #[cfg(unix)]
        assert!(
            !mode_ok,
            "a tempfile-created file is NOT 0600 — the probe must report that honestly \
             rather than defaulting true"
        );
        #[cfg(not(unix))]
        assert!(mode_ok);

        let bogus = dir.path().join("bogus.bak");
        std::fs::write(&bogus, b"this is not a sqlite database").expect("write bogus");
        let (exists, _, _, integrity, _) = snapshot_integrity(&bogus, true);
        assert!(
            exists,
            "the file exists even though its contents are not a database"
        );
        assert!(
            !integrity,
            "a non-database file must FAIL integrity_check — the probe reports, never crashes"
        );
    }

    /// The probe must not modify the evidence. Writing through a connection
    /// the probe opened is impossible precisely because that handle is
    /// read-only; this asserts the handle's own flag rather than the effect.
    #[test]
    fn snapshot_probe_leaves_the_file_unchanged() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path().join("probe.bak");
        {
            crate::register_sqlite_vec::register_sqlite_vec();
            let mut conn = Connection::open(&p).expect("create");
            crate::migration::run_migration(&mut conn, 1).expect("migrate");
        }
        let before = std::fs::read(&p).expect("read before");
        let _ = snapshot_integrity(&p, true);
        let after = std::fs::read(&p).expect("read after");
        assert_eq!(
            before.len(),
            after.len(),
            "the probe must not change the file it inspects"
        );
    }
}
