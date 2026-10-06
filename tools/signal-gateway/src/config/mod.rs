//! Configuration for Signal Gateway
//!
//! YAML configuration files with security settings.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub signal: SignalConfig,
    /// brain-server Switchboard seam; absent = edge runs channel-dark.
    #[serde(default)]
    pub brain: Option<BrainConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrainConfig {
    /// brain-server base URL, e.g. "http://127.0.0.1:8765"
    pub url: String,
    /// The SHARED 0600 bridge credential file
    /// (`channel-{kind}-{tenant}.json` in the server's connector dir).
    pub bridge_config_path: String,
    /// Drain poll cadence in seconds (default: 30).
    #[serde(default = "default_drain_secs")]
    pub drain_interval_secs: u64,
}

fn default_drain_secs() -> u64 {
    30
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Address to bind to. MUST be loopback (127.0.0.1) unless the env
    /// override SIGNAL_GATEWAY_ALLOW_REMOTE=1 is set at boot.
    pub address: String,

    /// When set, every API request must carry `Authorization: Bearer <token>`.
    /// Strongly recommended whenever anything but 127.0.0.1 could reach the
    /// port. None → unauthenticated (loopback posture).
    #[serde(default)]
    pub auth_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalConfig {
    /// Directory for Signal database
    pub data_dir: String,
    /// Directory for attachments
    pub attachments_dir: String,

    // Security settings
    /// Command channel capacity (default: 64)
    #[serde(default = "default_command_capacity")]
    pub command_channel_capacity: usize,

    /// Message broadcast capacity (default: 256)
    #[serde(default = "default_message_capacity")]
    pub message_broadcast_capacity: usize,

    /// Command timeout in milliseconds (default: 30000)
    #[serde(default = "default_command_timeout_ms")]
    pub command_timeout_ms: u64,

    /// Max sends per second for rate limiting (default: 5)
    #[serde(default = "default_max_sends_per_second")]
    pub max_sends_per_second: usize,

    /// Public presentation of THIS account (recommended: your Signal
    /// username, created on the primary app with number-discovery OFF).
    /// None → the gateway emits masked digits only.
    #[serde(default)]
    pub display_name: Option<String>,
}

fn default_command_capacity() -> usize {
    64
}
fn default_message_capacity() -> usize {
    256
}
fn default_command_timeout_ms() -> u64 {
    30_000
}
fn default_max_sends_per_second() -> usize {
    5
}

impl Default for SignalConfig {
    fn default() -> Self {
        Self {
            data_dir: "./data".to_string(),
            attachments_dir: "./attachments".to_string(),
            command_channel_capacity: default_command_capacity(),
            message_broadcast_capacity: default_message_capacity(),
            command_timeout_ms: default_command_timeout_ms(),
            max_sends_per_second: default_max_sends_per_second(),
            display_name: None,
        }
    }
}

impl Config {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        // The config carries `server.auth_token` — a secret file. The 0600
        // law the bridge credential, the relay secret and the presage store
        // already enforce applies here too: refuse group/world-readable (or
        // -writable) modes instead of quietly reading the token through
        // them. This was the one secret file in the edge without the law.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(path.as_ref())
                .with_context(|| {
                    format!("Failed to stat config file: {}", path.as_ref().display())
                })?
                .permissions()
                .mode();
            if mode & 0o077 != 0 {
                anyhow::bail!(
                    "config file {} is mode {:o} — group/world bits set. A file carrying \
                     server.auth_token must be 0600. Fix: chmod 600 {}",
                    path.as_ref().display(),
                    mode & 0o777,
                    path.as_ref().display()
                );
            }
        }
        let contents = fs::read_to_string(path.as_ref())
            .with_context(|| format!("Failed to read config file: {}", path.as_ref().display()))?;

        let config: Config =
            serde_yaml::from_str(&contents).context("Failed to parse config file")?;

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    // Test-only: the crate denies panic vectors in PRODUCTION code
    // (`Cargo.toml` `[lints.clippy]`); tests must fail loudly.
    #![allow(clippy::expect_used, clippy::panic)]
    use super::*;

    fn write_config(dir: &std::path::Path, mode: u32) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join("config.yaml");
        std::fs::write(
            &p,
            "server:\n  address: 127.0.0.1:8080\nsignal:\n  data_dir: /tmp/sg\n  \
             attachments_dir: /tmp/sg-att\n",
        )
        .expect("fixture write");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).expect("fixture chmod");
        p
    }

    #[test]
    fn a_world_readable_config_is_refused_not_read() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = write_config(dir.path(), 0o644);
        let err =
            Config::load(&p).expect_err("a 0644 config carries the auth token and must be refused");
        assert!(
            err.to_string().contains("0600"),
            "the refusal must name the remedy: {err}"
        );
    }

    #[test]
    fn a_private_config_loads() {
        // Anti-vacuity: the check must not learn to refuse everything.
        let dir = tempfile::tempdir().expect("tempdir");
        let p = write_config(dir.path(), 0o600);
        let cfg = Config::load(&p).expect("a 0600 config loads");
        assert_eq!(cfg.server.address, "127.0.0.1:8080");
    }
}
