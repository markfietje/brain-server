//! The secret broker: the one seam through which engine-facing key material
//! resolves. Secrets never cross a hostcall boundary in plaintext — callers
//! get a handle name, the broker reads the file at use time.
//!
//! Fail-closed invariant: a `BRAIN_*_KEY` file that exists with group/world
//! bits refuses resolution (the `check_secret_permissions` posture) — a wide
//! mode is an incident, never a downgrade to environment fallback. Missing
//! configuration also refuses loudly; callers surface
//! `AuthStoreUnavailable`/`Internal`, never an empty secret.

use crate::auth::check_secret_permissions;

/// Resolution failure vocabulary.
#[derive(Debug, Clone, PartialEq)]
pub enum SecretError {
    /// The store itself is unusable (unreadable / unsafe mode) — deny, audit.
    AuthStoreUnavailable(String),
    /// No source configured for this name.
    NotConfigured(String),
}

impl std::fmt::Display for SecretError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SecretError::AuthStoreUnavailable(m) => write!(f, "auth store unavailable: {m}"),
            SecretError::NotConfigured(name) => write!(f, "no secret configured for `{name}`"),
        }
    }
}

impl std::error::Error for SecretError {}

/// Env var holding the file path for a named secret.
fn file_var(name: &str) -> String {
    format!("BRAIN_{}{}_KEY_FILE", name.to_ascii_uppercase(), "")
}

/// Resolve named key material: `BRAIN_<NAME>_KEY_FILE` (mode-checked) →
/// `BRAIN_<NAME>_KEY` (inline, last resort). A wide-mode file fails closed;
/// it does NOT fall through to any other source.
pub fn resolve(name: &str) -> Result<String, SecretError> {
    if let Ok(path) = std::env::var(file_var(name)) {
        check_secret_permissions(std::path::Path::new(&path))
            .map_err(SecretError::AuthStoreUnavailable)?;
        return std::fs::read_to_string(&path)
            .map(|s| s.trim().to_string())
            .map_err(|e| SecretError::AuthStoreUnavailable(format!("read failed: {e}")));
    }
    match std::env::var(format!("BRAIN_{}_KEY", name.to_ascii_uppercase())) {
        Ok(v) if !v.trim().is_empty() => Ok(v.trim().to_string()),
        _ => Err(SecretError::NotConfigured(name.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_env(k: impl AsRef<str>, v: impl AsRef<str>) {
        // SAFETY (test-only): single-threaded env mutation in this module's tests.
        unsafe { std::env::set_var(k.as_ref(), v.as_ref()) }
    }

    fn del_env(k: impl AsRef<str>) {
        // SAFETY (test-only): single-threaded env mutation in this module's tests.
        unsafe { std::env::remove_var(k.as_ref()) }
    }

    #[test]
    fn missing_configuration_refuses_loudly() {
        // Name chosen so neither env var is set in any environment.
        let name = "definitely_unconfigured_test_secret";
        del_env(format!("BRAIN_{}_KEY", name.to_ascii_uppercase()));
        del_env(file_var(name));
        assert_eq!(
            resolve(name).unwrap_err(),
            SecretError::NotConfigured(name.to_string())
        );
    }

    #[test]
    fn inline_env_source_resolves_trimmed() {
        let name = "broker_inline_test";
        let var = format!("BRAIN_{}_KEY", name.to_ascii_uppercase());
        set_env(&var, "  sekrit \n");
        let out = resolve(name);
        del_env(&var);
        assert_eq!(out.unwrap(), "sekrit");
    }

    #[test]
    fn wide_mode_file_fails_closed_without_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wide.key");
        std::fs::write(&path, "material").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let name = "broker_wide_test";
        set_env(file_var(name), path.to_string_lossy().as_ref());
        // Even with an inline env value present, the wide-mode FILE refuses —
        // fail-closed means no silent downgrade to another source.
        set_env(
            format!("BRAIN_{}_KEY", name.to_ascii_uppercase()),
            "fallback-material",
        );
        let out = resolve(name);
        del_env(file_var(name));
        del_env(format!("BRAIN_{}_KEY", name.to_ascii_uppercase()));
        assert!(
            matches!(out, Err(SecretError::AuthStoreUnavailable(_))),
            "a group/world-readable key file must refuse, not fall back"
        );
    }

    #[test]
    fn owner_only_file_resolves() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tight.key");
        std::fs::write(&path, " material \n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let name = "broker_tight_test";
        set_env(file_var(name), path.to_string_lossy().as_ref());
        let out = resolve(name);
        del_env(file_var(name));
        assert_eq!(out.unwrap(), "material");
    }
}
