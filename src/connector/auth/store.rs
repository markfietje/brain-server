//! Per-connector credential store.
//!
//! Each connector instance has one JSON config file at:
//!
//! ```text
//! ~/.config/brain-server/connectors/{kind}-{instance}.json   (mode 0600)
//! ```
//!
//! The file contents are opaque to the server — the connector owns its
//! schema. Convention for *secret* values (PEM keys, refresh tokens, webhook
//! secrets): **store the path to a 0600 file holding the secret**, never the
//! secret itself. This matches the existing `AUTH_TOKEN_FILE` /
//! `BRAIN_TOKEN_FILE` ladder and keeps secret rotation a `cp` away.
//!
//! ## Threat model & honest ceiling
//!
//! No at-rest encryption. The 0600 mode + macOS FileVault / Linux LUKS is the
//! only at-rest protection. An attacker with filesystem read access has both
//! the config and any referenced secret files; the boundary is "filesystem
//! access" not "memory access". Adding AES-GCM with a master key file would
//! narrow the threat to "config-file backup leak" only — a real but narrow
//! case that doesn't justify the ~200 LOC of crypto plumbing.
//! `ponytail:` revisit if brain-server is ever deployed multi-tenant.

use anyhow::{Context, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};

/// Default root for per-connector config files. Overridable via
/// `BRAIN_CONNECTOR_CONFIG_DIR` for tests.
const DEFAULT_ROOT_DIR: &str = ".config/brain-server/connectors";

/// Per-connector config handle. Generic over the connector's own config
/// shape (`T`) so each connector binary gets typed access without the server
/// knowing the schema.
#[derive(Debug)]
pub struct CredentialStore<T> {
    config: T,
    config_path: PathBuf,
}

impl<T> CredentialStore<T>
where
    T: DeserializeOwned + Serialize,
{
    /// Load the config for `(kind, instance)`. The path resolves to
    /// `$BRAIN_CONNECTOR_CONFIG_DIR/{kind}-{instance}.json` (defaults to
    /// `~/.config/brain-server/connectors/...`). Errors if the file is
    /// missing or unparseable; the connector surfaces this loudly rather
    /// than silently degrading.
    pub fn load(kind: &str, instance: &str) -> Result<Self> {
        let path = path_for(kind, instance)?;
        Self::load_from(&path)
    }

    /// Load from an explicit path — used by tests that want a tempfile.
    pub fn load_from(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path)
            .with_context(|| format!("failed to read connector config {}", path.display()))?;
        let config: T = serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to parse connector config {}", path.display()))?;
        Ok(Self {
            config,
            config_path: path.to_path_buf(),
        })
    }

    /// Save the config back to its source path. Atomic on POSIX via
    /// `std::fs::rename` — write to a sibling temp file then rename. Used by
    /// OAuth refresh-token rotation and by `brain connect` writing
    /// the initial config.
    ///
    /// `ponytail:` uses a sibling tempfile + `std::fs::rename` instead of
    /// `tempfile::NamedTempFile::persist` to avoid promoting `tempfile` from
    /// a dev-dep to a regular dep just for one method.
    pub fn save(&self) -> Result<()>
    where
        T: Serialize,
    {
        let parent = self
            .config_path
            .parent()
            .context("config path has no parent")?;
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config dir {}", parent.display()))?;

        let tmp_suffix = format!(
            ".tmp.{}.{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let tmp_path = parent.join(format!(
            "{}{tmp_suffix}",
            self.config_path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("cfg")
        ));
        let bytes = serde_json::to_vec_pretty(&self.config)?;
        std::fs::write(&tmp_path, &bytes)?;
        // chmod 0600 on the tempfile BEFORE rename so the secret is never
        // world-readable on disk, even briefly.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600))
                .with_context(|| format!("failed to chmod 0600 {}", tmp_path.display()))?;
        }
        std::fs::rename(&tmp_path, &self.config_path).with_context(|| {
            format!(
                "failed to atomically rename {} -> {}",
                tmp_path.display(),
                self.config_path.display()
            )
        })?;
        Ok(())
    }

    /// Immutable access to the typed config.
    pub fn config(&self) -> &T {
        &self.config
    }

    /// The path this store was loaded from.
    pub fn path(&self) -> &Path {
        &self.config_path
    }
}

/// Compute the config path for `(kind, instance)`. Honors
/// `BRAIN_CONNECTOR_CONFIG_DIR`; otherwise falls back to
/// `$HOME/.config/brain-server/connectors/`.
///
/// `instance` may contain `/` (e.g. `markfietje/brain-server`) — we replace
/// it with `_` so the filename is portable and doesn't create nested dirs.
pub fn path_for(kind: &str, instance: &str) -> Result<PathBuf> {
    let root: PathBuf = match std::env::var("BRAIN_CONNECTOR_CONFIG_DIR") {
        Ok(s) if !s.trim().is_empty() => PathBuf::from(s),
        _ => {
            let home = std::env::var("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("."));
            home.join(DEFAULT_ROOT_DIR)
        }
    };
    let safe_instance = instance.replace('/', "_");
    Ok(root.join(format!("{kind}-{safe_instance}.json")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct FakeConfig {
        app_id: i64,
        installation_id: i64,
        private_key_path: String,
        webhook_secret_path: String,
    }

    fn sample() -> FakeConfig {
        FakeConfig {
            app_id: 123456,
            installation_id: 789012,
            private_key_path: "/etc/brain-server/gh-app.private-key.pem".to_string(),
            webhook_secret_path: "/etc/brain-server/gh-app.webhook-secret".to_string(),
        }
    }

    // Env-mutating tests must be serialized because Rust runs tests in
    // parallel and `BRAIN_CONNECTOR_CONFIG_DIR` is process-global. The mutex
    // is acquired for the duration of each env-touching test.
    //
    // ponytail: would be cleaner to inject the root via a parameter, but
    // that'd leak test concerns into the production API. A test-only mutex
    // is the smallest honest fix.
    fn env_test_lock() -> std::sync::MutexGuard<'static, ()> {
        use std::sync::Mutex;
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn test_path_for_honors_env_and_falls_back_to_home() {
        let _guard = env_test_lock();

        // Env set → use it, and replace `/` in instance with `_` for filename safety.
        unsafe { std::env::set_var("BRAIN_CONNECTOR_CONFIG_DIR", "/tmp/brain-cfg-test") };
        let p = path_for("github", "markfietje/brain-server").unwrap();
        assert_eq!(
            p,
            PathBuf::from("/tmp/brain-cfg-test/github-markfietje_brain-server.json")
        );

        // Env unset → fall back to $HOME/.config/brain-server/connectors/.
        unsafe { std::env::remove_var("BRAIN_CONNECTOR_CONFIG_DIR") };
        let p = path_for("github", "foo").unwrap();
        assert!(p.ends_with(".config/brain-server/connectors/github-foo.json"));
    }

    #[test]
    fn test_load_roundtrip_preserves_typed_config() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("github-test.json");
        std::fs::write(&path, serde_json::to_vec_pretty(&sample()).unwrap()).unwrap();

        let store: CredentialStore<FakeConfig> = CredentialStore::load_from(&path).unwrap();
        assert_eq!(store.config(), &sample());
        assert_eq!(store.path(), &path);
    }

    #[test]
    fn test_save_is_atomic_and_chmods_0600() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("github-save.json");
        let store = CredentialStore::<FakeConfig> {
            config: sample(),
            config_path: path.clone(),
        };
        store.save().unwrap();

        // File exists with the right contents.
        let store2: CredentialStore<FakeConfig> = CredentialStore::load_from(&path).unwrap();
        assert_eq!(store2.config(), &sample());

        // Mode is 0600.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(
                mode & 0o777,
                0o600,
                "config file should be 0600, got {:o}",
                mode & 0o777
            );
        }
    }

    #[test]
    fn test_load_errors_clearly_on_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("does-not-exist.json");
        let err = CredentialStore::<FakeConfig>::load_from(&path).unwrap_err();
        assert!(
            err.to_string().contains("failed to read"),
            "error should mention read failure, got: {err}"
        );
    }

    #[test]
    fn test_load_errors_clearly_on_malformed_json() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("bad.json");
        std::fs::write(&path, b"{not json").unwrap();
        let err = CredentialStore::<FakeConfig>::load_from(&path).unwrap_err();
        assert!(
            err.to_string().contains("failed to parse"),
            "error should mention parse failure, got: {err}"
        );
    }
}
