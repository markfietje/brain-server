//! scheduled rolling backups + integrity self-check.
//!
//! The encrypted-bundle path in `backup.rs` is for off-host transport
//! (`brain backup` CLI, manual operator action). This module adds the
//! always-on, zero-config rolling backup: a periodic task that
//! snapshots the live DB with `VACUUM INTO`, runs `PRAGMA integrity_check`
//! on the snapshot, keeps the last N copies, and exposes the result via
//! `/health`. Plain SQLite files (not encrypted bundles) — these are
//! operator-side snapshots for "the live DB ate itself" recovery, not for
//! off-host transport.
//!
//! The result is a single `Arc<RwLock<Snapshot>>` shared with `/health`. The
//! task is fire-and-forget; failures are logged and surface as `integrity_ok:
//! false` rather than aborting the server.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};

use rusqlite::Connection;
use tracing::{info, warn};

/// How often the scheduler runs `VACUUM INTO` + `integrity_check`. Default
/// 6h — frequent enough to bound data loss, cheap enough to ignore on the
/// target mini PC. The first run happens immediately on boot.
pub const SNAPSHOT_INTERVAL_SECS: u64 = 6 * 60 * 60;

/// Rolling copy count. The N+1'th snapshot deletes the oldest. Small because
/// these are operator-side snapshots, not archival backups.
pub const SNAPSHOT_KEEP: usize = 4;

/// Snapshot of the last backup+integrity cycle. Read by `/health`.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// ISO-8601 of the last successful snapshot, or empty if never run.
    pub last_backup: String,
    /// True when the last snapshot's `PRAGMA integrity_check` returned "ok".
    pub integrity_ok: bool,
    /// The retention posture of the last cycle: `"encrypted"` (a sealed v3
    /// bundle was retained) or `"check-only"` (integrity_check ran, nothing
    /// was retained — no passphrase is configured, and an unencrypted
    /// retained snapshot is a certified-erasure leak, not a recovery asset).
    pub retention: &'static str,
}

impl Snapshot {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "last_backup": self.last_backup,
            "integrity_ok": self.integrity_ok,
            "retention": self.retention,
        })
    }
}

/// Shared handle to the latest snapshot result. Clone cheaply; the writer is
/// the scheduler task, the readers are `/health` calls.
///
/// Lock bounds (Headroom): the critical sections are one struct clone (read)
/// / one assignment (set). No I/O, no nesting, no SQL. Poison: the read
/// falls back to the default snapshot (honest "not checked"), the write is
/// skipped. Readers are the `/health` scrape (request, wait-measured); the
/// writer is the background scheduler.
#[derive(Clone, Default)]
pub struct SnapshotState {
    inner: Arc<RwLock<Snapshot>>,
}

impl SnapshotState {
    pub fn read(&self) -> Snapshot {
        crate::concurrency::rwlock_read_measured(&self.inner)
            .map(|s| s.clone())
            .unwrap_or_default()
    }
    fn set(&self, snap: Snapshot) {
        if let Ok(mut g) = crate::concurrency::rwlock_write_measured(&self.inner) {
            *g = snap;
        }
    }
}

/// Spawn the periodic snapshot task. Runs once immediately, then every
/// `SNAPSHOT_INTERVAL_SECS`. Best-effort — failures are logged and surface as
/// `integrity_ok: false` in `/health`; the server keeps running.
pub fn spawn_scheduler(db_path: PathBuf, state: SnapshotState) {
    tokio::spawn(async move {
        // Resolved ONCE for the process lifetime: a passphrase file swap
        // takes effect at restart (disclosed in the runbook — snapshot
        // encryption is a posture, not a hot-reload knob).
        let passphrase = crate::config::backup_passphrase();
        if passphrase.is_none() {
            warn!(
                target: "brain::integrity",
                "BRAIN_BACKUP_PASSPHRASE_FILE is not set: the integrity scheduler will run                  its check but RETAIN NO snapshot (an unencrypted retained snapshot is an                  erasure leak, not a recovery asset). Set it to retain sealed v3 snapshots                  restorable with `brain restore`."
            );
        }
        warn_about_legacy_plaintext_snapshots(&db_path);
        let mut interval = tokio::time::interval(Duration::from_secs(SNAPSHOT_INTERVAL_SECS));
        // First tick completes immediately — run a snapshot on boot so a fresh
        // install's `/health` reports `integrity_ok: true` without a 6h wait.
        interval.tick().await;
        loop {
            let now = iso_now();
            match run_once_with_passphrase(&db_path, passphrase.as_deref()) {
                Ok(outcome) => {
                    state.set(Snapshot {
                        last_backup: now.clone(),
                        integrity_ok: outcome.integrity_ok,
                        retention: outcome.retention,
                    });
                    info!(target: "brain::integrity", "snapshot ok at {now} ({})", outcome.retention);
                }
                Err(e) => {
                    state.set(Snapshot {
                        last_backup: now,
                        integrity_ok: false,
                        retention: "none (last cycle failed)",
                    });
                    warn!(target: "brain::integrity", "snapshot failed: {e:#}");
                }
            }
            interval.tick().await;
        }
    });
}

/// Pre-round snapshots are PLAINTEXT `<db>.snapshot-<ts>.bak` files. They are
/// the operator's only recovery artifact from before the upgrade, so they are
/// NEVER deleted by this code — but they are disclosed loudly at boot (and
/// named in the shred runbook) because erased content in them stays greppable
/// until the operator disposes of them.
fn warn_about_legacy_plaintext_snapshots(db_path: &Path) {
    let Some(dir) = db_path.parent() else { return };
    let Some(stem) = db_path.file_name().and_then(|s| s.to_str()) else {
        return;
    };
    let prefix = format!("{stem}.snapshot-");
    let legacy: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| {
                    e.file_name()
                        .to_str()
                        .map(|n| n.starts_with(&prefix) && n.ends_with(".bsbak"))
                        .unwrap_or(false)
                })
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    if !legacy.is_empty() {
        warn!(
            target: "brain::integrity",
            "{} legacy PLAINTEXT snapshot(s) remain on disk (not deleted — they may be the              only pre-upgrade recovery copy): {}. Their content is unread by this build and              stays operator-level; dispose of them with `brain shred` when no longer needed.",
            legacy.len(),
            legacy.join(", ")
        );
    }
}

/// One backup+integrity cycle. Snapshot is `<db>.snapshot-<ts>.bak` in the
/// same directory; `SNAPSHOT_KEEP` rolling copies are kept, older ones pruned.
/// `integrity_check` runs on the snapshot (not the live DB) so a corrupt
/// result is reproducible.
pub fn run_once(db_path: &Path) -> anyhow::Result<SnapshotOutcome> {
    run_once_with_passphrase(db_path, crate::config::backup_passphrase().as_deref())
}

/// What one cycle produced: the integrity verdict plus the retention posture
/// that produced it (rides to `/health` so an operator can SEE that nothing
/// is being retained, not infer it from missing files).
pub struct SnapshotOutcome {
    pub integrity_ok: bool,
    pub retention: &'static str,
}

/// One backup+integrity cycle.
///
/// **Encrypted posture** (a passphrase is configured): the cycle writes a
/// sealed v3 bundle (`<db>.snapshot-<ts>.bsbak` + `.sha256` sidecar) through
/// the SAME writer `brain backup` uses — Argon2id/AES-256-GCM, GCM-AAD
/// header, manifest — so `brain restore` consumes scheduler snapshots with
/// no new format surface, and the writer's guarded temp means no plaintext
/// copy outlives the call. `integrity_check` then runs on the DECRYPTED
/// bytes of the sealed artifact (the restore path itself), and rotation
/// keeps the last `SNAPSHOT_KEEP` bundles.
///
/// **Check-only posture** (no passphrase): the cycle checkpoints, snapshots
/// to a zeroed-on-drop temp, checks it, and retains NOTHING — an
/// unencrypted retained snapshot beside a database that just certified an
/// erasure is a leak, not a recovery asset. The `/health` integrity signal
/// survives either way.
pub fn run_once_with_passphrase(
    db_path: &Path,
    passphrase: Option<&[u8]>,
) -> anyhow::Result<SnapshotOutcome> {
    let dir = db_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("db has no parent dir"))?;
    let stem = db_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("brain")
        .to_string();
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Checkpoint WAL first so the snapshot is consistent + WAL is small.
    {
        let conn = Connection::open(db_path)?;
        let _ = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
    }

    match passphrase {
        Some(pass) => {
            let out = dir.join(format!("{stem}.snapshot-{stamp}.bsbak"));
            crate::backup::backup_with_config_dir_and_format(
                db_path,
                &out,
                pass,
                None,
                crate::backup::BackupFormat::V3,
            )?;
            let plain = crate::backup::verify_sealed_bundle(&out, pass)?;
            let tmp = dir.join(format!(".brain-check-{stamp}.db"));
            let _guard = ZeroingGuard(tmp.clone());
            std::fs::write(&tmp, &plain)?;
            let ok = check_integrity(&tmp)?;
            prune(dir, db_path, SNAPSHOT_KEEP);
            Ok(SnapshotOutcome {
                integrity_ok: ok,
                retention: "encrypted",
            })
        }
        None => {
            let tmp = dir.join(format!(".brain-check-{stamp}.db"));
            let _guard = ZeroingGuard(tmp.clone());
            {
                let conn = Connection::open(db_path)?;
                crate::backup::vacuum_into(&conn, &tmp)?;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
            }
            let ok = check_integrity(&tmp)?;
            Ok(SnapshotOutcome {
                integrity_ok: ok,
                retention: "check-only",
            })
        }
    }
}

/// RAII: zero then delete — for plaintext temps that must leave no residue.
/// Best-effort at every step (a temp that cannot be zeroed is still deleted;
/// the failure is logged by the caller's cycle result if it matters).
struct ZeroingGuard(PathBuf);

impl Drop for ZeroingGuard {
    fn drop(&mut self) {
        let _ = std::fs::File::options()
            .write(true)
            .open(&self.0)
            .and_then(|mut f| {
                let len = f.metadata().map(|m| m.len()).unwrap_or(0);
                let zeros = vec![0u8; 64 * 1024];
                let mut left = len as usize;
                while left > 0 {
                    let n = left.min(zeros.len());
                    std::io::Write::write_all(&mut f, &zeros[..n])?;
                    left -= n;
                }
                Ok(())
            });
        let _ = std::fs::remove_file(&self.0);
    }
}

fn check_integrity(path: &Path) -> anyhow::Result<bool> {
    let conn = Connection::open(path)?;
    let result: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    Ok(result == "ok")
}

/// Delete the oldest snapshots beyond the keep count. Best-effort.
fn prune(dir: &Path, db_path: &Path, keep: usize) {
    let stem = match db_path.file_name().and_then(|s| s.to_str()) {
        Some(s) => s.to_string(),
        None => return,
    };
    let prefix = format!("{stem}.snapshot-");
    let mut snaps: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|s| s.to_str())
                    .map(|n| n.starts_with(&prefix) && n.ends_with(".bsbak"))
                    .unwrap_or(false)
            })
            .collect(),
        Err(_) => return,
    };
    if snaps.len() <= keep {
        return;
    }
    // Oldest first by mtime (stable on ties).
    snaps.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
    let drop_count = snaps.len().saturating_sub(keep);
    for p in snaps.iter().take(drop_count) {
        let _ = std::fs::remove_file(p);
    }
}

fn iso_now() -> String {
    // ponytail: hand-rolled ISO-8601 (no chrono dep on this path). The
    // `chrono` crate is already pulled in elsewhere; reusing it here would
    // add a `use` for one call. Keep it local — this is the only ISO string
    // this module needs.
    use std::time::SystemTime;
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // YYYY-MM-DDTHH:MM:SSZ from epoch seconds — no leap seconds, UTC only.
    let (y, mo, d, h, mi, s) = epoch_to_ymdhms(secs);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Civil date from Unix epoch seconds (UTC). Implements the standard
/// Howard Hinnant algorithm — noleap, era-based. Small enough to inline.
fn epoch_to_ymdhms(secs: i64) -> (i32, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let h = (rem / 3600) as u32;
    let mi = ((rem % 3600) / 60) as u32;
    let s = (rem % 60) as u32;
    // Hinnant's days_from_civil-inverse, era = floor((days >= 0 ? days : days - 146096) / 146097)
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i32 + era as i32 * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    (y, mo, d, h, mi, s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const NEEDLE: &str = "SECRET-NEEDLE-7f3a";

    fn fixture_db(dir: &std::path::Path) -> std::path::PathBuf {
        let db = dir.join("brain.db");
        // The REAL schema: a production database always carries the audit
        // chain, and the round-trip arm restores through the production verb
        // — which refuses chainless images by design.
        crate::register_sqlite_vec::register_sqlite_vec();
        let mut conn = Connection::open(&db).unwrap();
        crate::migration::run_migration(&mut conn, 1).unwrap();
        conn.execute(
            "INSERT INTO knowledge (content, source, content_hash) VALUES (?1, 'test', 'h1')",
            rusqlite::params![NEEDLE],
        )
        .unwrap();
        drop(conn);
        db
    }

    fn snapshot_files(dir: &std::path::Path, suffix: &str) -> Vec<std::path::PathBuf> {
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|s| s.to_str())
                    .map(|n| n.starts_with("brain.db.snapshot-") && n.ends_with(suffix))
                    .unwrap_or(false)
            })
            .collect()
    }

    /// Every file in the snapshot directory EXCEPT the live database and its
    /// WAL/SHM siblings must be free of the live row's plaintext — that is
    /// the whole finding: retained artifacts were greppable copies.
    fn no_plaintext_residue(dir: &std::path::Path) {
        for entry in std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("brain.db") && !name.contains("snapshot-") {
                continue; // the live DB (and -wal/-shm) legitimately holds it
            }
            if name.ends_with(".sha256") {
                continue; // hex digests of ciphertext
            }
            let bytes = std::fs::read(entry.path()).unwrap_or_default();
            assert!(
                !bytes.windows(NEEDLE.len()).any(|w| w == NEEDLE.as_bytes()),
                "plaintext residue in {name}: a retained/transient artifact leaked the live row"
            );
        }
    }

    #[test]
    fn encrypted_posture_seals_and_restores() {
        let dir = tempdir().unwrap();
        let db = fixture_db(dir.path());
        let pass = b"snapshot-passphrase";

        let outcome = run_once_with_passphrase(&db, Some(pass)).expect("cycle");
        assert!(outcome.integrity_ok, "fresh DB must pass integrity_check");
        assert_eq!(outcome.retention, "encrypted");

        let bsbaks = snapshot_files(dir.path(), ".bsbak");
        assert_eq!(
            bsbaks.len(),
            1,
            "exactly one sealed snapshot after one cycle"
        );
        let raw = std::fs::read(&bsbaks[0]).unwrap();
        assert!(
            raw.starts_with(b"BSBK"),
            "the retained snapshot must be a sealed v3 bundle, not a plaintext DB"
        );
        let sidecar = bsbaks[0].with_file_name(format!(
            "{}.sha256",
            bsbaks[0].file_name().unwrap().to_string_lossy()
        ));
        assert!(sidecar.exists(), "the checksum sidecar rides the bundle");

        no_plaintext_residue(dir.path());

        // The operator verb round-trips it: restore into a fresh copy and
        // the row (needle included) comes back.
        let restored = dir.path().join("restored.db");
        crate::backup::restore(&bsbaks[0], &restored, pass).expect("restore the sealed snapshot");
        let conn = Connection::open(&restored).unwrap();
        let note: String = conn
            .query_row(
                "SELECT content FROM knowledge WHERE content_hash = 'h1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(note, NEEDLE, "restored snapshot reproduces the live row");
    }

    #[test]
    fn check_only_posture_retains_nothing() {
        let dir = tempdir().unwrap();
        let db = fixture_db(dir.path());

        let outcome = run_once_with_passphrase(&db, None).expect("cycle");
        assert!(
            outcome.integrity_ok,
            "the integrity signal survives without a passphrase"
        );
        assert_eq!(outcome.retention, "check-only");
        assert!(
            snapshot_files(dir.path(), ".bsbak").is_empty(),
            "no sealed snapshot without a passphrase"
        );
        assert!(
            snapshot_files(dir.path(), ".bak").is_empty(),
            "no PLAINTEXT retained snapshot without a passphrase — that is the leak"
        );
        no_plaintext_residue(dir.path());
    }

    #[test]
    fn rotation_bounds_the_retained_set() {
        let dir = tempdir().unwrap();
        let db = fixture_db(dir.path());
        let pass = b"snapshot-passphrase";
        for _ in 0..(SNAPSHOT_KEEP as u32 + 2) {
            run_once_with_passphrase(&db, Some(pass)).expect("cycle");
            std::thread::sleep(Duration::from_millis(1100));
        }
        assert_eq!(
            snapshot_files(dir.path(), ".bsbak").len(),
            SNAPSHOT_KEEP,
            "prune keeps exactly SNAPSHOT_KEEP sealed snapshots"
        );
    }

    #[test]
    fn legacy_plaintext_snapshots_survive_the_cycle() {
        let dir = tempdir().unwrap();
        let db = fixture_db(dir.path());
        let legacy = dir.path().join("brain.db.snapshot-1.bak");
        std::fs::write(&legacy, b"pre-upgrade recovery copy").unwrap();

        run_once_with_passphrase(&db, Some(b"pass")).expect("cycle");

        assert!(
            legacy.exists(),
            "a pre-upgrade plaintext snapshot is the operator's only old recovery copy — \
             disclosed, never deleted by the cycle"
        );
        let mut kept = std::fs::read(&legacy).unwrap();
        kept.extend(b""); // silence unused-mut style lints if reflowed
        assert_eq!(kept, b"pre-upgrade recovery copy", "byte-untouched");
    }
}
