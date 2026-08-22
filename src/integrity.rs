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
}

impl Snapshot {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "last_backup": self.last_backup,
            "integrity_ok": self.integrity_ok,
        })
    }
}

/// Shared handle to the latest snapshot result. Clone cheaply; the writer is
/// the scheduler task, the readers are `/health` calls.
#[derive(Clone, Default)]
pub struct SnapshotState {
    inner: Arc<RwLock<Snapshot>>,
}

impl SnapshotState {
    pub fn read(&self) -> Snapshot {
        self.inner.read().map(|s| s.clone()).unwrap_or_default()
    }
    fn set(&self, snap: Snapshot) {
        if let Ok(mut g) = self.inner.write() {
            *g = snap;
        }
    }
}

/// Spawn the periodic snapshot task. Runs once immediately, then every
/// `SNAPSHOT_INTERVAL_SECS`. Best-effort — failures are logged and surface as
/// `integrity_ok: false` in `/health`; the server keeps running.
pub fn spawn_scheduler(db_path: PathBuf, state: SnapshotState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(SNAPSHOT_INTERVAL_SECS));
        // First tick completes immediately — run a snapshot on boot so a fresh
        // install's `/health` reports `integrity_ok: true` without a 6h wait.
        interval.tick().await;
        loop {
            let now = iso_now();
            match run_once(&db_path) {
                Ok(integrity_ok) => {
                    state.set(Snapshot {
                        last_backup: now.clone(),
                        integrity_ok,
                    });
                    info!(target: "brain::integrity", "snapshot ok at {now}");
                }
                Err(e) => {
                    state.set(Snapshot {
                        last_backup: now,
                        integrity_ok: false,
                    });
                    warn!(target: "brain::integrity", "snapshot failed: {e:#}");
                }
            }
            interval.tick().await;
        }
    });
}

/// One backup+integrity cycle. Snapshot is `<db>.snapshot-<ts>.bak` in the
/// same directory; `SNAPSHOT_KEEP` rolling copies are kept, older ones pruned.
/// `integrity_check` runs on the snapshot (not the live DB) so a corrupt
/// result is reproducible.
pub fn run_once(db_path: &Path) -> anyhow::Result<bool> {
    let dir = db_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("db has no parent dir"))?;
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let snap = dir.join(format!(
        "{}.snapshot-{}.bak",
        db_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("brain"),
        stamp
    ));
    let conn = Connection::open(db_path)?;
    // Checkpoint WAL first so the snapshot is consistent + WAL is small.
    let _ = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
    // The shared escaper (backup::vacuum_into) — never a hand-rolled literal.
    brain_server::backup::vacuum_into(&conn, &snap)?;
    drop(conn);

    // Snapshots are plaintext copies of the whole store; lock them to the
    // owner so a world-readable backup is never left behind.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&snap, std::fs::Permissions::from_mode(0o600))?;
    }

    // Integrity check on the SNAPSHOT (not the live DB) so a failure is
    // reproducible against the same bytes the operator could restore.
    let snap_conn = Connection::open(&snap)?;
    let result: String = snap_conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    let ok = result == "ok";

    prune(dir, db_path, SNAPSHOT_KEEP);
    Ok(ok)
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
                    .map(|n| n.starts_with(&prefix) && n.ends_with(".bak"))
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

    #[test]
    fn run_once_produces_integrity_ok_snapshot() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("brain.db");
        // A fresh DB is created on open; VACUUM INTO needs at least one table.
        let conn = Connection::open(&db).unwrap();
        conn.execute("CREATE TABLE t(x INTEGER)", []).unwrap();
        conn.execute("INSERT INTO t VALUES (42)", []).unwrap();
        drop(conn);

        let ok = run_once(&db).expect("run_once");
        assert!(ok, "fresh DB snapshot must pass integrity_check");

        // One .bak file exists, named after the DB.
        let mut bak_count = 0;
        for e in std::fs::read_dir(dir.path()).unwrap().flatten() {
            if e.path()
                .file_name()
                .and_then(|s| s.to_str())
                .map(|n| n.starts_with("brain.db.snapshot-") && n.ends_with(".bak"))
                .unwrap_or(false)
            {
                bak_count += 1;
            }
        }
        assert_eq!(bak_count, 1, "exactly one snapshot after one run");
    }

    #[test]
    fn prune_keeps_only_n_newest() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("brain.db");
        Connection::open(&db)
            .unwrap()
            .execute("CREATE TABLE t(x INTEGER)", [])
            .unwrap();
        // Run keep+2 times, sleeping so each stamp differs.
        for _ in 0..(SNAPSHOT_KEEP as u32 + 2) {
            run_once(&db).unwrap();
            std::thread::sleep(Duration::from_millis(1100));
        }
        let bak_count = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map(|n| n.starts_with("brain.db.snapshot-") && n.ends_with(".bak"))
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(
            bak_count, SNAPSHOT_KEEP,
            "prune keeps exactly SNAPSHOT_KEEP"
        );
    }

    #[test]
    fn snapshot_state_round_trip() {
        let s = SnapshotState::default();
        assert!(!s.read().integrity_ok, "default is not-ok");
        s.set(Snapshot {
            last_backup: "2026-07-28T00:00:00Z".into(),
            integrity_ok: true,
        });
        let r = s.read();
        assert!(r.integrity_ok);
        assert_eq!(r.last_backup, "2026-07-28T00:00:00Z");
    }

    /// v1.27.27 M1 (F-26 class): a POISONED snapshot lock must read as the
    /// fail-closed posture (`integrity_ok = false`) — `/health`'s backup/
    /// integrity claim degrades to not-ok, never keeps certifying the last
    /// healthy cycle. Companion to `alert::poisoned_chain_watch_reads_as_not_ok`.
    #[test]
    fn poisoned_snapshot_reads_as_not_ok() {
        let s = SnapshotState::default();
        s.set(Snapshot {
            last_backup: "2026-07-28T00:00:00Z".into(),
            integrity_ok: true,
        });
        assert!(s.read().integrity_ok, "sanity: healthy before poisoning");
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = s.inner.write().expect("lock before panic");
            panic!("poison the snapshot lock");
        }));
        let r = s.read();
        assert!(
            !r.integrity_ok,
            "a poisoned snapshot must report NOT ok (fail closed)"
        );
    }

    /// Civil-date sanity: epoch 0 = 1970-01-01T00:00:00Z.
    #[test]
    fn epoch_zero_is_1970_01_01() {
        let (y, mo, d, h, mi, s) = epoch_to_ymdhms(0);
        assert_eq!((y, mo, d, h, mi, s), (1970, 1, 1, 0, 0, 0));
    }
}
