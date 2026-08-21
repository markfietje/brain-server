//! Per-domain database registry.
//!
//! Maps a domain name to a SQLite connection pool, lazily. This is the seam for
//! per-domain isolation: in **multi-db mode** (`BRAIN_MULTI_DB=true`) each
//! non-`global` domain gets its own `brain-<domain>.db` file + pool; in **shim
//! mode** (default, flag off) every domain resolves to the shared global pool,
//! so behavior is byte-for-byte identical to the legacy single-DB. That makes
//! the rollout safe: flip the flag to opt into per-domain files, no data move
//! required for `global`.
//!
//! `global` always resolves to the legacy `brain.db` (the existing data), so an
//! upgrade never redistributes existing rows. Cross-domain federation + centroid
//! routing land in the next phase; today a search runs against a single domain
//! pool.
//!
//! Security: a domain name becomes a filename, so [`DomainRegistry::is_valid_domain`]
//! enforces the same charset as the handler regex (`^[a-z0-9][a-z0-9_-]{0,62}$`)
//! — no path separators, no `..` — before any path is built.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;

use crate::config;
use crate::Pool as BrainPool;

/// Errors raised by domain resolution/opening.
#[derive(Debug)]
pub enum DomainRegistryError {
    /// Domain name failed validation (unsafe as a filename / not a real domain).
    Invalid(String),
    /// The domain is valid but NOT registered (multi-db mode only): `pool_for`
    /// refuses to create files for unregistered names
    /// — no lazy unbounded disk fill from an attacker-probeable API.
    Unknown(String),
    /// The registration cap (`BRAIN_MAX_DOMAIN_DBS`) was reached.
    Capacity(usize),
    /// A pool could not be opened (I/O / r2d2).
    Open(String),
    /// The per-DB migration failed on first open.
    Migration(String),
    /// The registry lock was poisoned.
    Poisoned,
}

impl std::fmt::Display for DomainRegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(d) => write!(f, "invalid domain name {d:?}"),
            Self::Unknown(d) => write!(f, "unknown domain {d:?}"),
            Self::Capacity(n) => write!(f, "domain DB registration limit ({n}) reached"),
            Self::Open(e) => write!(f, "failed to open domain DB: {e}"),
            Self::Migration(e) => write!(f, "domain DB migration failed: {e}"),
            Self::Poisoned => write!(f, "domain registry lock poisoned"),
        }
    }
}

impl std::error::Error for DomainRegistryError {}

/// Registry of per-domain connection pools.
pub struct DomainRegistry {
    /// The shared legacy pool backing `global` (and every domain in shim mode).
    global_pool: BrainPool,
    /// Directory holding `brain.db` and `brain-<domain>.db` files.
    dir: PathBuf,
    /// When false, all domains resolve to `global_pool` (legacy single-DB).
    multi_db: bool,
    /// Lazily-opened non-global pools (multi-db mode only).
    pools: Mutex<HashMap<String, BrainPool>>,
    /// The registered set (multi-db mode only): domains that may be opened —
    /// seeded from the on-disk `brain-<domain>.db` files at [`Self::new`] and
    /// grown exclusively via [`Self::register`] (cap-bounded) or the
    /// clients-table boot seed. `pool_for` REFUSES anything outside it, so an
    /// unauthenticated-probeable surface can never create a DB file.
    registered: Mutex<HashSet<String>>,
}

impl DomainRegistry {
    /// Build a registry over an already-initialized `global_pool`. `global_path`
    /// determines the directory in which per-domain files are created.
    pub fn new(global_pool: BrainPool, global_path: &Path, multi_db: bool) -> Self {
        let dir = global_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let registered = Mutex::new(scan_domain_files(&dir).into_iter().collect::<HashSet<_>>());
        Self {
            global_pool,
            dir,
            multi_db,
            pools: Mutex::new(HashMap::new()),
            registered,
        }
    }

    /// Whether per-domain file isolation is active. Read by the graph read
    /// paths: in shim mode the domain is a *label*
    /// (rows of every label share one pool), so reads scope by the column;
    /// in multi-db mode the pool itself is the domain's territory.
    pub fn is_multi_db(&self) -> bool {
        self.multi_db
    }

    /// Validate a domain name is safe to use as a filename (matches the handler
    /// regex `^[a-z0-9][a-z0-9_-]{0,62}$`). Rejects empty, path separators,
    /// `..`, and uppercase. Delegates to `storage_layout::is_valid_domain` so
    /// the security check lives in exactly one place (shared with the
    /// `brain-migrate-rehearse` binary).
    pub fn is_valid_domain(domain: &str) -> bool {
        brain_server::storage_layout::is_valid_domain(domain)
    }

    /// Resolve `domain` to its pool. In shim mode (or for `global`) this always
    /// returns the shared global pool. In multi-db mode a non-global domain
    /// resolves ONLY if it is registered ([`Self::register`] or seeded from the
    /// on-disk file set); anything else is [`DomainRegistryError::Unknown`] and
    /// NEVER creates a file.
    pub fn pool_for(&self, domain: &str) -> Result<BrainPool, DomainRegistryError> {
        if !Self::is_valid_domain(domain) {
            return Err(DomainRegistryError::Invalid(domain.to_string()));
        }
        if !self.multi_db || domain == "global" {
            return Ok(self.global_pool.clone());
        }
        if !self.is_registered(domain) {
            return Err(DomainRegistryError::Unknown(domain.to_string()));
        }
        let mut pools = self
            .pools
            .lock()
            .map_err(|_| DomainRegistryError::Poisoned)?;
        if let Some(p) = pools.get(domain) {
            return Ok(p.clone());
        }
        let path = self.dir.join(format!("brain-{domain}.db"));
        let pool = open_with_migration(&path)?;
        pools.insert(domain.to_string(), pool.clone());
        Ok(pool)
    }

    /// Register a domain so its pool may be opened (multi-db only). This is the
    /// ONE creation path: cap-bounded by `BRAIN_MAX_DOMAIN_DBS` so nobody can
    /// fill the disk through a probeable API. Idempotent (returns the pool).
    /// In shim mode it is a warm no-op over the shared global pool — legacy
    /// single-DB behavior is byte-for-byte unchanged.
    ///
    /// Note: `DELETE /domains` clears a domain's data but keeps its file (the
    /// audit segment survives), so the domain keeps its registered slot; an
    /// operator removing the files frees the slot at next boot.
    pub fn register(&self, domain: &str) -> Result<BrainPool, DomainRegistryError> {
        if !Self::is_valid_domain(domain) {
            return Err(DomainRegistryError::Invalid(domain.to_string()));
        }
        if !self.multi_db {
            return Ok(self.global_pool.clone());
        }
        self.seed_registered(domain)?;
        // Below the pools lock: opening may be slow, never hold both.
        self.pool_for(domain)
    }

    /// add `domain` to the registered set
    /// WITHOUT opening its pool — the boot-time clients-table seed (no eager
    /// connection pools at startup; the first access opens lazily, and a
    /// vanished file is recreated on demand, always bounded by the cap). The
    /// only creation path that opens is [`Self::register`].
    pub fn seed_registered(&self, domain: &str) -> Result<(), DomainRegistryError> {
        if !Self::is_valid_domain(domain) {
            return Err(DomainRegistryError::Invalid(domain.to_string()));
        }
        if !self.multi_db {
            return Ok(());
        }
        let mut registered = self
            .registered
            .lock()
            .map_err(|_| DomainRegistryError::Poisoned)?;
        if !registered.contains(domain) {
            let cap = config::max_domain_dbs();
            if registered.len() >= cap {
                return Err(DomainRegistryError::Capacity(cap));
            }
            registered.insert(domain.to_string());
        }
        Ok(())
    }

    /// Whether `domain` is registered (multi-db). `global` is always registered;
    /// in shim mode every valid name resolves anyway.
    pub fn is_registered(&self, domain: &str) -> bool {
        if !self.multi_db || domain == "global" {
            return true;
        }
        self.registered
            .lock()
            .map(|r| r.contains(domain))
            .unwrap_or(false)
    }

    /// Domain names known to the registry: `global` plus any `brain-<domain>.db`
    /// files present on disk (multi-db mode).
    pub fn known_domains(&self) -> Vec<String> {
        let mut out = vec!["global".to_string()];
        out.extend(scan_domain_files(&self.dir));
        out.sort();
        out.dedup();
        out
    }
}

/// `brain-<domain>.db` file names in `dir` (validated, `global` excluded —
/// `brain.db` has no `brain-` prefix).
fn scan_domain_files(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if let Some(rest) = name
                    .strip_prefix("brain-")
                    .and_then(|s| s.strip_suffix(".db"))
                {
                    if DomainRegistry::is_valid_domain(rest) {
                        out.push(rest.to_string());
                    }
                }
            }
        }
    }
    out
}

/// Open a per-domain SQLite pool at `path` and run the standard migration once
/// (creates tables; WAL is persistent; per-connection PRAGMAs via `with_init`).
/// sqlite-vec is already registered process-wide via `sqlite3_auto_extension`,
/// so vec0 is available on every connection without per-pool setup.
fn open_with_migration(path: &Path) -> Result<BrainPool, DomainRegistryError> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).map_err(|e| DomainRegistryError::Open(e.to_string()))?;
    }
    let manager = SqliteConnectionManager::file(path).with_init(|c| {
        c.execute_batch(
            "PRAGMA journal_mode=WAL; \
             PRAGMA synchronous=NORMAL; \
             PRAGMA foreign_keys=ON; \
             PRAGMA cache_size=-64000; \
             PRAGMA temp_store=MEMORY; \
             PRAGMA busy_timeout=5000;",
        )
    });
    let pool: Pool<SqliteConnectionManager> = r2d2::Pool::builder()
        .max_size(config::POOL_MAX_SIZE)
        .min_idle(Some(config::POOL_MIN_IDLE))
        .connection_timeout(std::time::Duration::from_secs(
            config::POOL_CONNECTION_TIMEOUT_SECS,
        ))
        .max_lifetime(Some(std::time::Duration::from_secs(
            config::POOL_MAX_LIFETIME_SECS,
        )))
        .idle_timeout(Some(std::time::Duration::from_secs(
            config::POOL_IDLE_TIMEOUT_SECS,
        )))
        .build(manager)
        .map_err(|e| DomainRegistryError::Open(e.to_string()))?;

    let mut conn = pool
        .get()
        .map_err(|e| DomainRegistryError::Open(e.to_string()))?;
    brain_server::migration::run_migration(&mut conn, config::DB_MMAP_SIZE_MIB)
        .map_err(|e| DomainRegistryError::Migration(e.to_string()))?;
    // A FRESH domain DB (zero audit rows) starts
    // directly on the hmac256 chain epoch when the process holds a chain key
    // (the server inits it before any pool opens). A no-op everywhere else —
    // in particular every unit test that never installs a key.
    brain_server::audit::bootstrap_epoch(&conn);
    Ok(pool)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_domains() {
        for d in ["global", "work", "my-domain", "d1", "a_b", "proj-2"] {
            assert!(DomainRegistry::is_valid_domain(d), "{d:?} should be valid");
        }
    }

    #[test]
    fn invalid_domains_cannot_become_filenames() {
        // Path-traversal / separator / case / shape rejection — security-critical.
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
            assert!(
                !DomainRegistry::is_valid_domain(d),
                "{d:?} should be INVALID"
            );
        }
    }

    /// A registry in shim mode resolves every domain to the global pool handle,
    /// so single-DB behavior is preserved.
    #[test]
    fn shim_mode_routes_everything_to_global() {
        let mgr = SqliteConnectionManager::memory();
        let pool: BrainPool = r2d2::Pool::builder()
            .build(mgr)
            .expect("build in-memory pool");
        let reg = DomainRegistry::new(pool.clone(), Path::new("/tmp/whatever.db"), false);
        assert!(!reg.is_multi_db());
        // global, work, anything-valid all return Ok (the shared handle).
        assert!(reg.pool_for("global").is_ok());
        assert!(reg.pool_for("work").is_ok());
        // invalid still rejected even in shim mode.
        assert!(matches!(
            reg.pool_for("../evil"),
            Err(DomainRegistryError::Invalid(_))
        ));
    }

    /// Multi-db mode opens a real per-domain file and migrates it.
    #[test]
    fn multi_db_opens_per_domain_file() {
        crate::register_sqlite_vec();
        let dir = std::env::temp_dir().join(format!(
            "brain-registry-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let global_path = dir.join("brain.db");
        let mgr = SqliteConnectionManager::file(&global_path);
        let pool: BrainPool = r2d2::Pool::builder().build(mgr).expect("build global pool");
        let reg = DomainRegistry::new(pool, &global_path, true);
        assert!(reg.is_multi_db());

        // Opening "research" creates brain-research.db in the same dir
        // (v1.27.16 M5/F-41: creation goes through `register`).
        let p = reg.register("research").expect("open research domain");
        assert!(p.get().is_ok(), "research pool must yield a connection");
        let domain_file = dir.join("brain-research.db");
        assert!(domain_file.exists(), "per-domain file must be created");

        // known_domains now includes research + global.
        let known = reg.known_domains();
        assert!(known.contains(&"global".to_string()));
        assert!(known.contains(&"research".to_string()));

        std::fs::remove_dir_all(&dir).ok();
    }

    /// an unregistered multi-db name is
    /// refused with `Unknown` and — critically — leaves NO file behind: the
    /// registered-only rule is the anti-disk-fill bound on probeable reads.
    #[test]
    fn pool_for_rejects_unregistered_domain_without_creating_file() {
        crate::register_sqlite_vec();
        let dir = std::env::temp_dir().join(format!(
            "brain-unregistered-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let global_path = dir.join("brain.db");
        let mgr = SqliteConnectionManager::file(&global_path);
        let pool: BrainPool = r2d2::Pool::builder().build(mgr).expect("build global pool");
        let reg = DomainRegistry::new(pool, &global_path, true);

        let err = reg
            .pool_for("never-registered")
            .expect_err("unregistered name must be refused");
        assert!(matches!(err, DomainRegistryError::Unknown(_)), "{err:?}");
        assert!(
            !dir.join("brain-never-registered.db").exists(),
            "a refused probe must not create a file"
        );
        // In shim mode the registered set is irrelevant — every valid name
        // still resolves to the shared pool (legacy behavior untouched).
        let shim = DomainRegistry::new(reg.global_pool.clone(), &global_path, false);
        assert!(shim.pool_for("never-registered").is_ok());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// `register` is the single creation path
    /// and the cap (`BRAIN_MAX_DOMAIN_DBS`, default 256) bounds how many
    /// files may exist — creation is refused (Capacity, no file) beyond it.
    #[test]
    fn register_respects_domain_cap_and_creates_no_file_beyond() {
        crate::register_sqlite_vec();
        let dir = std::env::temp_dir().join(format!(
            "brain-cap-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let global_path = dir.join("brain.db");
        let mgr = SqliteConnectionManager::file(&global_path);
        let pool: BrainPool = r2d2::Pool::builder().build(mgr).expect("build global pool");
        let reg = DomainRegistry::new(pool, &global_path, true);

        // Exhaust the (default 256) cap via the seed seam — same counted set
        // `register` checks, same default the shipped server uses.
        for i in 0..config::MAX_DOMAIN_DBS {
            reg.seed_registered(&format!("d{i}")).expect("seed fits");
        }
        match reg.seed_registered("over-cap") {
            Err(DomainRegistryError::Capacity(n)) => {
                assert_eq!(n, config::MAX_DOMAIN_DBS);
            }
            other => panic!("expected Capacity, got {other:?}"),
        }
        let err = reg
            .register("also-over-cap")
            .expect_err("register beyond cap must be refused");
        assert!(matches!(err, DomainRegistryError::Capacity(_)));
        assert!(
            !dir.join("brain-also-over-cap.db").exists(),
            "no file beyond the cap"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// the boot seed registers names WITHOUT
    /// opening a pool (no eager connections); the first access opens lazily.
    #[test]
    fn seeded_domain_opened_lazily_on_first_access() {
        crate::register_sqlite_vec();
        let dir = std::env::temp_dir().join(format!(
            "brain-seed-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let global_path = dir.join("brain.db");
        let mgr = SqliteConnectionManager::file(&global_path);
        let pool: BrainPool = r2d2::Pool::builder().build(mgr).expect("build global pool");
        let reg = DomainRegistry::new(pool, &global_path, true);

        reg.seed_registered("seeded-but-cold").expect("seed ok");
        // Has a file appeared? No — seeding is a reference, not creation.
        assert!(!dir.join("brain-seeded-but-cold.db").exists());
        // First access opens + migrates it.
        assert!(reg.pool_for("seeded-but-cold").is_ok());
        assert!(dir.join("brain-seeded-but-cold.db").exists());
        // A NEW registry (fresh boot) rescans the dir → still registered.
        let reg2 = DomainRegistry::new(reg.global_pool.clone(), &global_path, true);
        assert!(reg2.pool_for("seeded-but-cold").is_ok());

        std::fs::remove_dir_all(&dir).ok();
    }
}
