//! The bridge config: `channel-whatsapp-{tenant}.json`, owner-only (0600),
//! the SAME shared substrate file the kernel reads (domain + webhook_secret)
//! enriched with the WhatsApp edge keys. Fail-closed everywhere: bad perms,
//! unreadable secrets, or absurd sizes refuse at LOAD, never silently run.

use anyhow::{Context, Result};
use serde_json::Value;
use std::path::Path;

const MAX_SECRET_BYTES: usize = 256;

#[derive(Debug, Clone)]
pub(crate) struct BridgeConfig {
    /// The `{kind}` route segment (always `whatsapp` in this crate).
    pub(crate) kind: String,
    /// The `{tenant}` segment of the config filename.
    pub(crate) tenant: String,
    /// The registered domain every case under this bridge lives in.
    pub(crate) domain: String,
    /// Standard-Webhooks secret (kernel seam signing).
    pub(crate) webhook_secret: Vec<u8>,
    /// Meta subscription verify token (handshake echo gate).
    pub(crate) verify_token: String,
    /// Cloud API phone-number id.
    pub(crate) phone_number_id: String,
    /// Meta app secret bytes (hub signature verification).
    pub(crate) app_secret: Vec<u8>,
    /// Permanent system-user token bytes (Cloud API sends).
    pub(crate) access_token: Vec<u8>,
    /// Graph API version stamp (`v21.0` etc., pinned per deployment).
    pub(crate) graph_api_version: String,
    /// SHA-256 of the FULL config-file bytes (mount evidence; the kernel
    /// recomputes it from its own copy — neither side self-certifies).
    pub(crate) config_sha256: String,
}

fn read_perm_file(p: &Path) -> Result<Vec<u8>> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(p).with_context(|| format!("stat {}", p.display()))?;
        if meta.permissions().mode() & 0o077 != 0 {
            anyhow::bail!("{} must be owner-only (0600); refusing", p.display());
        }
    }
    let mut bytes = std::fs::read(p).with_context(|| format!("read {}", p.display()))?;
    if bytes.len() > MAX_SECRET_BYTES {
        anyhow::bail!("{} exceeds {MAX_SECRET_BYTES} bytes; refusing", p.display());
    }
    while matches!(bytes.last(), Some(b'\n') | Some(b'\r') | Some(b' ')) {
        bytes.pop();
    }
    if bytes.is_empty() {
        anyhow::bail!("{} is empty after trim; refusing", p.display());
    }
    Ok(bytes)
}

fn str_field(v: &Value, key: &str) -> Result<String> {
    let s = v
        .get(key)
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty() && s.len() <= 256)
        .context(format!("missing/oversized string field {key}"))?;
    Ok(s.to_string())
}

fn valid_segment(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

impl BridgeConfig {
    /// Load + validate one bridge config. Any suspicion refuses loudly — a
    /// misconfigured edge must be visible, never silently dark.
    pub(crate) fn load(path: &Path) -> Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta =
                std::fs::metadata(path).with_context(|| format!("stat {}", path.display()))?;
            if meta.permissions().mode() & 0o077 != 0 {
                anyhow::bail!("config must be owner-only (0600); refusing to trust it");
            }
        }
        let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let v: Value =
            serde_json::from_slice(&bytes).with_context(|| "config is not valid JSON")?;

        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .context("unrepresentable config name")?
            .to_string();
        let stem = name
            .strip_prefix("channel-")
            .and_then(|s| s.strip_suffix(".json"))
            .context("name must be channel-{kind}-{tenant}.json")?;
        let (kind, tenant) = stem
            .split_once('-')
            .context("name must carry kind and tenant segments")?;
        if kind != "whatsapp" {
            anyhow::bail!("this binary is the whatsapp edge (got kind {kind:?})");
        }
        if !valid_segment(tenant) {
            anyhow::bail!("invalid tenant segment {tenant:?}");
        }

        let domain = str_field(&v, "domain")?;
        if !valid_domain(&domain) {
            anyhow::bail!("invalid domain label {domain:?}");
        }
        let webhook_secret = str_field(&v, "webhook_secret")?.into_bytes();
        let verify_token = str_field(&v, "verify_token")?;
        let phone_number_id = str_field(&v, "phone_number_id")?;
        let app_secret_path = str_field(&v, "app_secret_path")
            .map_err(|e| e.context("app_secret_path resolves where the app secret lives"))
            .and_then(|p| resolve_path(path, &p))?;
        let access_token_path = str_field(&v, "access_token_path")
            .map_err(|e| e.context("access_token_path resolves where the system-user token lives"))
            .and_then(|p| resolve_path(path, &p))?;
        let app_secret = read_perm_file(&app_secret_path)?;
        let access_token = read_perm_file(&access_token_path)?;
        // Reject path-traversal resolved ABOVE against the config dir only:
        // absolute paths are allowed (operator-owned machines), but any
        // relative component that escapes is nonsense.
        if graph_version_invalid(&v) {
            anyhow::bail!("graph_api_version must look like vNN.N");
        }
        let graph_api_version = v
            .get("graph_api_version")
            .and_then(|x| x.as_str())
            .unwrap_or("v21.0")
            .to_string();

        Ok(Self {
            kind: kind.to_string(),
            tenant: tenant.to_string(),
            domain,
            webhook_secret,
            verify_token,
            phone_number_id,
            app_secret,
            access_token,
            graph_api_version,
            config_sha256: crate::hubsig::sha256_hex(&bytes),
        })
    }
}

fn valid_domain(d: &str) -> bool {
    !d.is_empty()
        && d.len() <= 63
        && d.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Resolve a PATH-typed config value relative to the CONFIG DIR when relative.
fn resolve_path(config_path: &Path, p: &str) -> Result<std::path::PathBuf> {
    let candidate = Path::new(p);
    let resolved = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        config_path
            .parent()
            .unwrap_or(Path::new("."))
            .join(candidate)
    };
    for comp in resolved.components() {
        if matches!(comp, std::path::Component::ParentDir) {
            anyhow::bail!("secret paths may not traverse upward ({p})");
        }
    }
    Ok(resolved)
}

fn graph_version_invalid(v: &Value) -> bool {
    match v.get("graph_api_version").and_then(|x| x.as_str()) {
        None => false,
        Some(s) => {
            !(s.starts_with('v')
                && s.len() > 2
                && s[1..]
                    .split('.')
                    .all(|part| part.bytes().all(|b| b.is_ascii_digit())))
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic, clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    const GOOD_BODY: &[u8] = br#"{
        "domain":"acme",
        "webhook_secret":"whsec-x",
        "verify_token":"vt",
        "phone_number_id":"1234567890",
        "app_secret_path":"app_secret.txt",
        "access_token_path":"token.txt"
    }"#;

    #[test]
    fn config_fail_closed_on_perms_and_traversal() {
        // World-readable config → refuse.
        let dir = tempfile::tempdir().unwrap();
        let loose = dir.path().join("channel-whatsapp-acme.json");
        std::fs::write(&loose, GOOD_BODY).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&loose, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        assert!(BridgeConfig::load(&loose).is_err());

        // Upward-traversing secret path → refuse even with clean perms.
        let tight = dir.path().join("channel-whatsapp-beta.json");
        std::fs::write(
            &tight,
            br#"{"domain":"beta","webhook_secret":"s","verify_token":"t",
                 "phone_number_id":"1","app_secret_path":"../esc.txt",
                 "access_token_path":"token.txt"}"#,
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tight, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert!(BridgeConfig::load(&tight).is_err());

        // Wrong KIND in filename → refuse (wrong binary for that config).
        let wrong = dir.path().join("channel-signal-acme.json");
        std::fs::write(&wrong, GOOD_BODY).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&wrong, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert!(BridgeConfig::load(&wrong).is_err());
    }
}
