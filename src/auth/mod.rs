//! Authentication (v1.1 "Harden" + v1.2 "AuthN").
//!
//! Two modes, discriminated at startup by `BRAIN_JWT_ISSUER`:
//!
//! - **Opaque** (v1.1 default): the existing `TokenStore` — a hot-rotating
//!   set of bearer strings compared with `subtle::ConstantTimeEq`. Single-
//!   tenant, single-user, loopback deployments. Back-compat: every v1.1
//!   install keeps working unchanged.
//! - **JWT** (v1.2 opt-in): RS256/ES256/EdDSA JWS verification against a
//!   local JWKS, `(jti, iss)` revocation, refresh-chain reuse detection,
//!   per-route AuthZ. Multi-tenant deployments.
//!
//! The middleware (`main.rs::auth_middleware`) calls into both paths through
//! this module; handlers receive an `Option<Principal>` in request extensions
//! (`None` = opaque/no-auth back-compat path = superuser).
//!
//! Submodules:
//! - [`jwt`] — verification core (OWASP JWT Cheat Sheet matrix).
//! - [`jwks`] — key loading + JWKS endpoint shape.
//! - [`policy`] — AuthZ trait + in-memory default.
//! - [`revocation`] — `(jti, iss)` denylist + refresh-chain reuse detection.
//! - [`token_store`] (this file) — the v1.1 fail-safe rotating token cache.

pub mod jwks;
pub mod jwt;
pub mod policy;
pub mod revocation;

/// a secret file is only acceptable when
/// owner-only (mode & 0o077 == 0). Returns an error message when the file
/// exists with group/world bits (e.g. a hand-created 0644 token file). The
/// server refuses to start rather than accept a leaked secret on disk.
/// ponytail: `install-service.sh`'s chmod is still the writer-side contract;
/// this is the reader-side enforcement, and non-Unix platforms are unchecked
/// (no POSIX modes to read).
pub use crate::secret_file::check_secret_permissions;

// Re-export the principal types used across handler boundaries.
// Truthful allows: `auth` is a bin-private module (compiled into the server
// binary only), so these `pub use` re-exports are crate-internal convenience
// and the bin does not consume every name (e.g. `ALLOWED_ALGS` is used by the
// jwt module itself + tests). Deleting the allows breaks `-D warnings`.
#[allow(unused_imports)]
pub use jwt::{ALLOWED_ALGS, AuthError, Claims, TokenType, VerifyingKey};
#[allow(unused_imports)]
pub use policy::{Action, Principal, Scope, client_authorized_domains, is_authorized};
#[allow(unused_imports)]
pub use revocation::{RevocationCache, purge_expired, revoke, revoke_chain};

/// The verified access token's `exp` claim, injected into request extensions
/// by the JWT auth middleware beside the [`Principal`]. Consumers: the
/// logout/revoke denylist writes, which keep the row alive exactly as long
/// as the token itself (clamped to a bounded TTL) instead of a fixed guess.
/// `Copy` newtype so an extension collision is impossible and extraction is
/// total.
#[derive(Debug, Clone, Copy)]
pub struct AccessTokenExp(pub u64);

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

use rusqlite::Connection;

use crate::audit::{self, AuditKind, AuditStatus};
use crate::config;

/// Which auth mode the server is running in. Resolved once at startup from
/// `BRAIN_JWT_ISSUER` (and the presence of a key dir). `Opaque` is the
/// default and the v1.1 back-compat path; `Jwt` is opt-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    Opaque,
    Jwt,
}

impl AuthMode {
    /// Resolve from the environment. JWT mode requires both an issuer AND a
    /// non-empty key set (a JWT server with no keys can't verify anything —
    /// better to fall back to opaque + log than to refuse every request).
    pub fn from_env(keys_loaded: usize) -> Self {
        let has_issuer = std::env::var("BRAIN_JWT_ISSUER")
            .ok()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        if has_issuer && keys_loaded > 0 {
            AuthMode::Jwt
        } else {
            AuthMode::Opaque
        }
    }

    pub fn is_jwt(self) -> bool {
        self == AuthMode::Jwt
    }
}

/// Cached accepted-token set + the file metadata that produced it. Cloning the
/// `Arc` is the cheap clone in the hot path; the inner write happens at most
/// once per rotation.
///
/// Lock bounds (Headroom): the critical sections are set/tuple arithmetic
/// only — a `HashSet` clone (read), an mtime/`HashSet` compare (read), a
/// single-assignment swap of `tokens` + `mtime` (write). No I/O inside the
/// lock (`fs::metadata` runs BEFORE acquisition; the audit write after
/// rotation runs on a fresh connection AFTER release), no nesting, no SQL.
/// Poison: the hot read FAILS CLOSED (`TokenRead::ReadFailed` → deny), the
/// rotation watcher reads fail-open and its write is skipped (the stale cache
/// keeps serving the last good tokens). Request-hot holder (every request),
/// so acquires are wait-measured.
#[derive(Clone)]
pub struct TokenStore {
    inner: Arc<RwLock<TokenState>>,
    /// `Some(path)` when reading from `AUTH_TOKEN_FILE`; `None` when auth is
    /// driven by `AUTH_TOKEN` env or disabled. Used by the reload task to know
    /// whether to `stat()` at all.
    file: Option<PathBuf>,
}

/// the token-store read outcome. The old
/// `tokens() -> HashSet` collapsed three worlds into one empty set — never
/// configured, configured-but-empty, and *read failure* — and the middleware
/// treated the empty set as "auth disabled" (allow-all). A poisoned lock must
/// DENY, never read as "no auth".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenRead {
    /// No token source ever resolved (env unset, no file) — the loopback
    /// no-auth posture, unchanged.
    NotConfigured,
    /// The currently accepted token set (post-rotation snapshot).
    Active(HashSet<String>),
    /// The token store could not be read (poisoned lock) — deny.
    ReadFailed,
}

#[derive(Default)]
struct TokenState {
    tokens: HashSet<String>,
    /// mtime of the file when `tokens` was last loaded. `None` until the first
    /// successful load — used to detect rotation.
    mtime: Option<SystemTime>,
    /// True once any load (successful or not) has run. Used by fail-safe: only
    /// preserve the cache after the first load completes.
    initialized: bool,
}

impl TokenStore {
    /// Build a store from the current environment. Performs the initial load
    /// eagerly so the server can refuse to start if `AUTH_TOKEN_FILE` points at
    /// a missing file in a strict mode (today: best-effort, logs a warning).
    pub fn new() -> Self {
        let file = config::auth_token_file();
        Self::from_file(file)
    }

    /// Construct from an explicit file path (or `None` for env-driven / no
    /// file). Tests use this to avoid env-var races under `cargo test`'s
    /// parallel runner. The server uses [`Self::new`] which reads the env once.
    pub fn from_file(file: Option<PathBuf>) -> Self {
        let mut state = TokenState::default();
        let initial = config::auth_tokens();
        state.tokens = initial.iter().cloned().collect();
        // `initialized` now means "a token source
        // is configured" — an explicit token file, an env token, or a resolved
        // token set. Previously the flag meant "a load ran", so a store with
        // zero tokens was indistinguishable from an unconfigured one. With a
        // configured-but-empty store the middleware now DENIES; only a truly
        // source-less store keeps the loopback posture.
        let env_source = std::env::var("AUTH_TOKEN")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .is_some();
        state.initialized = file.is_some() || env_source || !state.tokens.is_empty();
        if let Some(p) = &file {
            state.mtime = std::fs::metadata(p).and_then(|m| m.modified()).ok();
        }
        Self {
            inner: Arc::new(RwLock::new(state)),
            file,
        }
    }

    /// Snapshot of the currently accepted token set. `NotConfigured` when no
    /// token source ever resolved (auth disabled — the middleware keeps its
    /// loopback posture), `Active` with the set otherwise, `ReadFailed` when
    /// the lock is poisoned (deny — never an allow-all empty set). This is
    /// the hot-path call.
    pub fn tokens(&self) -> TokenRead {
        match crate::concurrency::rwlock_read_measured(&self.inner) {
            Ok(g) if !g.initialized => TokenRead::NotConfigured,
            Ok(g) => TokenRead::Active(g.tokens.clone()),
            Err(_) => TokenRead::ReadFailed,
        }
    }

    /// True when the store is watching `AUTH_TOKEN_FILE` (so the rotation
    /// watcher is meaningful). The server uses this to decide whether to spawn
    /// the watcher at startup.
    pub fn has_file(&self) -> bool {
        self.file.is_some()
    }

    /// Reload from disk if the file's mtime advanced since the last load.
    /// Fail-safe: if the file is missing/unreadable/empty AFTER a successful
    /// initial load, keep the cached tokens and log a warning. Returns `true`
    /// when a real rotation happened (caller may audit + log).
    ///
    /// Tests pass an explicit token string so no env var lookup is needed (the env
    /// layer is exercised by the server's `TokenStore::new` once at startup).
    pub fn reload_if_changed_from(&self, fresh_tokens: Vec<String>) -> bool {
        let path = match &self.file {
            Some(p) => p.clone(),
            None => return false,
        };
        let new_mtime = match std::fs::metadata(&path).and_then(|m| m.modified()) {
            Ok(m) => m,
            Err(_) => return false, // fail-safe: keep cache
        };
        let prev_mtime = crate::concurrency::rwlock_read_measured(&self.inner)
            .ok()
            .and_then(|s| s.mtime);
        if Some(new_mtime) == prev_mtime {
            return false;
        }
        if fresh_tokens.is_empty() {
            return false; // fail-safe: file became empty
        }
        let fresh_set: HashSet<String> = fresh_tokens.into_iter().collect();
        let changed = crate::concurrency::rwlock_read_measured(&self.inner)
            .map(|g| g.tokens != fresh_set)
            .unwrap_or(true);
        if let Ok(mut guard) = crate::concurrency::rwlock_write_measured(&self.inner) {
            // The swap is TWO single assignments under one exclusive guard —
            // no await, no call-outs (sync fn, compile-level). Pinned at
            // runtime by `token_rotation_swap_is_single_assignment` below.
            guard.tokens = fresh_set;
            guard.mtime = Some(new_mtime);
        }
        changed
    }

    /// Reload from disk via [`config::auth_tokens`] if the file's mtime advanced
    /// since the last load. Fail-safe: if the file is missing/unreadable/empty
    /// AFTER a successful initial load, keep the cached tokens. Returns `true`
    /// when a real rotation happened (caller may audit + log).
    pub fn reload_if_changed(&self) -> bool {
        self.reload_if_changed_from(config::auth_tokens())
    }
}

impl Default for TokenStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Spawn the rotation watcher. Polls every `TOKEN_ROTATION_POLL_SECS`; on a real
/// content change writes an `auth_token_rotated` audit row (no PII — target_hash
/// is the file path, detail_hash is the literal string "rotated"). Returns a
/// handle for tests; the server just lets it run for the process lifetime.
pub fn spawn_rotation_watcher(store: TokenStore, db_path: PathBuf) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(
            config::TOKEN_ROTATION_POLL_SECS,
        ));
        loop {
            interval.tick().await;
            if store.reload_if_changed() {
                // Audit: open a fresh connection (rare event — rotation happens
                // a handful of times in the server's lifetime). Best-effort.
                if let Ok(conn) = Connection::open(&db_path) {
                    audit::record(
                        &conn,
                        AuditKind::Auth,
                        "operator",
                        &db_path.to_string_lossy(),
                        AuditStatus::Ok,
                        "token-rotated",
                    );
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_token_file(contents: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        f.flush().unwrap();
        f
    }

    /// Construct a store pointed at an explicit path with an explicit initial
    /// token set — no env-var lookup, so tests are isolated from each other and
    /// from the server's own startup env read.
    fn store_for(path: PathBuf, initial_tokens: Vec<String>) -> TokenStore {
        let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        TokenStore {
            inner: Arc::new(RwLock::new(TokenState {
                tokens: initial_tokens.into_iter().collect(),
                mtime,
                initialized: true,
            })),
            file: Some(path),
        }
    }

    #[test]
    fn reload_picks_up_new_token() {
        let f = write_token_file("token-v1\n");
        let store = store_for(f.path().to_path_buf(), vec!["token-v1".to_string()]);
        let TokenRead::Active(initial) = store.tokens() else {
            panic!("configured store must read Active");
        };
        assert!(initial.contains("token-v1"));

        // Rewrite the file with a different token. Sleep >1s to guarantee mtime
        // advances past the second boundary (APFS/HFS+/ext4 all use 1-s mtime
        // resolution by default) — avoids a same-second false negative.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(f.path(), "token-v2\n").unwrap();

        assert!(
            store.reload_if_changed_from(vec!["token-v2".to_string()]),
            "rotation must be detected"
        );
        let TokenRead::Active(after) = store.tokens() else {
            panic!("configured store must read Active");
        };
        assert!(after.contains("token-v2"), "new token must be active");
        assert!(!after.contains("token-v1"), "old token must be gone");
    }

    #[test]
    fn reload_keeps_cache_when_file_deleted() {
        let f = write_token_file("only-token\n");
        let path = f.path().to_path_buf();
        let store = store_for(path.clone(), vec!["only-token".to_string()]);
        let TokenRead::Active(first) = store.tokens() else {
            panic!("configured store must read Active");
        };
        assert!(first.contains("only-token"));

        // Simulate deletion. Disown the temp file so its Drop doesn't panic.
        let _ = std::fs::remove_file(&path);
        let _ = f.keep();

        assert!(
            !store.reload_if_changed_from(vec!["only-token".to_string()]),
            "deletion must NOT be reported as a rotation"
        );
        let TokenRead::Active(cached) = store.tokens() else {
            panic!("configured store must read Active");
        };
        assert!(
            cached.contains("only-token"),
            "fail-safe: cached token must stay in effect after deletion"
        );
    }

    #[test]
    fn reload_keeps_cache_when_file_emptied() {
        let f = write_token_file("real-token\n");
        let store = store_for(f.path().to_path_buf(), vec!["real-token".to_string()]);
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(f.path(), "   \n").unwrap();
        // An emptied file must NOT clear auth — fail-safe keeps the cache.
        assert!(
            !store.reload_if_changed_from(vec![]),
            "empty load is not a rotation"
        );
        let TokenRead::Active(cached) = store.tokens() else {
            panic!("configured store must read Active");
        };
        assert!(cached.contains("real-token"));
    }

    /// a poisoned lock must read as
    /// `ReadFailed` (deny at the middleware), never as an empty set
    /// ("auth disabled" → allow-all).
    #[test]
    fn poisoned_token_store_reads_as_read_failed() {
        let state = TokenState {
            tokens: std::collections::HashSet::from(["only-token".to_string()]),
            mtime: None,
            initialized: true,
        };
        let inner = Arc::new(RwLock::new(state));
        // Poison the lock from another thread: acquire the write guard and
        // panic while holding it (join contains the child panic; the lock is
        // poisoned for every subsequent reader).
        let handle = {
            let inner = inner.clone();
            std::thread::spawn(move || {
                let _guard = inner.write().expect("lock before panic");
                panic!("poison the token lock");
            })
        };
        let _ = handle.join();
        let store = TokenStore {
            inner,
            file: Some(PathBuf::from("/nonexistent/token-file")),
        };
        assert_eq!(store.tokens(), TokenRead::ReadFailed);
    }

    /// Headroom pin: the rotation swap is SINGLE-ASSIGNMENT at runtime — a
    /// reader hammering `tokens()` while a writer rotates between two sets
    /// only ever observes one of the two valid sets, never a torn state
    /// (empty, merged, or partial). The sync write guard makes an await
    /// inside the swap impossible at compile time; this is the runtime twin.
    /// Runs through the REAL reload path (a temp token file whose mtime
    /// advances each write) so the swap actually executes.
    #[test]
    fn token_rotation_swap_is_single_assignment() {
        let set_a: HashSet<String> = ["token-a"].iter().map(|s| s.to_string()).collect();
        let set_b: HashSet<String> = ["token-b"].iter().map(|s| s.to_string()).collect();
        let f = write_token_file("token-a\n");
        let store = std::sync::Arc::new(store_for(
            f.path().to_path_buf(),
            vec!["token-a".to_string()],
        ));

        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut readers = Vec::new();
        for _ in 0..4 {
            let store = std::sync::Arc::clone(&store);
            let stop = Arc::clone(&stop);
            let a = set_a.clone();
            let b = set_b.clone();
            readers.push(std::thread::spawn(move || {
                let mut torn = 0usize;
                let mut reads = 0usize;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    reads += 1;
                    match store.tokens() {
                        TokenRead::Active(t) => {
                            if t != a && t != b {
                                torn += 1; // neither the old nor the new set
                            }
                        }
                        // Auth unconfigured/failed mid-swap would ALSO be a
                        // tear for a store known to be initialized.
                        _ => torn += 1,
                    }
                }
                (reads, torn)
            }));
        }

        // Rotate A→B→A … while the readers hammer the hot read. The file
        // write advances the mtime so reload_if_changed_from takes the swap
        // path; the 1 ms pacing gives the concurrent window real width.
        let mut flipped_to_b = false;
        let mut swaps = 0usize;
        for _ in 0..200 {
            let (content, tokens) = if flipped_to_b {
                ("token-a\n", vec!["token-a".to_string()])
            } else {
                ("token-b\n", vec!["token-b".to_string()])
            };
            std::fs::write(f.path(), content).expect("rotate token file");
            if store.reload_if_changed_from(tokens) {
                swaps += 1;
            }
            flipped_to_b = !flipped_to_b;
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(
            swaps > 50,
            "the rotation must actually execute the swap path ({swaps} swaps)"
        );

        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let (total_reads, total_torn): (usize, usize) = readers
            .into_iter()
            .map(|h| h.join().expect("reader thread"))
            .fold((0, 0), |(r, t), (r2, t2)| (r + r2, t + t2));
        assert!(
            total_reads > 1_000,
            "the readers must have actually exercised the hot path ({total_reads} reads)"
        );
        assert_eq!(
            total_torn, 0,
            "every read must observe exactly one of the two valid sets"
        );
    }

    /// a secret file with group/world bits is refused —
    /// the server fails closed rather than serve a leaked token. Owner-only
    /// (0600/0400) passes; a missing file errors (it cannot be validated).
    #[cfg(unix)]
    #[test]
    fn check_secret_permissions_enforces_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let f = write_token_file("tok\n");
        let path = f.path();

        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(check_secret_permissions(path), Ok(()));
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o400)).unwrap();
        assert_eq!(check_secret_permissions(path), Ok(()));

        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = check_secret_permissions(path).unwrap_err();
        assert!(err.contains("644"), "error names the offending mode: {err}");

        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let missing = std::env::temp_dir().join("brain-test-no-such-token-file");
        let _ = std::fs::remove_file(&missing);
        assert!(
            check_secret_permissions(&missing).is_err(),
            "unstatable file cannot be validated"
        );
    }
}
