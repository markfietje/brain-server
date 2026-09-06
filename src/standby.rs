//! Warm standby — encrypted base + shipped WAL chunks + a rehearsed promote.
//!
//! Single-node SQLite is the doctrine; losing the box loses the memory. The
//! honest answer at this scale is a WARM standby built entirely from shipped
//! mechanisms: the backup v3 writer (AES-256-GCM / Argon2id) for the base,
//! a v3-wrapped copy of the WAL for the thin tail the base misses, and a
//! REHEARSED, timed promote. This module is library code consumed ONLY by
//! the `brain standby` CLI subcommands — an operator-run process, NOT a
//! server thread (a shipper inside the server it protects is a correlated
//! failure). NO hot failover, no consensus, no RPO=0 claims anywhere.
//!
//! One ship cycle (in THIS order — load-bearing):
//!   1. `PRAGMA wal_checkpoint(PASSIVE)` on the source — folds pending
//!      frames into the main db file so the base captures them.
//!   2. the EXISTING backup v3 writer → `base.v3` (its snapshot step runs a
//!      TRUNCATE checkpoint + `VACUUM INTO`, so the base is a consistent,
//!      encrypted post-checkpoint image). Because the writer truncates the
//!      WAL, the chunk MUST be copied after this step — a chunk copied
//!      earlier would hold pre-base frames and replaying it over the newer
//!      base would roll the restore back.
//!   3. copy the (now post-truncate) WAL as `wal/NNNN.frame-chunk`,
//!      wrapped through [`crate::backup::encrypt_v3_blob`] — the same v3
//!      envelope as the base, so NO unencrypted byte sits at rest on the
//!      follower.
//!   4. write + sign `manifest.json` (Ed25519 via the parcels signing
//!      convention, [`crate::ump_integrity::sign_manifest_bytes`]) LAST —
//!      the manifest vouches for artifacts already on disk. Artifacts land
//!      via temp+rename; an interrupted cycle therefore leaves the previous
//!      consistent pair and `status` fails closed until the next cycle
//!      heals it.
//!
//! Follower layout: `<dir>/base.v3` + `<dir>/base.v3.sha256` (the backup
//! writer's own checksum) + `<dir>/wal/NNNN.frame-chunk` (per-cycle history;
//! only the LATEST cycle's manifest + artifacts matter — restore uses the
//! latest base plus its chunk) + `manifest.json` + `manifest.sig.json`.
//!
//! The manifest carries: cycle id, ts, interval, checkpoint_lag_ms (the
//! measured checkpoint→chunk-copy window — the follower-side twin of the
//! v1.28.58 `brain_wal_pages_pending` gauge, which is the primary-side view
//! of the same pending work), the source db page-file sha256 (informational
//! identity), and sha256+size of the two shipped artifacts. `wal.frames`
//! counts the frames the chunk carries (positions 0..N of the post-truncate
//! wal stream — the chunk is always the full current wal, never a delta).

use crate::backup::BackupFormat;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

pub const MANIFEST_KIND: &str = "brain-standby-manifest";
pub const MANIFEST_VERSION: u32 = 1;
pub const BASE_FILE: &str = "base.v3";
const MANIFEST_FILE: &str = "manifest.json";
const MANIFEST_SIG_FILE: &str = "manifest.sig.json";

/// One warm-standby cycle's signed truth about the follower dir.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StandbyManifest {
    pub kind: String,
    pub version: u32,
    /// Monotonic per-follower-dir cycle id (1-based).
    pub cycle: u32,
    /// Unix seconds at cycle end (after the chunk landed).
    pub ts: i64,
    /// The shipper's cycle interval, seconds — the recurring unshipped window.
    pub interval_secs: u64,
    /// Measured checkpoint-start → chunk-copy-end duration, ms. The plan's
    /// "checkpoint lag": data committed inside the cycle lands in the base
    /// or the chunk; commits after the chunk wait one interval.
    pub checkpoint_lag_ms: u64,
    /// sha256 of the SOURCE db page file at ship time (informational
    /// identity — the encrypted base's own hash is `base.sha256`).
    pub source_db_sha256: String,
    pub base: ArtifactRef,
    pub wal: WalRef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub file: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WalRef {
    pub file: String,
    pub sha256: String,
    pub bytes: u64,
    /// Frames the chunk carries: positions 0..N of the post-truncate wal
    /// stream (the chunk is the full current wal file, never a delta).
    pub frames: u64,
    pub page_size: u32,
}

/// The `manifest.sig.json` sidecar — signature over the exact
/// `manifest.json` bytes, parcels convention.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManifestSig {
    pub signature: String,
    pub signed_by: String,
}

/// What a rehearsed promote measured.
#[derive(Debug, Clone)]
pub struct PromoteReport {
    pub cycle: u32,
    /// restore (decrypt + write base + decrypt/write chunk): seconds.
    pub restore_secs: f64,
    /// open + WAL recovery + `PRAGMA integrity_check`: seconds.
    pub verify_secs: f64,
    /// THE number: restore + open + verify, end to end.
    pub rto_secs: f64,
    /// Worst-case data-loss window: interval + checkpoint lag (seconds).
    pub rpo_max_secs: f64,
    /// How far behind the follower is right now: now − last cycle ts.
    pub follower_age_secs: i64,
    /// `PRAGMA integrity_check` verdict on the promoted db.
    pub integrity: String,
    /// Where the promote ran (kept when `preserve_workdir` is set).
    pub workdir: PathBuf,
}

/// Default follower dir: `BRAIN_STANDBY_DIR` env override, else
/// `~/.local/share/brain-server/standby`.
pub fn default_standby_dir() -> PathBuf {
    if let Ok(d) = std::env::var("BRAIN_STANDBY_DIR") {
        let p = PathBuf::from(d.trim());
        if !p.as_os_str().is_empty() {
            return p;
        }
    }
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    home.join(".local/share/brain-server/standby")
}

/// Frames in a wal file of `wal_len` bytes: 32-byte header, then
/// (24-byte frame header + page) per frame. A torn/absent tail is not
/// counted here (the recovery on promote stops at the last valid frame
/// anyway); this is the manifest's bookkeeping number.
pub fn wal_frame_count(wal_len: usize, page_size: u32) -> u64 {
    if page_size == 0 || wal_len <= 32 {
        return 0;
    }
    ((wal_len - 32) / (24 + page_size as usize)) as u64
}

/// THE RPO math, pinned by `promote_check_rpo_math`: the worst-case data
/// window = the ship interval (commits after the last chunk wait a cycle) +
/// the measured checkpoint lag (the in-cycle window the artifacts take to
/// land). No other term is honest to claim.
pub fn rpo_max_secs(interval_secs: u64, checkpoint_lag_ms: u64) -> f64 {
    interval_secs as f64 + checkpoint_lag_ms as f64 / 1000.0
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut h = Sha256::new();
    h.update(&bytes);
    Ok(hex::encode(h.finalize()))
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// temp-write + rename + fsync — an interrupted write never lands half an
/// artifact where a manifest (or a previous good one) expects whole bytes.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    {
        let mut f =
            std::fs::File::create(&tmp).map_err(|e| format!("create {}: {e}", tmp.display()))?;
        f.write_all(bytes)
            .map_err(|e| format!("write {}: {e}", tmp.display()))?;
        f.sync_all()
            .map_err(|e| format!("fsync {}: {e}", tmp.display()))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| format!("rename {}: {e}", path.display()))?;
    Ok(())
}

/// One ship cycle: PASSIVE checkpoint → base.v3 via the shipped backup v3
/// writer → encrypted WAL chunk → signed manifest, written LAST. Refuses
/// before ANY write when the operator signing key is absent (a follower
/// that cannot prove its manifest provenance is not a standby).
pub fn ship_cycle(
    db_path: &Path,
    dir: &Path,
    passphrase: &[u8],
    interval_secs: u64,
    cycle: u32,
) -> Result<StandbyManifest, String> {
    let Some((_, sk)) = crate::handlers::ump::operator_signing_key() else {
        return Err(
            "no operator signing key (UMP key dir) — standby refuses to ship an \
             unsigned manifest"
                .to_string(),
        );
    };
    if !db_path.is_file() {
        return Err(format!("source db missing: {}", db_path.display()));
    }
    std::fs::create_dir_all(dir.join("wal"))
        .map_err(|e| format!("create follower dir {}: {e}", dir.display()))?;
    let t0 = Instant::now();

    // 1. PASSIVE checkpoint — fold pending frames into the db file. PASSIVE
    //    never truncates; the writer in step 2 does.
    let page_size: u32 = {
        let conn = Connection::open(db_path).map_err(|e| format!("open source db: {e}"))?;
        conn.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |_| Ok(()))
            .map_err(|e| format!("wal_checkpoint(PASSIVE): {e}"))?;
        conn.query_row("PRAGMA page_size", [], |r| r.get::<_, i64>(0))
            .map_err(|e| format!("read page_size: {e}"))? as u32
    };

    // 2. Base: the EXISTING v3 writer (TRUNCATE + VACUUM INTO inside) —
    //    encrypted at rest with the same KDF. Written to a temp name and
    //    renamed so an interrupted cycle never leaves a half base.
    let base_tmp = dir.join(format!("{BASE_FILE}.cycle-tmp"));
    crate::backup::backup_with_config_dir_and_format(
        db_path,
        &base_tmp,
        passphrase,
        None,
        BackupFormat::V3,
    )
    .map_err(|e| format!("base backup failed: {e:#}"))?;
    let base_final = dir.join(BASE_FILE);
    std::fs::rename(&base_tmp, &base_final).map_err(|e| format!("promote base into place: {e}"))?;
    let base_sum_tmp = dir.join(format!("{BASE_FILE}.cycle-tmp.sha256"));
    if base_sum_tmp.is_file() {
        std::fs::rename(&base_sum_tmp, dir.join(format!("{BASE_FILE}.sha256")))
            .map_err(|e| format!("promote base checksum: {e}"))?;
    }

    // 3. Chunk: the wal AS IT STANDS after the truncate — only frames the
    //    base can miss. Wrapped in the same v3 envelope (no plaintext at
    //    rest on the follower). A missing/empty wal is a legitimate zero-
    //    frame chunk.
    let wal_name = format!(
        "{}-wal",
        db_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "brain.db".to_string())
    );
    let wal_plain = std::fs::read(db_path.with_file_name(&wal_name)).unwrap_or_default();
    let chunk_name = format!("wal/{cycle:04}.frame-chunk");
    let chunk_bytes = crate::backup::encrypt_v3_blob("wal-chunk", &wal_plain, passphrase)
        .map_err(|e| format!("encrypt wal chunk: {e:#}"))?;
    write_atomic(&dir.join(&chunk_name), &chunk_bytes)?;
    let checkpoint_lag_ms = t0.elapsed().as_millis() as u64;

    // 4. Manifest, signed over its exact bytes, written LAST.
    let manifest = StandbyManifest {
        kind: MANIFEST_KIND.to_string(),
        version: MANIFEST_VERSION,
        cycle,
        ts: unix_now(),
        interval_secs,
        checkpoint_lag_ms,
        source_db_sha256: sha256_file(db_path)?,
        base: ArtifactRef {
            file: BASE_FILE.to_string(),
            sha256: sha256_file(&base_final)?,
            bytes: std::fs::metadata(&base_final)
                .map_err(|e| format!("stat base: {e}"))?
                .len(),
        },
        wal: WalRef {
            file: chunk_name,
            sha256: {
                let mut h = Sha256::new();
                h.update(&chunk_bytes);
                hex::encode(h.finalize())
            },
            bytes: chunk_bytes.len() as u64,
            frames: wal_frame_count(wal_plain.len(), page_size),
            page_size,
        },
    };
    let manifest_json =
        serde_json::to_string(&manifest).map_err(|e| format!("serialize manifest: {e}"))?;
    let (sig_hex, signed_by) =
        crate::ump_integrity::sign_manifest_bytes(&sk, manifest_json.as_bytes());
    write_atomic(&dir.join(MANIFEST_FILE), manifest_json.as_bytes())?;
    let sig = ManifestSig {
        signature: sig_hex,
        signed_by,
    };
    let sig_json = serde_json::to_string(&sig).map_err(|e| format!("serialize sig: {e}"))?;
    write_atomic(&dir.join(MANIFEST_SIG_FILE), sig_json.as_bytes())?;
    Ok(manifest)
}

/// The integrity self-check, shared by `status` and `promote-check`: verify
/// the manifest's Ed25519 signature over its exact file bytes, then
/// recompute sha256 of every artifact the manifest vouches for. ANY
/// mismatch — tamper, truncation, a torn interrupted cycle — is `Err`
/// (fail closed).
pub fn verify_follower(dir: &Path) -> Result<StandbyManifest, String> {
    let manifest_bytes =
        std::fs::read(dir.join(MANIFEST_FILE)).map_err(|e| format!("read manifest: {e}"))?;
    let sig_json = std::fs::read_to_string(dir.join(MANIFEST_SIG_FILE))
        .map_err(|e| format!("read manifest signature: {e}"))?;
    let sig: ManifestSig =
        serde_json::from_str(&sig_json).map_err(|e| format!("parse manifest signature: {e}"))?;
    if !crate::ump_integrity::verify_manifest_bytes(&sig.signed_by, &sig.signature, &manifest_bytes)
    {
        return Err(format!(
            "follower manifest signature FAILED (dir {}) — the follower is not \
             trustworthy; re-ship a cycle before trusting it",
            dir.display()
        ));
    }
    let manifest: StandbyManifest =
        serde_json::from_slice(&manifest_bytes).map_err(|e| format!("parse manifest: {e}"))?;
    if manifest.kind != MANIFEST_KIND || manifest.version != MANIFEST_VERSION {
        return Err(format!(
            "unexpected manifest kind/version {}/{} (want {}/{}), dir {}",
            manifest.kind,
            manifest.version,
            MANIFEST_KIND,
            MANIFEST_VERSION,
            dir.display()
        ));
    }
    for (what, want, path) in [
        (
            "base",
            (manifest.base.sha256.as_str(), manifest.base.bytes),
            dir.join(&manifest.base.file),
        ),
        (
            "wal chunk",
            (manifest.wal.sha256.as_str(), manifest.wal.bytes),
            dir.join(&manifest.wal.file),
        ),
    ] {
        let bytes = std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let mut h = Sha256::new();
        h.update(&bytes);
        let got = hex::encode(h.finalize());
        if got != want.0 || bytes.len() as u64 != want.1 {
            return Err(format!(
                "follower {what} integrity FAILED ({}): sha256/size mismatch — \
                 tamper or a torn cycle; re-ship before trusting this follower",
                path.display()
            ));
        }
    }
    Ok(manifest)
}

/// Seconds since the follower's last completed cycle.
pub fn follower_age_secs(manifest: &StandbyManifest) -> i64 {
    unix_now() - manifest.ts
}

/// Whole cycles the follower is behind the shipper's cadence: age / interval.
pub fn cycles_behind(manifest: &StandbyManifest) -> u64 {
    if manifest.interval_secs == 0 {
        return 0;
    }
    (follower_age_secs(manifest).max(0) as u64) / manifest.interval_secs
}

/// The cycle id a (re)starting shipper resumes from: the verified manifest's
/// cycle, else the highest chunk number on disk (a torn/missing manifest
/// must not reset the counter — chunk names would collide), else 0.
pub fn resume_cycle(dir: &Path) -> u32 {
    if let Ok(m) = verify_follower(dir) {
        return m.cycle;
    }
    std::fs::read_dir(dir.join("wal"))
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| {
                    e.file_name()
                        .to_str()?
                        .split('.')
                        .next()?
                        .parse::<u32>()
                        .ok()
                })
                .max()
                .unwrap_or(0)
        })
        .unwrap_or(0)
}

/// THE DRILL: verify the follower, restore its base into a fresh temp dir,
/// replay the chunk as the restored db's WAL (SQLite recovery folds it in on
/// open), run `PRAGMA integrity_check`, and time every step. Prints nothing —
/// the CLI renders [`PromoteReport`]. On success the workdir is removed
/// unless `preserve_workdir`; on failure it is KEPT and the error names it
/// (forensics).
pub fn promote_check(
    dir: &Path,
    passphrase: &[u8],
    preserve_workdir: bool,
) -> Result<PromoteReport, String> {
    let manifest = verify_follower(dir)?;
    let workdir = std::env::temp_dir().join(format!(
        "brain-standby-promote-{}-{}",
        manifest.cycle,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&workdir).map_err(|e| format!("mkdir {}: {e}", workdir.display()))?;
    let t0 = Instant::now();
    let restored_db = workdir.join("brain.db");
    // The restore path is the SHIPPED one — checksum verify, KDF+GCM decrypt,
    // atomic write, audit-chain attestation on the restored copy.
    crate::backup::restore(&dir.join(&manifest.base.file), &restored_db, passphrase).map_err(
        |e| {
            format!(
                "restore failed (workdir kept for forensics: {}): {e:#}",
                workdir.display()
            )
        },
    )?;
    // The chunk becomes the restored db's WAL: SQLite recovery replays its
    // valid frames on open — the thin tail the base can miss.
    let chunk_cipher =
        std::fs::read(dir.join(&manifest.wal.file)).map_err(|e| format!("read chunk: {e}"))?;
    let chunk_plain = crate::backup::decrypt_v3_blob(&chunk_cipher, passphrase)
        .map_err(|e| format!("decrypt wal chunk (workdir {}): {e:#}", workdir.display()))?;
    std::fs::write(workdir.join("brain.db-wal"), &chunk_plain)
        .map_err(|e| format!("write restored wal: {e}"))?;
    let restore_secs = t0.elapsed().as_secs_f64();

    let t1 = Instant::now();
    let conn = Connection::open(&restored_db).map_err(|e| format!("open promoted db: {e}"))?;
    let rows: Vec<String> = conn
        .prepare("PRAGMA integrity_check")
        .and_then(|mut s| s.query_map([], |r| r.get::<_, String>(0))?.collect())
        .map_err(|e| format!("integrity_check: {e}"))?;
    let integrity = if rows.len() == 1 && rows[0] == "ok" {
        "ok".to_string()
    } else {
        rows.join("; ")
    };
    if integrity != "ok" {
        return Err(format!(
            "promoted db FAILED integrity_check: {integrity} (workdir kept for \
             forensics: {})",
            workdir.display()
        ));
    }
    let verify_secs = t1.elapsed().as_secs_f64();
    let report = PromoteReport {
        cycle: manifest.cycle,
        restore_secs,
        verify_secs,
        rto_secs: restore_secs + verify_secs,
        rpo_max_secs: rpo_max_secs(manifest.interval_secs, manifest.checkpoint_lag_ms),
        follower_age_secs: follower_age_secs(&manifest),
        integrity,
        workdir: workdir.clone(),
    };
    if !preserve_workdir {
        let _ = std::fs::remove_dir_all(&workdir);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use std::sync::{Mutex, MutexGuard, PoisonError};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn lock_env() -> MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A fixture operator key in a 0600 seed file (the OperatorKey idiom
    /// from parcels) — signing requires the operator key, fail closed.
    struct OperatorKey(tempfile::TempDir);
    impl OperatorKey {
        fn new() -> OperatorKey {
            let dir = tempfile::TempDir::new().unwrap();
            std::fs::write(dir.path().join("operator.key"), [7u8; 32]).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(
                    dir.path().join("operator.key"),
                    std::fs::Permissions::from_mode(0o600),
                )
                .unwrap();
            }
            // SAFETY: single-threaded under ENV_LOCK — the documented env-mutation posture.
            unsafe { std::env::set_var("BRAIN_UMP_KEY_DIR", dir.path()) };
            OperatorKey(dir)
        }
    }
    impl Drop for OperatorKey {
        fn drop(&mut self) {
            // SAFETY: single-threaded under ENV_LOCK.
            unsafe { std::env::remove_var("BRAIN_UMP_KEY_DIR") };
        }
    }

    /// A small WAL-mode db with `rows` note rows (the fixture corpus).
    fn make_wal_db(path: &Path, rows: usize) -> Connection {
        let conn = Connection::open(path).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn.execute_batch("CREATE TABLE notes(id INTEGER PRIMARY KEY, body TEXT NOT NULL)")
            .unwrap();
        for i in 0..rows {
            conn.execute(
                "INSERT INTO notes(body) VALUES (?1)",
                params![format!("note {i} — the standby roundtrip corpus")],
            )
            .unwrap();
        }
        conn
    }

    fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "standby-test-{tag}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    // ── pure math pins ────────────────────────────────────────────────

    /// promote_check_rpo_math — the printed RPO is interval + lag, exactly.
    /// No third term, no rounding slop: the runbook's RPO claim and the
    /// promote-check output must agree to the millisecond.
    #[test]
    fn promote_check_rpo_math() {
        assert_eq!(rpo_max_secs(30, 1_500), 31.5);
        assert_eq!(rpo_max_secs(0, 0), 0.0);
        assert_eq!(rpo_max_secs(60, 999), 60.999);
        assert_eq!(rpo_max_secs(5, 0), 5.0);
        // The interval dominates: a fast cycle barely moves the number.
        assert!(rpo_max_secs(30, 2_000) > 30.0 && rpo_max_secs(30, 2_000) < 33.0);
    }

    #[test]
    fn wal_frame_count_arithmetic() {
        assert_eq!(wal_frame_count(0, 4096), 0);
        assert_eq!(wal_frame_count(32, 4096), 0, "header only");
        assert_eq!(wal_frame_count(31, 4096), 0, "torn header");
        // header + 3 frames of (24 + 4096)
        assert_eq!(wal_frame_count(32 + 3 * (24 + 4096), 4096), 3);
        assert_eq!(
            wal_frame_count(32 + 3 * (24 + 4096) + 17, 4096),
            3,
            "torn tail"
        );
        assert_eq!(wal_frame_count(32 + 24, 0), 0, "degenerate page size");
    }

    /// A restarting shipper resumes the cycle counter: from the verified
    /// manifest when the follower is whole, from the highest chunk number
    /// when the manifest is torn (a reset to 0 would collide chunk names),
    /// from 0 on an empty dir.
    #[test]
    fn resume_cycle_reads_manifest_then_chunks() {
        let _guard = lock_env();
        let _key = OperatorKey::new();
        let work = tmp_dir("resume");
        let db_path = work.join("brain.db");
        make_wal_db(&db_path, 2);
        let follower = work.join("follower");
        assert_eq!(resume_cycle(&follower), 0, "empty dir");
        ship_cycle(&db_path, &follower, b"p", 30, 1).unwrap();
        assert_eq!(resume_cycle(&follower), 1, "whole manifest");
        // Tear the manifest: the highest chunk number carries the counter.
        std::fs::write(follower.join(MANIFEST_FILE), b"{ torn").unwrap();
        ship_cycle(&db_path, &follower, b"p", 30, 7).unwrap();
        std::fs::write(follower.join(MANIFEST_FILE), b"{ torn").unwrap();
        assert_eq!(resume_cycle(&follower), 7, "highest chunk wins");
        let _ = std::fs::remove_dir_all(&work);
    }

    // ── the cycle, end to end ─────────────────────────────────────────

    /// manifest_sig_verified_after_copy — a shipped cycle's manifest
    /// verifies against the artifacts actually on disk in the follower dir
    /// (signature over the exact copied bytes + recomputed artifact hashes),
    /// and every manifest field is sane.
    #[test]
    fn manifest_sig_verified_after_copy() {
        let _guard = lock_env();
        let _key = OperatorKey::new();
        let work = tmp_dir("sig-after-copy");
        let db_path = work.join("brain.db");
        make_wal_db(&db_path, 5);
        let follower = work.join("follower");
        let m = ship_cycle(&db_path, &follower, b"drill-pass", 30, 1).expect("ship cycle");
        assert!(
            _key.0.path().join("operator.key").is_file(),
            "fixture sanity: the operator seed file must exist for the cycle to sign"
        );
        assert_eq!(m.cycle, 1);
        assert_eq!(m.kind, MANIFEST_KIND);
        assert_eq!(m.base.file, BASE_FILE);
        assert!(m.ts > 0);
        assert!(
            m.checkpoint_lag_ms > 0,
            "the lag is measured, not stamped zero"
        );
        // The chunk is a v3 envelope (magic bytes), never plaintext frames.
        let chunk = std::fs::read(follower.join(&m.wal.file)).unwrap();
        assert_eq!(&chunk[..4], b"BSBK", "chunk must be encrypted at rest");
        // The shipped self-check passes on the copied artifacts.
        let verified = verify_follower(&follower).expect("follower verifies");
        assert_eq!(verified, m);
        // And the tamper twin (same fixture): flip a manifest byte → FAIL.
        let mut mb = std::fs::read(follower.join(MANIFEST_FILE)).unwrap();
        let last = mb.len() - 1;
        mb[last] ^= 0x01;
        std::fs::write(follower.join(MANIFEST_FILE), &mb).unwrap();
        assert!(
            verify_follower(&follower).is_err(),
            "a flipped manifest byte must fail the signature check"
        );
        let _ = std::fs::remove_dir_all(&work);
    }

    /// follower_tamper_detected — flip ONE byte in the shipped WAL chunk and
    /// the self-check fails closed (sha256 mismatch); flip one in the base,
    /// same verdict. Restoring the byte restores the verdict (proves the
    /// check reads the artifacts, not stale state).
    #[test]
    fn follower_tamper_detected() {
        let _guard = lock_env();
        let _key = OperatorKey::new();
        let work = tmp_dir("tamper");
        let db_path = work.join("brain.db");
        make_wal_db(&db_path, 3);
        let follower = work.join("follower");
        let m = ship_cycle(&db_path, &follower, b"drill-pass", 30, 1).unwrap();
        assert!(verify_follower(&follower).is_ok());

        let chunk_path = follower.join(&m.wal.file);
        let original = std::fs::read(&chunk_path).unwrap();
        let mut tampered = original.clone();
        let mid = original.len() / 2;
        tampered[mid] ^= 0x01;
        std::fs::write(&chunk_path, &tampered).unwrap();
        assert!(
            verify_follower(&follower).is_err(),
            "one flipped byte in the wal chunk must fail status closed"
        );
        std::fs::write(&chunk_path, &original).unwrap();
        assert!(
            verify_follower(&follower).is_ok(),
            "restored bytes, restored verdict"
        );

        let base_path = follower.join(&m.base.file);
        let original = std::fs::read(&base_path).unwrap();
        let mut tampered = original.clone();
        let last = original.len() - 1;
        tampered[last] ^= 0x80;
        std::fs::write(&base_path, &tampered).unwrap();
        assert!(
            verify_follower(&follower).is_err(),
            "base tamper must fail too"
        );
        let _ = std::fs::remove_dir_all(&work);
    }

    /// No operator key ⇒ the cycle refuses BEFORE any artifact exists.
    #[test]
    fn ship_cycle_refuses_without_operator_key() {
        let _guard = lock_env();
        let empty = tempfile::TempDir::new().unwrap();
        // SAFETY: single-threaded under ENV_LOCK.
        unsafe { std::env::set_var("BRAIN_UMP_KEY_DIR", empty.path()) };
        let work = tmp_dir("no-key");
        let db_path = work.join("brain.db");
        make_wal_db(&db_path, 1);
        let follower = work.join("follower");
        let err = ship_cycle(&db_path, &follower, b"p", 30, 1).unwrap_err();
        assert!(err.contains("no operator signing key"), "{err}");
        assert!(
            !follower.exists(),
            "a refusing cycle must not have created the follower dir"
        );
        // SAFETY: single-threaded under ENV_LOCK.
        unsafe { std::env::remove_var("BRAIN_UMP_KEY_DIR") };
        let _ = std::fs::remove_dir_all(&work);
    }

    /// THE roundtrip property (proptest): backup→follower→restore→
    /// integrity-check green for any row count — every row shipped before
    /// the last cycle is on the promoted db, byte-for-byte readable, on the
    /// frozen fixture corpus shape.
    use proptest::prelude::*;
    proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(4))]
        #[test]
        fn standby_roundtrip_restores_every_shipped_row(rows in 1usize..200) {
            let _guard = lock_env();
            let _key = OperatorKey::new();
            let work = tmp_dir("roundtrip");
            let db_path = work.join("brain.db");
            make_wal_db(&db_path, rows);
            let follower = work.join("follower");
            ship_cycle(&db_path, &follower, b"drill-pass", 30, 1).unwrap();
            // More rows ship in cycle 2; rows inserted AFTER cycle 2's chunk
            // are the honest RPO window and are NOT asserted present.
            let conn = Connection::open(&db_path).unwrap();
            for i in 0..7 {
                conn.execute(
                    "INSERT INTO notes(body) VALUES (?1)",
                    params![format!("cycle-2 note {i}")],
                )
                .unwrap();
            }
            drop(conn);
            let m2 = ship_cycle(&db_path, &follower, b"drill-pass", 30, 2).unwrap();
            let report = promote_check(&follower, b"drill-pass", true)
                .expect("the drill promotes");
            assert_eq!(report.integrity, "ok");
            assert_eq!(report.cycle, 2);
            assert_eq!(report.rpo_max_secs, rpo_max_secs(30, m2.checkpoint_lag_ms));
            // Every row shipped by cycle 2 is on the promoted db.
            let promoted = Connection::open(report.workdir.join("brain.db")).unwrap();
            let count: i64 = promoted
                .query_row("SELECT COUNT(*) FROM notes", [], |r| r.get(0))
                .unwrap();
            prop_assert_eq!(count, rows as i64 + 7);
            drop(promoted);
            let _ = std::fs::remove_dir_all(&work);
        }
    }
}
