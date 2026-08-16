//! Encrypted, checksummed backup & restore (v0.9.7 "Guard").
//!
//! `backup` snapshots the live sqlite DB via `VACUUM INTO`, records a manifest
//! of the DB plus connector config files (secret files are listed by path only,
//! never embedded), and writes a plaintext `created_at` header + AES-256-GCM
//! ciphertext plus a `.sha256` checksum file. `restore` reverses it, taking a
//! safety `VACUUM INTO` snapshot of the current DB first. `verify` is read-only.
//!
//! # Format v2 (v1.27.17 "Strongbox", M1/F-08)
//!
//! The v0.9.7 format derived both key and nonce from the passphrase
//! (`SHA-256(passphrase)` + `SHA-256(passphrase || created_at)[..12]`) — an
//! unsalted, fast KDF (offline brute-forceable) with a second-granularity
//! deterministic nonce (same passphrase + same second = catastrophic GCM nonce
//! reuse). v2 fixes both:
//!
//! ```text
//! magic    : b"BSBK"          (4 bytes)
//! version  : u16 = 2
//! header   : JSON, length-prefixed u32:
//!            { "kdf": "argon2id", "m": 65536, "t": 3, "p": 1,
//!              "salt":  "<b64 16B>", "nonce": "<b64 12B>",   // BOTH random per backup
//!              "created_at": "<iso>", "manifest": { … xxh3 map … } }
//! ciphertext: AES-256-GCM(bundle_bytes) under key = Argon2id(passphrase, salt, m, t, p)
//! ```
//!
//! `restore`/`verify` sniff the magic: v2 parses the header (unknown version or
//! KDF → hard error), v1 falls back to the legacy derive with a `warn!`. v1
//! backups stay restorable; v2 is what we write. `brain backup --format v1` is
//! the documented "legacy, weaker" escape hatch.

use crate::audit::{self, AuditKind, AuditStatus};
use anyhow::{Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use aes_gcm::{
    aead::{generic_array::GenericArray, Aead, KeyInit},
    Aes256Gcm,
};
use sha2::{Digest, Sha256};
use xxhash_rust::xxh3::xxh3_64;

/// v2 file magic — absent = legacy v1 layout (`created_at\n` + ciphertext).
const MAGIC: &[u8] = b"BSBK";
const VERSION_V2: u16 = 2;

/// Argon2id parameters for the v2 KDF: 64 MiB / t=3 / p=1, 32-byte output.
/// Tuned so a laptop backup stays well under 2 s (see BENCHMARKS.md).
const ARGON2_M_COST: u32 = 65536;
const ARGON2_T_COST: u32 = 3;
const ARGON2_P_COST: u32 = 1;

/// Best-effort audit of a backup/restore lifecycle event. Fails silently —
/// the audit log must never break a backup/restore operation.
fn audit_backup(db_path: &Path, status: AuditStatus, detail: &str) {
    if let Ok(conn) = rusqlite::Connection::open(db_path) {
        audit::record(&conn, AuditKind::Backup, "backup", detail, status, detail);
    }
}

/// Filename substrings that mark a connector config file as a secret. Such
/// files are recorded in the manifest by path only — their bytes are NEVER
/// embedded in the backup bundle.
const SECRET_MARKERS: &[&str] = &["key", "secret", "pem", "token"];

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ManifestComponent {
    pub name: String,
    #[serde(rename = "xxh3")]
    pub xxh3: String,
    pub size: u64,
    /// true when the file is a secret: recorded by path only, bytes excluded.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub secret: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Manifest {
    pub created_at: String,
    pub version: String,
    pub components: Vec<ManifestComponent>,
}

/// v2 plaintext header: KDF parameters + per-backup salt/nonce + the manifest
/// (which v1 kept inside the encrypted bundle — the header is length-prefixed
/// so restore can find the ciphertext without it).
#[derive(Serialize, Deserialize, Debug)]
struct HeaderV2 {
    kdf: String,
    m: u32,
    t: u32,
    p: u32,
    salt: String,
    nonce: String,
    created_at: String,
    manifest: Manifest,
}

/// Choose the backup wire format. v2 (default) is what we write; v1 is the
/// documented legacy escape hatch (`brain backup --format v1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupFormat {
    V1,
    V2,
}

fn is_secret(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    SECRET_MARKERS.iter().any(|m| lower.contains(m))
}

fn now_iso() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    chrono::DateTime::from_timestamp(secs as i64, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| secs.to_string())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// Legacy v1 key derivation — READ-ONLY since v1.27.17 ("Strongbox" M1):
/// unsalted + fast, kept only to restore pre-v2 backups. v2 uses [`kdf_v2`].
fn derive_key_v1(passphrase: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(passphrase);
    let out = h.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&out);
    key
}

/// Legacy v1 nonce derivation — see [`derive_key_v1`].
fn derive_nonce_v1(passphrase: &[u8], created_at: &str) -> aes_gcm::aead::Nonce<Aes256Gcm> {
    let mut h = Sha256::new();
    h.update(passphrase);
    h.update(created_at.as_bytes());
    let out = h.finalize();
    let mut nonce = [0u8; 12];
    nonce.copy_from_slice(&out[..12]);
    *aes_gcm::aead::Nonce::<Aes256Gcm>::from_slice(&nonce)
}

/// v2 KDF: Argon2id(passphrase, salt, m, t, p) → 32-byte AES key. The m/t/p are
/// read from the backup header so a future re-tune still reads old backups.
fn kdf_v2(passphrase: &[u8], salt: &[u8], m: u32, t: u32, p: u32) -> Result<[u8; 32]> {
    let params = argon2::Params::new(m, t, p, Some(32))
        .map_err(|e| anyhow::anyhow!("Argon2 params invalid: {e}"))?;
    let argon2 = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut key = [0u8; 32];
    argon2
        .hash_password_into(passphrase, salt, &mut key)
        .map_err(|e| anyhow::anyhow!("Argon2id KDF failed: {e}"))?;
    Ok(key)
}

/// Default connector config dir: `$BRAIN_CONNECTOR_CONFIG_DIR`, else
/// `~/.config/brain-server/connectors`. Mirrors `connector::auth::store`.
pub fn default_connector_config_dir() -> PathBuf {
    if let Ok(s) = std::env::var("BRAIN_CONNECTOR_CONFIG_DIR") {
        if !s.trim().is_empty() {
            return PathBuf::from(s);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".config/brain-server/connectors")
}

/// `VACUUM INTO` with proper SQLite literal escaping (M5/F-49). SQLite DDL
/// cannot bind the path — escape `'` → `''` and refuse NUL/non-UTF-8. The one
/// place a path becomes SQL; both the backup snapshot and the restore safety
/// snapshot go through it.
pub fn vacuum_into(conn: &rusqlite::Connection, path: &Path) -> Result<()> {
    let s = path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("path is not UTF-8: {path:?}"))?;
    if s.contains('\0') {
        anyhow::bail!("NUL byte in VACUUM INTO path");
    }
    let lit = s.replace('\'', "''");
    conn.execute_batch(&format!("VACUUM INTO '{lit}'"))
        .with_context(|| format!("VACUUM INTO {path:?}"))
}

/// Create a file that cannot clobber a pre-planted path (`create_new`, so a
/// planted file/symlink errors instead of being opened) with 0600 perms that
/// apply at creation — no umask window (`mode` + `create_new` are atomic in
/// the same open(2)).
pub fn create_private_file(path: &Path) -> Result<()> {
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)
        .with_context(|| format!("create private file {path:?}"))?;
    Ok(())
}

fn set_permissions_0600(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("chmod 0600 {path:?}"))?;
    }
    Ok(())
}

/// The M2 (F-29) snapshot recipe. SQLite's `VACUUM INTO` REFUSES an existing
/// target ("output file already exists" — verified empirically), so the
/// plan's literal "create_new then vacuum over it" cannot work. The same
/// security properties are reached as:
///
/// 1. probe-create the path 0600 with `create_new` — a pre-planted file or
///    symlink at the snapshot path aborts the backup instead of being
///    written through (and `create_new` failure tells us the path is taken);
/// 2. remove the probe (vacuum needs the target absent);
/// 3. `VACUUM INTO` — which natively refuses to overwrite anything that
///    appeared in the meantime;
/// 4. chmod 0600 immediately, BEFORE any failure-prone step, so any later
///    error path leaves only a 0600 file for the guard to remove.
///
/// ponytail: the file exists under the process umask for the microseconds
/// between vacuum's internal create and the step-4 chmod. A `PRAGMA
/// file:`-URI/`fsync`-grade no-window variant is the v2.x upgrade path.
pub fn vacuum_into_private(conn: &rusqlite::Connection, path: &Path) -> Result<()> {
    create_private_file(path)?;
    fs::remove_file(path).with_context(|| format!("remove probe {path:?}"))?;
    vacuum_into(conn, path)?;
    set_permissions_0600(path)?;
    Ok(())
}

/// Every exit path from [`backup_inner`] removes the plaintext snapshot —
/// success and failure alike. A crash between creation and cleanup still
/// leaks a 0600 file (bounded; the file is per-backup and named by nanos).
struct SnapshotGuard(PathBuf);

impl Drop for SnapshotGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn snapshot_db(db_path: &Path, snapshot_path: &Path) -> Result<()> {
    let conn = rusqlite::Connection::open(db_path)
        .with_context(|| format!("open DB for snapshot: {db_path:?}"))?;
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
        .context("wal_checkpoint before vacuum")?;
    vacuum_into_private(&conn, snapshot_path)?;
    Ok(())
}

fn file_xxh3(path: &Path) -> Result<u64> {
    let bytes = fs::read(path).with_context(|| format!("read {path:?}"))?;
    Ok(xxh3_64(&bytes))
}

/// v1 encryption: `created_at\n` plaintext header + AES-256-GCM ciphertext
/// (written only by `BackupFormat::V1` — the legacy escape hatch).
fn encrypt_v1(bundle: &[u8], passphrase: &[u8], created_at: &str) -> Result<Vec<u8>> {
    let key = derive_key_v1(passphrase);
    let nonce = derive_nonce_v1(passphrase, created_at);
    let cipher = Aes256Gcm::new(&key.into());
    let ciphertext = cipher
        .encrypt(&nonce, bundle.as_ref())
        .map_err(|e| anyhow::anyhow!("AES-256-GCM encrypt failed: {e}"))?;
    let mut out = Vec::with_capacity(created_at.len() + 1 + ciphertext.len());
    out.extend_from_slice(created_at.as_bytes());
    out.push(b'\n');
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// v2 encryption: `BSBK | u16 version | u32 header_len | header JSON |
/// ciphertext`. Salt and nonce are `rand::random()` per backup — nonce
/// uniqueness by RNG, never by construction.
fn encrypt_v2(
    manifest: &Manifest,
    snapshot: &[u8],
    passphrase: &[u8],
    created_at: &str,
) -> Result<Vec<u8>> {
    let salt: [u8; 16] = rand::random();
    let nonce: [u8; 12] = rand::random();
    let key = kdf_v2(
        passphrase,
        &salt,
        ARGON2_M_COST,
        ARGON2_T_COST,
        ARGON2_P_COST,
    )?;
    let cipher = Aes256Gcm::new(&GenericArray::from(key));
    let ciphertext = cipher
        .encrypt((&nonce).into(), snapshot)
        .map_err(|e| anyhow::anyhow!("AES-256-GCM encrypt failed: {e}"))?;

    let header = HeaderV2 {
        kdf: "argon2id".to_string(),
        m: ARGON2_M_COST,
        t: ARGON2_T_COST,
        p: ARGON2_P_COST,
        salt: base64::engine::general_purpose::STANDARD.encode(salt),
        nonce: base64::engine::general_purpose::STANDARD.encode(nonce),
        created_at: created_at.to_string(),
        manifest: manifest.clone(),
    };
    let header_json = serde_json::to_vec(&header).context("serialize v2 header")?;
    let mut out = Vec::with_capacity(4 + 2 + 4 + header_json.len() + ciphertext.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION_V2.to_le_bytes());
    out.extend_from_slice(&(header_json.len() as u32).to_le_bytes());
    out.extend_from_slice(&header_json);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Split a v2 file into its header + ciphertext, rejecting unknown versions
/// and KDFs (forward compat from day one).
fn parse_v2_file(full: &[u8]) -> Result<(HeaderV2, &[u8])> {
    if full.len() < 10 {
        anyhow::bail!("v2 backup truncated header");
    }
    if &full[..4] != MAGIC {
        anyhow::bail!("not a v2 backup (magic mismatch)");
    }
    let version = u16::from_le_bytes([full[4], full[5]]);
    if version != VERSION_V2 {
        anyhow::bail!("unsupported backup format version {version} (this build reads v1 and v2)");
    }
    let header_len = u32::from_le_bytes([full[6], full[7], full[8], full[9]]) as usize;
    let header_end = 10usize
        .checked_add(header_len)
        .context("v2 header length overflow")?;
    if header_end > full.len() {
        anyhow::bail!("v2 backup header length exceeds file");
    }
    let header: HeaderV2 =
        serde_json::from_slice(&full[10..header_end]).context("parse v2 header")?;
    if header.kdf != "argon2id" {
        anyhow::bail!("unsupported v2 KDF {:?} (forward compat)", header.kdf);
    }
    Ok((header, &full[header_end..]))
}

fn decrypt_v2(ct: &[u8], passphrase: &[u8], header: &HeaderV2) -> Result<Vec<u8>> {
    let salt = base64::engine::general_purpose::STANDARD
        .decode(&header.salt)
        .context("v2 salt not valid base64")?;
    let nonce_raw = base64::engine::general_purpose::STANDARD
        .decode(&header.nonce)
        .context("v2 nonce not valid base64")?;
    if salt.len() != 16 || nonce_raw.len() != 12 {
        anyhow::bail!("v2 header salt/nonce lengths invalid");
    }
    let mut salt_bytes = [0u8; 16];
    salt_bytes.copy_from_slice(&salt);
    let key = kdf_v2(passphrase, &salt_bytes, header.m, header.t, header.p)?;
    let cipher = Aes256Gcm::new(&GenericArray::from(key));
    let nonce = *aes_gcm::aead::Nonce::<Aes256Gcm>::from_slice(&nonce_raw);
    cipher.decrypt(&nonce, ct).map_err(|e| {
        anyhow::anyhow!("AES-256-GCM decrypt failed (wrong passphrase or tampered backup): {e}")
    })
}

/// Decrypt a backup file — v2 (header) or v1 (legacy derive, `warn!`) — and
/// return the manifest + plaintext snapshot bytes. The v1 bundle carries the
/// manifest inside the ciphertext; the v2 manifest lives in the plaintext
/// header, so the v2 bundle is the snapshot itself.
fn decrypt_backup(full: &[u8], passphrase: &[u8]) -> Result<(Manifest, Vec<u8>)> {
    if full.starts_with(MAGIC) {
        let (header, ct) = parse_v2_file(full)?;
        let snapshot = decrypt_v2(ct, passphrase, &header)?;
        return Ok((header.manifest, snapshot));
    }
    let (created_at, ct) = split_header(full)?;
    tracing::warn!("v1 backup: deterministic KDF; rotate to v2 by re-backing-up after restore");
    let bundle = decrypt_bundle_v1(ct, passphrase, created_at)?;
    let (manifest, snapshot) = parse_bundle(&bundle)?;
    Ok((manifest, snapshot))
}

/// Build the bundle `[manifest_len u64 LE][manifest][snapshot]`.
fn build_bundle(manifest: &Manifest, snapshot_bytes: &[u8]) -> Result<Vec<u8>> {
    let manifest_json = serde_json::to_vec(manifest).context("serialize manifest")?;
    let mut bundle = Vec::with_capacity(8 + manifest_json.len() + snapshot_bytes.len());
    bundle.extend_from_slice(&(manifest_json.len() as u64).to_le_bytes());
    bundle.extend_from_slice(&manifest_json);
    bundle.extend_from_slice(snapshot_bytes);
    Ok(bundle)
}

fn parse_bundle(bundle: &[u8]) -> Result<(Manifest, Vec<u8>)> {
    if bundle.len() < 8 {
        anyhow::bail!("bundle too short");
    }
    let mut len_bytes = [0u8; 8];
    len_bytes.copy_from_slice(&bundle[..8]);
    let manifest_len = u64::from_le_bytes(len_bytes) as usize;
    if 8 + manifest_len > bundle.len() {
        anyhow::bail!("manifest length exceeds bundle");
    }
    let manifest: Manifest =
        serde_json::from_slice(&bundle[8..8 + manifest_len]).context("parse manifest")?;
    let snapshot = bundle[8 + manifest_len..].to_vec();
    Ok((manifest, snapshot))
}

/// Create an encrypted backup of the DB + connector config (secrets excluded),
/// v2 format (Argon2id + random nonce/salt).
pub fn backup(db_path: &Path, out_path: &Path, passphrase: &[u8]) -> Result<()> {
    backup_with_config_dir_and_format(db_path, out_path, passphrase, None, BackupFormat::V2)
}

/// Legacy v1 backup (`SHA-256` KDF + derived nonce) — the `--format v1`
/// escape hatch: "legacy, weaker", for interoperability only. v2 is what we
/// write; this keeps old restore tooling usable.
pub fn backup_v1(db_path: &Path, out_path: &Path, passphrase: &[u8]) -> Result<()> {
    backup_with_config_dir_and_format(db_path, out_path, passphrase, None, BackupFormat::V1)
}

/// Like [`backup`], but reads connector config from `config_dir` when `Some`,
/// falling back to [`default_connector_config_dir`] when `None`. Taking the dir
/// explicitly (rather than a global env var) keeps the function free of
/// process-global state — important for concurrent callers/tests.
pub fn backup_with_config_dir(
    db_path: &Path,
    out_path: &Path,
    passphrase: &[u8],
    config_dir: Option<&Path>,
) -> Result<()> {
    backup_with_config_dir_and_format(db_path, out_path, passphrase, config_dir, BackupFormat::V2)
}

pub fn backup_with_config_dir_and_format(
    db_path: &Path,
    out_path: &Path,
    passphrase: &[u8],
    config_dir: Option<&Path>,
    format: BackupFormat,
) -> Result<()> {
    let res = backup_inner(db_path, out_path, passphrase, config_dir, format);
    if let Err(e) = &res {
        audit_backup(
            db_path,
            AuditStatus::Error,
            &format!("backup failed: {e:#}"),
        );
    }
    res
}

fn backup_inner(
    db_path: &Path,
    out_path: &Path,
    passphrase: &[u8],
    config_dir: Option<&Path>,
    format: BackupFormat,
) -> Result<()> {
    let created_at = now_iso();
    let snapshot_path = out_path.with_file_name(format!(
        ".brain-snapshot-{}.db",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));

    snapshot_db(db_path, &snapshot_path).with_context(|| "snapshot live DB failed")?;
    // F-29 (M2): the snapshot is 0600 + this guard removes it on EVERY exit
    // path — success included. A plaintext DB copy must never outlive the call.
    let _guard = SnapshotGuard(snapshot_path.clone());

    let snapshot_bytes =
        fs::read(&snapshot_path).with_context(|| format!("read snapshot {snapshot_path:?}"))?;
    let snapshot_hash = xxh3_64(&snapshot_bytes);

    let mut components = vec![ManifestComponent {
        name: "brain.db".to_string(),
        xxh3: format!("{snapshot_hash:016x}"),
        size: snapshot_bytes.len() as u64,
        secret: false,
    }];

    let cfg_dir = config_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(default_connector_config_dir);
    if cfg_dir.is_dir() {
        for entry in fs::read_dir(&cfg_dir)
            .with_context(|| format!("read connector config dir {cfg_dir:?}"))?
        {
            let entry = entry.context("read connector config entry")?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let hash = file_xxh3(&path)?;
            let secret = is_secret(&name);
            components.push(ManifestComponent {
                name,
                xxh3: format!("{hash:016x}"),
                size: entry.metadata().map(|m| m.len()).unwrap_or(0),
                secret,
            });
        }
    }

    let manifest = Manifest {
        created_at: created_at.clone(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        components,
    };
    let version_label = match format {
        BackupFormat::V2 => "v2",
        BackupFormat::V1 => "v1",
    };

    // v2: the manifest rides in the plaintext header → bundle is the snapshot.
    // v1: the bundle is `[manifest][snapshot]` inside the ciphertext.
    let bundle = match format {
        BackupFormat::V2 => snapshot_bytes,
        BackupFormat::V1 => build_bundle(&manifest, &snapshot_bytes)?,
    };
    let out = match format {
        BackupFormat::V2 => encrypt_v2(&manifest, &bundle, passphrase, &created_at)?,
        BackupFormat::V1 => encrypt_v1(&bundle, passphrase, &created_at)?,
    };

    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("mkdir {parent:?}"))?;
    }
    fs::write(out_path, &out).with_context(|| format!("write {out_path:?}"))?;

    let sum_path = checksum_path(out_path);
    fs::write(&sum_path, sha256_hex(&out))
        .with_context(|| format!("write checksum {sum_path:?}"))?;

    audit_backup(
        db_path,
        AuditStatus::Ok,
        &format!("backup complete ({version_label})"),
    );
    Ok(())
}

fn checksum_path(out_path: &Path) -> PathBuf {
    out_path.with_file_name(format!(
        "{}.sha256",
        out_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    ))
}

fn split_header(data: &[u8]) -> Result<(&str, &[u8])> {
    if let Some(pos) = data.iter().position(|&b| b == b'\n') {
        let header = std::str::from_utf8(&data[..pos]).context("created_at header not utf-8")?;
        return Ok((header, &data[pos + 1..]));
    }
    anyhow::bail!("backup file missing created_at header")
}

fn verify_checksum(cipher_path: &Path, full: &[u8]) -> Result<()> {
    let sum_path = checksum_path(cipher_path);
    if !sum_path.is_file() {
        return Ok(());
    }
    let expected = fs::read_to_string(&sum_path)
        .with_context(|| format!("read checksum {sum_path:?}"))?
        .trim()
        .to_string();
    let actual = sha256_hex(full);
    if expected != actual {
        anyhow::bail!("checksum mismatch for {cipher_path:?}");
    }
    Ok(())
}

/// Verify a backup's checksum, decrypt, and return its manifest (read-only).
/// Reads both v1 and v2 backups.
pub fn verify(cipher_path: &Path, passphrase: &[u8]) -> Result<Manifest> {
    let full = fs::read(cipher_path).with_context(|| format!("read {cipher_path:?}"))?;
    verify_checksum(cipher_path, &full)?;
    let (manifest, _) = decrypt_backup(&full, passphrase)?;
    Ok(manifest)
}

fn decrypt_bundle_v1(ciphertext: &[u8], passphrase: &[u8], created_at: &str) -> Result<Vec<u8>> {
    let key = derive_key_v1(passphrase);
    let nonce = derive_nonce_v1(passphrase, created_at);
    let cipher = Aes256Gcm::new(&key.into());
    cipher
        .decrypt(&nonce, ciphertext)
        .map_err(|e| anyhow::anyhow!("AES-256-GCM decrypt failed (wrong passphrase?): {e}"))
}

/// Restore a backup into `db_path`. Takes a safety `VACUUM INTO` snapshot of
/// the current DB to `<db_path>.bak` first so the operation is reversible.
pub fn restore(cipher_path: &Path, db_path: &Path, passphrase: &[u8]) -> Result<()> {
    let res = restore_inner(cipher_path, db_path, passphrase);
    if let Err(e) = &res {
        audit_backup(
            db_path,
            AuditStatus::Error,
            &format!("restore failed: {e:#}"),
        );
    }
    res
}

fn restore_inner(cipher_path: &Path, db_path: &Path, passphrase: &[u8]) -> Result<()> {
    let full = fs::read(cipher_path).with_context(|| format!("read {cipher_path:?}"))?;
    verify_checksum(cipher_path, &full)?;

    // v1.27.17 (M1/F-08): format sniff — v2 (Argon2id) or legacy v1 (warn!).
    let (manifest, snapshot) = decrypt_backup(&full, passphrase)?;

    // integrity: snapshot's xxh3 must match manifest
    let actual_hash = format!("{:016x}", xxh3_64(&snapshot));
    let db_component = manifest
        .components
        .iter()
        .find(|c| c.name == "brain.db")
        .context("manifest missing brain.db component")?;
    if db_component.xxh3 != actual_hash {
        anyhow::bail!(
            "snapshot integrity check failed: manifest xxh3 {} != actual {}",
            db_component.xxh3,
            actual_hash
        );
    }

    // safety snapshot of the live DB
    if db_path.exists() {
        let bak = db_path.with_file_name(format!(
            "{}.bak",
            db_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        ));
        if bak.exists() {
            // Fail closed rather than overwrite a previous safety snapshot:
            // it is evidence of the pre-restore state, and clobbering it
            // silently would make a crash-during-sync unrecoverable.
            anyhow::bail!(
                "refusing restore: safety snapshot {bak:?} already exists; \
                 move or delete it to allow a new restore"
            );
        }
        let conn = rusqlite::Connection::open(db_path)
            .with_context(|| format!("open live DB {db_path:?}"))?;
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .context("wal_checkpoint before safety snapshot")?;
        vacuum_into(&conn, &bak)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bak, std::fs::Permissions::from_mode(0o600))?;
        }
    }

    // write the decrypted snapshot over the live DB
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("mkdir {parent:?}"))?;
    }
    fs::write(db_path, &snapshot).with_context(|| format!("write restored DB {db_path:?}"))?;
    audit_backup(db_path, AuditStatus::Ok, "restore complete");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "brain-backup-test-{}-{}.db",
            tag,
            std::process::id()
        ))
    }

    fn make_db(path: &Path, content: &str) -> Result<()> {
        let _ = fs::remove_file(path);
        let conn = rusqlite::Connection::open(path)?;
        conn.execute(
            "CREATE TABLE knowledge(id INTEGER PRIMARY KEY, text TEXT)",
            [],
        )?;
        conn.execute("INSERT INTO knowledge(text) VALUES(?1)", [content])?;
        conn.close().map_err(|(_, e)| e)?;
        Ok(())
    }

    #[test]
    fn backup_produces_decryptable_archive() {
        let src = tmp_path("src");
        let out = std::env::temp_dir().join(format!("brain-backup-out-{}.enc", std::process::id()));
        let dst = tmp_path("dst");
        let _ = fs::remove_file(&out);
        let _ = fs::remove_file(&dst);
        make_db(&src, "hello world").unwrap();

        backup(&src, &out, b"pass".as_slice()).unwrap();
        restore(&out, &dst, b"pass".as_slice()).unwrap();

        let conn = rusqlite::Connection::open(&dst).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM knowledge", [], |r| r.get(0))
            .unwrap();
        let text: String = conn
            .query_row("SELECT text FROM knowledge", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(text, "hello world");

        let _ = fs::remove_file(&src);
        let _ = fs::remove_file(&out);
        let _ = fs::remove_file(&dst);
    }

    #[test]
    fn restore_verifies_checksum() {
        let src = tmp_path("src2");
        let out = std::env::temp_dir().join(format!("brain-backup-chk-{}.enc", std::process::id()));
        let dst = tmp_path("dst2");
        let _ = fs::remove_file(&out);
        let _ = fs::remove_file(&dst);
        make_db(&src, "data").unwrap();

        backup(&src, &out, b"pass".as_slice()).unwrap();

        // Tamper the ciphertext (after the created_at header newline).
        let mut ct = fs::read(&out).unwrap();
        let last = ct.len() - 1;
        ct[last] ^= 0xFF;
        fs::write(&out, &ct).unwrap();

        let res = restore(&out, &dst, b"pass".as_slice());
        assert!(res.is_err(), "restore must fail on tampered ciphertext");

        let _ = fs::remove_file(&src);
        let _ = fs::remove_file(&out);
        let _ = fs::remove_file(&dst);
    }

    #[test]
    fn backup_excludes_secret_contents() {
        // Point connector config dir at a temp dir with a secret pem file.
        let cfg = std::env::temp_dir().join(format!("brain-cfg-{}/", std::process::id()));
        fs::create_dir_all(&cfg).unwrap();
        let secret_str = "SUPER_SECRET_PRIVATE_KEY_CONTENTS";
        let pem = cfg.join("github-x.pem");
        let mut f = fs::File::create(&pem).unwrap();
        f.write_all(secret_str.as_bytes()).unwrap();
        drop(f);

        let src = tmp_path("src3");
        let out = std::env::temp_dir().join(format!("brain-backup-sec-{}.enc", std::process::id()));
        let _ = fs::remove_file(&out);
        make_db(&src, "public").unwrap();

        // Pass the temp config dir explicitly — no global env mutation, so the
        // test is safe under parallel execution.
        backup_with_config_dir(&src, &out, b"pass".as_slice(), Some(&cfg)).unwrap();

        // The manifest lists the pem by name (secret=true) but the bundle must
        // not contain the secret bytes (only the DB snapshot is embedded).
        let manifest = verify(&out, b"pass".as_slice()).unwrap();
        let pem_comp = manifest
            .components
            .iter()
            .find(|c| c.name == "github-x.pem")
            .expect("pem recorded in manifest");
        assert!(pem_comp.secret, "pem must be marked secret");

        let full = fs::read(&out).unwrap();
        assert!(
            !full
                .windows(secret_str.len())
                .any(|w| w == secret_str.as_bytes()),
            "secret contents must not appear in the backup file"
        );

        let _ = fs::remove_file(&src);
        let _ = fs::remove_file(&out);
        let _ = fs::remove_dir_all(&cfg);
    }

    #[test]
    fn restore_to_empty_host_reproduces_data() {
        // Plan M3: "restore to an empty host". The destination must not exist
        // beforehand; restore creates it and reproduces the searchable data.
        let src = tmp_path("src4");
        let out =
            std::env::temp_dir().join(format!("brain-backup-empty-{}.enc", std::process::id()));
        let dst = tmp_path("dst4");
        let _ = fs::remove_file(&out);
        let _ = fs::remove_file(&dst);
        assert!(!dst.exists(), "destination must start absent (empty host)");
        make_db(&src, "empty host payload").unwrap();

        backup(&src, &out, b"pass".as_slice()).unwrap();
        restore(&out, &dst, b"pass".as_slice()).unwrap();
        assert!(dst.exists(), "restore must create the DB on an empty host");

        let conn = rusqlite::Connection::open(&dst).unwrap();
        let text: String = conn
            .query_row("SELECT text FROM knowledge", [], |r| r.get(0))
            .unwrap();
        assert_eq!(text, "empty host payload");

        let _ = fs::remove_file(&src);
        let _ = fs::remove_file(&out);
        let _ = fs::remove_file(&dst);
    }

    #[test]
    fn restore_rejects_wrong_passphrase() {
        let src = tmp_path("src5");
        let out = std::env::temp_dir().join(format!("brain-backup-wp-{}.enc", std::process::id()));
        let dst = tmp_path("dst5");
        let _ = fs::remove_file(&out);
        let _ = fs::remove_file(&dst);
        make_db(&src, "secret data").unwrap();

        backup(&src, &out, b"correct".as_slice()).unwrap();
        let res = restore(&out, &dst, b"wrong".as_slice());
        assert!(res.is_err(), "restore must fail with a wrong passphrase");

        let _ = fs::remove_file(&src);
        let _ = fs::remove_file(&out);
        let _ = fs::remove_file(&dst);
    }

    #[test]
    fn backup_manifest_integrity() {
        let src = tmp_path("src6");
        let out = std::env::temp_dir().join(format!("brain-backup-man-{}.enc", std::process::id()));
        let _ = fs::remove_file(&out);
        make_db(&src, "integrity").unwrap();

        backup(&src, &out, b"pass".as_slice()).unwrap();
        let manifest = verify(&out, b"pass".as_slice()).unwrap();
        assert!(!manifest.version.is_empty(), "manifest version must be set");
        let db_comp = manifest
            .components
            .iter()
            .find(|c| c.name == "brain.db")
            .expect("brain.db component present");
        assert!(!db_comp.xxh3.is_empty(), "brain.db xxh3 must be set");
        assert!(db_comp.size > 0, "brain.db size must be > 0");

        let _ = fs::remove_file(&src);
        let _ = fs::remove_file(&out);
    }

    #[test]
    fn corrupt_source_db_backup_errors() {
        // Plan M3: "corrupt DB" resilience at the backup boundary. A non-sqlite
        // source file must make `backup` fail gracefully (no panic).
        let src = tmp_path("src7");
        let out =
            std::env::temp_dir().join(format!("brain-backup-corrupt-{}.enc", std::process::id()));
        let _ = fs::remove_file(&src);
        let _ = fs::remove_file(&out);
        fs::write(&src, b"this is not a sqlite database").unwrap();

        let res = backup(&src, &out, b"pass".as_slice());
        assert!(res.is_err(), "backup of a corrupt DB must error, not panic");

        let _ = fs::remove_file(&src);
        let _ = fs::remove_file(&out);
    }

    #[test]
    fn restore_over_existing_takes_safety_snapshot() {
        // Plan M3: crash-during-sync recovery. Restoring over an EXISTING
        // destination must first take a `.bak` safety snapshot so the operation
        // is reversible, and then succeed with the restored content.
        let src = tmp_path("src8");
        let out =
            std::env::temp_dir().join(format!("brain-backup-snap-{}.enc", std::process::id()));
        let dst = tmp_path("dst8");
        let _ = fs::remove_file(&out);
        let _ = fs::remove_file(&dst);
        let _ = fs::remove_file(dst.with_file_name(format!(
            "{}.bak",
            dst.file_name().unwrap().to_string_lossy()
        )));

        make_db(&src, "restored content").unwrap();
        // Pre-populate the destination with different content.
        make_db(&dst, "original live content").unwrap();
        let dst_bak = dst.with_file_name(format!(
            "{}.bak",
            dst.file_name().unwrap().to_string_lossy()
        ));

        backup(&src, &out, b"pass".as_slice()).unwrap();
        restore(&out, &dst, b"pass".as_slice()).unwrap();

        assert!(dst_bak.exists(), "safety snapshot (.bak) must be created");
        let conn = rusqlite::Connection::open(&dst).unwrap();
        let text: String = conn
            .query_row("SELECT text FROM knowledge", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            text, "restored content",
            "destination now holds restored data"
        );

        let _ = fs::remove_file(&src);
        let _ = fs::remove_file(&out);
        let _ = fs::remove_file(&dst);
        let _ = fs::remove_file(&dst_bak);
    }

    // ── v1.27.17 "Strongbox" (M1/M2/M5): format v2 + snapshot hygiene ───────

    #[test]
    fn v2_backup_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.db");
        let out = dir.path().join("out.enc");
        let dst = dir.path().join("dst.db");
        make_db(&src, "v2 payload").unwrap();

        backup(&src, &out, b"pass".as_slice()).unwrap();
        restore(&out, &dst, b"pass".as_slice()).unwrap();

        let conn = rusqlite::Connection::open(&dst).unwrap();
        let text: String = conn
            .query_row("SELECT text FROM knowledge", [], |r| r.get(0))
            .unwrap();
        assert_eq!(text, "v2 payload");
    }

    #[test]
    fn two_v2_backups_same_second_use_different_nonces() {
        // The audit exploit (F-08): the v1 nonce derived from
        // SHA-256(passphrase || created_at)[..12] — same passphrase + same
        // second = identical nonce across different plaintexts (catastrophic
        // GCM nonce reuse). v2 nonces/salts must come from the RNG, per backup.
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.db");
        let out1 = dir.path().join("a.enc");
        let out2 = dir.path().join("b.enc");
        make_db(&src, "first").unwrap();

        backup(&src, &out1, b"pass".as_slice()).unwrap();
        backup(&src, &out2, b"pass".as_slice()).unwrap();

        let h1 = parse_v2_file(&fs::read(&out1).unwrap()).unwrap().0;
        let h2 = parse_v2_file(&fs::read(&out2).unwrap()).unwrap().0;
        assert_ne!(h1.nonce, h2.nonce, "nonces must be random per backup");
        assert_ne!(h1.salt, h2.salt, "salts must be random per backup");
    }

    #[test]
    fn v1_backup_still_restores_with_warning() {
        // Read compat: a legacy v1 file restores through the v2-aware path
        // (the warn! is logged; restore must succeed).
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.db");
        let out = dir.path().join("v1.enc");
        let dst = dir.path().join("dst.db");
        make_db(&src, "legacy").unwrap();

        backup_v1(&src, &out, b"pass".as_slice()).unwrap();
        assert!(
            !fs::read(&out).unwrap().starts_with(MAGIC),
            "v1 files must keep the legacy layout"
        );
        restore(&out, &dst, b"pass".as_slice()).unwrap();

        let conn = rusqlite::Connection::open(&dst).unwrap();
        let text: String = conn
            .query_row("SELECT text FROM knowledge", [], |r| r.get(0))
            .unwrap();
        assert_eq!(text, "legacy");
    }

    #[test]
    fn wrong_passphrase_fails_authentication() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.db");
        let out = dir.path().join("out.enc");
        let dst = dir.path().join("dst.db");
        make_db(&src, "secret data").unwrap();

        backup(&src, &out, b"correct".as_slice()).unwrap();
        let res = restore(&out, &dst, b"wrong".as_slice());
        assert!(res.is_err(), "v2 restore must fail with a wrong passphrase");
    }

    #[test]
    fn argon2_params_benchmark() {
        // Regression guard, not a UX latency budget. The plan's 2 s target was
        // tuned on the dev laptop; the CI runner is ~2x slower (measured
        // 3.0-4.5 s for 64 MiB / t=3). A wall-clock assert pinned to the
        // laptop made CI flaky. This honours the intent — catching an
        // accidental (order-of-magnitude) `ARGON2_M_COST`/`ARGON2_T_COST`
        // bump — while being tolerant of slow shared runners. Prefer a real
        // latency measurement (`brain bench`) if you need a fidelity target.
        let t0 = std::time::Instant::now();
        kdf_v2(
            b"benchmark passphrase",
            &[7u8; 16],
            ARGON2_M_COST,
            ARGON2_T_COST,
            ARGON2_P_COST,
        )
        .unwrap();
        let elapsed = t0.elapsed();
        assert!(
            elapsed < std::time::Duration::from_secs(10),
            "Argon2id 64MiB/t=3 took {elapsed:?}; re-tune ARGON2_M_COST"
        );
    }

    #[test]
    fn tampered_header_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.db");
        let out = dir.path().join("out.enc");
        let dst = dir.path().join("dst.db");
        make_db(&src, "data").unwrap();
        backup(&src, &out, b"pass".as_slice()).unwrap();

        // Flip two bytes inside the header's `argon2id` value (bytes 10.. are
        // the header JSON; offset 10+6 is the 'a' of "argon2id"). A tampered
        // header must hard-error — unknown KDF, or decrypt failure, both error.
        let mut ct = fs::read(&out).unwrap();
        ct[16] ^= 0xFF;
        ct[17] ^= 0xFF;
        fs::write(&out, &ct).unwrap();

        let res = restore(&out, &dst, b"pass".as_slice());
        assert!(res.is_err(), "tampered v2 header must be rejected");
    }

    #[test]
    fn snapshot_file_is_0600_at_creation() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir = tempfile::tempdir().unwrap();
            let src = dir.path().join("src.db");
            let snap = dir.path().join("snap.db");
            make_db(&src, "data").unwrap();
            snapshot_db(&src, &snap).unwrap();

            let mode = fs::metadata(&snap).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "snapshot must be 0600 at creation");
        }
    }

    #[test]
    fn pre_planted_snapshot_refused() {
        // A planted file/symlink at the snapshot path must abort the backup
        // (create_new), never be clobbered or written through.
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.db");
        let snap = dir.path().join("snap.db");
        make_db(&src, "data").unwrap();
        fs::write(&snap, b"PLANTED").unwrap();

        let res = snapshot_db(&src, &snap);
        assert!(res.is_err(), "pre-planted snapshot path must refuse");
        assert_eq!(
            fs::read(&snap).unwrap(),
            b"PLANTED",
            "planted file must be untouched"
        );
    }

    #[test]
    fn snapshot_removed_on_encrypt_failure() {
        // Inject a failure AFTER snapshot creation (an unreadable connector
        // config dir fails the manifest build) — the guard must remove the
        // plaintext snapshot on that error path.
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.db");
        let out = dir.path().join("out.enc");
        let cfg = dir.path().join("cfg");
        fs::create_dir_all(&cfg).unwrap();
        fs::write(cfg.join("github-x.pem"), b"SECRET").unwrap();
        make_db(&src, "data").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&cfg, fs::Permissions::from_mode(0o000)).unwrap();
        }
        // Order matters: chmod back BEFORE the cleanup assert + drops.
        let res = backup_with_config_dir_and_format(
            &src,
            &out,
            b"pass".as_slice(),
            Some(&cfg),
            BackupFormat::V2,
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&cfg, fs::Permissions::from_mode(0o700)).unwrap();
        }
        assert!(
            res.is_err(),
            "unreadable connector config dir must fail the backup"
        );
        let leftovers: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".brain-snapshot-"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "snapshot must be removed on failure, left: {leftovers:?}"
        );
    }

    #[test]
    fn vacuum_into_escapes_quotes() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.db");
        let quoted = dir.path().join("it's-a-quote.db");
        make_db(&src, "escaping").unwrap();
        let conn = rusqlite::Connection::open(&src).unwrap();

        vacuum_into(&conn, &quoted).unwrap();
        assert!(quoted.exists(), "escaped literal must create the file");
        let c2 = rusqlite::Connection::open(&quoted).unwrap();
        let text: String = c2
            .query_row("SELECT text FROM knowledge", [], |r| r.get(0))
            .unwrap();
        assert_eq!(text, "escaping");
    }

    #[test]
    fn restore_refuses_to_clobber_safety_snapshot() {
        // The .bak is evidence of the pre-restore state; a second restore
        // must fail closed rather than overwrite it.
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.db");
        let out = dir.path().join("out.enc");
        let dst = dir.path().join("dst.db");
        make_db(&src, "data").unwrap();
        make_db(&dst, "original").unwrap();

        backup(&src, &out, b"pass".as_slice()).unwrap();
        restore(&out, &dst, b"pass".as_slice()).unwrap();
        let res = restore(&out, &dst, b"pass".as_slice());
        assert!(res.is_err(), "second restore must fail closed");
        let err = format!("{res:?}");
        assert!(
            err.contains("safety snapshot"),
            "error must name the .bak: {err}"
        );
    }

    #[test]
    fn create_private_file_0600_and_no_clobber() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("p.db");
        create_private_file(&p).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "private file is 0600 at creation");
        }
        // create_new: a second create on the same path must refuse.
        let res = create_private_file(&p);
        assert!(res.is_err(), "create_new must refuse an existing path");
    }
}
