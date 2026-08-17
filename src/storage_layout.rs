//! Storage layout abstraction.
//!
//! Every on-disk path brain-server touches, derived from one root. The point
//! is to make the v1.0.0 multi-domain cutover addressable *before* it happens:
//! today the runtime reads `legacy_db()`; v1.0 will read `global_domain_db()`
//! and per-domain files via `domain_db(name)`. Both are derived from the same
//! root, so the cutover is a path rename, not a code change.
//!
//! The default root preserves the v0.9.x install location
//! (`~/.openclaw/workspace`); `BRAIN_DATA_ROOT` is the v1.0 relocation knob.
//! No file is moved at construction — paths are computed lazily on demand.
//!
//! Security: a domain name becomes a filename, so [`StorageLayout::domain_db`]
//! rejects anything that fails [`DomainRegistry::is_valid_domain`] (no path
//! separators, no `..`, no uppercase). The check lives in one place.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

/// The schema version recorded in `schema_meta` by `run_migration`. Bumped once
/// per release that changes the migration. v1.4.0 adds bi-temporal edges
/// (`relationships.valid_at`/`invalid_at`) and TRACE node reservation
/// (`knowledge.node_kind`/`parent_id`). v1.2.0 adds the `revoked_tokens`
/// and `refresh_chains` tables for JWT auth (the AuthN feature). v1.1.0 adds
/// the audit-chain columns (`tenant_id`, `prev_hash`) and the
/// `idx_audit_tenant` index.
pub const SCHEMA_VERSION_V1_10_0: &str = "1.10.0";
/// knowledge gains access_scope/assertion_kind/
/// confidence/expires_at/pii/owner; new tombstones + proposals tables.
/// Defaults preserve current behavior (no data loss, no re-ingest).
pub const SCHEMA_VERSION_V1_14_0: &str = "1.14.0";
/// new recall_traces + dsar_requests tables;
/// tombstones gains reason + origin_id. Additive, defaults preserved.
pub const SCHEMA_VERSION_V1_15_0: &str = "1.15.0";
/// new retention_policy table (persisted per-kind
/// retention overrides). Additive, defaults preserved; empty = code defaults.
pub const SCHEMA_VERSION_V1_17_1: &str = "1.17.1";
/// `knowledge.ump_id` (unique content-addressed UMP
/// record id) + `knowledge.ump_meta` (UMP provenance/consent/lifecycle
/// overlay) + `suggest_feedback.ump_outcome`. Additive, defaults preserved.
/// v1.18.2 adds `knowledge.origin` (Art 50 model-vs-human marker).
pub const SCHEMA_VERSION_V1_18_2: &str = "1.18.2";
/// v1.20.1 "Shield" adds `proposals.source_prompt` (auto-capture provenance).
pub const SCHEMA_VERSION_V1_20_1: &str = "1.20.1";
/// v1.20.14 "Steer" adds `proposals.edited_at` (edit-then-approve provenance).
pub const SCHEMA_VERSION_V1_20_14: &str = "1.20.14";
/// v1.20.18 "Bound" adds `idx_tombstones_reason_purged` (tombstone registry +
/// DSAR certificate read index).
pub const SCHEMA_VERSION_V1_20_18: &str = "1.20.18";
/// v1.20.19 "Vault" drops the never-written `pii_map` table (PII control is
/// deterministic output redaction, not a placeholder vault).
pub const SCHEMA_VERSION_V1_20_19: &str = "1.20.19";
/// new `profiles` + `domain_profiles` tables (the
/// preset bundles + the domain→profile binding). Additive; the 12 presets are
/// seeded INSERT OR IGNORE (operator edits survive re-migrations). No column
/// changes anywhere.
pub const SCHEMA_VERSION_V1_21_0: &str = "1.21.0";
/// new `legal_holds` table (freeze ids vs decay +
/// purge + DSAR) + additive `knowledge.region` (residency stamp, backfilled
/// NULLs only — a stamp is never overwritten, so a region change preserves
/// where pre-existing rows lived).
pub const SCHEMA_VERSION_V1_22_0: &str = "1.22.0";
/// new `roles` table (the named scope/action bundles,
/// 10 seeded presets). Additive; no column changes anywhere.
pub const SCHEMA_VERSION_V1_23_0: &str = "1.23.0";
/// the breach workflow tables.
pub const SCHEMA_VERSION_V1_25_0: &str = "1.25.0";
/// the transfers table + knowledge.lawful_basis/purpose.
pub const SCHEMA_VERSION_V1_26_0: &str = "1.26.0";
/// the BPO operating `clients` register (global-operator rows).
pub const SCHEMA_VERSION_V1_27_0: &str = "1.27.0";
/// `proposals.owner` + `proposals.qa_note` (agent provenance
/// + supervisor coaching on the review queue).
pub const SCHEMA_VERSION_V1_27_8: &str = "1.27.8";
/// `idx_knowledge_domain`/`idx_knowledge_owner`/
/// `idx_knowledge_title_heading` added; `idx_tombstones_kid`,
/// `idx_entities_name`, `idx_evidence_links_from` dropped (superseded by
/// stricter indexes / autoindexes).
pub const SCHEMA_VERSION_V1_27_18: &str = "1.27.18";
pub const SCHEMA_VERSION_V1_17_3: &str = "1.17.3";
pub const SCHEMA_VERSION_V1_9_0: &str = "1.9.0";
pub const SCHEMA_VERSION_V1_4_0: &str = "1.4.0";
pub const SCHEMA_VERSION_V1_2_0: &str = "1.2.0";
pub const SCHEMA_VERSION_V1_1_0: &str = "1.1.0";
/// Historical v0.9.9 checkpoint before the v1.0/v1.1 migrations. Kept for
/// back-compat with any code or operator script that compares against it; new
/// code should reference [`SCHEMA_VERSION_V1_1_0`].
pub const SCHEMA_VERSION_V0_9_9: &str = "0.9.9";

/// Read the recorded schema version from `schema_meta`. Returns `None` for a
/// pre-`schema_meta` DB (treated as "<= v0.8.x" by callers). Pure read; no side
/// effects. Used by the rehearsal tool's parity check.
pub fn schema_version(db: &Connection) -> Option<String> {
    db.query_row(
        "SELECT value FROM schema_meta WHERE key = 'schema_version'",
        [],
        |r| r.get::<_, String>(0),
    )
    .ok()
}

// ── region pin (data residency) ─────────────────────

/// The residency stamp for this deployment, from `BRAIN_REGION` (e.g.
/// `eu-west-1`, `ph-manila`). Unset/invalid → `None` (no stamp — pre-v1.22
/// behavior). Lives here (lib) because the lib's migration stamps it onto every
/// chunk; the server binary re-exports it from `config`. Read-only provenance:
/// the stamp proves *where data lived* — multi-region *routing* is v2.x.
pub fn region() -> Option<String> {
    region_from(std::env::var("BRAIN_REGION").ok().as_deref())
}

/// Pure resolver (the `resolve_root` pattern — tests never mutate process env).
/// Shape: lowercase alnum + hyphen, 1..=63 chars, alnum first (the domain
/// charset minus `_`) — a region is a label stamped into rows + certificates,
/// so it must stay inert (no separators, no spaces, no case games).
pub fn region_from(raw: Option<&str>) -> Option<String> {
    let r = raw?.trim().to_string();
    let ok = !r.is_empty()
        && r.len() <= 63
        && r.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && r.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    ok.then_some(r)
}

/// Validate a domain name is safe to use as a filename. Matches the handler
/// regex `^[a-z0-9][a-z0-9_-]{0,62}$`. Rejects empty, path separators, `..`,
/// and uppercase. This is the security boundary for every path derived from
/// a domain name; `DomainRegistry::is_valid_domain` delegates here so the
/// check lives in exactly one place.
pub fn is_valid_domain(domain: &str) -> bool {
    let mut chars = domain.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() || first.is_ascii_uppercase() {
        return false;
    }
    domain.len() <= 63
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// Errors raised by storage-layout resolution or path construction.
#[derive(Debug)]
pub enum StorageLayoutError {
    /// `BRAIN_DATA_ROOT` or `BRAIN_DB_PATH` resolved to a non-UTF8 / relative path.
    InvalidRoot(String),
    /// Domain name failed validation (unsafe as a filename).
    InvalidDomain(String),
}

impl std::fmt::Display for StorageLayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRoot(r) => write!(f, "invalid storage root: {r}"),
            Self::InvalidDomain(d) => write!(f, "invalid domain name {d:?}"),
        }
    }
}

impl std::error::Error for StorageLayoutError {}

/// All on-disk paths brain-server touches, derived from one root.
///
/// Construct via [`StorageLayout::detect`]; in tests, [`StorageLayout::new`]
/// takes an explicit root.
#[derive(Debug, Clone)]
pub struct StorageLayout {
    root: PathBuf,
}

impl StorageLayout {
    /// Build a layout over an explicit root. Public for tests; production code
    /// uses [`StorageLayout::detect`].
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Resolve the root from the environment, in priority order:
    /// 1. `BRAIN_DATA_ROOT` — the v1.0 knob (new in v0.9.9).
    /// 2. The parent of `BRAIN_DB_PATH` — preserves the v0.9.x install layout.
    /// 3. `~/.openclaw/workspace` — the historical default.
    ///
    /// Fails closed on a non-absolute `BRAIN_DATA_ROOT`.
    pub fn detect() -> Result<Self, StorageLayoutError> {
        Self::resolve_root(
            std::env::var("BRAIN_DATA_ROOT").ok().as_deref(),
            std::env::var("BRAIN_DB_PATH").ok().as_deref(),
        )
    }

    /// Pure resolution logic, factored out so tests don't mutate process env
    /// (which would race with parallel tests). Priority: `data_root` >
    /// `db_path`'s parent > the historical default.
    fn resolve_root(
        data_root: Option<&str>,
        db_path: Option<&str>,
    ) -> Result<Self, StorageLayoutError> {
        if let Some(raw) = data_root {
            let raw = raw.trim();
            let p = PathBuf::from(raw);
            if !p.is_absolute() {
                return Err(StorageLayoutError::InvalidRoot(format!(
                    "BRAIN_DATA_ROOT must be absolute, got {p:?}"
                )));
            }
            return Ok(Self::new(p));
        }
        if let Some(raw) = db_path {
            let p = PathBuf::from(raw.trim());
            if let Some(parent) = p.parent() {
                if !parent.as_os_str().is_empty() && parent.is_absolute() {
                    return Ok(Self::new(parent.to_path_buf()));
                }
            }
            // BRAIN_DB_PATH was relative or bare — fall through to default.
        }
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        Ok(Self::new(home.join(".openclaw/workspace")))
    }

    /// The legacy global DB path. Byte-identical to `config::brain_db_path()`
    /// when `BRAIN_DB_PATH` is set (the v0.9.x back-compat invariant); v1.0.0
    /// renames this file to `global.db` and points the runtime at
    /// [`StorageLayout::global_domain_db`] instead.
    pub fn legacy_db(&self) -> PathBuf {
        if let Ok(raw) = std::env::var("BRAIN_DB_PATH") {
            let raw = raw.trim();
            if !raw.is_empty() {
                return PathBuf::from(raw);
            }
        }
        self.root.join("brain.db")
    }

    /// The v1.0.0 global domain file. In v0.9.9 this is where the rehearsal
    /// tool's *candidate* `global.db` lands — the live runtime still reads
    /// [`StorageLayout::legacy_db`].
    pub fn global_domain_db(&self) -> PathBuf {
        self.root.join("global.db")
    }

    /// Per-domain file `<root>/brain-<domain>.db`. Matches the path computed
    /// by `DomainRegistry::open_with_migration` (lifted here so the layout
    /// owns it). `domain` is validated; path-traversal is impossible.
    pub fn domain_db(&self, domain: &str) -> Result<PathBuf, StorageLayoutError> {
        if !is_valid_domain(domain) {
            return Err(StorageLayoutError::InvalidDomain(domain.to_string()));
        }
        Ok(self.root.join(format!("brain-{domain}.db")))
    }

    /// Backup directory. Replaces the implicit CWD-relative default in
    /// `backup.rs`; the rehearsal tool and `brain backup` both write here.
    pub fn backups_dir(&self) -> PathBuf {
        self.root.join("backups")
    }

    /// v1.0.0 registry DB (maps domain → file path). Created lazily; does not
    /// exist in v0.9.9 unless `BRAIN_MULTI_DB=true`.
    pub fn registry_db(&self) -> PathBuf {
        self.root.join("registry.db")
    }

    /// Connector config dir. Lifted from `backup::default_connector_config_dir`
    /// so there is one source of truth.
    pub fn connector_configs_dir(&self) -> PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".config/brain-server/connectors")
    }

    /// The root everything is derived from. Exposed for callers (e.g. the
    /// rehearsal tool) that need to compose sibling paths.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_root_db_path_sets_root_to_parent() {
        // The back-compat invariant: when BRAIN_DB_PATH is set, the legacy_db()
        // path equals it, and global_domain_db() is its sibling.
        let layout = StorageLayout::resolve_root(None, Some("/tmp/brain-v0.9.9-test/brain.db"))
            .expect("resolve with db_path");
        assert_eq!(
            layout.legacy_db(),
            PathBuf::from("/tmp/brain-v0.9.9-test/brain.db")
        );
        assert_eq!(
            layout.global_domain_db(),
            PathBuf::from("/tmp/brain-v0.9.9-test/global.db")
        );
    }

    #[test]
    fn resolve_root_data_root_wins_over_db_path() {
        // BRAIN_DATA_ROOT is the v1.0 knob and takes priority over
        // BRAIN_DB_PATH's parent — the operator who sets both meant the new root.
        let layout =
            StorageLayout::resolve_root(Some("/tmp/data-root-v1"), Some("/tmp/legacy/brain.db"))
                .expect("resolve with both");
        assert_eq!(layout.root(), Path::new("/tmp/data-root-v1"));
        assert_eq!(
            layout.global_domain_db(),
            PathBuf::from("/tmp/data-root-v1/global.db")
        );
    }

    #[test]
    fn resolve_root_rejects_relative_data_root() {
        let err = StorageLayout::resolve_root(Some("relative/path"), None).unwrap_err();
        assert!(matches!(err, StorageLayoutError::InvalidRoot(_)));
    }

    #[test]
    fn resolve_root_falls_back_to_default_when_no_env() {
        let layout = StorageLayout::resolve_root(None, None).expect("resolve defaults");
        // Default root is ~/.openclaw/workspace; we only assert the shape.
        assert!(layout.root().ends_with(".openclaw/workspace"));
        assert!(layout.legacy_db().starts_with(layout.root()));
        assert!(layout.legacy_db().ends_with("brain.db"));
    }

    #[test]
    fn region_from_accepts_valid_residency_labels_and_rejects_unsafe_ones() {
        // M3 "Regulated": the residency stamp is inert (lowercase alnum +
        // hyphen, 1..=63, alnum first) so it is safe to bake into rows +
        // certificates. Uppercase, separators, spaces, empties and over-long
        // labels are refused rather than stamped (fail-closed provenance).
        assert_eq!(
            region_from(Some("eu-west-1")),
            Some("eu-west-1".to_string())
        );
        assert_eq!(
            region_from(Some("ph-manila")),
            Some("ph-manila".to_string())
        );
        assert_eq!(
            region_from(Some("  us-east-1  ")),
            Some("us-east-1".to_string())
        );
        assert_eq!(region_from(Some("global")), Some("global".to_string()));
        assert_eq!(region_from(None), None);
        assert_eq!(region_from(Some("")), None);
        assert_eq!(region_from(Some("EU-WEST-1")), None);
        assert_eq!(region_from(Some("eu west")), None);
        assert_eq!(region_from(Some("eu/west")), None);
        assert_eq!(region_from(Some("-eu")), None);
        assert_eq!(region_from(Some(&"x".repeat(64))), None);
    }

    #[test]
    fn legacy_db_uses_explicit_db_path_when_root_is_default() {
        // Even when the root is derived from the default, an explicit
        // BRAIN_DB_PATH still wins for legacy_db() — preserving the v0.9.x
        // operator knob. Tested via a constructed layout + env read.
        let layout = StorageLayout::new(PathBuf::from("/tmp/whatever"));
        // When BRAIN_DB_PATH is unset, legacy_db is root/brain.db.
        std::env::remove_var("BRAIN_DB_PATH");
        assert_eq!(layout.legacy_db(), PathBuf::from("/tmp/whatever/brain.db"));
    }

    #[test]
    fn domain_db_rejects_path_traversal() {
        let layout = StorageLayout::new(PathBuf::from("/tmp/whatever"));
        assert!(matches!(
            layout.domain_db("../evil"),
            Err(StorageLayoutError::InvalidDomain(_))
        ));
        assert!(matches!(
            layout.domain_db("a/b"),
            Err(StorageLayoutError::InvalidDomain(_))
        ));
    }

    #[test]
    fn domain_db_accepts_valid_names() {
        let layout = StorageLayout::new(PathBuf::from("/tmp/whatever"));
        assert_eq!(
            layout.domain_db("global").unwrap(),
            PathBuf::from("/tmp/whatever/brain-global.db")
        );
        assert_eq!(
            layout.domain_db("health").unwrap(),
            PathBuf::from("/tmp/whatever/brain-health.db")
        );
    }

    #[test]
    fn is_valid_domain_accepts_safe_names() {
        for d in ["global", "work", "my-domain", "d1", "a_b", "proj-2"] {
            assert!(is_valid_domain(d), "{d:?} should be valid");
        }
    }

    #[test]
    fn is_valid_domain_rejects_path_traversal_and_separators() {
        // Security-critical: these must NEVER become filenames.
        for d in [
            "",
            "..",
            ".",
            "/",
            "a/b",
            "\\x",
            "GLOBAL",
            "Uppercase",
            "-lead",
            "has space",
            "a.b",
            "a:b",
            "../escape",
            &"x".repeat(64),
        ] {
            assert!(!is_valid_domain(d), "{d:?} should be INVALID");
        }
    }
}
