//! append-only audit events.
//!
//! Every audit row stores **identifiers and hashes only** — never raw indexed
//! content, token values, or secret-file contents. The `record` helper is a
//! one-liner callers use at trust boundaries (auth, ingest, webhook verification,
//! reconcile, backup). Hashing is SHA-256 (upgraded from xxh3-64) and is
//! **keyed (HMAC-SHA256) whenever a chain key is installed** — that keying,
//! not the digest width, is what makes a stored `target_hash`/`detail_hash`/
//! `query_hash` derived from low-entropy content non-offline-recoverable: an
//! attacker holding only a DB backup cannot dictionary-attack a value they
//! cannot MAC. Keyless deployments fall back to unkeyed SHA-256, which is
//! pseudonymisation, NOT anonymisation — see [`hash`].
//!
//! Security hardening columns:
//! - `tenant_id` column (default 'global') enables per-tenant scoping at the
//!   SQL layer (`WHERE tenant_id = ?`), so forgetting the param cannot leak
//!   cross-tenant audit rows.
//! - `prev_hash` column implements a tamper-evident hash chain. Each row stores
//!   a link over the prior row's fields. Reads verify the chain;
//!   `verify_chain` returns false on any break. The chain is computed inside
//!   the same tx as the insert, so SQLite's single-writer serializes the
//!   read-modify-write atomically.
//!
//! Two evidence-format layers sit on top of the original chain (both gated
//! on a per-DB **epoch** stamped in `schema_meta`: absent/`legacy` = the
//! historical 5-field SHA-256 link, byte-identical to every chain written
//! before the epoch system existed; `sha7` = the corrected 7-field link;
//! `hmac256` = the re-anchored scheme):
//! - **7-field links:** the `sha7` link is SHA-256 over the prior row's
//!   `(ts, kind, actor, target_hash, status, detail_hash, prev_hash)`,
//!   length-prefixed so no separator an attacker-controlled string could
//!   shift. It closes the gap the 5-field `legacy` link left — `status` and
//!   `detail_hash` were OUTSIDE its preimage, so an audit row's outcome could
//!   be rewritten undetected. It is still **unkeyed**: field coverage is what
//!   it adds, keying is `hmac256`.
//! - **8-field keyed links:** the `hmac256` link is HMAC-SHA256 over
//!   the prior row's `(id, ts, kind, actor, target_hash, status, detail_hash,
//!   prev_hash)` — the full row — so a recomputed chain must reproduce every
//!   field, and a reconstructed chain from attacker-chosen content cannot
//!   verify without the key. The key never lives in the DB it protects
//!   (`BRAIN_AUDIT_CHAIN_KEY` / `BRAIN_AUDIT_CHAIN_KEY_FILE` / a 0600
//!   `audit-chain.key` beside the DB; see [`init_chain_key`]).
//! - **Head pin:** `schema_meta.audit_chain_head` pins `(id, hash, epoch)`
//!   of the chain head on every commit; `verify_chain` compares it against
//!   the recomputed head, so truncation/extension of an otherwise-valid chain
//!   is detected, and `backup::restore` compares pre/post pins to disclose a
//!   rolled-back chain.
//!
//! Which format a DB uses is decided ONCE, by its first audit row, and then
//! frozen in the stamp:
//! - An **unstamped DB that already has rows** is a pre-`sha7` chain: it stays
//!   `legacy`, byte-for-byte, forever. Its links are recomputed exactly as they
//!   always were, so **upgrading the binary never makes an existing chain fail
//!   `verify_chain`** — the single most dangerous outcome of a chain-format
//!   change on evidence an operator is regulated on.
//! - An **unstamped DB with no rows** has no format to preserve, so the first
//!   append stamps `sha7` in the same tx ([`record_tenant_checked`]). New
//!   evidence is never written in the weaker form again.
//! - [`bootstrap_epoch`] still takes a row-less DB straight to `hmac256` when a
//!   chain key is available, so production boot reaches the keyed epoch first.
//!
//! An existing chain is NEVER silently converted — the format flips only via
//! the offline `brain-server --re-audit` re-anchor (see [`reanchor_to_hmac`]).
//!
//! Schema (additive migration in `main.rs::run_migration`):
//! ```sql
//! CREATE TABLE IF NOT EXISTS audit_events(
//!   id INTEGER PRIMARY KEY AUTOINCREMENT,
//!   ts TEXT DEFAULT CURRENT_TIMESTAMP,
//!   kind TEXT NOT NULL,     -- 'auth'|'ingest'|'webhook'|'reconcile'|'backup'|'connector'|'forget'|'procedure'|...
//!   actor TEXT,             -- connector kind/instance, 'api', or 'loopback'
//!   target_hash TEXT,       -- SHA-256 of the affected uri/id (NOT the content)
//!   status TEXT,            -- 'ok'|'denied'|'error'
//!   detail_hash TEXT,       -- SHA-256 of a short detail string (no secrets)
//!   tenant_id TEXT NOT NULL DEFAULT 'global',  -- per-tenant scoping
//!   prev_hash TEXT          -- tamper-evidence chain link
//! );
//! ```

use hmac::{Hmac, KeyInit, Mac};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Lowercase-hex encode any hash output (sha2 0.11 dropped `LowerHex` on its array type).
pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub mod decision;
pub use decision::{DecisionInput, DecisionRecord};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tracing::error;

/// Audit event categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditKind {
    Auth,
    Ingest,
    Webhook,
    Reconcile,
    Backup,
    Connector,
    /// read-event kinds — a `/recall`, `/search`,
    /// `/get`, or `/multi-get` that injected memory into a decision path.
    /// Recorded only when the read-event audit is enabled (JWT mode default).
    Recall,
    Search,
    Get,
    /// breach workflow events (open/notification/
    /// close) — the DPO incident ledger mirrors the hash chain.
    Breach,
    /// transfer-register writes (Art 30/Art 46
    /// evidence) — every recorded cross-border flow is hash-chained.
    Transfer,
    /// the BPO operating-register lifecycle (register, and
    /// later onboard/dpa/dsar/hold/termination writes) — every client-level
    /// action is hash-chained.
    Client,
    /// the edge-history surface (`GET /graph/relationships/{id}/history`).
    /// A read of the supersession lineage (retired versions carry entity names
    /// that a "current belief" read would redact) is itself evidence; the read
    /// is invokable by a Read-granted principal but the ROW level is Admin.
    GraphRead,
    /// audit-retention prune events: the deletion of
    /// audit evidence must itself be evidenced — a prune writes one row
    /// recording the cutoff + pruned count.
    Retention,
    /// governed-workflow writes: every
    /// workflow run/step/outbox/finding/contradiction mutation hash-chains a
    /// row so the evidence tables are derivable from the audit, never the
    /// other way (the `AuditKind::Breach` precedent).
    Workflow,
    /// chain-format re-anchor events: the
    /// `--re-audit` epoch flip writes one row on the NEW chain recording
    /// the scheme change, so the evidence epoch boundary is itself evidence.
    Anchor,
    /// per-decision evidence rows (Art. 12): every
    /// `decision_records` append also extends THIS chain, so the decision
    /// ledger inherits the audit chain's tamper-evidence (extended, never a
    /// separate trust root).
    Decision,
    /// erasure evidence: chunk-forget rows and
    /// forget-driven proposal-scrub rows. A dedicated kind (not `Ingest`)
    /// so kind-filtered audit consumers can find erasures without
    /// detail-hash mining.
    Forget,
    /// stored-procedure writes (root + ordered step chunks): one row per
    /// procedure write, target = the hashed root chunk id, detail names the
    /// step count. Caller content got no evidence row before this kind
    /// existed — the audit-per-write law applies to every write path.
    Procedure,
    /// chat-session lifecycle (the chat spine's durable store): lazy
    /// session create, turn-pair persistence, rename, erase.
    /// A dedicated kind (the `Forget`/`Procedure` precedent) so kind-filtered
    /// audit consumers can find transcript writes without detail-hash mining;
    /// the chain stores hashes of target + detail, so no session key and no
    /// turn text rides the chain raw.
    Chat,
    /// governance-profile rule rows (the `rules` tables' ONE door — the CLI
    /// door closing C1's `the_rules_insert_door_is_missing`): every insert and
    /// revision through `brain rules` or the pack installer hashes one row.
    /// A dedicated kind (the `Procedure` precedent) so kind-filtered
    /// consumers find jurisdiction-law writes without detail-hash mining;
    /// the body rides the chain as a hash only.
    Rules,
    /// pack lifecycle (plan 28 §4): one row per pack IMPORT act —
    /// the signed chain's terminal witness (verify → validate → import,
    /// each per-row act audited under ITS kind; this row names the pack:
    /// id, version, hashes in the detail). The register row
    /// (`installed_packs`) carries the plaintext id/version; this row is
    /// the chain's evidence that the import HAPPENED.
    Plugin,
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
            AuditKind::Recall => "recall",
            AuditKind::Search => "search",
            AuditKind::Get => "get",
            AuditKind::Breach => "breach",
            AuditKind::Transfer => "transfer",
            AuditKind::Client => "client",
            AuditKind::GraphRead => "graphread",
            AuditKind::Retention => "retention",
            AuditKind::Workflow => "workflow",
            AuditKind::Anchor => "anchor",
            AuditKind::Decision => "decision",
            AuditKind::Forget => "forget",
            AuditKind::Procedure => "procedure",
            AuditKind::Chat => "chat",
            AuditKind::Rules => "rules",
            AuditKind::Plugin => "plugin",
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
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            AuditStatus::Ok => "ok",
            AuditStatus::Denied => "denied",
            AuditStatus::Error => "error",
        }
    }
}

/// Hash an identifier/detail string. Used so the audit log never stores the
/// raw value (content, token, uri-with-secret, etc.). The value fed in may
/// itself be a pre-computed digest.
///
/// **Keyed when a chain key is installed** (the `init_chain_key` ladder: env →
/// 0600 key file → generated). Under a key this is HMAC-SHA256 over
/// `brain-audit-hash|<value>` — domain-separated from the chain-link and pin
/// MACs that share the key — so a stored digest of low-entropy content cannot
/// be dictionary-attacked by an attacker holding only a DB backup: they cannot
/// compute candidate MACs without the key. This is what makes the
/// "not offline-recoverable" claim in the module doc true.
///
/// **Unkeyed when no chain key is installed:** the historical plain SHA-256,
/// byte-identical to every digest written before this change, so keyless and
/// legacy deployments keep working unchanged. That form IS offline
/// dictionary-recoverable — a keyless deployment is pseudonymised, not
/// anonymised, and the module doc no longer claims otherwise.
pub fn hash(s: &str) -> String {
    match chain_key() {
        Some(key) => {
            let mut mac = Hmac::<Sha256>::new_from_slice(key.as_ref())
                .expect("HMAC accepts keys of any length");
            mac.update(b"brain-audit-hash|");
            mac.update(s.as_bytes());
            hex_encode(&mac.finalize().into_bytes())
        }
        None => hex_encode(&Sha256::digest(s.as_bytes())),
    }
}

/// SHA-256 hex digest of the chain-link payload. The payload is the
/// concatenation of the prior row's `(ts, kind, actor, target_hash, prev_hash)`
/// — every field a tamperer would need to touch to rewrite history. `id` is
/// deliberately excluded so a renumbered restore keeps the chain intact.
fn chain_link(ts: &str, kind: &str, actor: &str, target_hash: &str, prev_hash: &str) -> String {
    let mut h = Sha256::new();
    h.update(ts.as_bytes());
    h.update(b"|");
    h.update(kind.as_bytes());
    h.update(b"|");
    h.update(actor.as_bytes());
    h.update(b"|");
    h.update(target_hash.as_bytes());
    h.update(b"|");
    h.update(prev_hash.as_bytes());
    hex_encode(&h.finalize())
}

// ── chain epochs, keyed links, head pin ──────────────────────────────
//
// The chain's FORMAT is per-DB state stamped in `schema_meta`: the epoch.
// `legacy` (or absent) keeps the byte-identical 5-field SHA-256 link every
// release before 1.27.31 wrote; `sha7` is the corrected 7-field link (the
// first append to an unstamped, row-less DB stamps it); `hmac256` is the
// re-anchored 8-field HMAC-SHA256 link. Nothing flips an existing chain
// implicitly — only [`reanchor_to_hmac`] (the offline `--re-audit`) or
// [`bootstrap_epoch`] (row-less fresh DB, key available) writes the stamp
// for a DB that already has a format to keep.

/// `schema_meta` key holding the chain epoch (`legacy` | `sha7` | `hmac256`).
pub const EPOCH_META_KEY: &str = "audit_chain_epoch";
/// `schema_meta` key holding the head pin (JSON [`HeadPin`]).
pub const HEAD_PIN_META_KEY: &str = "audit_chain_head";
/// Epoch value: the historical 5-field SHA-256 link (default when unstamped).
pub const EPOCH_LEGACY: &str = "legacy";
/// Epoch value: unkeyed SHA-256 over all seven link-committed fields — the
/// corrected link. Still unkeyed: what it adds over `legacy` is field
/// COVERAGE (`status` and `detail_hash` are inside the preimage), not a key.
/// A keyless deployment that can be re-anchored should be, to `hmac256`.
pub const EPOCH_SHA7: &str = "sha7";
/// Epoch value: HMAC-SHA256 over the full 8-field row.
pub const EPOCH_HMAC: &str = "hmac256";

/// The chain epoch a DB is currently written under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainEpoch {
    /// 5-field SHA-256 links — byte-identical to pre-1.27.31 chains.
    Legacy,
    /// 7-field SHA-256 links — [`chain_link_sha7`], the corrected link.
    Sha7,
    /// 8-field HMAC-SHA256 links — requires [`chain_key`].
    Hmac256,
}

/// Read the DB's chain epoch. Tolerant by design: a missing `schema_meta`
/// table, a missing key, or an unrecognized value all read as `Legacy` (the
/// historical default — every chain in existence before 1.27.31 is legacy,
/// so absence IS the legacy answer, and a chain that HAS rows must never be
/// reinterpreted under a link format it was not written with). An
/// unrecognized non-empty value is warned about: it can only come from
/// tampering or a newer binary.
fn read_epoch(conn: &Connection) -> ChainEpoch {
    let v: Option<String> = conn
        .query_row(
            &format!("SELECT value FROM schema_meta WHERE key = '{EPOCH_META_KEY}'"),
            [],
            |r| r.get(0),
        )
        .ok();
    match v.as_deref() {
        Some(EPOCH_HMAC) => ChainEpoch::Hmac256,
        Some(EPOCH_SHA7) => ChainEpoch::Sha7,
        Some(EPOCH_LEGACY) | None => ChainEpoch::Legacy,
        Some(other) => {
            tracing::warn!("audit chain epoch '{other}' unrecognized — reading as legacy");
            ChainEpoch::Legacy
        }
    }
}

/// The link scheme a chain is computed under.
#[derive(Clone)]
enum Scheme {
    /// Historical 5-field SHA-256.
    Legacy,
    /// 7-field SHA-256 — the corrected unkeyed link.
    Sha7,
    /// 8-field HMAC-SHA256 keyed by the process chain key.
    Hmac(Arc<[u8; 32]>),
}

/// The epoch string a scheme corresponds to (pin stamping).
fn scheme_epoch(scheme: &Scheme) -> &'static str {
    match scheme {
        Scheme::Legacy => EPOCH_LEGACY,
        Scheme::Sha7 => EPOCH_SHA7,
        Scheme::Hmac(_) => EPOCH_HMAC,
    }
}

/// Resolve the DB's epoch into a computable scheme. `None` = the DB is on
/// `hmac256` but no key is available in this process — callers fail closed
/// (verify returns false, writes/prunes are refused): a keyed chain cannot
/// be attested or extended without its key.
fn current_scheme(conn: &Connection) -> Option<Scheme> {
    match read_epoch(conn) {
        ChainEpoch::Legacy => Some(Scheme::Legacy),
        ChainEpoch::Sha7 => Some(Scheme::Sha7),
        ChainEpoch::Hmac256 => chain_key().map(Scheme::Hmac),
    }
}

/// Every field a `hmac256` link commits (the full row — id, ts, kind,
/// actor, target_hash, status, detail_hash, prev_hash). `prev_hash` is
/// `Option` because leading NULL backrefs (legacy prefix / genesis) carry no
/// link input (NULL ≡ "" in the link, same as the legacy computation).
#[derive(Debug, Clone)]
struct ChainRowFull {
    id: i64,
    ts: String,
    kind: String,
    actor: String,
    target_hash: String,
    status: String,
    detail_hash: String,
    prev_hash: Option<String>,
}

/// The link for `row` under `scheme` — the value the NEXT row stores in its
/// `prev_hash`, and the value the head pin pins.
fn row_link(scheme: &Scheme, row: &ChainRowFull) -> String {
    match scheme {
        Scheme::Legacy => chain_link(
            &row.ts,
            &row.kind,
            &row.actor,
            &row.target_hash,
            row.prev_hash.as_deref().unwrap_or(""),
        ),
        Scheme::Sha7 => chain_link_sha7(row),
        Scheme::Hmac(key) => chain_link_hmac(key.as_ref(), row),
    }
}

/// SHA-256 over the seven link-committed fields — the `sha7` epoch's link, and
/// the correction to [`chain_link`]: that one leaves `status` and
/// `detail_hash` outside the preimage, so a legacy chain's audit OUTCOME can
/// be rewritten with the chain still verifying.
///
/// Length-prefixed exactly like [`chain_link_hmac`] (u64 LE byte-length before
/// each field) so no separator an attacker-controlled `actor`/`kind` string
/// could shift. `id` stays excluded for the same reason [`chain_link`]
/// excludes it: a renumbered restore keeps its chain.
fn chain_link_sha7(row: &ChainRowFull) -> String {
    let mut h = Sha256::new();
    for field in [
        row.ts.as_bytes(),
        row.kind.as_bytes(),
        row.actor.as_bytes(),
        row.target_hash.as_bytes(),
        row.status.as_bytes(),
        row.detail_hash.as_bytes(),
        row.prev_hash.as_deref().unwrap_or("").as_bytes(),
    ] {
        h.update((field.len() as u64).to_le_bytes());
        h.update(field);
    }
    hex_encode(&h.finalize())
}

/// HMAC-SHA256 over the FULL row. Length-prefixed fields (u64 LE
/// byte-length before each field) make the input unambiguous — no separator
/// an attacker-controlled `actor`/`kind` string could shift. `id` is included
/// as raw LE bytes so a renumbered restore cannot keep its links.
fn chain_link_hmac(key: &[u8], row: &ChainRowFull) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts keys of any length");
    mac.update(&row.id.to_le_bytes());
    for field in [
        row.ts.as_bytes(),
        row.kind.as_bytes(),
        row.actor.as_bytes(),
        row.target_hash.as_bytes(),
        row.status.as_bytes(),
        row.detail_hash.as_bytes(),
        row.prev_hash.as_deref().unwrap_or("").as_bytes(),
    ] {
        mac.update(&(field.len() as u64).to_le_bytes());
        mac.update(field);
    }
    hex_encode(&mac.finalize().into_bytes())
}

// ── the chain key ─────────────────────────────────────────────────────

/// Process-wide chain key. Set once at binary boot via [`init_chain_key`]
/// (server `main_inner`, `brain` CLI) — never env-sniffed lazily, so unit
/// tests and library consumers stay deterministic-legacy unless they
/// explicitly opt in.
///
/// Lock bounds (Headroom): the critical sections are one `Option` replace
/// (boot write) and one `Arc` clone (read). No I/O, no nesting, no SQL —
/// the env parse + key-file read/persist all happen BEFORE the write.
/// Poison: fail-open at the lock (skip / `None`) — but `None` makes hmac256
/// writes FAIL CLOSED downstream (documented posture), so the posture is
/// preserved. Request-path holder (the per-write HMAC settle reads this),
/// so acquires are wait-measured.
static CHAIN_KEY: RwLock<Option<Arc<[u8; 32]>>> = RwLock::new(None);

/// Failures of [`init_chain_key`] — all refuse the key (the caller decides
/// whether that is fatal; writes to an `hmac256` DB fail closed without it).
#[derive(Debug)]
pub enum ChainKeyError {
    /// `BRAIN_AUDIT_CHAIN_KEY` is not 64 hex chars.
    InvalidHex,
    /// The key file exists but cannot be read.
    Unreadable(String),
    /// The key file is group/world-readable (the auth-secret posture).
    BadPermissions(u32),
    /// A new key could not be persisted.
    WriteFailed(String),
}

impl std::fmt::Display for ChainKeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChainKeyError::InvalidHex => {
                write!(f, "BRAIN_AUDIT_CHAIN_KEY must be 64 hex chars (32 bytes)")
            }
            ChainKeyError::Unreadable(e) => write!(f, "audit chain key file unreadable: {e}"),
            ChainKeyError::BadPermissions(mode) => write!(
                f,
                "audit chain key file is group/world-readable (mode {mode:o}) — chmod 600 it"
            ),
            ChainKeyError::WriteFailed(e) => write!(f, "audit chain key could not be written: {e}"),
        }
    }
}

impl std::error::Error for ChainKeyError {}

/// Resolve + install the process chain key. Resolution order:
/// 1. `BRAIN_AUDIT_CHAIN_KEY` (64 hex chars) — secret-manager deployments.
/// 2. `BRAIN_AUDIT_CHAIN_KEY_FILE` — a 0600 file of 64 hex chars.
/// 3. `<default_dir>/audit-chain.key` — read if present, otherwise generated
///    (32 random bytes) and written 0600 create-new.
///
/// The key DELIBERATELY never lives inside a DB it protects: an attacker who
/// can rewrite `audit_events` (the threat the keyed chain exists for) cannot
/// read the key from it. Losing the key makes an `hmac256` chain unverifiable
/// — by design; the pre-anchor backup stays readable under `legacy`.
pub fn init_chain_key(default_dir: &Path) -> Result<(), ChainKeyError> {
    let key: [u8; 32] = if let Some(hex) = std::env::var("BRAIN_AUDIT_CHAIN_KEY")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        parse_key_hex(&hex)?
    } else if let Some(path) = std::env::var("BRAIN_AUDIT_CHAIN_KEY_FILE")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
    {
        read_key_file(&path)?
    } else {
        let path = default_dir.join("audit-chain.key");
        if path.exists() {
            read_key_file(&path)?
        } else {
            let key = generate_key();
            write_key_file(&path, &key)?;
            tracing::info!("generated audit chain key at {:?}", path);
            key
        }
    };
    if let Ok(mut slot) = crate::concurrency::rwlock_write_measured(&CHAIN_KEY) {
        *slot = Some(Arc::new(key));
    }
    Ok(())
}

/// The installed process chain key, if any.
pub fn chain_key() -> Option<Arc<[u8; 32]>> {
    crate::concurrency::rwlock_read_measured(&CHAIN_KEY)
        .ok()
        .and_then(|k| k.clone())
}

fn parse_key_hex(s: &str) -> Result<[u8; 32], ChainKeyError> {
    let bytes = hex::decode(s).map_err(|_| ChainKeyError::InvalidHex)?;
    bytes.try_into().map_err(|_| ChainKeyError::InvalidHex)
}

fn read_key_file(path: &Path) -> Result<[u8; 32], ChainKeyError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path)
            .map_err(|e| ChainKeyError::Unreadable(e.to_string()))?
            .permissions()
            .mode()
            & 0o777;
        if mode & 0o077 != 0 {
            return Err(ChainKeyError::BadPermissions(mode));
        }
    }
    let raw =
        std::fs::read_to_string(path).map_err(|e| ChainKeyError::Unreadable(e.to_string()))?;
    parse_key_hex(raw.trim())
}

fn generate_key() -> [u8; 32] {
    // rand 0.10: the OS entropy source is `rngs::SysRng` (fallible — it can
    // only fail if the OS entropy source itself is broken, which aborts the
    // boot rather than minting a weak chain key).
    use rand::{TryRng, rngs::SysRng};
    let mut key = [0u8; 32];
    SysRng
        .try_fill_bytes(&mut key)
        .expect("OS entropy source failed while generating the audit chain key");
    key
}

fn write_key_file(path: &Path, key: &[u8; 32]) -> Result<(), ChainKeyError> {
    // create_new + 0600 in one open (no umask window, no write-through of a
    // pre-planted file/symlink) — the backup module's primitive owns the path;
    // the write itself goes through a reopen (perms are already 0600).
    crate::backup::create_private_file(path)
        .map_err(|e| ChainKeyError::WriteFailed(e.to_string()))?;
    let body = format!("{}\n", hex::encode(key));
    std::fs::write(path, body).map_err(|e| ChainKeyError::WriteFailed(e.to_string()))?;
    std::fs::File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(|e| ChainKeyError::WriteFailed(e.to_string()))?;
    Ok(())
}

// ── the chain pins (head + genesis) ──────────────────────────────────

/// `schema_meta` key holding the GENESIS pin: `(id, hash, epoch)` of the
/// chain's FIRST row. The head pin anchors the tip (tail-truncation
/// detection); the genesis pin anchors the origin (head-deletion
/// detection — removing the oldest rows changes which row is first, and
/// the pin's `id` disagrees). Both pins are MAC-protected under the
/// `hmac256` epoch (see [`HeadPin::mac`]) so an attacker with SQL write
/// access cannot rewrite them to bless a truncated chain: the MAC key
/// lives outside the DB, beside the chain key it already cannot read.
pub const GENESIS_PIN_META_KEY: &str = "audit_chain_genesis";

/// The pinned chain head: `(id, hash, epoch)` of the last committed row's
/// link. Written in the same tx as every audit row ([`record_tenant`]),
/// re-written by the retention prune's re-anchor, and compared by
/// [`verify_chain`] (truncation/extension detection) and `backup::restore`
/// (rollback disclosure).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeadPin {
    /// Row id of the pinned row.
    pub id: i64,
    /// Link hash of the pinned row (the value the next row chains from —
    /// or, for the genesis pin, the link that commits the first row's full
    /// content).
    pub hash: String,
    /// Epoch the pin was computed under (`legacy` | `hmac256`).
    pub epoch: String,
    /// HMAC-SHA256 over `"brain-audit-pin|{epoch}|{id}|{hash}"` with the
    /// process chain key. Present exactly when the epoch is `hmac256`:
    /// pins are state an attacker with SQL write access could otherwise
    /// freely rewrite (they live in the same DB as the rows they guard),
    /// and the MAC moves forging them behind the key the chain already
    /// keeps outside the DB. `None` on legacy-epoch pins (no key exists to
    /// MAC with — the legacy posture is unchanged and documented).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac: Option<String>,
}

impl HeadPin {
    /// The pin a row would pin under `scheme` (MAC included when keyed).
    fn for_row(scheme: &Scheme, row: &ChainRowFull) -> Self {
        let id = row.id;
        let hash = row_link(scheme, row);
        let epoch = scheme_epoch(scheme).to_string();
        let mac = match scheme {
            Scheme::Legacy | Scheme::Sha7 => None,
            Scheme::Hmac(key) => Some(pin_mac(key.as_ref(), &epoch, id, &hash)),
        };
        Self {
            id,
            hash,
            epoch,
            mac,
        }
    }

    /// Validate this pin's MAC under `key`. `false` when the MAC is absent
    /// (a keyed-epoch pin must carry one) or does not verify.
    fn mac_is_valid(&self, key: &[u8; 32]) -> bool {
        self.mac.as_deref().is_some_and(|got| {
            constant_time_eq_hex(got, &pin_mac(key, &self.epoch, self.id, &self.hash))
        })
    }
}

/// Pin MAC input: domain-separated, unambiguous (fixed field order, `|`
/// cannot appear in epoch/id/hash — epoch is a closed vocabulary, id is an
/// integer, hash is hex).
fn pin_mac(key: &[u8; 32], epoch: &str, id: i64, hash: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts keys of any length");
    mac.update(b"brain-audit-pin|");
    mac.update(epoch.as_bytes());
    mac.update(b"|");
    mac.update(&id.to_le_bytes());
    mac.update(b"|");
    mac.update(hash.as_bytes());
    hex_encode(&mac.finalize().into_bytes())
}

/// Hex-string equality without early exit on length or prefix (the compared
/// values are attacker-influenced pin fields; the MAC itself is the secret
/// check, so this is belt-and-braces, not the load-bearing constant time).
fn constant_time_eq_hex(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Read the head pin. `None` when unstamped (fresh DB, pre-1.27.31 DB the
/// migration has not pinned, or `schema_meta` absent). A present-but-corrupt
/// value warns and reads as `None` — the pin is a detector, not evidence
/// itself; the walk in [`verify_chain`] remains the authority.
pub fn read_head_pin(conn: &Connection) -> Option<HeadPin> {
    read_pin(conn, HEAD_PIN_META_KEY)
}

/// Read the genesis pin (same semantics as [`read_head_pin`]).
pub fn read_genesis_pin(conn: &Connection) -> Option<HeadPin> {
    read_pin(conn, GENESIS_PIN_META_KEY)
}

fn read_pin(conn: &Connection, key: &str) -> Option<HeadPin> {
    let raw: Option<String> = conn
        .query_row(
            &format!("SELECT value FROM schema_meta WHERE key = '{key}'"),
            [],
            |r| r.get(0),
        )
        .ok();
    let raw = raw?;
    match serde_json::from_str(&raw) {
        Ok(pin) => Some(pin),
        Err(e) => {
            tracing::warn!("audit pin '{key}' unparseable ({e}) — treating as absent");
            None
        }
    }
}

/// Persist a pin under `key`. Creates `schema_meta` when absent (unit-test
/// DBs and pre-v0.9 fixtures) so the pin never silently skips. Best-effort
/// at the call sites that cannot fail their primary action (the audit row
/// itself); callers inside a tx get atomicity for free.
fn write_pin(conn: &Connection, key: &str, pin: &HeadPin) -> bool {
    let Ok(json) = serde_json::to_string(pin) else {
        return false;
    };
    conn.execute_batch("CREATE TABLE IF NOT EXISTS schema_meta(key TEXT PRIMARY KEY, value TEXT);")
        .is_ok()
        && conn
            .execute(
                &format!(
                    "INSERT INTO schema_meta(key, value) VALUES ('{key}', ?1)
                     ON CONFLICT(key) DO UPDATE SET value = ?1"
                ),
                params![json],
            )
            .is_ok()
}

/// Persist the head pin (see [`write_pin`]).
fn write_head_pin(conn: &Connection, pin: &HeadPin) -> bool {
    write_pin(conn, HEAD_PIN_META_KEY, pin)
}

/// Persist the genesis pin (see [`write_pin`]).
fn write_genesis_pin(conn: &Connection, pin: &HeadPin) -> bool {
    write_pin(conn, GENESIS_PIN_META_KEY, pin)
}

fn delete_pin(conn: &Connection, key: &str) {
    let _ = conn.execute(&format!("DELETE FROM schema_meta WHERE key = '{key}'"), []);
}

/// Persist the chain epoch. Creates `schema_meta` when absent (unit-test DBs
/// and pre-v0.9 fixtures), like [`write_pin`]. Called ONLY from the first
/// append to a row-less chain (inside the audit tx), which is the one moment a
/// DB has no format to preserve: from then on the stamp is the format, and no
/// code path rewrites it for a chain that already has rows except the
/// explicit [`reanchor_to_hmac`].
fn write_epoch_stamp(conn: &Connection, epoch: &str) -> bool {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS schema_meta(key TEXT PRIMARY KEY, value TEXT);")
        .is_ok()
        && conn
            .execute(
                &format!(
                    "INSERT INTO schema_meta(key, value) VALUES ('{EPOCH_META_KEY}', ?1)
                     ON CONFLICT(key) DO UPDATE SET value = ?1"
                ),
                params![epoch],
            )
            .is_ok()
}

/// The legacy-scheme pin for a DB's CURRENT tip — used by the
/// migration to stamp the initial pin on upgrade (an existing chain is
/// pre-re-anchor by definition). `None` when the chain is empty or unreadable.
pub fn initial_head_pin(conn: &Connection) -> Option<HeadPin> {
    tip_row(conn).map(|tip| HeadPin::for_row(&Scheme::Legacy, &tip))
}

/// The legacy-scheme pin for a DB's CURRENT first row — the genesis
/// counterpart of [`initial_head_pin`], used by the same migration stamp.
/// `None` when the chain is empty or unreadable.
pub fn initial_genesis_pin(conn: &Connection) -> Option<HeadPin> {
    first_row(conn).map(|first| HeadPin::for_row(&Scheme::Legacy, &first))
}

/// The current tip as a full row (the fields a link commits).
fn tip_row(conn: &Connection) -> Option<ChainRowFull> {
    conn.query_row(
        "SELECT id, ts, kind, actor, target_hash, status, detail_hash, prev_hash \
          FROM audit_events ORDER BY id DESC LIMIT 1",
        [],
        map_full_row,
    )
    .ok()
}

/// The chain's FIRST row as a full row — the genesis pin's subject.
fn first_row(conn: &Connection) -> Option<ChainRowFull> {
    conn.query_row(
        "SELECT id, ts, kind, actor, target_hash, status, detail_hash, prev_hash \
          FROM audit_events ORDER BY id ASC LIMIT 1",
        [],
        map_full_row,
    )
    .ok()
}

fn map_full_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ChainRowFull> {
    Ok(ChainRowFull {
        id: r.get(0)?,
        ts: r.get(1)?,
        kind: r.get(2)?,
        actor: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
        target_hash: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
        status: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
        detail_hash: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
        prev_hash: r.get(7)?,
    })
}

/// Recompute + persist BOTH pins for the current chain extent under the
/// DB's own epoch — the head from the tip, the genesis from the first row.
/// Returns the refreshed head pin. Used by paths that legitimately rewrite
/// the chain shape in-tx (retention prune, re-anchor): the prune deletes
/// the oldest rows, so the genesis must move with it or the very next
/// verify would (correctly) report a head mismatch.
fn refresh_head_pin(conn: &Connection, scheme: &Scheme) -> Option<HeadPin> {
    match tip_row(conn) {
        Some(tip) => {
            let pin = HeadPin::for_row(scheme, &tip);
            write_head_pin(conn, &pin);
            if let Some(first) = first_row(conn) {
                write_genesis_pin(conn, &HeadPin::for_row(scheme, &first));
            }
            Some(pin)
        }
        None => {
            // Empty chain carries no pins (the first write re-pins).
            delete_pin(conn, HEAD_PIN_META_KEY);
            delete_pin(conn, GENESIS_PIN_META_KEY);
            None
        }
    }
}

/// How a restore changed the pinned head. Pure — `backup::restore`
/// logs/audits from this, tests pin it.
#[derive(Debug, Clone, PartialEq)]
pub enum HeadComparison {
    /// The live DB had no pin (fresh, or pre-1.27.31 unpinned).
    NoPrePin,
    /// The restored DB has no pin (a pre-1.27.31 backup, or an empty chain).
    NoPostPin,
    /// Same id + hash — the restore landed on the same chain position.
    Match,
    /// The restored chain is OLDER (pre-id < post-id is false and ids differ
    /// downward) — the restore rewound evidence; the pre-restore `.bak`
    /// retains the newer chain.
    RolledBack { pre_id: i64, post_id: i64 },
    /// A different head at or beyond the pre-pin (a newer backup restored, or
    /// a divergent chain) — still a change of the evidence position.
    Diverged { pre_id: i64, post_id: i64 },
}

/// Compare pre-restore vs post-restore head pins. Rolled-back vs diverged is
/// decided by id ORDER (the monotonic rowid): a restored head with a smaller
/// id rewound the chain; an equal-or-larger id with a different hash is a
/// divergence (side-ways or forward), disclosed but not classified as a
/// rollback.
pub fn classify_restored_head(pre: Option<&HeadPin>, post: Option<&HeadPin>) -> HeadComparison {
    use HeadComparison::*;
    match (pre, post) {
        (None, _) => NoPrePin,
        (Some(_), None) => NoPostPin,
        (Some(pre), Some(post)) => {
            if pre.id == post.id && pre.hash == post.hash {
                Match
            } else if post.id < pre.id {
                RolledBack {
                    pre_id: pre.id,
                    post_id: post.id,
                }
            } else {
                Diverged {
                    pre_id: pre.id,
                    post_id: post.id,
                }
            }
        }
    }
}

/// Default tenant id for rows written before the tenant column existed and for
/// callers that don't
/// track tenancy. Kept as a constant so every defaulting site uses the same
/// spelling; the migration's `DEFAULT 'global'` matches it byte-for-byte.
pub const DEFAULT_TENANT: &str = "global";

/// Append one audit event. Best-effort: audit must never fail the primary
/// action, so errors are swallowed (logged at debug). Callers pass already-
/// hashed identifiers where the value is sensitive. `tenant` scopes the row;
/// pass [`DEFAULT_TENANT`] when the caller has no tenant context.
///
/// Returns the inserted row id (`Some`) or `None` if the write failed. Most
/// callers ignore the return; the DSAR/trace paths use it to key a replayable
/// trace row.
///
/// The hash-chain link is read + written inside a single transaction so the
/// read-modify-write is atomic under SQLite's single-writer lock.
pub fn record(
    conn: &Connection,
    kind: AuditKind,
    actor: &str,
    target: &str,
    status: AuditStatus,
    detail: &str,
) -> Option<i64> {
    record_tenant(conn, kind, actor, target, status, detail, DEFAULT_TENANT)
}

/// Per-tenant variant of [`record`]. Same best-effort semantics; returns the
/// inserted row id on success, `None` on failure.
///
/// The chain-tip read + INSERT must be atomic so concurrent writers can't
/// both read the same tip and fork the chain. The right transaction kind
/// depends on whether the caller already holds a transaction:
///
/// - **Autocommit caller** (the majority — e.g. `approve_proposal` commits
///   its own tx, *then* calls `audit::record` on a fresh autocommit
///   connection): use `BEGIN IMMEDIATE` so the read-modify-write serializes
///   at `BEGIN`. SQLite's single-writer rule guarantees the second writer
///   blocks until the first commits, then re-reads the fresh tip. This is
///   the fix for the chain-fork race (the earlier SAVEPOINT fix only
///   covered the inside-caller-tx case; on an autocommit caller SAVEPOINT
///   is equivalent to `BEGIN DEFERRED`, which does NOT serialize readers).
/// - **Inside a caller's transaction** (`delete_quarantine` etc.): use a
///   `SAVEPOINT` (a `BEGIN` would error "cannot start a transaction within
///   a transaction"). The outer tx already holds the write lock, so the
///   read-modify-write is serialized by it.
///
/// Legacy callers receive `None` on failure and decide their own failure
/// posture. Missing evidence need not produce a detectable chain gap.
pub fn record_tenant(
    conn: &Connection,
    kind: AuditKind,
    actor: &str,
    target: &str,
    status: AuditStatus,
    detail: &str,
    tenant: &str,
) -> Option<i64> {
    record_tenant_checked(conn, kind, actor, target, status, detail, tenant).ok()
}

/// Fixed stages contain no backend text. `Indeterminate` requires connection
/// quarantine and reconciliation, not an automatic retry of this audit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditWriteError {
    Begin,
    Epoch,
    Key,
    Tip,
    Insert,
    Timestamp,
    Pin,
    Finalize,
    Indeterminate,
}

impl std::fmt::Display for AuditWriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Begin => "audit_begin",
            Self::Epoch => "audit_epoch",
            Self::Key => "audit_key",
            Self::Tip => "audit_tip",
            Self::Insert => "audit_insert",
            Self::Timestamp => "audit_timestamp",
            Self::Pin => "audit_pin",
            Self::Finalize => "audit_finalize",
            Self::Indeterminate => "audit_indeterminate",
        })
    }
}

impl std::error::Error for AuditWriteError {}

/// Write the row and required head pin through one transaction. A nested
/// success confirms only this savepoint; the owner must still commit its outer
/// transaction. Optional compliance-pack evidence is not certified here.
pub fn record_tenant_checked(
    conn: &Connection,
    kind: AuditKind,
    actor: &str,
    target: &str,
    status: AuditStatus,
    detail: &str,
    tenant: &str,
) -> Result<i64, AuditWriteError> {
    let top = conn.is_autocommit();
    let begin = if top {
        "BEGIN IMMEDIATE"
    } else {
        "SAVEPOINT audit_link"
    };
    conn.execute_batch(begin).map_err(|e| {
        record_commit_failure(&e);
        AuditWriteError::Begin
    })?;
    let result = (|| {
        // A genuinely absent metadata table is the historical legacy format;
        // a present but unreadable epoch must never silently select that format.
        let metadata: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='schema_meta')",
            [], |r| r.get(0),
        ).map_err(|_| AuditWriteError::Epoch)?;
        let epoch: Option<String> = if metadata {
            conn.query_row(
                "SELECT value FROM schema_meta WHERE key=?1",
                [EPOCH_META_KEY],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| AuditWriteError::Epoch)?
        } else {
            None
        };
        let tip = conn.query_row(
            "SELECT id, ts, kind, actor, target_hash, status, detail_hash, prev_hash FROM audit_events ORDER BY id DESC LIMIT 1",
            [], map_full_row,
        ).optional().map_err(|_| AuditWriteError::Tip)?;
        let is_first_row = tip.is_none();
        let scheme = match epoch.as_deref() {
            // An unstamped DB that already has rows is a pre-`sha7` chain:
            // fall through to `legacy` below and keep recomputing its links
            // byte-for-byte, so upgrading the binary cannot make an existing
            // chain fail verify. An unstamped DB with NO rows has no format to
            // preserve — this first append picks the corrected 7-field link and
            // stamps it in the same tx, so new evidence is never written in the
            // weaker form again. Production reaches `hmac256` first via
            // `bootstrap_epoch` at boot (a row-less DB + a chain key).
            None if is_first_row => {
                if !write_epoch_stamp(conn, EPOCH_SHA7) {
                    return Err(AuditWriteError::Epoch);
                }
                Scheme::Sha7
            }
            None | Some(EPOCH_LEGACY) => Scheme::Legacy,
            Some(EPOCH_SHA7) => Scheme::Sha7,
            Some(EPOCH_HMAC) => Scheme::Hmac(chain_key().ok_or(AuditWriteError::Key)?),
            Some(_) => return Err(AuditWriteError::Epoch),
        };
        let prev_hash = tip.map(|row| row_link(&scheme, &row));
        let target_hash = hash(target);
        let detail_hash = hash(detail);
        conn.execute(
            "INSERT INTO audit_events(kind, actor, target_hash, status, detail_hash, tenant_id, prev_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![kind.as_str(), actor, target_hash, status.as_str(), detail_hash, tenant, prev_hash],
        ).map_err(|_| AuditWriteError::Insert)?;
        let id = conn.last_insert_rowid();
        let ts = conn
            .query_row("SELECT ts FROM audit_events WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .map_err(|_| AuditWriteError::Timestamp)?;
        let row = ChainRowFull {
            id,
            ts,
            kind: kind.as_str().into(),
            actor: actor.into(),
            target_hash,
            status: status.as_str().into(),
            detail_hash,
            prev_hash,
        };
        let pin = HeadPin::for_row(&scheme, &row);
        if !write_head_pin(conn, &pin) {
            return Err(AuditWriteError::Pin);
        }
        // First row ever appended: the same tx also stamps the genesis pin
        // so head-deletion is detectable from the chain's first moment. A
        // genesis written here is trustworthy by construction (the chain
        // was empty); the boot-time upgrade path stamps older chains and
        // warns when it does.
        if is_first_row {
            let genesis = HeadPin::for_row(&scheme, &row);
            if !write_genesis_pin(conn, &genesis) {
                return Err(AuditWriteError::Pin);
            }
        }
        conn.execute_batch(if top {
            "COMMIT"
        } else {
            "RELEASE SAVEPOINT audit_link"
        })
        .map_err(|e| {
            record_commit_failure(&e);
            AuditWriteError::Finalize
        })?;
        if top && !conn.is_autocommit() {
            return Err(AuditWriteError::Indeterminate);
        }
        Ok(id)
    })();
    if let Err(stage) = result {
        note_chain_health_failure("checked audit write refused");
        // If a failed finalization already ended the transaction, its result is
        // uncertain. Do not certify rollback merely from autocommit alone.
        if conn.is_autocommit() {
            return Err(AuditWriteError::Indeterminate);
        }
        let rollback = if top {
            "ROLLBACK"
        } else {
            "ROLLBACK TO SAVEPOINT audit_link; RELEASE SAVEPOINT audit_link"
        };
        if conn.execute_batch(rollback).is_err() || (top && !conn.is_autocommit()) {
            return Err(AuditWriteError::Indeterminate);
        }
        return Err(stage);
    }
    result
}

/// Settle-failure counter: the audit chain's "the row may not be durable"
/// signal. Incremented by [`record_tenant`] when the COMMIT/ROLLBACK of a
/// best-effort audit row fails — or when an hmac256-epoch chain's key is
/// unavailable and the row is refused; read by `/health` so the absence is
/// visible to operators, not just the log.
static COMMIT_FAILURES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// SQLite busy events surfaced as commit failures (SQLITE_BUSY after the
/// 5s `busy_timeout` burn). Monotonic; surfaced on `/metrics` as
/// `brain_db_busy_total` — the law-13 contention gauge (frozen concurrency:
/// this OBSERVES busy events, it never changes locking behavior).
static BUSY_HITS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Whether a rusqlite error is a SQLITE_BUSY family event (busy /
/// busy_snapshot / busy_recovery — all "the writer waited and lost").
/// `pub(crate)` so the Throughput contention counter
/// (`concurrency::note_busy_error`) reuses the one classification —
/// two busy definitions would drift.
pub(crate) fn is_busy_error(e: &rusqlite::Error) -> bool {
    matches!(e, rusqlite::Error::SqliteFailure(ffi, _)
        if ffi.extended_code == rusqlite::ffi::SQLITE_BUSY
            || ffi.extended_code == rusqlite::ffi::SQLITE_BUSY_SNAPSHOT
            || ffi.extended_code == rusqlite::ffi::SQLITE_BUSY_RECOVERY)
}

/// Count a busy event. Called from the audit commit-failure path (every
/// write tx crosses the audit seam) so contention is observable without
/// touching the frozen busy-handler semantics.
fn note_busy_hit(e: &rusqlite::Error) {
    if is_busy_error(e) {
        BUSY_HITS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Busy events since process start (see [`note_busy_hit`]). Monotonic;
/// surfaced on `/metrics`.
pub fn busy_hits() -> usize {
    BUSY_HITS.load(std::sync::atomic::Ordering::Relaxed)
}

fn note_chain_health_failure(msg: &str) {
    COMMIT_FAILURES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    error!("audit chain health failure — {msg}");
}

fn record_commit_failure(e: &rusqlite::Error) {
    note_busy_hit(e);
    note_chain_health_failure("audit transaction failed; evidence not certified");
}

/// Number of failed audit-tx settles since process start (see
/// [`record_tenant`]). Monotonic; surfaced on the gated `/health` body.
pub fn audit_commit_failures() -> usize {
    COMMIT_FAILURES.load(std::sync::atomic::Ordering::Relaxed)
}

/// record a read event AND persist its replayable
/// decision-path trace. The audit row is hash-only (chunk ids + scores go into
/// `detail_hash`; never content). The full trace detail (non-content decision
/// metadata: ids, scores, ranks, decision, scope, principal, query) lives in
/// the `recall_traces` side table keyed by the audit row id, so `/recall/{id}/
/// trace` can replay it without touching the tamper-evident chain. Returns the
/// audit row id (the trace id), or `None` if the audit write failed.
///
/// `trace_detail` is optional — non-recall read events (`/search`, `/get`,
/// `/multi-get`) record the audit row only, with no replay artifact.
pub fn record_read_event(
    conn: &Connection,
    kind: AuditKind,
    actor: &str,
    target: &str,
    trace_detail: Option<&str>,
    tenant: &str,
) -> Option<i64> {
    let detail = trace_detail.unwrap_or(target);
    let id = record_tenant(conn, kind, actor, target, AuditStatus::Ok, detail, tenant)?;
    if let Some(t) = trace_detail {
        let _ = conn.execute(
            "INSERT INTO recall_traces(audit_id, trace_json) VALUES (?1, ?2)",
            params![id, t],
        );
    }
    Some(id)
}

/// Fetch a stored recall trace by audit row id (the `?trace=true` id returned
/// by `/recall`). Returns the raw JSON string or `None` when absent.
pub fn read_trace(conn: &Connection, audit_id: i64) -> Option<String> {
    conn.query_row(
        "SELECT trace_json FROM recall_traces WHERE audit_id = ?1",
        params![audit_id],
        |r| r.get(0),
    )
    .ok()
}

/// bounded audit retention. Removes rows older than
/// `retention_days` and re-anchors the hash chain so the oldest surviving row
/// becomes the new genesis. Called on read-event writes (only when
/// `BRAIN_AUDIT_RETENTION_DAYS` is set), guarded so a steady-state pass with
/// nothing to prune costs one cheap COUNT. Returns the number pruned.
///
/// `audit_events.ts` is stored as SQLite `CURRENT_TIMESTAMP` (`YYYY-MM-DD
/// HH:MM:SS` UTC), which sorts lexicographically, so the cutoff is computed in
/// SQL and compared as text.
///
/// ponytail: re-anchoring rewrites `prev_hash` for every surviving row (O(n))
/// and only runs when there ARE expired rows — rare, so the occasional cost is
/// acceptable for a multi-thousand-row audit log. A >1M-row log would want a
/// periodic checkpoint instead (verify_chain already notes the same ceiling).
pub fn prune_audit_retention(conn: &Connection, retention_days: u32) -> Option<i64> {
    // was `.ok()?` — a silent skip hid the prune's
    // failure from the only diagnostic seam (its caller). Warn instead.
    let cutoff: String = match conn.query_row(
        "SELECT datetime('now', ?1)",
        params![format!("-{retention_days} days")],
        |r| r.get(0),
    ) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("audit retention prune: cutoff query failed: {e}");
            return None;
        }
    };
    let expired: i64 = match conn.query_row(
        "SELECT COUNT(*) FROM audit_events WHERE ts < ?1",
        params![cutoff],
        |r| r.get(0),
    ) {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!("audit retention prune: count query failed: {e}");
            return None;
        }
    };
    if expired == 0 {
        return Some(0);
    }
    // The re-anchor below recomputes links under the DB's OWN
    // epoch — an hmac256 epoch without its key refuses the prune (a keyless
    // rewrite would downgrade the chain format).
    let Some(scheme) = current_scheme(conn) else {
        tracing::warn!(
            "audit retention prune REFUSED: chain key unavailable on an hmac256-epoch chain"
        );
        return None;
    };
    // VERIFY BEFORE PRUNE — the re-anchor below
    // recomputes every survivor's prev_hash, which would re-bless a tampered
    // chain into a freshly-verifying one (evidence laundering). A chain that
    // does not verify is preserved verbatim for forensics; pruning it is
    // refused and the operator sees the warning.
    if !verify_chain(conn) {
        tracing::warn!(
            "audit retention prune REFUSED: chain does not verify \
             ({expired} expired rows preserved for forensics)"
        );
        return None;
    }
    // IMMEDIATE (not the default DEFERRED that `unchecked_transaction`
    // uses) so the re-anchor's read-then-rewrite of every survivor's prev_hash
    // is serialized against concurrent `record_tenant` writers. Without this,
    // a `record_tenant` INSERT sneaked between prune's SELECT and its first
    // UPDATE would chain its prev_hash against a tip the prune is about to
    // rewrite — forking the chain. Same root cause as the record_tenant fix.
    // Raw SQL (not `transaction_with_behavior`) keeps the `&Connection` signature
    // callers already use; we COMMIT/ROLLBACK explicitly.
    if conn.execute("BEGIN IMMEDIATE", []).is_err() {
        return None;
    }
    // Remove expired rows from the head.
    if conn
        .execute("DELETE FROM audit_events WHERE ts < ?1", params![cutoff])
        .is_err()
    {
        let _ = conn.execute("ROLLBACK", []);
        return None;
    }
    // Re-anchor: the oldest survivor becomes the genesis (NULL prev_hash), and
    // every subsequent survivor's prev_hash is recomputed so the retained
    // window stays internally tamper-evident.
    let mut ids: Vec<i64> = Vec::new();
    {
        let mut stmt = match conn.prepare("SELECT id FROM audit_events ORDER BY id ASC") {
            Ok(s) => s,
            Err(_) => {
                let _ = conn.execute("ROLLBACK", []);
                return None;
            }
        };
        let rows = match stmt.query_map([], |r| r.get::<_, i64>(0)) {
            Ok(r) => r,
            Err(_) => {
                let _ = conn.execute("ROLLBACK", []);
                return None;
            }
        };
        for v in rows.flatten() {
            ids.push(v);
        }
    }
    let mut prev: Option<String> = None;
    for (i, id) in ids.iter().enumerate() {
        let row: Option<ChainRowFull> = conn
            .query_row(
                "SELECT id, ts, kind, actor, target_hash, status, detail_hash, prev_hash \
                  FROM audit_events WHERE id = ?1",
                params![id],
                map_full_row,
            )
            .ok();
        let Some(full) = row else {
            let _ = conn.execute("ROLLBACK", []);
            return None;
        };
        let old_prev = full.prev_hash.clone();
        let new_prev = if i == 0 || old_prev.is_none() {
            None // genesis / leading legacy NULL run keeps its NULL backref
        } else {
            Some(prev.clone().unwrap_or_default())
        };
        // A failed re-anchor UPDATE propagates —
        // swallowing it here and COMMITting anyway would persist a
        // half-rewritten chain that then fails verify (the exact `let _ =`
        // residue pattern).
        if let Err(e) = conn.execute(
            "UPDATE audit_events SET prev_hash = ?1 WHERE id = ?2",
            params![new_prev, id],
        ) {
            let _ = conn.execute("ROLLBACK", []);
            tracing::warn!("audit retention prune: re-anchor UPDATE failed (id {id}): {e}");
            return None;
        }
        let mut linked = full;
        linked.prev_hash = new_prev;
        prev = Some(row_link(&scheme, &linked));
    }
    // Refresh the head pin INSIDE the prune tx — the re-anchor moved the
    // tip, and a pin that lags its commit would false-alarm the next verify.
    refresh_head_pin(conn, &scheme);
    // sweep orphaned trace artifacts. `recall_traces` is keyed by the
    // audit row id with no FK; retention-pruned audit rows would otherwise
    // leave their replay traces behind forever. Delete any trace whose audit
    // row is gone. (The DSAR/purge cascade is handled in gate::purge_chunk_ids;
    // this covers the retention path.)
    if conn
        .execute(
            "DELETE FROM recall_traces
          WHERE audit_id NOT IN (SELECT id FROM audit_events)",
            [],
        )
        .is_err()
    {
        let _ = conn.execute("ROLLBACK", []);
        return None;
    }
    if conn.execute("COMMIT", []).is_err() {
        let _ = conn.execute("ROLLBACK", []);
        return None;
    }
    // The deletion of audit evidence is itself
    // evidenced — one row on the (re-anchored) chain recording cutoff + count.
    // Best-effort (a failure here must not unwind the committed prune) but
    // never silent.
    if record(
        conn,
        AuditKind::Retention,
        "system",
        &cutoff,
        AuditStatus::Ok,
        &format!("pruned:{expired}"),
    )
    .is_none()
    {
        tracing::warn!("audit retention prune: prune-event record failed (count {expired})");
    }
    Some(expired)
}

/// The current hash-chain head: the link a new audit row would chain from.
/// Used by DSAR certificates as the `chain_head` evidence of a valid chain at
/// certification time. Epoch-aware; `None` on an empty chain or when an
/// hmac256-epoch chain has no key in this process (cannot attest what it
/// cannot compute).
pub fn chain_head(conn: &Connection) -> Option<String> {
    let scheme = current_scheme(conn)?;
    tip_row(conn).map(|tip| row_link(&scheme, &tip))
}

/// Verify the audit hash chain end-to-end. Returns `false` if any chained row's
/// stored `prev_hash` disagrees with the link recomputed from the prior row —
/// under the DB's OWN epoch (legacy chains verify exactly as they always
/// did; hmac256 chains verify against the keyed 8-field link, and fail
/// closed when the key is unavailable: an unverifiable chain is not `ok`).
/// Legacy rows (NULL `prev_hash`, written before the chain existed) carry no
/// backref and are skipped — a migrated DB may have thousands of them, followed
/// by the first chained row that links back to the last NULL row's recomputed
/// link.
///
/// NULL `prev_hash` is legal
/// only as a PREFIX. `record_tenant` always writes a non-NULL backref once a
/// tip exists, and the retention prune re-anchors to a single leading genesis
/// NULL — so a NULL appearing AFTER the first non-NULL row can only be the
/// result of tampering (an attacker erasing a row's link to its predecessor).
/// The prefix rule makes that detectable without changing any stored hash.
///
/// After the walk passes, the head pin is compared against the
/// recomputed head — every legitimate mutation (record, prune re-anchor,
/// re-anchor) rewrites the pin in the same tx, so a pin that disagrees with
/// the chain means rows were added or removed OUTSIDE those paths: truncation
/// or extension of an otherwise-internally-valid chain, detected.
pub fn verify_chain(conn: &Connection) -> bool {
    verify_chain_inner(conn, true)
}

/// The walk + pin comparison shared by [`verify_chain`] (MACs enforced)
/// and the one-shot upgrade path (MACs tolerated, because a chain written
/// before pins carried MACs still has sound LINKS — the upgrade re-stamps
/// the pins only after this tolerant walk passes).
///
/// SNAPSHOT DISCIPLINE — THE TWO READS MUST SEE ONE INSTANT.
///
/// The body reads the chain TWICE from two different tables: the walk reads
/// `audit_events` (the tip) and the pin check reads `schema_meta` (the head
/// pin). When the connection is in AUTOCOMMIT, each of those is its own read
/// transaction and therefore its OWN SNAPSHOT. On a WAL database a concurrent
/// audit append that commits BETWEEN them leaves the pin one-or-more rows
/// NEWER than the tip the walk saw — and the comparison below reports a pin
/// mismatch, i.e. "rows were added or removed outside the audited write
/// paths", for a chain that is perfectly intact. The invariant the check
/// relies on ("every legitimate mutation rewrites the pin in the same tx") is
/// true at any single instant; comparing a tip from instant T1 against a pin
/// from instant T2 > T1 simply asks a question no single instant answers.
///
/// This was NOT theoretical. The production WAL chain-check loop
/// (`alert.rs`) walks hundreds of thousands of rows on a timer while request
/// traffic appends; over ~20h it logged 42 false mismatches, EVERY one with
/// the pin AHEAD of the tip by 1..63 rows (never behind — behind would mean
/// real loss), the gap being however many appends landed mid-walk.
///
/// The fix is to hold ONE read snapshot across the whole body: `BEGIN
/// DEFERRED` takes the snapshot at the first read and keeps it until commit,
/// so the walk, the pin, and the genesis pin all describe the same instant.
/// An append that commits after the snapshot is simply not seen by this
/// verify — the next tick sees it with its own pin, consistent.
fn verify_chain_inner(conn: &Connection, require_pin_macs: bool) -> bool {
    // Already inside a caller's transaction: its snapshot already pins every
    // read below, so run inline (and a nested BEGIN would error).
    if !conn.is_autocommit() {
        return verify_chain_in_snapshot(conn, require_pin_macs);
    }
    // Autocommit: open a read transaction so the whole body shares one
    // snapshot. `BEGIN DEFERRED` acquires no write lock — it only fixes the
    // read view — so this adds no contention and still lets writers proceed.
    if conn.execute_batch("BEGIN DEFERRED").is_err() {
        // Fail closed: if we cannot establish the snapshot the two reads would
        // be taken from different instants, which is the bug.
        return false;
    }
    let ok = verify_chain_in_snapshot(conn, require_pin_macs);
    // Read-only body; discard the transaction. ROLLBACK (not COMMIT) so the
    // snapshot is released without ever implying a write.
    let _ = conn.execute_batch("ROLLBACK");
    ok
}

/// The walk + pin comparison proper. Runs entirely inside one read snapshot
/// (established by [`verify_chain_inner`] or by a caller's transaction), so
/// every read here observes the same instant.
fn verify_chain_in_snapshot(conn: &Connection, require_pin_macs: bool) -> bool {
    let Some(scheme) = current_scheme(conn) else {
        tracing::warn!(
            "audit chain verify: hmac256-epoch chain has no key in this process — not ok"
        );
        return false;
    };
    let keyed_mac = match &scheme {
        Scheme::Legacy | Scheme::Sha7 => None,
        Scheme::Hmac(key) => Some(*key.as_ref()),
    };
    let mut stmt = match conn.prepare(
        "SELECT id, ts, kind, actor, target_hash, status, detail_hash, prev_hash FROM audit_events \
          ORDER BY id ASC",
    ) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let rows = match stmt.query_map([], map_full_row) {
        Ok(r) => r,
        Err(_) => return false,
    };
    // `prev_link` is the chain link computed from the prior row; the next row's
    // stored `prev_hash` (if it has one) must equal it. NULL `prev_hash` rows
    // are legacy rows (or the very first row / the prune genesis) and carry no
    // backref to verify — they only contribute their own link to the next row.
    // A migrated DB has arbitrarily many consecutive NULL rows at the head, so
    // a leading NULL run never fails. Once a row carries a non-NULL
    // backref, the chain has started — any LATER NULL is tamper.
    let mut prev_link: Option<(i64, String)> = None;
    let mut chain_started = false;
    let mut first_row: Option<(i64, String)> = None;
    for row in rows.flatten() {
        match &row.prev_hash {
            Some(got) => {
                chain_started = true;
                match &prev_link {
                    Some((_, want)) if want == got => {}
                    Some(_) => return false, // tampered or out-of-order chained row
                    None => {}               // first row overall — chain origin
                }
            }
            None if chain_started => {
                // A NULL backref after the chain started. Legitimate
                // writers always chain from the tip once one exists.
                return false;
            }
            None => {}
        }
        // Advance: every row contributes its link, including NULL ones (the
        // first chained row after a NULL run links back to the last NULL row).
        let id = row.id;
        let link = row_link(&scheme, &row);
        if first_row.is_none() {
            first_row = Some((id, link.clone()));
        }
        prev_link = Some((id, link));
    }
    // Under the keyed epoch, both pins must carry a MAC that verifies — a
    // pin is state an attacker with SQL write access could otherwise
    // rewrite at will (it lives in the same DB it guards). The MAC key is
    // the chain key: already outside the DB, already required to forge a
    // link. Skipped for legacy- and sha7-epoch chains (the write path commits
    // no key for them; their posture keeps the documented weaker pins) and for
    // the one-shot upgrade walk that re-stamps old mac-less pins.
    if let (Some(key), true) = (&keyed_mac, require_pin_macs) {
        let head = read_head_pin(conn);
        let genesis = read_genesis_pin(conn);
        // A non-empty keyed chain always carries both pins (every append,
        // prune, and re-anchor writes them in the same tx) — a missing pin
        // on a live chain is not "never stamped", it is removed.
        if first_row.is_some() && (head.is_none() || genesis.is_none()) {
            tracing::error!(
                "audit chain pin missing on a non-empty keyed chain \
                 (head={}, genesis={}) — pin removal",
                head.is_some(),
                genesis.is_some()
            );
            return false;
        }
        for (name, pin) in [("head", head), ("genesis", genesis)] {
            if let Some(pin) = pin
                && !pin.mac_is_valid(key)
            {
                tracing::error!(
                    "audit chain {name} pin MAC missing or invalid (id {}) — pin tamper",
                    pin.id
                );
                return false;
            }
        }
    }
    // Genesis pin check — the FIRST row must be the pinned row. A chain
    // whose oldest rows were deleted now starts at a later row: the pinned
    // id disagrees, exactly like a tail-truncated chain disagrees with the
    // head pin. Without this, deleting prefix rows was free: survivor links
    // chain from their immediate predecessors only, and the tip (hence the
    // head pin) never moves.
    if let Some(pin) = read_genesis_pin(conn) {
        match first_row {
            Some((first_id, first_link)) if first_id == pin.id && first_link == pin.hash => {}
            Some((first_id, _)) => {
                tracing::error!(
                    "audit chain genesis pin mismatch: pin=(id {}, hash {}..) first=(id {first_id}) \
                     — the oldest rows were removed outside the audited write paths",
                    pin.id,
                    &pin.hash[..pin.hash.len().min(8)]
                );
                return false;
            }
            None => {
                tracing::error!(
                    "audit chain genesis pin exists (id {}) but the chain is empty — full truncation",
                    pin.id
                );
                return false;
            }
        }
    }
    // Head-pin check — only when a pin exists (fresh/unpinned DBs skip).
    if let Some(pin) = read_head_pin(conn) {
        match prev_link {
            Some((tip_id, tip_link)) if tip_id == pin.id && tip_link == pin.hash => {}
            Some((tip_id, _)) => {
                tracing::error!(
                    "audit chain head pin mismatch: pin=(id {}, hash {}..) tip=(id {tip_id}) \
                     — rows were added or removed outside the audited write paths",
                    pin.id,
                    &pin.hash[..pin.hash.len().min(8)]
                );
                return false;
            }
            // A pin with zero rows: the chain was fully truncated below its
            // own pinned genesis — tamper by any reading.
            None => {
                tracing::error!(
                    "audit chain head pin exists (id {}) but the chain is empty — full truncation",
                    pin.id
                );
                return false;
            }
        }
    }
    true
}

/// The offline `--re-audit` body: replay an existing chain under the
/// hmac256 scheme — every non-NULL `prev_hash` is recomputed as the keyed
/// 8-field link, the epoch stamp flips, and the head pin is rewritten. The
/// leading NULL run (legacy migration prefix) keeps its NULL backrefs, exactly
/// as [`verify_chain`]'s prefix rule expects.
///
/// GUARDS:
/// - Refuses a chain that does not verify under its CURRENT epoch (replaying
///   a broken chain would launder it into a freshly-verifying one — the same
///   rule the retention prune enforces).
/// - Refuses when no key is available (an unkeyed "re-anchor" is a no-op that
///   lies about the format).
/// - Idempotent: a chain already on hmac256 verifies, refreshes its pin, and
///   returns `Ok(0)`.
///
/// Returns the number of rows whose links were rewritten. The caller records
/// the `AuditKind::Anchor` evidence row on the NEW chain.
pub fn reanchor_to_hmac(conn: &Connection) -> anyhow::Result<usize> {
    use anyhow::Context;
    let key = chain_key().context(
        "audit chain key unavailable — set BRAIN_AUDIT_CHAIN_KEY / BRAIN_AUDIT_CHAIN_KEY_FILE \
         or make the DB directory writable so audit-chain.key can be created",
    )?;
    if read_epoch(conn) == ChainEpoch::Hmac256 {
        if !verify_chain(conn) {
            anyhow::bail!(
                "chain already hmac256 but does not verify — refusing to touch it \
                 (restore the pre-anchor snapshot and retry)"
            );
        }
        refresh_head_pin(conn, &Scheme::Hmac(key));
        return Ok(0);
    }
    if !verify_chain(conn) {
        anyhow::bail!(
            "legacy chain does not verify — refusing to re-anchor a broken chain \
             (evidence-laundering guard; repair or restore from the pre-anchor snapshot first)"
        );
    }
    // Collect the full rows first (the walk borrows the connection; the
    // UPDATE loop needs it back).
    let rows: Vec<ChainRowFull> = {
        let mut stmt = conn.prepare(
            "SELECT id, ts, kind, actor, target_hash, status, detail_hash, prev_hash \
              FROM audit_events ORDER BY id ASC",
        )?;
        let mapped = stmt.query_map([], map_full_row)?;
        mapped.filter_map(|r| r.ok()).collect()
    };
    if conn.execute("BEGIN IMMEDIATE", []).is_err() {
        anyhow::bail!("re-anchor: BEGIN IMMEDIATE failed");
    }
    let mut prev_link: Option<String> = None;
    let mut rewritten = 0usize;
    for row in &rows {
        let new_prev = if row.prev_hash.is_none() {
            None // leading legacy NULL run keeps its NULL backrefs
        } else {
            prev_link.clone()
        };
        if new_prev != row.prev_hash {
            if let Err(e) = conn.execute(
                "UPDATE audit_events SET prev_hash = ?1 WHERE id = ?2",
                params![new_prev, row.id],
            ) {
                let _ = conn.execute("ROLLBACK", []);
                anyhow::bail!("re-anchor UPDATE failed (id {}): {e}", row.id);
            }
            rewritten += 1;
        }
        let mut linked = row.clone();
        linked.prev_hash = new_prev;
        prev_link = Some(row_link(&Scheme::Hmac(key.clone()), &linked));
    }
    // Flip the epoch + rewrite the pin in the SAME tx as the replay: a crash
    // mid-re-anchor leaves either the old epoch fully intact or the new one
    // fully consistent — never a half-converted chain.
    if let Err(e) = conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS schema_meta(key TEXT PRIMARY KEY, value TEXT);
         INSERT INTO schema_meta(key, value) VALUES ('{EPOCH_META_KEY}', '{EPOCH_HMAC}')
         ON CONFLICT(key) DO UPDATE SET value = '{EPOCH_HMAC}';"
    )) {
        let _ = conn.execute("ROLLBACK", []);
        anyhow::bail!("re-anchor epoch stamp failed: {e}");
    }
    refresh_head_pin(conn, &Scheme::Hmac(key));
    if conn.execute("COMMIT", []).is_err() {
        let _ = conn.execute("ROLLBACK", []);
        anyhow::bail!("re-anchor COMMIT failed");
    }
    // Post-condition: the replayed chain must verify under the new scheme
    // before the caller stamps the Anchor evidence row onto it.
    if !verify_chain(conn) {
        anyhow::bail!("re-anchor post-condition failed: replayed chain does not verify");
    }
    Ok(rewritten)
}

/// Fresh-DB epoch bootstrap: a DB with ZERO audit rows may start directly on
/// hmac256 when a key is available. A DB with even one row keeps its legacy
/// epoch until the explicit [`reanchor_to_hmac`] — an audit chain is
/// evidence, and its format changes only with the announced re-anchor.
/// Called at boot (global pool) and at lazy domain-pool open; a no-op in every
/// test context that never installs a key. Returns whether the stamp landed.
pub fn bootstrap_epoch(conn: &Connection) -> bool {
    if chain_key().is_none() {
        return false;
    }
    if read_epoch(conn) != ChainEpoch::Legacy {
        return false;
    }
    let rows: i64 = match conn.query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0)) {
        Ok(n) => n,
        Err(_) => return false,
    };
    if rows > 0 {
        return false;
    }
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS schema_meta(key TEXT PRIMARY KEY, value TEXT);
         INSERT INTO schema_meta(key, value) VALUES ('{EPOCH_META_KEY}', '{EPOCH_HMAC}')
         ON CONFLICT(key) DO UPDATE SET value = '{EPOCH_HMAC}';"
    ))
    .is_ok()
}

/// One-shot pin upgrade for existing chains, called at boot after the key
/// resolves and the migration runs: stamps the genesis pin when absent and
/// adds MACs to pins written before pins carried them. From this point
/// [`verify_chain`] requires both pins present + MAC-valid on every
/// non-empty keyed chain, so the upgrade is what keeps pre-existing
/// hmac256 deployments verifying instead of failing closed on old pins.
///
/// GUARDS (the prune/re-anchor laundering rule, applied to pins):
/// - Runs only when a key is installed AND the chain is non-empty.
/// - The chain must pass the MAC-TOLERANT walk first — stamping pins onto a
///   broken chain would bless it (a genesis pinned from a chain whose head
///   was already deleted records the deletion as the origin). A chain that
///   fails the walk is left verbatim for forensics and the operator sees
///   the verify failure.
/// - A genesis stamped onto an existing chain is DISCLOSED, not silent: the
///   trustworthy genesis is the one written by the first append (the chain
///   was empty). Here the chain already had history, so the stamp records
///   the chain AS IT NOW EXISTS and says so at warn level. An operator who
///   suspects pre-upgrade head deletion has the off-host `brain anchor`
///   fingerprints to compare against.
/// - Pins that already carry a MAC are never rewritten (an invalid MAC is a
///   tamper signal, not an upgrade gap — fixing it here would erase one).
/// - Legacy-epoch chains get the structural genesis stamp (detection
///   without a key) but no MACs — there is no key to MAC with.
pub fn upgrade_pins(conn: &Connection) -> bool {
    let Some(scheme) = current_scheme(conn) else {
        return false;
    };
    let (Some(first), Some(_tip)) = (first_row(conn), tip_row(conn)) else {
        return false; // empty chain: the first append pins both
    };
    // Fast path (every boot after the first): nothing to stamp — genesis
    // present, and under the keyed epoch both pins already carry MACs.
    // Skips the O(n) tolerant walk entirely for steady-state boots.
    {
        let head = read_head_pin(conn);
        let genesis = read_genesis_pin(conn);
        let macs_done = match &scheme {
            Scheme::Legacy | Scheme::Sha7 => true,
            Scheme::Hmac(_) => {
                head.as_ref().is_some_and(|p| p.mac.is_some())
                    && genesis.as_ref().is_some_and(|p| p.mac.is_some())
            }
        };
        if genesis.is_some() && macs_done {
            return false;
        }
    }
    if !verify_chain_inner(conn, false) {
        tracing::warn!(
            "audit pin upgrade skipped: chain does not verify — evidence preserved verbatim"
        );
        return false;
    }
    let mut changed = false;
    let genesis = read_genesis_pin(conn);
    if genesis.is_none() {
        tracing::warn!(
            "audit genesis pin stamped onto an existing chain from row {} — it records the \
             chain as it now exists; pre-upgrade head deletion is outside its evidence \
             (compare off-host `brain anchor` fingerprints if suspected)",
            first.id
        );
        changed |= write_genesis_pin(conn, &HeadPin::for_row(&scheme, &first));
    }
    // Add MACs to pins that predate MACs (absent field) — never overwrite a
    // present one.
    for (name, pin, rewrite) in [
        (
            "head",
            read_head_pin(conn),
            write_head_pin as fn(&Connection, &HeadPin) -> bool,
        ),
        ("genesis", read_genesis_pin(conn), write_genesis_pin),
    ] {
        if let Some(pin) = pin
            && pin.mac.is_none()
        {
            let Scheme::Hmac(key) = &scheme else {
                continue; // legacy: nothing to MAC with
            };
            let stamped = HeadPin {
                mac: Some(pin_mac(key.as_ref(), &pin.epoch, pin.id, &pin.hash)),
                ..pin
            };
            tracing::warn!("audit {name} pin upgraded with a MAC (id {})", pin.id);
            changed |= rewrite(conn, &stamped);
        }
    }
    changed
}

#[cfg(test)]
mod test_key_gate {
    /// Serializes every test that installs a process-global chain key. The
    /// key is visible to concurrently-running tests (fresh-DB bootstraps,
    /// hmac writes), so keyed tests must hold this mutex for their whole
    /// scenario and clear the key on exit.
    ///
    /// The census is TWO mutators, and both now hold it: this module's own
    /// keyed tests, and `anchor::tests::anchor_root_is_independent_of_the_
    /// chain_key`, which installs key A, then key B, then clears — and did
    /// not fence. `hmac_epoch_without_key_fails_closed` asserts the *no-key*
    /// path, so an unfenced flip turned it red; its panic then poisoned this
    /// mutex and took eight sibling keyed tests down with it (observed as a
    /// nine-failure `compliance-pack` lane, 2026-10-12, where only the first
    /// failure had a real cause). A new mutator is a third-party edit to
    /// this global, so it holds this lock.
    pub(crate) static KEY_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());
}

#[cfg(test)]
pub(crate) fn set_chain_key_for_test(key: Option<[u8; 32]>) {
    let mut slot = CHAIN_KEY.write().expect("chain key lock poisoned");
    *slot = key.map(Arc::new);
}

/// The fence every process-global chain-key mutator holds for its whole
/// scenario. `pub(crate)` because the second mutator lives outside this
/// module — one definition of "who serializes the key", not one per file.
#[cfg(test)]
pub(crate) fn key_gate_for_test() -> std::sync::MutexGuard<'static, ()> {
    test_key_gate::KEY_GATE.lock().expect("key gate poisoned")
}

/// Read recent audit events (operator diagnostics only). Bounded by `limit`.
/// optional `tenant` filter — when `Some`, scoped to that tenant only
/// at the SQL layer so a forgotten app-level filter cannot leak cross-tenant.
pub fn recent(
    conn: &Connection,
    kind: Option<&str>,
    limit: usize,
) -> rusqlite::Result<Vec<AuditRow>> {
    recent_tenant(conn, kind, None, None, None, limit, 0)
}

/// Per-tenant variant of [`recent`]. `tenant = None` returns rows across all
/// tenants (operator diagnostics); `Some(t)` enforces `WHERE tenant_id = ?`.
/// `actor = None` returns every principal's rows; `Some(a)` filters to ONE
/// principal's audited footprint (the `actor` column, exact match — the
/// `/audit?principal=` filter). The filter reads rows; it never
/// re-verifies the chain differently, and the verdict surface is untouched.
/// `since = None` is unbounded; `Some(s)` is an INCLUSIVE lower bound on the
/// row timestamp at the SQL layer (`ts >= s`, the `/audit?since=` filter —
/// the caller normalizes the wire's RFC 3339 value to the chain's
/// CURRENT_TIMESTAMP format, so the lexicographic comparison is
/// chronological; the boundary row itself is IN the answer).
/// `offset` is the pagination cursor (`ORDER BY id DESC LIMIT ? OFFSET ?`).
pub fn recent_tenant(
    conn: &Connection,
    kind: Option<&str>,
    tenant: Option<&str>,
    actor: Option<&str>,
    since: Option<&str>,
    limit: usize,
    offset: usize,
) -> rusqlite::Result<Vec<AuditRow>> {
    let mut sql = String::from(
        "SELECT id, ts, kind, actor, target_hash, status, detail_hash, tenant_id \
           FROM audit_events",
    );
    let mut clauses: Vec<&'static str> = Vec::new();
    let mut params: Vec<&dyn rusqlite::ToSql> = Vec::new();
    if kind.is_some() {
        clauses.push("kind = ?");
        params.push(&kind);
    }
    if tenant.is_some() {
        clauses.push("tenant_id = ?");
        params.push(&tenant);
    }
    if actor.is_some() {
        clauses.push("actor = ?");
        params.push(&actor);
    }
    if since.is_some() {
        clauses.push("ts >= ?");
        params.push(&since);
    }
    if !clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&clauses.join(" AND "));
    }
    sql.push_str(" ORDER BY id DESC LIMIT ? OFFSET ?");
    let limit_i: i64 = limit as i64;
    params.push(&limit_i);
    let offset_i: i64 = offset as i64;
    params.push(&offset_i);

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params.as_slice(), row_mapper)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

fn row_mapper(row: &rusqlite::Row<'_>) -> rusqlite::Result<AuditRow> {
    Ok(AuditRow {
        id: row.get(0)?,
        ts: row.get(1)?,
        kind: row.get(2)?,
        actor: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
        target_hash: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
        status: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
        detail_hash: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
        tenant_id: row
            .get::<_, Option<String>>(7)?
            .unwrap_or_else(|| DEFAULT_TENANT.to_string()),
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
    pub tenant_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The chain-key mutator census.
    ///
    /// `CHAIN_KEY` is process-global, so installing/clearing it inside one
    /// test is visible to every other test running on another thread. The
    /// keyed tests fence that with `KEY_GATE`; the fence only works while
    /// EVERY mutator honours it, and the second mutator
    /// (`anchor::tests::anchor_root_is_independent_of_the_chain_key`) did not
    /// — it flipped the key under nine keyed assertions, turned
    /// `hmac_epoch_without_key_fails_closed` red, and that test's panic then
    /// poisoned the shared mutex so one real failure reported as nine
    /// (observed: the `compliance-pack` lane, 2026-10-12).
    ///
    /// This is the structural half of that fix. The behavioural half (the
    /// race itself) does not reproduce on demand — 30 runs of the interacting
    /// pair stayed green even with the window widened 300× — so the durable
    /// guard has to be the census, not the race.
    ///
    /// Deliberately fail-closed: it reads the tree from `CARGO_MANIFEST_DIR`
    /// (not the cwd), scans every `src/**/*.rs`, and asserts each mutator's
    /// enclosing function body holds the gate. The setter's own definition
    /// and the two accessors are excluded by name, and the walk stops at the
    /// first column-0 `}`, which is where a top-level test fn ends — an
    /// unparseable shape fails the arm rather than passing it silently.
    #[test]
    fn every_process_global_chain_key_mutator_holds_the_gate() {
        fn walks(dir: &Path, out: &mut Vec<PathBuf>) {
            let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
                .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
                .map(|e| e.expect("dir entry").path())
                .collect();
            entries.sort();
            for path in entries {
                if path.is_dir() {
                    walks(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    out.push(path);
                }
            }
        }

        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        walks(&root, &mut files);
        assert!(
            files.len() > 100,
            "the census walked {} files — it must see the whole src tree, \
             not a slice (a truncated walk passes vacuously)",
            files.len()
        );

        let mut mutators: Vec<(String, usize)> = Vec::new();
        for file in &files {
            let text = std::fs::read_to_string(file).expect("read source");
            for (i, line) in text.lines().enumerate() {
                let t = line.trim_start();
                // The mutator call, not the definition and not the accessors.
                if !t.starts_with("set_chain_key_for_test(")
                    && !t.contains("set_chain_key_for_test(")
                {
                    continue;
                }
                let is_definition = t.starts_with("pub(crate) fn set_chain_key_for_test");
                let is_comment = t.starts_with("//") || t.starts_with("///");
                if is_definition || is_comment {
                    continue;
                }
                let rel = file
                    .strip_prefix(&root)
                    .unwrap_or(file)
                    .display()
                    .to_string();
                mutators.push((rel, i + 1));
            }
        }

        assert!(
            mutators.len() >= 10,
            "census found only {} chain-key mutators — it must see this \
             module's own keyed tests, or the scan is broken",
            mutators.len()
        );

        let unfenced: Vec<String> = mutators
            .iter()
            .filter(|(rel, line)| {
                let path = root.join(rel);
                let text = std::fs::read_to_string(&path).expect("read source");
                // Slice from this call back to the enclosing `fn`, then
                // forward to the fn's closing brace at column 0.
                let lines: Vec<&str> = text.lines().collect();
                let start = (0..*line).rev().find(|i| lines[*i].starts_with("fn "));
                let Some(start) = start else {
                    return true; // no enclosing fn found — fail closed
                };
                let end = ((*line)..lines.len())
                    .find(|i| lines[*i] == "}")
                    .unwrap_or(lines.len());
                let body = lines[start..end].join("\n");
                !body.contains("key_gate_for_test(")
            })
            .map(|(rel, line)| format!("{rel}:{line}"))
            .collect();

        assert!(
            unfenced.is_empty(),
            "every process-global chain-key mutator must hold \
             `key_gate_for_test()` for its whole scenario; unfenced: {unfenced:?}"
        );

        // Anti-vacuity: the census must actually SEE the second mutator, the
        // one that was missed. If a future refactor moves it, this arm fails
        // rather than letting the census shrink around a live mutator.
        assert!(
            mutators.iter().any(|(rel, _)| rel.ends_with("anchor.rs")),
            "the census no longer sees anchor.rs's chain-key mutator — \
             either it moved (re-point this arm) or it is gone"
        );
    }

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
               detail_hash TEXT,
               tenant_id TEXT NOT NULL DEFAULT 'global',
               prev_hash TEXT);",
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

    /// the audit/trace hash must be SHA-256 (64 hex) — the xxh3-64
    /// fingerprint of low-entropy content (an SSN, name, short query) was
    /// offline-brute-forceable. A stored target_hash/detail_hash/query_hash
    /// derived from such a value must not be a fast non-crypto fingerprint.
    #[test]
    fn hash_is_sha256_not_xxh3() {
        let h = hash("alice@example.com");
        assert_eq!(
            h.len(),
            64,
            "SHA-256 hex is 64 chars, got {}: {}",
            h.len(),
            h
        );
        assert!(
            h.chars().all(|c| c.is_ascii_hexdigit()),
            "hash must be hex: {h}"
        );
        assert_ne!(h.len(), 16, "must not be the legacy 16-char xxh3-64 form");
        // Determinism: same input -> same digest.
        assert_eq!(h, hash("alice@example.com"));
        // A stored target_hash must not reveal the input offline (spot-check
        // the stored value is not a direct copy of the low-entropy input).
        assert!(!h.contains("alice"));
    }

    #[test]
    fn hash_chain_detects_tampering() {
        let db = db();
        // Three rows build a chain: r1 (no prev), r2 (links to r1), r3 (links to r2).
        record(&db, AuditKind::Ingest, "api", "c1", AuditStatus::Ok, "d1");
        record(&db, AuditKind::Ingest, "api", "c2", AuditStatus::Ok, "d2");
        record(&db, AuditKind::Ingest, "api", "c3", AuditStatus::Ok, "d3");
        assert!(verify_chain(&db), "unmodified chain must verify");

        // Tamper with row 2's prev_hash — the chain must break at row 2.
        let _ = db.execute(
            "UPDATE audit_events SET prev_hash = 'deadbeef' WHERE id = 2",
            [],
        );
        assert!(
            !verify_chain(&db),
            "a tampered prev_hash must fail the chain check"
        );
    }

    #[test]
    fn hash_chain_survives_migration_with_many_null_rows() {
        // Regression: the v1.1.0 migration adds `prev_hash` as a nullable
        // column, so every pre-v1.1 row is NULL. The original `verify_chain`
        // assumed at most one NULL row at the start and returned false on the
        // second NULL — a migrated DB with thousands of existing rows would
        // fail `/audit/verify` and `brain_audit_chain_ok` immediately.
        let db = db();
        // Simulate 5000 pre-v1.1 rows by inserting them directly with NULL
        // prev_hash (exactly what ALTER TABLE ADD COLUMN produces).
        for i in 0..5_000 {
            let _ = db.execute(
                "INSERT INTO audit_events(kind, actor, target_hash, status, detail_hash, prev_hash)
                 VALUES ('ingest', 'api', ?1, 'ok', ?2, NULL)",
                params![format!("c{i}"), format!("d{i}")],
            );
        }
        // Now record three v1.1 rows via the real writer. The first one links
        // back to the last NULL row; the next two chain normally.
        record(
            &db,
            AuditKind::Ingest,
            "api",
            "v1.1-a",
            AuditStatus::Ok,
            "d",
        );
        record(
            &db,
            AuditKind::Ingest,
            "api",
            "v1.1-b",
            AuditStatus::Ok,
            "d",
        );
        record(
            &db,
            AuditKind::Ingest,
            "api",
            "v1.1-c",
            AuditStatus::Ok,
            "d",
        );
        assert!(
            verify_chain(&db),
            "migrated DB with many NULL prev_hash rows must still verify"
        );

        // Tamper protection is preserved across the NULL boundary: editing the
        // first v1.1 row's prev_hash must still break the chain.
        let first_v1_1: i64 = db
            .query_row(
                "SELECT id FROM audit_events WHERE prev_hash IS NOT NULL ORDER BY id ASC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let _ = db.execute(
            "UPDATE audit_events SET prev_hash = 'deadbeef' WHERE id = ?1",
            params![first_v1_1],
        );
        assert!(
            !verify_chain(&db),
            "tampering with a v1.1 row's prev_hash must still fail after migration"
        );
    }

    #[test]
    #[allow(clippy::missing_transmute_annotations)]
    fn hash_chain_survives_real_v1_0_to_v1_1_migration() {
        // Closing the "fixture-based migration test" ceiling: actually run
        // `run_migration` against a DB whose `audit_events` table was created
        // with the pre-v1.1 schema (no `tenant_id`, no `prev_hash`) and already
        // has data. This is exactly the upgrade path the live v1.0 DB takes.
        use crate::migration::run_migration;

        // Register sqlite-vec so the full migration (which includes vec0
        // tables) runs the same way it does against the live DB. Local copy
        // because this test is in the lib crate (which doesn't share main.rs's
        // helper). See main.rs::register_sqlite_vec for the safety proof.
        // SAFETY: sqlite3_vec_init is extern "C" with the signature
        // sqlite3_auto_extension expects; the pointer is process-lifetime
        // static. See main.rs::register_sqlite_vec for the full proof.
        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
                sqlite_vec::sqlite3_vec_init as *const (),
            )));
        }
        let mut db = Connection::open_in_memory().expect("open in-memory DB");
        // 1. Build the v1.0 audit_events schema (the version before M2 added
        //    the two columns).
        db.execute_batch(
            "CREATE TABLE schema_meta(key TEXT PRIMARY KEY, value TEXT);
             CREATE TABLE audit_events(
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               ts TEXT DEFAULT CURRENT_TIMESTAMP,
               kind TEXT NOT NULL,
               actor TEXT,
               target_hash TEXT,
               status TEXT,
               detail_hash TEXT
             );
             CREATE INDEX idx_audit_kind ON audit_events(kind);
             CREATE INDEX idx_audit_ts ON audit_events(ts);",
        )
        .expect("create pre-v1.1 audit_events");
        // 2. Populate it with pre-v1.1 rows exactly as v1.0 wrote them — no
        //    prev_hash, no tenant_id column at all.
        for i in 0..1_000 {
            db.execute(
                "INSERT INTO audit_events(kind, actor, target_hash, status, detail_hash)
                 VALUES ('ingest', 'api', ?1, 'ok', ?2)",
                params![format!("legacy-{i}"), format!("d-{i}")],
            )
            .unwrap();
        }
        // 3. Run the real migration — adds `tenant_id` + `prev_hash` via
        //    ALTER TABLE ADD COLUMN. Existing rows must get NULL prev_hash and
        //    'global' tenant_id by default.
        run_migration(&mut db, 0).expect("v1.1 migration on populated v1.0 DB");

        // 4. Assert the back-compat defaults the migration promises.
        let null_count: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE prev_hash IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            null_count, 1_000,
            "every legacy row must have NULL prev_hash"
        );
        let global_count: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE tenant_id = 'global'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            global_count, 1_000,
            "every legacy row must default to 'global' tenant"
        );

        // 5. Now write v1.1 rows via the real writer and verify the chain holds
        //    across the NULL → Some boundary. This is the scenario the original
        //    `verify_chain` bug choked on.
        record(
            &db,
            AuditKind::Ingest,
            "api",
            "post-migration-1",
            AuditStatus::Ok,
            "d",
        );
        record(
            &db,
            AuditKind::Ingest,
            "api",
            "post-migration-2",
            AuditStatus::Ok,
            "d",
        );
        record(
            &db,
            AuditKind::Ingest,
            "api",
            "post-migration-3",
            AuditStatus::Ok,
            "d",
        );
        assert!(
            verify_chain(&db),
            "v1.0→v1.1 migrated DB with real data must verify end-to-end"
        );
    }

    #[test]
    fn record_tenant_is_safe_inside_caller_transaction() {
        // Closing the "record_tenant not wrapped in tx" ceiling: callers like
        // `delete_quarantine` are already inside their own transaction when they
        // audit. The SAVEPOINT must nest cleanly (BEGIN would error), and a
        // failure of the audit INSERT must NOT roll back the caller's work.
        let db = db();
        // Caller opens its own tx and does some work.
        let tx = db.unchecked_transaction().unwrap();
        tx.execute(
            "INSERT INTO audit_events(kind, actor, target_hash, status, detail_hash)
             VALUES ('ingest', 'caller', 'caller-row', 'ok', 'd')",
            [],
        )
        .unwrap();
        // Now audit happens inside the caller's tx. This used to rely on
        // autocommit semantics; now it nests via SAVEPOINT.
        record_tenant(
            &tx,
            AuditKind::Ingest,
            "api",
            "inside-tx",
            AuditStatus::Ok,
            "d",
            "team-a",
        );
        // Caller commits.
        tx.commit().unwrap();

        let rows = recent(&db, None, 10).unwrap();
        assert_eq!(rows.len(), 2, "caller row + audit row both landed");
        assert!(
            verify_chain(&db),
            "chain holds when audit ran inside a caller tx"
        );
    }

    /// a failed COMMIT/ROLLBACK settle of a best-effort
    /// audit row bumps `audit_commit_failures()` (surfaced on `/health`) and
    /// logs at error level — the row may not be durable and that must be
    /// visible, not silent. Forced here for real: a second connection holds a
    /// SHARED lock (plain BEGIN) while this connection's COMMIT needs EXCLUSIVE;
    /// with `busy_timeout=0` the settle fails with SQLITE_BUSY — no waiting.
    #[test]
    fn audit_commit_failure_alerts() {
        let tmp = tempfile::NamedTempFile::new().expect("temp file");
        let path = tmp.path();
        let holder = Connection::open(path).expect("holder conn");
        let writer = Connection::open(path).expect("writer conn");
        writer
            .execute_batch(
                "CREATE TABLE audit_events(
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    ts TEXT DEFAULT CURRENT_TIMESTAMP,
                    kind TEXT NOT NULL,
                    actor TEXT,
                    target_hash TEXT,
                    status TEXT,
                    detail_hash TEXT,
                    tenant_id TEXT NOT NULL DEFAULT 'global',
                    prev_hash TEXT);
                 PRAGMA busy_timeout=0;",
            )
            .expect("writer schema");
        // Holder takes a read tx and reads (acquiring the SHARED lock) — COMMIT on
        // the writer then cannot get the EXCLUSIVE lock it needs.
        holder.execute_batch("BEGIN;").expect("holder read tx");
        holder
            .query_row("SELECT COUNT(*) FROM audit_events", [], |r| {
                r.get::<_, i64>(0)
            })
            .expect("holder read acquires SHARED");

        let before = audit_commit_failures();
        record_tenant(
            &writer,
            AuditKind::Ingest,
            "api",
            "subject",
            AuditStatus::Ok,
            "d",
            "team-a",
        );
        assert!(
            audit_commit_failures() > before,
            "a failed settle must bump the /health counter"
        );
        // A failed settlement must not be acknowledged as a durable row.
        holder.execute_batch("ROLLBACK;").expect("release holder");
    }

    #[test]
    fn settlement_commit_failure_never_acknowledges_or_leaks_transaction() {
        let tmp = tempfile::NamedTempFile::new().expect("temporary audit database");
        let writer = Connection::open(tmp.path()).expect("audit writer");
        writer
            .execute_batch(
                "PRAGMA journal_mode=DELETE;
                 PRAGMA busy_timeout=0;
                 CREATE TABLE audit_events(
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    ts TEXT DEFAULT CURRENT_TIMESTAMP,
                    kind TEXT NOT NULL, actor TEXT, target_hash TEXT,
                    status TEXT, detail_hash TEXT,
                    tenant_id TEXT NOT NULL DEFAULT 'global', prev_hash TEXT);
                 CREATE TABLE schema_meta(key TEXT PRIMARY KEY, value TEXT);",
            )
            .expect("temporary audit schema");
        let observer = Connection::open(tmp.path()).expect("independent audit observer");
        observer
            .execute_batch("BEGIN")
            .expect("hold reader transaction");
        let before: i64 = observer
            .query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))
            .expect("acquire shared lock");
        assert_eq!(before, 0);

        // The reader permits BEGIN IMMEDIATE and INSERT, but prevents COMMIT
        // from taking the exclusive lock in rollback-journal mode.
        let receipt = record_tenant(
            &writer,
            AuditKind::Workflow,
            "settlement-test",
            "run:1",
            AuditStatus::Error,
            "aborted",
            "acme",
        );
        let settled = writer.is_autocommit();
        let local_rows: i64 = writer
            .query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))
            .expect("writer row count");
        // Clean up before asserting: even the pre-repair failing test leaves
        // no pending transaction or lock behind.
        if !settled {
            assert_eq!(local_rows, 1, "INSERT reached the real COMMIT failure");
            writer.execute_batch("ROLLBACK").expect("fixture rollback");
            eprintln!(
                "active_after_failed_commit={} autocommit_after_rollback={}",
                !settled,
                writer.is_autocommit()
            );
        }
        observer.execute_batch("ROLLBACK").expect("release reader");
        let visible: i64 = observer
            .query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))
            .expect("independent committed row count");
        assert_eq!(visible, 0, "failed COMMIT has no durable audit evidence");
        assert!(read_head_pin(&observer).is_none());
        assert!(
            receipt.is_none() && settled,
            "failed COMMIT must return no ID and restore autocommit"
        );
    }

    // Exercise the legacy wrapper too: it must never acknowledge a failure
    // rejected by the shared checked writer.
    fn settlement_candidate(conn: &Connection) -> Option<i64> {
        record_tenant(
            conn,
            AuditKind::Workflow,
            "harness",
            "run:1",
            AuditStatus::Denied,
            "RunEnd:aborted",
            "acme",
        )
    }

    fn settlement_row_count(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))
            .expect("audit row count")
    }

    #[test]
    fn settlement_tip_read_failure_is_not_empty_chain() {
        let conn = db_with_meta();
        settlement_candidate(&conn).expect("seed audit row");
        // A stored BLOB cannot decode as the required timestamp String. The
        // table remains writable: this fails the tip read, not the INSERT.
        conn.execute("UPDATE audit_events SET ts = x'80'", [])
            .expect("synthetic unreadable tip");
        assert!(
            conn.query_row(
                "SELECT id, ts, kind, actor, target_hash, status, detail_hash, prev_hash
                 FROM audit_events ORDER BY id DESC LIMIT 1",
                [],
                map_full_row,
            )
            .is_err(),
            "fixture must fail the actual row decoder"
        );
        let receipt = settlement_candidate(&conn);
        assert!(receipt.is_none(), "unreadable tip cannot become genesis");
        assert!(conn.is_autocommit());
        assert_eq!(settlement_row_count(&conn), 1);
    }

    #[test]
    fn settlement_epoch_read_failure_cannot_downgrade() {
        let conn = db_with_meta();
        conn.execute(
            "INSERT INTO schema_meta(key, value) VALUES (?1, x'80')",
            params![EPOCH_META_KEY],
        )
        .expect("synthetic unreadable epoch");
        assert!(
            conn.query_row(
                "SELECT value FROM schema_meta WHERE key = ?1",
                params![EPOCH_META_KEY],
                |r| r.get::<_, String>(0),
            )
            .is_err(),
            "present epoch must fail decoding rather than appear absent"
        );
        let receipt = settlement_candidate(&conn);
        assert!(receipt.is_none(), "epoch read failure cannot select legacy");
        assert!(conn.is_autocommit());
        assert_eq!(settlement_row_count(&conn), 0);
    }

    #[test]
    fn settlement_pin_failure_rolls_back_row() {
        let conn = db_with_meta();
        settlement_candidate(&conn).expect("seed row and pin");
        let pin = read_head_pin(&conn).expect("seed pin");
        conn.execute_batch(
            "CREATE TRIGGER settlement_reject_pin BEFORE INSERT ON schema_meta
             WHEN NEW.key = 'audit_chain_head'
             BEGIN SELECT RAISE(ABORT, 'fixture_pin_refused'); END;",
        )
        .expect("pin-only failure trigger");
        let receipt = settlement_candidate(&conn);
        assert!(
            receipt.is_none(),
            "row without its required pin is not success"
        );
        assert!(conn.is_autocommit());
        assert_eq!(settlement_row_count(&conn), 1);
        assert_eq!(read_head_pin(&conn), Some(pin));
        assert!(verify_chain(&conn));
    }

    #[test]
    fn settlement_timestamp_read_failure_rolls_back_row() {
        let conn = db_with_meta();
        conn.execute_batch(
            "CREATE TRIGGER settlement_unreadable_timestamp AFTER INSERT ON audit_events
             BEGIN UPDATE audit_events SET ts = x'80' WHERE id = NEW.id; END;",
        )
        .expect("post-insert timestamp failure trigger");
        let receipt = settlement_candidate(&conn);
        assert!(
            receipt.is_none(),
            "unreadable timestamp cannot skip head pin"
        );
        assert!(conn.is_autocommit());
        assert_eq!(settlement_row_count(&conn), 0);
        assert!(read_head_pin(&conn).is_none());
    }

    #[test]
    fn settlement_insert_failure_preserves_chain_and_connection() {
        let conn = db_with_meta();
        settlement_candidate(&conn).expect("seed row and pin");
        let pin = read_head_pin(&conn).expect("seed pin");
        conn.execute_batch(
            "CREATE TRIGGER settlement_reject_row BEFORE INSERT ON audit_events
             BEGIN SELECT RAISE(ABORT, 'fixture_insert_refused'); END;",
        )
        .expect("insert failure trigger");
        assert!(settlement_candidate(&conn).is_none());
        assert!(conn.is_autocommit());
        assert_eq!(settlement_row_count(&conn), 1);
        assert_eq!(read_head_pin(&conn), Some(pin));
        assert!(verify_chain(&conn));
    }

    #[test]
    fn record_tenant_rollback_does_not_undo_caller_work() {
        // Negative path of the SAVEPOINT wrap: if the audit INSERT itself fails
        // (e.g. constraint violation), the savepoint rolls back ONLY the audit
        // work, not the caller's. We simulate failure by dropping the table
        // mid-call isn't feasible without a different schema, so we verify the
        // positive invariant instead: caller work before + after a successful
        // audit survives a commit. This test exists to pin the savepoint shape.
        //
        // F-03 (pass-3): the caller rows are written CHAINED (genesis NULL,
        // then backrefs) — a raw mid-chain NULL insert is now tamper by the
        // verify prefix rule, and production writers never produce one.
        let db = db();
        let tx = db.unchecked_transaction().unwrap();
        tx.execute(
            "INSERT INTO audit_events(kind, actor, target_hash, status, detail_hash, prev_hash)
             VALUES ('ingest', 'before', 'before-audit', 'ok', 'd', NULL)",
            [],
        )
        .unwrap();
        let before_link: Option<String> = tx
            .query_row(
                "SELECT ts, kind, actor, target_hash, prev_hash FROM audit_events WHERE actor = 'before'",
                [],
                |r| {
                    Ok(chain_link(
                        &r.get::<_, String>(0)?,
                        &r.get::<_, String>(1)?,
                        &r.get::<_, String>(2)?,
                        &r.get::<_, String>(3)?,
                        r.get::<_, Option<String>>(4)?.unwrap_or_default().as_str(),
                    ))
                },
            )
            .ok();
        record(
            &tx,
            AuditKind::Ingest,
            "api",
            "audit-event",
            AuditStatus::Ok,
            "d",
        );
        let audit_link: Option<String> = tx
            .query_row(
                "SELECT ts, kind, actor, target_hash, prev_hash FROM audit_events WHERE actor = 'api' ORDER BY id DESC LIMIT 1",
                // Selected by ACTOR, not by a recomputed `hash(...)`: `hash()` is
                // keyed when a chain key is installed and this test holds no key
                // gate, so a parallel key change made the lookup miss and the
                // assertion silently degraded. Actor is the stable identity.
                [],
                |r| {
                    Ok(chain_link(
                        &r.get::<_, String>(0)?,
                        &r.get::<_, String>(1)?,
                        &r.get::<_, String>(2)?,
                        &r.get::<_, String>(3)?,
                        r.get::<_, Option<String>>(4)?.unwrap_or_default().as_str(),
                    ))
                },
            )
            .ok();
        tx.execute(
            "INSERT INTO audit_events(kind, actor, target_hash, status, detail_hash, prev_hash)
             VALUES ('ingest', 'after', 'after-audit', 'ok', 'd', ?1)",
            params![audit_link],
        )
        .unwrap();
        let _ = before_link;
        tx.commit().unwrap();
        // v1.27.31: the manual chained insert above bypassed `record_tenant`,
        // so the head pin still points at the audit row — re-pin before
        // verify (the contract every production writer gets for free; manual
        // chained writes must re-pin or read as truncation/extension).
        refresh_head_pin(&db, &Scheme::Legacy);
        let count: i64 = db
            .query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count, 3,
            "caller work before + audit + caller work after all landed"
        );
        assert!(verify_chain(&db));
    }

    /// F-03 (pass-3): a NULL backref after the chain started is tamper —
    /// the prefix rule. Leading NULLs (pre-v1.1 rows / genesis) stay legal.
    #[test]
    fn hash_chain_rejects_mid_chain_null_backref() {
        let db = db();
        // Leading NULL (pre-v1.1-style) is fine…
        db.execute(
            "INSERT INTO audit_events(kind, actor, target_hash, status, detail_hash, prev_hash)
             VALUES ('ingest', 'api', 'legacy', 'ok', 'd', NULL)",
            [],
        )
        .unwrap();
        assert!(verify_chain(&db), "leading NULLs are the legal prefix");
        // …then chained rows (what record() writes)…
        record(&db, AuditKind::Ingest, "api", "c1", AuditStatus::Ok, "d1");
        record(&db, AuditKind::Ingest, "api", "c2", AuditStatus::Ok, "d2");
        assert!(verify_chain(&db), "NULL-prefix then chained rows verifies");
        // …then erase the LAST row's backref mid-chain (the tamper the rule
        // exists for — a prior non-NULL backref exists, so the prefix is over).
        //
        // Select the row by POSITION, never by recomputing `hash("c2")`:
        // `hash()` is keyed when a chain key is installed and this test holds
        // no key gate, so a parallel test installing or removing a key made
        // the lookup miss (or hit another row) and the tamper went undetected.
        // `ORDER BY id DESC LIMIT 1` is what "the last row" actually means.
        let last_id: i64 = db
            .query_row(
                "SELECT id FROM audit_events ORDER BY id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .expect("last row id");
        db.execute(
            "UPDATE audit_events SET prev_hash = NULL WHERE id = ?1",
            params![last_id],
        )
        .unwrap();
        assert!(
            !verify_chain(&db),
            "a NULL backref after the chain started must fail verify"
        );
    }

    /// S2-16 (pass-3): the retention prune (a) REFUSES to run on a chain that
    /// does not verify (no evidence laundering) and (b) records a prune event
    /// on the chain when it does run.
    #[test]
    fn retention_prune_refuses_tampered_chain_and_records_event() {
        let db = db();
        // The prune sweeps orphaned `recall_traces` rows — the minimal audit
        // fixture needs the table to exist.
        db.execute(
            "CREATE TABLE recall_traces(audit_id INTEGER PRIMARY KEY, trace_json TEXT)",
            [],
        )
        .unwrap();
        // Two old rows + one fresh, chained via record().
        db.execute_batch(
            "INSERT INTO audit_events(id, ts, kind, actor, target_hash, status, detail_hash, prev_hash)
             VALUES (1, datetime('now', '-30 days'), 'recall', 'alice', 't1', 'ok', 'd1', NULL),
                    (2, datetime('now', '-29 days'), 'recall', 'alice', 't2', 'ok', 'd2', NULL);",
        )
        .unwrap();
        record(
            &db,
            AuditKind::Recall,
            "alice",
            "fresh",
            AuditStatus::Ok,
            "d3",
        );
        // Tamper: break row 2's fields without re-chaining.
        db.execute("UPDATE audit_events SET actor = 'mallory' WHERE id = 2", [])
            .unwrap();
        assert!(!verify_chain(&db));
        // The prune must REFUSE (rows preserved).
        assert!(
            prune_audit_retention(&db, 7).is_none(),
            "tampered chain: prune refused"
        );
        let count: i64 = db
            .query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 3, "nothing pruned on a tampered chain");

        // Repair (restore the field) → prune runs and records its event.
        db.execute("UPDATE audit_events SET actor = 'alice' WHERE id = 2", [])
            .unwrap();
        assert!(verify_chain(&db));
        let pruned = prune_audit_retention(&db, 7).expect("prune on healthy chain");
        assert_eq!(pruned, 2, "the two old rows expired");
        let events: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE kind = 'retention'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(events, 1, "the prune wrote its evidence row");
        // And the re-anchored chain still verifies (event chains from genesis).
        assert!(verify_chain(&db), "chain verifies after prune + event");
    }

    #[test]
    fn hash_chain_rejects_tampered_kind() {
        let db = db();
        record(&db, AuditKind::Ingest, "api", "c1", AuditStatus::Ok, "d1");
        record(&db, AuditKind::Ingest, "api", "c2", AuditStatus::Ok, "d2");
        assert!(verify_chain(&db));
        // Rewrite row 1's kind without updating row 2's prev_hash → link breaks.
        let _ = db.execute("UPDATE audit_events SET kind = 'webhook' WHERE id = 1", []);
        assert!(!verify_chain(&db), "rewriting a field must break the chain");
    }

    #[test]
    fn tenant_filter_is_enforced_at_sql_layer() {
        let db = db();
        record_tenant(
            &db,
            AuditKind::Ingest,
            "api",
            "c1",
            AuditStatus::Ok,
            "d1",
            "team-a",
        );
        record_tenant(
            &db,
            AuditKind::Ingest,
            "api",
            "c2",
            AuditStatus::Ok,
            "d2",
            "team-b",
        );
        let a = recent_tenant(&db, None, Some("team-a"), None, None, 100, 0).unwrap();
        let b = recent_tenant(&db, None, Some("team-b"), None, None, 100, 0).unwrap();
        assert_eq!(a.len(), 1, "team-a must see only its own row");
        assert_eq!(b.len(), 1, "team-b must see only its own row");
        assert_eq!(a[0].tenant_id, "team-a");
        assert_eq!(b[0].tenant_id, "team-b");
        // Forgetting the tenant filter returns both — proves the SQL filter is
        // the enforcement point, not a missing app-level guard.
        let all = recent_tenant(&db, None, None, None, None, 100, 0).unwrap();
        assert_eq!(all.len(), 2);
    }

    /// pagination cursor. `limit`+`offset` pages the newest-first
    /// stream with no overlap and no dupes; an offset past the end is empty.
    #[test]
    fn recent_tenant_paginates_with_offset() {
        let db = db();
        for i in 0..10 {
            record_tenant(
                &db,
                AuditKind::Ingest,
                "api",
                &format!("c{i}"),
                AuditStatus::Ok,
                "d",
                "team-a",
            );
        }
        // Newest-first: id 10 (the last insert) is page[0].
        let page0 = recent_tenant(&db, None, Some("team-a"), None, None, 4, 0).unwrap();
        let page1 = recent_tenant(&db, None, Some("team-a"), None, None, 4, 4).unwrap();
        let page2 = recent_tenant(&db, None, Some("team-a"), None, None, 4, 8).unwrap();
        assert_eq!(page0.len(), 4);
        assert_eq!(page1.len(), 4);
        assert_eq!(page2.len(), 2);
        // Newest-first must be asserted on the ORDER KEY, not inferred from
        // `target_hash`. This test used `page0[0].target_hash >
        // page0[3].target_hash` as a proxy for descending id — which only held
        // while `hash()` was plain SHA-256 and the targets were `c0..c9` (their
        // digests happened to sort the same way). `hash()` is now KEYED when a
        // chain key is installed, so the digest order is unrelated to insertion
        // order and the proxy became a coin flip that flaked whenever a
        // parallel test installed a key. Assert the real property: descending id.
        assert!(
            page0[0].id > page0[3].id,
            "newest-first: page0[0] must be the higher id (got {} vs {})",
            page0[0].id,
            page0[3].id
        );
        // No overlap / no gap: the union is exactly ids 1..=10.
        let all: Vec<i64> = [page0, page1, page2]
            .into_iter()
            .flatten()
            .map(|r| r.id)
            .collect();
        assert_eq!(all.len(), 10);
        let mut sorted = all.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (1..=10).collect::<Vec<i64>>());
        // Offset past the end → empty, not an error.
        assert!(
            recent_tenant(&db, None, Some("team-a"), None, None, 4, 40)
                .unwrap()
                .is_empty()
        );
    }

    /// concurrent autocommit `record_tenant` callers must not
    /// fork the chain. Two pooled connections, a `Barrier` so both threads
    /// reach the audit call simultaneously, then verify the chain holds.
    /// Mirrors the proven `concurrent_refresh_serializes_exactly_one_winner`
    /// shape in `auth/revocation.rs`. Before the IMMEDIATE fix, this test
    /// failed intermittently under load (both threads read the same tip,
    /// both INSERTed the same prev_hash).
    #[test]
    fn audit_chain_survives_concurrent_autocommit_writers() {
        use r2d2::Pool;
        use r2d2_sqlite::SqliteConnectionManager;
        use std::sync::{Arc, Barrier};
        use std::thread;

        let db_file = tempfile::NamedTempFile::new().unwrap();
        // Set a busy_timeout on every connection so concurrent writers wait
        // rather than fail. Done in `with_init` so it applies to every pooled
        // connection, not just the schema-creating one.
        let mgr = SqliteConnectionManager::file(db_file.path()).with_init(|c| {
            c.execute_batch("PRAGMA busy_timeout=5000;")?;
            Ok(())
        });
        let pool: Pool<SqliteConnectionManager> = Pool::builder().max_size(8).build(mgr).unwrap();
        {
            let c = pool.get().unwrap();
            c.execute_batch(
                "CREATE TABLE IF NOT EXISTS audit_events (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    ts TEXT DEFAULT CURRENT_TIMESTAMP,
                    kind TEXT NOT NULL,
                    actor TEXT,
                    target_hash TEXT,
                    status TEXT,
                    detail_hash TEXT,
                    tenant_id TEXT,
                    prev_hash TEXT
                );",
            )
            .unwrap();
        }

        let barrier = Arc::new(Barrier::new(2));
        let mut handles = Vec::new();
        for i in 0..2 {
            let p = pool.clone();
            let b = barrier.clone();
            handles.push(thread::spawn(move || {
                let conn = p.get().unwrap();
                // Synchronize the start so both threads race the tip read.
                b.wait();
                for j in 0..10 {
                    record(
                        &conn,
                        AuditKind::Ingest,
                        &format!("t{i}"),
                        &format!("c{i}-{j}"),
                        AuditStatus::Ok,
                        "d",
                    );
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        let c = pool.get().unwrap();
        let count: i64 = c
            .query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 20, "all 20 audit rows landed");
        assert!(
            verify_chain(&c),
            "chain verifies after concurrent autocommit writers — no fork"
        );
    }

    /// REGRESSION (v1.29.22): a chain verify must NOT read the tip and the
    /// head pin from two different snapshots.
    ///
    /// `verify_chain_inner` walks `audit_events` (the tip) and then reads
    /// `schema_meta` (the head pin). In AUTOCOMMIT each statement is its own
    /// read transaction, so on a WAL database a concurrent append that commits
    /// between them leaves the pin NEWER than the tip the walk saw — a false
    /// "rows were added or removed outside the audited write paths" against a
    /// chain that is perfectly intact. This is the exact production symptom: the
    /// periodic WAL chain-check loop logged 42 such mismatches over ~20h, EVERY
    /// one with the pin AHEAD of the tip (never behind — behind would mean real
    /// loss), the gap being however many appends landed mid-walk.
    ///
    /// With the snapshot fix the verify is immune by construction: an append
    /// that commits after the verify's snapshot is simply not visible to the
    /// in-flight verify, and the NEXT tick sees it together with its own pin.
    #[test]
    fn chain_verify_is_immune_to_a_concurrent_append() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::thread;

        fn open_conn(p: &std::path::Path) -> Connection {
            let c = Connection::open(p).expect("open");
            c.execute_batch(
                "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA busy_timeout=5000;",
            )
            .unwrap();
            c
        }

        let path = std::env::temp_dir().join(format!("audit_race_{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));

        let setup = open_conn(&path);
        setup
            .execute_batch(
                "CREATE TABLE audit_events(
                   id INTEGER PRIMARY KEY AUTOINCREMENT,
                   ts TEXT DEFAULT CURRENT_TIMESTAMP,
                   kind TEXT NOT NULL,
                   actor TEXT,
                   target_hash TEXT,
                   status TEXT,
                   detail_hash TEXT,
                   tenant_id TEXT NOT NULL DEFAULT 'global',
                   prev_hash TEXT);
                 CREATE TABLE schema_meta(key TEXT PRIMARY KEY, value TEXT);",
            )
            .unwrap();
        // Seed a chain big enough that the walk takes real wall-clock time, so
        // an append is very likely to commit inside the verify's read window.
        for i in 0..3000 {
            record(
                &setup,
                AuditKind::Ingest,
                "api",
                &format!("c{i}"),
                AuditStatus::Ok,
                "d",
            );
        }
        assert!(
            verify_chain(&setup),
            "the seeded chain verifies before any race"
        );
        drop(setup);

        let stop = Arc::new(AtomicBool::new(false));
        let writer_path = path.clone();
        let writer_stop = stop.clone();
        let writer = thread::spawn(move || {
            let w = open_conn(&writer_path);
            let mut i = 0u64;
            // Hammer appends for the whole window the verifier runs in. Each
            // `record` commits the row AND its pin in one tx — the chain stays
            // internally consistent; only the verifier's two-snapshot read is
            // at risk. The cap bounds the chain so a verify never walks an
            // unbounded table (the race is caught on the FIRST verify; this
            // budget only keeps later verifies in the race too).
            while !writer_stop.load(Ordering::Relaxed) && i < 30_000 {
                record(
                    &w,
                    AuditKind::Auth,
                    "api",
                    &format!("w{i}"),
                    AuditStatus::Ok,
                    "d",
                );
                i += 1;
            }
        });

        let v = open_conn(&path);
        for n in 0..15 {
            assert!(
                verify_chain(&v),
                "verify #{n}: a concurrent append must not make an intact chain \
                 read as tampered (the tip and pin must come from one snapshot)"
            );
        }
        stop.store(true, Ordering::Relaxed);
        writer.join().unwrap();

        // And once the writer is done, the chain still verifies.
        assert!(
            verify_chain(&v),
            "chain verifies after the concurrent writer stops"
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    /// F-23 (v1.27.26 "Notarize"): a failed BEGIN IMMEDIATE must NOT fall
    /// through to an unserialized tip-read + INSERT. Two writers reading the
    /// same tip under autocommit would insert rows sharing a `prev_hash` —
    /// a permanent fork `verify_chain` reports forever. Dropping the row is
    /// fail-safe (a missing entry reads as a gap, never as a forge), and the
    /// failure is surfaced via `audit_commit_failures` + `warn!`.
    #[test]
    fn begin_immediate_failure_skips_and_warns_not_forks() {
        // File-backed DB so a second connection can hold a conflicting write
        // lock (an in-memory DB is private to its connection).
        let dir = std::env::temp_dir();
        let path = dir.join(format!("audit_f23_{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let blocker = Connection::open(&path).expect("open blocker connection");
        blocker
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS audit_events(
                   id INTEGER PRIMARY KEY AUTOINCREMENT,
                   ts TEXT DEFAULT CURRENT_TIMESTAMP,
                   kind TEXT NOT NULL,
                   actor TEXT,
                   target_hash TEXT,
                   status TEXT,
                   detail_hash TEXT,
                   tenant_id TEXT NOT NULL DEFAULT 'global',
                   prev_hash TEXT
                 );",
            )
            .expect("create audit_events");
        // The writer connection must FAIL its BEGIN IMMEDIATE: busy_timeout 0
        // so a held write lock makes BEGIN IMMEDIATE error immediately.
        let writer = Connection::open(&path).expect("open writer connection");
        writer
            .execute_batch("PRAGMA busy_timeout=0;")
            .expect("set busy_timeout");

        // Seed one row first so a tip exists.
        record(
            &writer,
            AuditKind::Auth,
            "api",
            "pre",
            AuditStatus::Ok,
            "seed",
        )
        .expect("seed row should write (no lock held yet)");

        // Now the blocker takes an IMMEDIATE write lock and holds it.
        blocker
            .execute_batch("BEGIN IMMEDIATE;")
            .expect("blocker holds the write lock");
        let before = audit_commit_failures();
        let id = record(&writer, AuditKind::Auth, "api", "t", AuditStatus::Ok, "d");
        blocker
            .execute_batch("ROLLBACK;")
            .expect("release the blocker lock");

        assert!(id.is_none(), "a forking write must be refused, not emitted");
        assert!(
            audit_commit_failures() > before,
            "the refused audit write must bump the /health counter"
        );
        let count: i64 = writer
            .query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "only the seed row exists — no partial fork row");
        assert!(
            verify_chain(&writer),
            "the chain still verifies after the refused write"
        );
        let _ = std::fs::remove_file(&path);
    }

    // ── v1.27.31 "AuditRepair" ────────────────────────────────────────────

    /// audit_events + schema_meta (the pin/epoch live in schema_meta; the bare
    /// `db()` helper exercises the missing-table tolerance instead).
    fn db_with_meta() -> Connection {
        let db = db();
        db.execute_batch("CREATE TABLE schema_meta(key TEXT PRIMARY KEY, value TEXT);")
            .expect("create schema_meta");
        db
    }

    /// Every keyed test holds this for its whole scenario: the chain key is
    /// process-global, and a key visible to a concurrently-running fresh-DB
    /// bootstrap would flip that test's epoch. Hold → set → … → clear.
    fn key_gate() -> std::sync::MutexGuard<'static, ()> {
        key_gate_for_test()
    }

    /// M2: the hmac256 link commits the FULL row — mutating any one field of a
    /// prior row (ts, kind, actor, target_hash, status, detail_hash) breaks
    /// the next row's backref, and renumbering `id` breaks it too.
    #[test]
    fn chain_hash_includes_all_fields() {
        let _gate = key_gate();
        set_chain_key_for_test(Some([7u8; 32]));
        let tampers: [(&str, &str); 7] = [
            (
                "ts",
                "UPDATE audit_events SET ts = '1999-01-01 00:00:00' WHERE id = 1",
            ),
            (
                "kind",
                "UPDATE audit_events SET kind = 'webhook' WHERE id = 1",
            ),
            (
                "actor",
                "UPDATE audit_events SET actor = 'mallory' WHERE id = 1",
            ),
            (
                "target_hash",
                "UPDATE audit_events SET target_hash = 'deadbeef' WHERE id = 1",
            ),
            (
                "status",
                "UPDATE audit_events SET status = 'denied' WHERE id = 1",
            ),
            (
                "detail_hash",
                "UPDATE audit_events SET detail_hash = 'deadbeef' WHERE id = 1",
            ),
            ("id", "UPDATE audit_events SET id = 999 WHERE id = 1"),
        ];
        for (label, sql) in tampers {
            let db = db_with_meta();
            assert!(
                bootstrap_epoch(&db),
                "fresh db bootstraps to hmac256 with a key"
            );
            record(&db, AuditKind::Ingest, "api", "c1", AuditStatus::Ok, "d1");
            record(&db, AuditKind::Ingest, "api", "c2", AuditStatus::Ok, "d2");
            record(&db, AuditKind::Ingest, "api", "c3", AuditStatus::Ok, "d3");
            assert!(verify_chain(&db), "[{label}] unmodified chain must verify");
            db.execute(sql, []).unwrap();
            assert!(
                !verify_chain(&db),
                "[{label}] mutating a committed field must break verify"
            );
        }
        set_chain_key_for_test(None);
    }

    /// M6: a reconstructed chain from attacker-chosen content cannot verify —
    /// neither by recomputing the 8-field links with PLAIN SHA-256 (no key) nor
    /// by signing with the WRONG key.
    #[test]
    fn keyed_chain_rejects_attacker_content() {
        let _gate = key_gate();
        set_chain_key_for_test(Some([7u8; 32]));
        let db = db_with_meta();
        assert!(bootstrap_epoch(&db));
        record(&db, AuditKind::Ingest, "api", "c1", AuditStatus::Ok, "d1");
        record(&db, AuditKind::Ingest, "api", "c2", AuditStatus::Ok, "d2");
        record(&db, AuditKind::Ingest, "api", "c3", AuditStatus::Ok, "d3");
        assert!(verify_chain(&db));

        // The attacker's link constructions — both computed from the REAL
        // (tampered) row 2, so the only thing missing is the key:
        // (a) plain SHA-256 over the same length-prefixed 8-field payload
        //     `chain_link_hmac` feeds the MAC;
        // (b) HMAC with the WRONG key.
        let tampered_row = |actor: &str| {
            db.query_row(
                "SELECT id, ts, kind, actor, target_hash, status, detail_hash, prev_hash \
                  FROM audit_events WHERE id = 2",
                [],
                |r| {
                    let mut row = map_full_row(r).unwrap();
                    row.actor = actor.to_string();
                    Ok(row)
                },
            )
            .unwrap()
        };
        let unkeyed_link = |row: &ChainRowFull| {
            let mut h = Sha256::new();
            h.update(row.id.to_le_bytes());
            for f in [
                row.ts.as_bytes(),
                row.kind.as_bytes(),
                row.actor.as_bytes(),
                row.target_hash.as_bytes(),
                row.status.as_bytes(),
                row.detail_hash.as_bytes(),
                row.prev_hash.as_deref().unwrap_or("").as_bytes(),
            ] {
                h.update((f.len() as u64).to_le_bytes());
                h.update(f);
            }
            hex_encode(&h.finalize())
        };

        // Attacker rewrites row 2's actor and re-signs row 3's backref with
        // each keyless construction. The walk compares stored backrefs
        // against the KEYED link — both forgeries must fail.
        let mallory = tampered_row("mallory");
        for forge in [
            unkeyed_link(&mallory),
            chain_link_hmac(&crate::testkeys::unit_hmac_key(9), &mallory),
        ] {
            db.execute("UPDATE audit_events SET actor = 'mallory' WHERE id = 2", [])
                .unwrap();
            db.execute(
                "UPDATE audit_events SET prev_hash = ?1 WHERE id = 3",
                params![forge],
            )
            .unwrap();
            assert!(
                !verify_chain(&db),
                "an attacker-recomputed link must never verify"
            );
            // Reset for the next variant.
            db.execute("UPDATE audit_events SET actor = 'api' WHERE id = 2", [])
                .unwrap();
        }
        // The pin also survived pointing at the true head — restore the true
        // backref and the chain verifies again.
        let true_link = {
            let row = db
                .query_row(
                    "SELECT id, ts, kind, actor, target_hash, status, detail_hash, prev_hash \
                      FROM audit_events WHERE id = 2",
                    [],
                    map_full_row,
                )
                .unwrap();
            row_link(&Scheme::Hmac(std::sync::Arc::new([7u8; 32])), &row)
        };
        db.execute(
            "UPDATE audit_events SET prev_hash = ?1 WHERE id = 3",
            params![true_link],
        )
        .unwrap();
        assert!(
            verify_chain(&db),
            "the true keyed link restores verification"
        );
        set_chain_key_for_test(None);
    }

    /// M3: every committed row re-pins the head — id, hash (== `chain_head`),
    /// epoch — in the same transaction.
    #[test]
    fn head_pin_updates_on_commit() {
        let db = db_with_meta();
        assert!(read_head_pin(&db).is_none(), "fresh DB carries no pin");
        record(&db, AuditKind::Ingest, "api", "c1", AuditStatus::Ok, "d1");
        let pin1 = read_head_pin(&db).expect("pin after first write");
        assert_eq!(pin1.id, 1);
        assert_eq!(pin1.hash, chain_head(&db).expect("head"));
        assert_eq!(pin1.epoch, EPOCH_SHA7);
        record(&db, AuditKind::Ingest, "api", "c2", AuditStatus::Ok, "d2");
        let pin2 = read_head_pin(&db).expect("pin after second write");
        assert_eq!(pin2.id, 2, "the pin follows the tip");
        assert_eq!(pin2.hash, chain_head(&db).expect("head"));
        assert_ne!(pin1.hash, pin2.hash);
        assert!(verify_chain(&db));
    }

    /// M3: truncation of an internally-valid chain is detected by the stale
    /// pin (the prefix still walks clean — the pin is what notices the tail is
    /// gone). Removing the pin removes the detection (proving the source).
    #[test]
    fn verify_detects_truncation_via_stale_pin() {
        let db = db_with_meta();
        record(&db, AuditKind::Ingest, "api", "c1", AuditStatus::Ok, "d1");
        record(&db, AuditKind::Ingest, "api", "c2", AuditStatus::Ok, "d2");
        record(&db, AuditKind::Ingest, "api", "c3", AuditStatus::Ok, "d3");
        assert!(verify_chain(&db));
        // Truncate the tail (a row deleted outside every audited path).
        db.execute("DELETE FROM audit_events WHERE id = 3", [])
            .unwrap();
        assert!(
            !verify_chain(&db),
            "truncation must be detected via the stale pin"
        );
        // Control: drop the pin and the same (shorter) chain verifies — the
        // pin is the detector, not the walk.
        db.execute(
            &format!("DELETE FROM schema_meta WHERE key = '{HEAD_PIN_META_KEY}'"),
            [],
        )
        .unwrap();
        assert!(verify_chain(&db));
    }

    /// Fail-closed posture of an hmac256 epoch whose key is unavailable in
    /// this process: writes are refused (never unkeyed), verify is not-ok.
    #[test]
    fn hmac_epoch_without_key_fails_closed() {
        let _gate = key_gate();
        set_chain_key_for_test(Some([7u8; 32]));
        let db = db_with_meta();
        assert!(bootstrap_epoch(&db));
        assert!(record(&db, AuditKind::Ingest, "api", "c1", AuditStatus::Ok, "d1").is_some());
        set_chain_key_for_test(None);
        let before = audit_commit_failures();
        assert!(record(&db, AuditKind::Ingest, "api", "c2", AuditStatus::Ok, "d2").is_none());
        assert!(
            audit_commit_failures() > before,
            "the refused write must bump the /health counter"
        );
        assert!(
            !verify_chain(&db),
            "cannot attest a keyed chain without the key"
        );
        set_chain_key_for_test(Some([7u8; 32]));
        assert!(verify_chain(&db), "key restored — chain attests again");
        set_chain_key_for_test(None);
    }

    /// The genesis pin: written by the FIRST append in the same tx, carrying a
    /// MAC under the keyed epoch; anchored at the chain's first row so
    /// deleting prefix rows is detectable (the head pin alone cannot see it —
    /// survivor links chain from immediate predecessors only, and the tip
    /// never moves when the OLDEST rows go).
    #[test]
    fn genesis_pin_written_on_first_append_and_detects_head_deletion() {
        let _gate = key_gate();
        set_chain_key_for_test(Some([7u8; 32]));
        let db = db_with_meta();
        assert!(bootstrap_epoch(&db));
        for i in 1..=6 {
            record(
                &db,
                AuditKind::Ingest,
                "api",
                &format!("c{i}"),
                AuditStatus::Ok,
                "d",
            );
        }
        let genesis = read_genesis_pin(&db).expect("genesis pin after first append");
        assert_eq!(genesis.id, 1, "anchored at the first row");
        assert_eq!(genesis.epoch, EPOCH_HMAC);
        assert!(
            genesis.mac.is_some(),
            "keyed-epoch pins carry a MAC from the first append"
        );
        assert!(read_head_pin(&db).unwrap().mac.is_some());
        assert!(verify_chain(&db));

        // Head deletion: the oldest rows vanish; the tip (and head pin) never
        // move. The genesis pin is what disagrees.
        db.execute("DELETE FROM audit_events WHERE id <= 3", [])
            .unwrap();
        assert!(
            !verify_chain(&db),
            "prefix deletion must trip the genesis pin"
        );
        set_chain_key_for_test(None);
    }

    /// The pin-forgery escalation: after a prefix deletion the attacker
    /// rewrites the genesis pin to bless the new first row. The MAC makes the
    /// rewrite worthless without the key — including a fully-specified
    /// forged JSON with the right id and hash but a bogus MAC, and the
    /// edge of a mac-less pin (stripped) under the keyed epoch.
    #[test]
    fn forged_or_stripped_genesis_pin_fails_verification() {
        let _gate = key_gate();
        set_chain_key_for_test(Some([7u8; 32]));
        let db = db_with_meta();
        assert!(bootstrap_epoch(&db));
        for i in 1..=4 {
            record(
                &db,
                AuditKind::Ingest,
                "api",
                &format!("c{i}"),
                AuditStatus::Ok,
                "d",
            );
        }
        assert!(verify_chain(&db));
        db.execute("DELETE FROM audit_events WHERE id = 1", [])
            .unwrap();
        // Forged pin: correct new-first row id (2), its TRUE link hash (copied
        // from the live head computation for row 2 via the walk is overkill —
        // any hash does, since the MAC is what fails), bogus MAC.
        let forged = format!(
            "{{\"id\":2,\"hash\":\"{}\",\"epoch\":\"hmac256\",\"mac\":\"{}\"}}",
            "a".repeat(64),
            "b".repeat(64)
        );
        db.execute(
            "INSERT INTO schema_meta(key, value) VALUES ('audit_chain_genesis', ?1)
             ON CONFLICT(key) DO UPDATE SET value = ?1",
            [&forged],
        )
        .unwrap();
        assert!(!verify_chain(&db), "bogus MAC must fail");
        // Stripped pin (mac field removed) under the keyed epoch: fail closed.
        let stripped = format!(
            "{{\"id\":2,\"hash\":\"{}\",\"epoch\":\"hmac256\"}}",
            "a".repeat(64)
        );
        db.execute(
            "INSERT INTO schema_meta(key, value) VALUES ('audit_chain_genesis', ?1)
             ON CONFLICT(key) DO UPDATE SET value = ?1",
            [&stripped],
        )
        .unwrap();
        assert!(
            !verify_chain(&db),
            "a mac-less pin on a keyed chain must fail closed"
        );
        // Pin-row removal: a non-empty keyed chain always carries both pins.
        db.execute(
            "DELETE FROM schema_meta WHERE key = 'audit_chain_genesis'",
            [],
        )
        .unwrap();
        assert!(
            !verify_chain(&db),
            "pin removal on a live keyed chain must fail"
        );
        set_chain_key_for_test(None);
    }

    /// The boot upgrade: an EXISTING keyed chain written before pins carried
    /// MACs (or a genesis) gets both — and a chain that does not verify is
    /// left verbatim (the laundering guard applies to pins too).
    #[test]
    fn upgrade_pins_stamps_old_chains_and_refuses_broken_ones() {
        let _gate = key_gate();
        set_chain_key_for_test(Some([7u8; 32]));
        let db = db_with_meta();
        assert!(bootstrap_epoch(&db));
        for i in 1..=3 {
            record(
                &db,
                AuditKind::Ingest,
                "api",
                &format!("c{i}"),
                AuditStatus::Ok,
                "d",
            );
        }
        // Simulate the pre-upgrade shape: strip MACs + drop the genesis pin.
        for key in [HEAD_PIN_META_KEY, GENESIS_PIN_META_KEY] {
            if let Some(pin) = read_pin(&db, key) {
                let bare = HeadPin { mac: None, ..pin };
                let json = serde_json::to_string(&bare).unwrap();
                db.execute(
                    "INSERT INTO schema_meta(key, value) VALUES (?1, ?2)
                     ON CONFLICT(key) DO UPDATE SET value = ?2",
                    params![key, json],
                )
                .unwrap();
            }
        }
        db.execute(
            "DELETE FROM schema_meta WHERE key = 'audit_chain_genesis'",
            [],
        )
        .unwrap();
        // The stripped head pin still fails strict verify (mac required) —
        // this is what makes the upgrade mandatory at boot.
        assert!(!verify_chain(&db));
        assert!(
            upgrade_pins(&db),
            "the upgrade must stamp the missing pieces"
        );
        assert!(read_genesis_pin(&db).is_some());
        assert!(read_head_pin(&db).unwrap().mac.is_some());
        assert!(read_genesis_pin(&db).unwrap().mac.is_some());
        assert!(
            verify_chain(&db),
            "post-upgrade the chain verifies strictly"
        );
        assert!(!upgrade_pins(&db), "second boot: fast path, nothing to do");

        // The laundering guard: a tampered chain is refused verbatim.
        let db2 = db_with_meta();
        assert!(bootstrap_epoch(&db2));
        for i in 1..=3 {
            record(
                &db2,
                AuditKind::Ingest,
                "api",
                &format!("c{i}"),
                AuditStatus::Ok,
                "d",
            );
        }
        db2.execute("UPDATE audit_events SET actor = 'mallory' WHERE id = 2", [])
            .unwrap();
        assert!(!verify_chain(&db2));
        assert!(
            !upgrade_pins(&db2),
            "a broken chain must not be pin-upgraded"
        );
        assert!(
            !verify_chain(&db2),
            "the broken chain still fails — nothing was re-blessed"
        );
        // The genesis stamped at FIRST APPEND is authentic (pre-tamper) and
        // stays; what the guard refuses is STAMPING onto a broken chain —
        // proven by stripping db2's pins and showing the upgrade will not
        // restore them while the chain is broken.
        db2.execute(
            "DELETE FROM schema_meta WHERE key = 'audit_chain_genesis'",
            [],
        )
        .unwrap();
        db2.execute(
            "UPDATE schema_meta SET value = json_set(value, '$.mac', json('null')) \
             WHERE key = 'audit_chain_head'",
            [],
        )
        .unwrap();
        assert!(!upgrade_pins(&db2), "still refused with pins stripped");
        assert!(
            read_genesis_pin(&db2).is_none(),
            "the upgrade must not stamp a genesis onto the broken chain"
        );
        set_chain_key_for_test(None);
    }

    /// The retention prune moves the genesis with the re-anchor: pruning the
    /// oldest rows is the ONE legitimate head-deletion, and the re-anchored
    /// chain verifies WITH its genesis re-stamped at the new oldest survivor.
    /// (Old rows are seeded as a NULL-prefix run with aged timestamps —
    /// editing a live row's `ts` would change its link and rightly refuse.)
    #[test]
    fn prune_reanchors_genesis_with_the_new_oldest_survivor() {
        let _gate = key_gate();
        set_chain_key_for_test(Some([7u8; 32]));
        let db = db_with_meta();
        // The prune sweeps orphaned `recall_traces` rows — the minimal audit
        // fixture needs the table to exist (same as the sibling prune test).
        db.execute(
            "CREATE TABLE recall_traces(audit_id INTEGER PRIMARY KEY, trace_json TEXT)",
            [],
        )
        .unwrap();
        assert!(bootstrap_epoch(&db));
        // Aged NULL-prefix rows (the pre-chain history a long-running
        // deployment carries), then fresh chained rows on top.
        db.execute_batch(
            "INSERT INTO audit_events(id, ts, kind, actor, target_hash, status, detail_hash, prev_hash)
             VALUES (1, datetime('now', '-40 days'), 'recall', 'alice', 't1', 'ok', 'd1', NULL),
                    (2, datetime('now', '-39 days'), 'recall', 'alice', 't2', 'ok', 'd2', NULL);",
        )
        .unwrap();
        // Pin the seeded extent before growing it (the seeded chain has no
        // pins; the upgrade path is what an existing deployment would run —
        // exercised explicitly here so the grow path starts pinned).
        assert!(upgrade_pins(&db), "seeded extent gets its pins stamped");
        for i in 1..=2 {
            record(
                &db,
                AuditKind::Ingest,
                "api",
                &format!("c{i}"),
                AuditStatus::Ok,
                "d",
            );
        }
        assert!(verify_chain(&db));
        assert_eq!(read_genesis_pin(&db).unwrap().id, 1);
        assert!(
            db.query_row(
                "SELECT COUNT(*) FROM audit_events WHERE ts < datetime('now', '-30 days')",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
                >= 2,
            "fixture sanity: the aged rows are expired under a 30-day window"
        );

        let pruned = prune_audit_retention(&db, 30).expect("prune on healthy keyed chain");
        assert_eq!(pruned, 2, "the two aged rows expired");
        assert!(
            verify_chain(&db),
            "the pruned chain verifies with its new genesis"
        );
        let genesis = read_genesis_pin(&db).expect("genesis re-stamped by the prune");
        assert_eq!(
            first_row(&db).map(|r| r.id),
            Some(genesis.id),
            "the genesis follows the new oldest survivor"
        );
        assert!(genesis.id > 2, "the aged rows are gone from the pin's view");
        set_chain_key_for_test(None);
    }

    /// An UNKEYED epoch's genesis is structural (no key exists to MAC
    /// with): prefix deletion is detected by id+hash alone, pin rewriting is
    /// not (the documented unkeyed ceiling — the keyed epoch is the upgrade
    /// path). No key installed → the first append stamps the unkeyed `sha7`
    /// epoch.
    #[test]
    fn legacy_genesis_is_structural_only() {
        let db = db_with_meta(); // no key installed → unkeyed (sha7) epoch
        for i in 1..=3 {
            record(
                &db,
                AuditKind::Ingest,
                "api",
                &format!("c{i}"),
                AuditStatus::Ok,
                "d",
            );
        }
        let genesis = read_genesis_pin(&db).expect("legacy genesis stamped at append");
        assert!(genesis.mac.is_none(), "no key, no MAC — structural only");
        assert!(verify_chain(&db));
        db.execute("DELETE FROM audit_events WHERE id = 1", [])
            .unwrap();
        assert!(
            !verify_chain(&db),
            "structural detection still catches deletion"
        );
    }

    /// M6 re-anchor: the legacy chain replays under hmac256, keeps its leading
    /// NULL prefix, verifies, continues to grow under the keyed links, and the
    /// second run is an idempotent no-op.
    #[test]
    fn reanchor_replays_legacy_chain_under_hmac() {
        let _gate = key_gate();
        // Legacy chain (no key installed → legacy epoch default).
        let db = db_with_meta();
        db.execute(
            "INSERT INTO audit_events(kind, actor, target_hash, status, detail_hash, prev_hash)
             VALUES ('ingest', 'legacy-api', 'l1', 'ok', 'd', NULL)",
            [],
        )
        .unwrap();
        record(&db, AuditKind::Ingest, "api", "c1", AuditStatus::Ok, "d1");
        record(&db, AuditKind::Ingest, "api", "c2", AuditStatus::Ok, "d2");
        let legacy_head = chain_head(&db).expect("legacy head");
        assert_eq!(read_epoch(&db), ChainEpoch::Legacy);

        set_chain_key_for_test(Some([7u8; 32]));
        let rewritten = reanchor_to_hmac(&db).expect("re-anchor");
        // Row 1 keeps its NULL genesis; rows 2..3 were rewritten keyed.
        assert_eq!(rewritten, 2, "the two chained rows were replayed");
        assert_eq!(read_epoch(&db), ChainEpoch::Hmac256);
        assert!(verify_chain(&db), "replayed chain verifies under hmac256");
        let pin = read_head_pin(&db).expect("pin after re-anchor");
        assert_eq!(pin.epoch, EPOCH_HMAC);
        assert_eq!(pin.hash, chain_head(&db).expect("hmac head"));
        assert_ne!(pin.hash, legacy_head, "the epoch change moved the head");

        // New growth continues under keyed links + re-pins.
        record(&db, AuditKind::Ingest, "api", "c3", AuditStatus::Ok, "d3");
        assert!(verify_chain(&db));
        assert_eq!(read_head_pin(&db).unwrap().id, 4);

        // Idempotent: a second run verifies + refreshes, rewrites nothing.
        assert_eq!(reanchor_to_hmac(&db).expect("idempotent re-anchor"), 0);
        assert!(verify_chain(&db));
        set_chain_key_for_test(None);
    }

    /// The re-anchor refuses a broken chain (evidence laundering guard — the
    /// same rule the retention prune enforces).
    #[test]
    fn reanchor_refuses_broken_chain() {
        let _gate = key_gate();
        let db = db_with_meta();
        record(&db, AuditKind::Ingest, "api", "c1", AuditStatus::Ok, "d1");
        record(&db, AuditKind::Ingest, "api", "c2", AuditStatus::Ok, "d2");
        db.execute("UPDATE audit_events SET actor = 'mallory' WHERE id = 1", [])
            .unwrap();
        assert!(!verify_chain(&db));
        set_chain_key_for_test(Some([7u8; 32]));
        assert!(
            reanchor_to_hmac(&db).is_err(),
            "a broken chain must not be replayed"
        );
        set_chain_key_for_test(None);
    }

    /// Fresh-DB bootstrap: only a row-less DB with a key available starts on
    /// hmac256; anything with history keeps its own epoch until --re-audit.
    #[test]
    fn bootstrap_epoch_only_for_fresh_dbs_with_key() {
        let _gate = key_gate();
        // No key → never bootstraps.
        let no_key = db_with_meta();
        assert!(!bootstrap_epoch(&no_key));
        assert_eq!(read_epoch(&no_key), ChainEpoch::Legacy);

        // Key + empty DB → bootstraps.
        set_chain_key_for_test(Some([7u8; 32]));
        let fresh = db_with_meta();
        assert!(bootstrap_epoch(&fresh));
        assert_eq!(read_epoch(&fresh), ChainEpoch::Hmac256);
        // Already stamped → idempotent no-op.
        assert!(!bootstrap_epoch(&fresh));

        // Key + rows → keeps its own epoch (evidence format changes only via
        // --re-audit). The key is irrelevant here: this chain's FIRST append
        // already happened, unstamped, and stamped the corrected `sha7` link.
        let historical = db_with_meta();
        record(
            &historical,
            AuditKind::Ingest,
            "api",
            "c1",
            AuditStatus::Ok,
            "d1",
        );
        assert!(!bootstrap_epoch(&historical));
        assert_eq!(read_epoch(&historical), ChainEpoch::Sha7);
        set_chain_key_for_test(None);
    }
}
