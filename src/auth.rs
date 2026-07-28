//! v1.1.0 "Harden" M1.4 — fail-safe token rotation.
//!
//! Wraps [`config::auth_tokens`] in an in-memory cache that:
//! - reads from disk only when the file's mtime changes (every 5s), so the auth
//!   hot path is a cheap `RwLock` read instead of a `stat`+`read` per request;
//! - audited rotation event when the accepted token set actually changes;
//! - fail-safe: if the file is deleted, goes empty, or becomes unreadable, the
//!   last-good token set stays in effect. Auth is never silently cleared.
//!
//! Uses `auth::record` (the append-only audit) on rotation, not the request
//! path, so the audit row fires once per real change — never per request.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

use rusqlite::Connection;

use crate::audit::{self, AuditKind, AuditStatus};
use crate::config;

/// Cached accepted-token set + the file metadata that produced it. Cloning the
/// `Arc` is the cheap clone in the hot path; the inner write happens at most
/// once per rotation.
#[derive(Clone)]
pub struct TokenStore {
    inner: Arc<RwLock<TokenState>>,
    /// `Some(path)` when reading from `AUTH_TOKEN_FILE`; `None` when auth is
    /// driven by `AUTH_TOKEN` env or disabled. Used by the reload task to know
    /// whether to `stat()` at all.
    file: Option<PathBuf>,
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
        let file = std::env::var("AUTH_TOKEN_FILE")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        Self::from_file(file)
    }

    /// Construct from an explicit file path (or `None` for env-driven / no
    /// file). Tests use this to avoid env-var races under `cargo test`'s
    /// parallel runner. The server uses [`Self::new`] which reads the env once.
    pub fn from_file(file: Option<PathBuf>) -> Self {
        let mut state = TokenState::default();
        let initial = config::auth_tokens();
        state.tokens = initial.iter().cloned().collect();
        state.initialized = true;
        if let Some(p) = &file {
            state.mtime = std::fs::metadata(p).and_then(|m| m.modified()).ok();
        }
        Self {
            inner: Arc::new(RwLock::new(state)),
            file,
        }
    }

    /// Snapshot of the currently accepted tokens. Empty set = auth disabled
    /// (when no source ever resolved a token). This is the hot-path call.
    pub fn tokens(&self) -> HashSet<String> {
        self.inner
            .read()
            .map(|s| s.tokens.clone())
            .unwrap_or_default()
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
        let prev_mtime = self.inner.read().ok().and_then(|s| s.mtime);
        if Some(new_mtime) == prev_mtime {
            return false;
        }
        if fresh_tokens.is_empty() {
            return false; // fail-safe: file became empty
        }
        let fresh_set: HashSet<String> = fresh_tokens.into_iter().collect();
        let changed = self
            .inner
            .read()
            .map(|g| g.tokens != fresh_set)
            .unwrap_or(true);
        if let Ok(mut guard) = self.inner.write() {
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
        let initial = store.tokens();
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
        let after = store.tokens();
        assert!(after.contains("token-v2"), "new token must be active");
        assert!(!after.contains("token-v1"), "old token must be gone");
    }

    #[test]
    fn reload_keeps_cache_when_file_deleted() {
        let f = write_token_file("only-token\n");
        let path = f.path().to_path_buf();
        let store = store_for(path.clone(), vec!["only-token".to_string()]);
        assert!(store.tokens().contains("only-token"));

        // Simulate deletion. Disown the temp file so its Drop doesn't panic.
        let _ = std::fs::remove_file(&path);
        let _ = f.keep();

        assert!(
            !store.reload_if_changed_from(vec!["only-token".to_string()]),
            "deletion must NOT be reported as a rotation"
        );
        assert!(
            store.tokens().contains("only-token"),
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
        assert!(store.tokens().contains("real-token"));
    }
}
