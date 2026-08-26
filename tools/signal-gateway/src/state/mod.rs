//! Application State

use anyhow::Result;

use crate::brain::{BrainClient, BridgeConfig, OutgoingEnvelope};
use crate::config::Config;
use crate::signal::{ManagerConfig, SignalHandle, SignalWorker};

#[derive(Clone)]
pub struct AppState {
    pub signal: SignalHandle,
}

impl AppState {
    pub fn new(config: Config) -> Result<Self> {
        let manager_config = ManagerConfig {
            db_path: format!("{}/signal.db", config.signal.data_dir),
            command_channel_capacity: config.signal.command_channel_capacity,
            message_broadcast_capacity: config.signal.message_broadcast_capacity,
            command_timeout_ms: config.signal.command_timeout_ms,
            max_sends_per_second: config.signal.max_sends_per_second,
            display_name: config.signal.display_name.clone(),
        };

        // Storage-permission hardening: signal.db holds identity keys +
        // registration data (including the account number). Enforce 0700 on
        // the directory and 0600 on the database file before presage opens
        // it; failures warn loudly but never silently pass.
        {
            use std::os::unix::fs::PermissionsExt;
            let dir = std::path::Path::new(&config.signal.data_dir);
            if let Err(e) = std::fs::create_dir_all(dir) {
                tracing::warn!("data_dir create failed: {e}");
            }
            if let Ok(meta) = std::fs::metadata(dir)
                && meta.permissions().mode() & 0o077 != 0
            {
                let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
                tracing::warn!("data_dir tightened to 0700");
            }
            let db = std::path::PathBuf::from(&manager_config.db_path);
            if !db.exists()
                && let Err(e) = std::fs::File::create(&db)
            {
                tracing::warn!("db pre-create failed: {e}");
            }
            if let Ok(meta) = std::fs::metadata(&db)
                && meta.permissions().mode() & 0o177 != 0
            {
                let _ = std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o600));
                tracing::warn!("signal.db tightened to 0600");
            }
        }

        let worker = SignalWorker::spawn(manager_config)?;
        let signal = worker.handle();
        std::mem::forget(worker);

        Ok(Self { signal })
    }

    pub async fn init_signal(&self) -> Result<bool> {
        let loaded = self.signal.load_registered().await?;
        if loaded && let Err(e) = self.signal.start_receiver().await {
            tracing::warn!("Failed to auto-start receiver: {}", e);
        }
        Ok(loaded)
    }

    /// Spawn the Switchboard seam: mount evidence at boot, the inbound
    /// forwarder (broadcast → signed envelope POST) and the outbound drain
    /// crank. Called only when `brain:` is configured — absent config means
    /// the edge runs channel-dark (the documented rollback posture).
    pub fn start_brain_adapter(&self, brain: &crate::config::BrainConfig) {
        // Tilde expansion so `~/.config/...` works in YAML configs.
        let raw = brain.bridge_config_path.as_str();
        let expanded: std::path::PathBuf = if let Some(rest) = raw.strip_prefix("~/") {
            std::env::var_os("HOME")
                .map(|h| std::path::PathBuf::from(h).join(rest))
                .unwrap_or_else(|| std::path::PathBuf::from(raw))
        } else {
            std::path::PathBuf::from(raw)
        };
        let cfg = match BridgeConfig::load(&expanded) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("brain adapter disabled: {e:#}");
                return;
            }
        };
        let client = match BrainClient::new(&brain.url) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("brain adapter disabled: {e:#}");
                return;
            }
        };
        tracing::info!(
            "brain adapter armed for {} → {}",
            cfg.bridge_id(),
            brain.url
        );

        // Registration: one-shot with bounded retries; a permanently refused
        // mount surfaces as repeated loud warnings, never silence.
        tokio::spawn(Self::mount_task(client.clone(), cfg.clone(), expanded));

        // Inbound: broadcast fan-out → envelope post (log failures; the bus
        // does not buffer for slow consumers, so failures are VISIBLE lag).
        let signal = self.signal.clone();
        let in_client = client.clone();
        let in_cfg = cfg.clone();
        tokio::spawn(async move {
            Self::inbound_forwarder(signal, in_client, in_cfg).await;
        });

        // Outbound: periodic drain crank (missed ticks delay, never storm).
        let secs = brain.drain_interval_secs.max(5);
        let drain_signal = self.signal.clone();
        let drain_cfg = cfg.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(secs));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                if let Err(e) = Self::drain_once(&client, &drain_cfg, &drain_signal).await {
                    tracing::warn!("drain failed: {e:#}");
                }
            }
        });
    }

    async fn mount_task(client: BrainClient, cfg: BridgeConfig, path: std::path::PathBuf) {
        for attempt in 1u32..=5 {
            match client.register_mount(&cfg, &path).await {
                Ok(()) => {
                    tracing::info!("mount evidence registered for {}", cfg.bridge_id());
                    return;
                }
                Err(e) => {
                    tracing::warn!("mount attempt {attempt}/5 failed: {e:#}");
                    tokio::time::sleep(std::time::Duration::from_secs(10u64 * u64::from(attempt)))
                        .await;
                }
            }
        }
        tracing::error!(
            "mount evidence NOT registered after 5 attempts — visible chain gap upstream"
        );
    }

    async fn inbound_forwarder(signal: SignalHandle, client: BrainClient, cfg: BridgeConfig) {
        let mut rx = signal.subscribe();
        tracing::info!("inbound forwarder started ({})", cfg.bridge_id());
        loop {
            match rx.recv().await {
                Ok(msg) => {
                    let Some(dm) = &msg.envelope.data_message else {
                        continue;
                    };
                    // Direct conversations only; non-empty text required.
                    // group_info.group_id is Option<String> already.
                    let group = dm.group_info.as_ref().and_then(|g| g.group_id.clone());
                    let Some(text) = dm.message.as_deref() else {
                        continue;
                    };
                    if !crate::brain::forwardable(Some(text), group.as_deref()) {
                        continue;
                    }
                    let sender_uuid = msg.envelope.source_uuid.as_deref().unwrap_or("unknown");
                    let ts = dm
                        .timestamp
                        .unwrap_or_else(|| msg.envelope.timestamp.unwrap_or_default());
                    let env = OutgoingEnvelope::new(
                        sender_uuid,
                        text,
                        &crate::brain::external_id(sender_uuid, ts),
                    );
                    match client.post_inbound(&cfg, &env).await {
                        Ok(resp) => {
                            tracing::info!(
                                "inbound posted ({}) → {}",
                                env.external_id,
                                resp.get("status").and_then(|s| s.as_str()).unwrap_or("?")
                            );
                        }
                        Err(e) => {
                            tracing::error!("inbound post failed for {}: {e:#}", env.external_id)
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    tracing::warn!("inbound forwarder: channel closed");
                    break;
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("inbound forwarder lagged by {n} messages");
                }
            }
        }
        tracing::info!("inbound forwarder stopped");
    }

    async fn drain_once(
        client: &BrainClient,
        cfg: &BridgeConfig,
        signal: &SignalHandle,
    ) -> Result<()> {
        for envelope in client.drain(cfg).await? {
            let event_id = envelope
                .get("event_id")
                .and_then(|v| v.as_i64())
                .unwrap_or_default();
            let conversation_ref = envelope
                .get("conversation_ref")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let text = envelope
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if text.is_empty() || conversation_ref.is_empty() {
                tracing::warn!("drained envelope {event_id} missing text/conversation — skipped");
                continue;
            }
            match signal.send_message(conversation_ref, text).await {
                Ok(id) => tracing::info!(
                    "delivered channel/out {event_id} to {conversation_ref} (signal ts {id})"
                ),
                Err(e) => {
                    // At-least-once delivery law: the row is already marked
                    // delivered server-side, so a failed send cannot be
                    // retried by the crank — it must be LOUD here.
                    tracing::error!("DELIVERY FAILED for {event_id} → {conversation_ref}: {e}");
                }
            }
        }
        Ok(())
    }
}
