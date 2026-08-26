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
        let contents = fs::read_to_string(path.as_ref())
            .with_context(|| format!("Failed to read config file: {}", path.as_ref().display()))?;

        let config: Config =
            serde_yaml::from_str(&contents).context("Failed to parse config file")?;

        Ok(config)
    }
}
