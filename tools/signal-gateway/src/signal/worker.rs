//! Signal Worker - Complete Implementation with Recipient Resolution
//! Supports: UUID, Phone Numbers, Usernames

use anyhow::{Context, Result};
use futures_util::StreamExt;
use parking_lot::Mutex;
use presage::libsignal_service::content::ContentBody;
use presage::libsignal_service::protocol::ServiceId;
use presage::manager::{Manager, Registered};
use presage::model::identity::OnNewIdentity;
use presage::model::messages::Received;
use presage_store_sqlite::SqliteStore;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tokio::runtime::Builder;
use tokio::sync::{Semaphore, broadcast, mpsc, oneshot};
use tokio::task::LocalSet;

use super::commands::SignalCommand;
use super::types::{
    ManagerConfig, SignalAttachment, SignalDataMessage, SignalEnvelope, SignalGroupInfo,
    SignalMessage, SignalQuote,
};
use base64::Engine;

/// Recipient cache for resolving phone numbers/addresses to UUIDs
#[derive(Clone, Default)]
pub struct RecipientCache {
    cache: Arc<Mutex<HashMap<String, String>>>,
    self_aci: Arc<Mutex<Option<String>>>,
}

impl RecipientCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, key: String, uuid: String) {
        tracing::info!("[CACHE] Mapping {} -> {}", key, uuid);
        self.cache.lock().insert(key, uuid);
    }

    pub fn get(&self, key: &str) -> Option<String> {
        self.cache.lock().get(key).cloned()
    }

    /// Reverse lookup: get phone number from UUID
    #[allow(dead_code)]
    pub fn reverse_get(&self, uuid: &str) -> Option<String> {
        let cache = self.cache.lock();
        for (key, value) in cache.iter() {
            if value == uuid && Self::is_phone(key) {
                tracing::info!("[CACHE] Reverse lookup: {} -> {}", uuid, key);
                return Some(key.clone());
            }
        }
        None
    }

    pub fn set_self_aci(&self, aci: String) {
        tracing::info!("[CACHE] Self ACI: {}", aci);
        *self.self_aci.lock() = Some(aci);
    }

    pub fn get_self_aci(&self) -> Option<String> {
        self.self_aci.lock().clone()
    }

    /// Check if string is a valid UUID format
    fn is_uuid(s: &str) -> bool {
        s.len() == 36 && s.chars().filter(|&c| c == '-').count() == 4
    }

    /// Check if string is a phone number (starts with +)
    fn is_phone(s: &str) -> bool {
        s.starts_with("+") && s.len() >= 10
    }

    /// Check if string is a username (contains . but not -)
    fn is_username(s: &str) -> bool {
        s.contains(".") && !s.contains("-") && !s.starts_with("+")
    }

    /// Resolve recipient to UUID
    pub fn resolve(&self, recipient: &str) -> Result<String> {
        // Already a UUID
        if Self::is_uuid(recipient) {
            tracing::info!("[RESOLVE] Already UUID: {}", recipient);
            return Ok(recipient.to_string());
        }

        // Check cache for phone/username
        if let Some(uuid) = self.get(recipient) {
            tracing::debug!("[RESOLVE] Cache hit");
            return Ok(uuid);
        }

        // Phone number - try to use self ACI for self-messaging
        if Self::is_phone(recipient)
            && let Some(self_aci) = self.get_self_aci()
        {
            tracing::info!(
                "[RESOLVE] Using self ACI for phone: {} -> {}",
                recipient,
                self_aci
            );
            // Cache it for future
            self.insert(recipient.to_string(), self_aci.clone());
            return Ok(self_aci);
        }

        // Username - needs external resolution (not implemented yet)
        if Self::is_username(recipient) {
            tracing::warn!(
                "[RESOLVE] Username resolution not yet implemented: {}",
                recipient
            );
            anyhow::bail!("Username resolution requires calling /v1/cache/seed first with the UUID")
        }

        tracing::warn!("[RESOLVE] Cannot resolve: {}", recipient);
        anyhow::bail!(
            "Cannot resolve recipient: {}. Use UUID or seed the cache with /v1/cache/seed",
            recipient
        )
    }

    /// Full resolution: cache fast-paths first, THEN presage's websocket
    /// username → ACI lookup. Requires a manager clone because the lookup
    /// opens its own request cycle — callers are exactly the send paths,
    /// which already hold one.
    async fn resolve_via_manager(
        &self,
        manager: &mut Manager<SqliteStore, Registered>,
        recipient: &str,
    ) -> Result<String> {
        if let Ok(uuid) = self.resolve(recipient) {
            return Ok(uuid);
        }
        if Self::is_username(recipient) {
            let Some(aci) = manager
                .lookup_username(recipient)
                .await
                .map_err(|e| anyhow::anyhow!("username lookup failed: {e:?}"))?
            else {
                anyhow::bail!("username '{recipient}' not found on Signal");
            };
            let uuid = aci.service_id_string();
            tracing::info!("[RESOLVE] username {recipient} -> {uuid}");
            self.insert(recipient.to_string(), uuid.clone());
            return Ok(uuid);
        }
        anyhow::bail!(
            "Cannot resolve recipient: {}. Use UUID or seed the cache with /v1/cache/seed",
            recipient
        )
    }
}

/// Presentation form of OUR OWN phone number for every surface that leaves
/// the process (API responses, logs, broadcast payloads): the operator's
/// configured display label when set, else a heavily masked variant. The raw
/// number stays protocol-internal only.
fn present_self_number(raw: &str, display_name: &Option<String>) -> String {
    if let Some(name) = display_name {
        return name.clone();
    }
    // Panic-free slicing: guarded windows instead of [..] indexing.
    let dot = '\u{2022}';
    let digits: Vec<char> = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() < 4 {
        return format!("+{dot}\u{2026}{dot}");
    }
    let mut head = String::new();
    for c in digits.iter().take(2) {
        head.push(*c);
    }
    let mut tail = String::new();
    for c in digits.iter().rev().take(2).rev() {
        tail.push(*c);
    }
    format!("+{head}\u{2026}{tail}")
}

const MAX_MESSAGE_LENGTH: usize = 2000;

#[allow(dead_code)]
// clippy: Linked's Manager grew past the size threshold with the libsignal
// 0.99 bump, but this state machine lives ONCE per worker thread and handlers
// deliberately clone the Manager across await points — boxing would touch six
// call sites to optimize nothing.
#[allow(clippy::large_enum_variant)]
enum WorkerState {
    Uninitialized,
    Linked {
        manager: Manager<SqliteStore, Registered>,
        receiver_handle: Option<tokio::task::JoinHandle<()>>,
    },
    ShuttingDown,
}

#[derive(Clone)]
pub struct SignalHandle {
    command_tx: mpsc::Sender<SignalCommand>,
    message_tx: broadcast::Sender<SignalMessage>,
    #[allow(dead_code)]
    account_number: Arc<Mutex<Option<String>>>,
    is_linked: Arc<AtomicBool>,
    #[allow(dead_code)]
    receiver_running: Arc<AtomicBool>,
    #[allow(dead_code)]
    shutdown_requested: Arc<AtomicBool>,
    send_rate_limiter: Arc<Semaphore>,
    command_timeout_ms: u64,
    recipient_cache: RecipientCache,
}

impl SignalHandle {
    pub fn is_linked(&self) -> bool {
        self.is_linked.load(Ordering::Acquire)
    }
    #[allow(dead_code)]
    pub fn is_receiver_running(&self) -> bool {
        self.receiver_running.load(Ordering::Acquire)
    }
    #[allow(dead_code)]
    pub fn account_number(&self) -> Option<String> {
        self.account_number.lock().clone()
    }
    pub fn subscribe(&self) -> broadcast::Receiver<SignalMessage> {
        self.message_tx.subscribe()
    }
    pub fn get_recipient_cache(&self) -> RecipientCache {
        self.recipient_cache.clone()
    }

    fn validate_message(text: &str) -> Result<()> {
        if text.is_empty() {
            anyhow::bail!("Message cannot be empty");
        }
        if text.len() > MAX_MESSAGE_LENGTH {
            anyhow::bail!("Message too long");
        }
        Ok(())
    }

    pub async fn load_registered(&self) -> Result<bool> {
        tracing::info!("[HANDLE] Sending LoadRegistered command");
        let (reply, rx) = oneshot::channel();
        self.command_tx
            .send(SignalCommand::LoadRegistered { reply })
            .await?;
        let result = tokio::time::timeout(Duration::from_millis(self.command_timeout_ms), rx).await;
        match result {
            Ok(r) => r?,
            Err(_) => Err(anyhow::anyhow!("LoadRegistered timeout")),
        }
    }

    pub async fn link_secondary_device(&self, device_name: String) -> Result<String> {
        let (reply, rx) = oneshot::channel();
        self.command_tx
            .send(SignalCommand::LinkDevice { device_name, reply })
            .await?;
        tokio::time::timeout(Duration::from_secs(120), rx).await??
    }

    pub async fn send_message(&self, recipient: &str, text: &str) -> Result<String> {
        let resolved_recipient = self.recipient_cache.resolve(recipient)?;
        Self::validate_message(text)?;
        let _permit = self.send_rate_limiter.acquire().await?;
        let (reply, rx) = oneshot::channel();
        self.command_tx
            .send(SignalCommand::SendMessage {
                recipient: resolved_recipient,
                text: text.to_string(),
                reply,
            })
            .await?;
        tokio::time::timeout(Duration::from_millis(self.command_timeout_ms), rx).await??
    }

    pub async fn get_profile(&self) -> Result<Option<String>> {
        let (reply, rx) = oneshot::channel();
        self.command_tx
            .send(SignalCommand::GetProfile { reply })
            .await?;
        tokio::time::timeout(Duration::from_millis(self.command_timeout_ms), rx).await??
    }

    pub async fn send_typing(&self, recipient: &str, stop: bool) -> Result<()> {
        let resolved = self.recipient_cache.resolve(recipient)?;
        let (reply, rx) = oneshot::channel();
        self.command_tx
            .send(SignalCommand::SendTyping {
                recipient: resolved,
                stop,
                reply,
            })
            .await?;
        tokio::time::timeout(Duration::from_millis(self.command_timeout_ms), rx).await??
    }

    pub async fn send_reaction(
        &self,
        recipient: &str,
        target_timestamp: u64,
        emoji: &str,
        remove: bool,
    ) -> Result<()> {
        let resolved = self.recipient_cache.resolve(recipient)?;
        let (reply, rx) = oneshot::channel();
        self.command_tx
            .send(SignalCommand::SendReaction {
                recipient: resolved,
                target_timestamp,
                emoji: emoji.to_string(),
                remove,
                reply,
            })
            .await?;
        tokio::time::timeout(Duration::from_millis(self.command_timeout_ms), rx).await??
    }

    pub async fn start_receiver(&self) -> Result<()> {
        tracing::info!("[HANDLE] Sending StartReceiver command");
        let (reply, rx) = oneshot::channel();
        self.command_tx
            .send(SignalCommand::StartReceiver { reply })
            .await?;
        tokio::time::timeout(Duration::from_millis(self.command_timeout_ms), rx).await??
    }

    #[allow(dead_code)]
    pub async fn cache_recipient(&self, key: String, uuid: String) -> Result<()> {
        let (reply, rx) = oneshot::channel();
        self.command_tx
            .send(SignalCommand::CacheRecipient {
                phone: key,
                uuid,
                reply,
            })
            .await?;
        tokio::time::timeout(Duration::from_millis(self.command_timeout_ms), rx).await??
    }

    pub async fn stop_receiver(&self) -> Result<()> {
        let (reply, rx) = oneshot::channel();
        self.command_tx
            .send(SignalCommand::StopReceiver { reply })
            .await?;
        tokio::time::timeout(Duration::from_millis(self.command_timeout_ms), rx).await??
    }
}

pub struct SignalWorker {
    #[allow(dead_code)]
    thread_handle: Option<JoinHandle<()>>,
    handle: SignalHandle,
}

impl SignalWorker {
    pub fn spawn(config: ManagerConfig) -> Result<Self> {
        let (command_tx, command_rx) = mpsc::channel(config.command_channel_capacity);
        let (message_tx, _) = broadcast::channel(config.message_broadcast_capacity);
        let is_linked = Arc::new(AtomicBool::new(false));
        let receiver_running = Arc::new(AtomicBool::new(false));
        let shutdown_requested = Arc::new(AtomicBool::new(false));
        let account_number = Arc::new(Mutex::new(None));
        let send_rate_limiter = Arc::new(Semaphore::new(config.max_sends_per_second));
        let command_timeout_ms = config.command_timeout_ms;
        let recipient_cache = RecipientCache::new();

        let handle = SignalHandle {
            command_tx: command_tx.clone(),
            message_tx: message_tx.clone(),
            account_number: account_number.clone(),
            is_linked: is_linked.clone(),
            receiver_running: receiver_running.clone(),
            shutdown_requested: shutdown_requested.clone(),
            send_rate_limiter,
            command_timeout_ms,
            recipient_cache: recipient_cache.clone(),
        };

        let thread_handle = thread::Builder::new()
            .name("signal-worker".into())
            .spawn(move || {
                if let Err(e) = Self::run_worker_thread(
                    command_rx,
                    message_tx,
                    config,
                    is_linked,
                    receiver_running,
                    shutdown_requested,
                    account_number,
                    recipient_cache,
                ) {
                    tracing::error!("[WORKER] Thread error: {}", e);
                }
            })
            .context("Failed to spawn worker")?;

        Ok(Self {
            thread_handle: Some(thread_handle),
            handle,
        })
    }

    pub fn handle(&self) -> SignalHandle {
        self.handle.clone()
    }

    #[allow(clippy::too_many_arguments)]
    fn run_worker_thread(
        mut command_rx: mpsc::Receiver<SignalCommand>,
        message_tx: broadcast::Sender<SignalMessage>,
        config: ManagerConfig,
        is_linked: Arc<AtomicBool>,
        receiver_running: Arc<AtomicBool>,
        shutdown_requested: Arc<AtomicBool>,
        account_number: Arc<Mutex<Option<String>>>,
        recipient_cache: RecipientCache,
    ) -> Result<()> {
        let rt = Builder::new_current_thread()
            .enable_all()
            .build()
            .context("Runtime failed")?;
        let local = LocalSet::new();

        std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&config.db_path)
            .context("DB file")?;
        let store = rt.block_on(async {
            SqliteStore::open(&config.db_path, OnNewIdentity::Trust)
                .await
                .context("SQLite")
        })?;

        let mut state = WorkerState::Uninitialized;
        let display_name = config.display_name.clone();

        rt.block_on(local.run_until(async {
            while let Some(cmd) = command_rx.recv().await {
                if shutdown_requested.load(Ordering::Acquire) {
                    break;
                }
                Self::handle_command(
                    cmd,
                    &mut state,
                    store.clone(),
                    &message_tx,
                    &is_linked,
                    &receiver_running,
                    &shutdown_requested,
                    &account_number,
                    &recipient_cache,
                    &display_name,
                )
                .await;
            }
        }));
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_command(
        cmd: SignalCommand,
        state: &mut WorkerState,
        store: SqliteStore,
        message_tx: &broadcast::Sender<SignalMessage>,
        is_linked: &Arc<AtomicBool>,
        receiver_running: &Arc<AtomicBool>,
        shutdown_requested: &Arc<AtomicBool>,
        account_number: &Arc<Mutex<Option<String>>>,
        cache: &RecipientCache,
        display_name: &Option<String>,
    ) {
        match cmd {
            SignalCommand::LoadRegistered { reply } => {
                let result = Self::handle_load_registered(
                    state,
                    store,
                    is_linked,
                    account_number,
                    cache,
                    display_name,
                )
                .await;
                let _ = reply.send(result);
            }
            SignalCommand::LinkDevice { device_name, reply } => {
                let _ = reply.send(
                    Self::handle_link_device(
                        state,
                        store,
                        device_name,
                        is_linked,
                        account_number,
                        cache,
                        display_name,
                    )
                    .await,
                );
            }
            SignalCommand::SendMessage {
                recipient,
                text,
                reply,
            } => {
                let _ = reply.send(Self::handle_send_message(state, recipient, text, cache).await);
            }
            SignalCommand::GetProfile { reply } => {
                let _ =
                    reply.send(Self::handle_get_profile(state, account_number, display_name).await);
            }
            SignalCommand::SendTyping {
                recipient,
                stop,
                reply,
            } => {
                let _ = reply.send(Self::handle_send_typing(state, recipient, stop, cache).await);
            }
            SignalCommand::SendReaction {
                recipient,
                target_timestamp,
                emoji,
                remove,
                reply,
            } => {
                let _ = reply.send(
                    Self::handle_send_reaction(
                        state,
                        recipient,
                        target_timestamp,
                        emoji,
                        remove,
                        cache,
                    )
                    .await,
                );
            }
            SignalCommand::StartReceiver { reply } => {
                let _ = reply.send(
                    Self::handle_start_receiver(
                        state,
                        message_tx.clone(),
                        receiver_running.clone(),
                        shutdown_requested.clone(),
                        account_number.clone(),
                        cache.clone(),
                    )
                    .await,
                );
            }
            SignalCommand::StopReceiver { reply } => {
                Self::handle_stop_receiver(state, receiver_running);
                let _ = reply.send(Ok(()));
            }
            SignalCommand::Shutdown { reply } => {
                if let WorkerState::Linked {
                    receiver_handle, ..
                } = state
                    && let Some(h) = receiver_handle.take()
                {
                    h.abort();
                }
                *state = WorkerState::ShuttingDown;
                let _ = reply.send(());
            }
            SignalCommand::CacheRecipient { phone, uuid, reply } => {
                cache.insert(phone, uuid);
                let _ = reply.send(Ok(()));
            }
        }
    }

    async fn handle_load_registered(
        state: &mut WorkerState,
        store: SqliteStore,
        is_linked: &Arc<AtomicBool>,
        account_number: &Arc<Mutex<Option<String>>>,
        cache: &RecipientCache,
        display_name: &Option<String>,
    ) -> Result<bool> {
        match Manager::load_registered(store.clone()).await {
            Ok(manager) => {
                // Get self ACI for self-messaging
                let aci = manager.registration_data().service_ids.aci;
                let aci_str = aci.to_string();
                cache.set_self_aci(aci_str.clone());
                tracing::info!("[WORKER] Self ACI: {}", aci_str);

                // Get phone number — NEVER emitted raw: API/log/payload
                // surfaces carry only the masked / configured presentation.
                let phone = manager.registration_data().phone_number.to_string();
                let presentation = present_self_number(&phone, display_name);
                *account_number.lock() = Some(presentation);

                // Cache self phone -> ACI mapping (in-memory key only).
                cache.insert(phone, aci_str);

                is_linked.store(true, Ordering::Release);
                *state = WorkerState::Linked {
                    manager,
                    receiver_handle: None,
                };
                Ok(true)
            }
            Err(e) => {
                tracing::info!("[WORKER] Not registered: {}", e);
                Ok(false)
            }
        }
    }

    async fn handle_link_device(
        state: &mut WorkerState,
        store: SqliteStore,
        device_name: String,
        is_linked: &Arc<AtomicBool>,
        account_number: &Arc<Mutex<Option<String>>>,
        cache: &RecipientCache,
        display_name: &Option<String>,
    ) -> Result<String> {
        let (tx, rx_url) = futures::channel::oneshot::channel();
        let manager = Manager::link_secondary_device(
            store,
            presage::libsignal_service::configuration::SignalServers::Production,
            device_name,
            tx,
        )
        .await
        .map_err(|e| anyhow::anyhow!("{:?}", e))?;

        let url = rx_url.await.context("Link URL")?;

        // Get self ACI
        let aci = manager.registration_data().service_ids.aci;
        let aci_str = aci.to_string();
        cache.set_self_aci(aci_str.clone());

        let phone = manager.registration_data().phone_number.to_string();
        let presentation = present_self_number(&phone, display_name);
        *account_number.lock() = Some(presentation);
        cache.insert(phone, aci_str);

        is_linked.store(true, Ordering::Release);
        *state = WorkerState::Linked {
            manager,
            receiver_handle: None,
        };
        Ok(url.to_string())
    }

    async fn handle_send_message(
        state: &mut WorkerState,
        recipient: String,
        text: String,
        cache: &RecipientCache,
    ) -> Result<String> {
        let mut manager = match state {
            WorkerState::Linked { manager, .. } => manager.clone(),
            _ => anyhow::bail!("Not linked"),
        };

        let resolved = cache.resolve_via_manager(&mut manager, &recipient).await?;
        let sid = ServiceId::parse_from_service_id_string(&resolved)
            .ok_or_else(|| anyhow::anyhow!("Invalid ServiceId: {}", resolved))?;

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as u64;

        manager
            .send_message(
                sid,
                ContentBody::DataMessage(presage::proto::DataMessage {
                    body: Some(text),
                    ..Default::default()
                }),
                ts,
            )
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        Ok(uuid::Uuid::new_v4().to_string())
    }

    async fn handle_get_profile(
        state: &WorkerState,
        account_number: &Arc<Mutex<Option<String>>>,
        display_name: &Option<String>,
    ) -> Result<Option<String>> {
        if let Some(n) = account_number.lock().as_ref() {
            return Ok(Some(n.clone()));
        }
        let manager = match state {
            WorkerState::Linked { manager, .. } => manager,
            _ => return Ok(None),
        };

        match tokio::time::timeout(std::time::Duration::from_secs(10), manager.whoami()).await {
            // Masking law: even this protocol-level fallback emits only the
            // masked/configured form — the raw e164 NEVER enters the arc.
            Ok(Ok(w)) => {
                let raw = w.number.to_string();
                let presentation = present_self_number(&raw, display_name);
                *account_number.lock() = Some(presentation.clone());
                Ok(Some(presentation))
            }
            _ => Ok(None),
        }
    }

    async fn handle_send_typing(
        state: &mut WorkerState,
        recipient: String,
        stop: bool,
        cache: &RecipientCache,
    ) -> Result<()> {
        let mut manager = match state {
            WorkerState::Linked { manager, .. } => manager.clone(),
            _ => anyhow::bail!("Not linked"),
        };

        let resolved = cache.resolve_via_manager(&mut manager, &recipient).await?;
        let sid = ServiceId::parse_from_service_id_string(&resolved)
            .ok_or_else(|| anyhow::anyhow!("Invalid ServiceId"))?;

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as u64;

        manager
            .send_message(
                sid,
                ContentBody::TypingMessage(presage::proto::TypingMessage {
                    action: Some(if stop { 1 } else { 0 }),
                    timestamp: Some(ts),
                    ..Default::default()
                }),
                ts,
            )
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        Ok(())
    }

    async fn handle_send_reaction(
        state: &mut WorkerState,
        recipient: String,
        target_timestamp: u64,
        emoji: String,
        remove: bool,
        cache: &RecipientCache,
    ) -> Result<()> {
        let mut manager = match state {
            WorkerState::Linked { manager, .. } => manager.clone(),
            _ => anyhow::bail!("Not linked"),
        };

        let resolved = cache.resolve_via_manager(&mut manager, &recipient).await?;
        let sid = ServiceId::parse_from_service_id_string(&resolved)
            .ok_or_else(|| anyhow::anyhow!("Invalid ServiceId"))?;

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as u64;

        manager
            .send_message(
                sid,
                ContentBody::DataMessage(presage::proto::DataMessage {
                    reaction: Some(presage::proto::data_message::Reaction {
                        emoji: Some(emoji),
                        target_sent_timestamp: Some(target_timestamp),
                        remove: Some(remove),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ts,
            )
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        Ok(())
    }

    async fn handle_start_receiver(
        state: &mut WorkerState,
        message_tx: broadcast::Sender<SignalMessage>,
        receiver_running: Arc<AtomicBool>,
        shutdown_requested: Arc<AtomicBool>,
        account_number: Arc<Mutex<Option<String>>>,
        cache: RecipientCache,
    ) -> Result<()> {
        match receiver_running.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed) {
            Ok(_) => {}
            Err(true) => return Ok(()),
            Err(false) => anyhow::bail!("Inconsistent state"),
        }

        let mut manager = match state {
            WorkerState::Linked { manager, .. } => manager.clone(),
            _ => {
                receiver_running.store(false, Ordering::Release);
                anyhow::bail!("Not linked");
            }
        };

        let mut stream = manager
            .receive_messages()
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        let handle = tokio::task::spawn_local(async move {
            Self::receiver_loop(
                &mut stream,
                message_tx,
                receiver_running,
                shutdown_requested,
                account_number,
                cache,
            )
            .await;
        });

        if let WorkerState::Linked {
            receiver_handle, ..
        } = state
        {
            *receiver_handle = Some(handle);
        }
        Ok(())
    }

    fn handle_stop_receiver(state: &mut WorkerState, receiver_running: &Arc<AtomicBool>) {
        receiver_running.store(false, Ordering::Release);
        if let WorkerState::Linked {
            receiver_handle, ..
        } = state
            && let Some(h) = receiver_handle.take()
        {
            h.abort();
        }
    }

    async fn receiver_loop(
        stream: &mut (impl StreamExt<Item = Received> + Unpin),
        message_tx: broadcast::Sender<SignalMessage>,
        receiver_running: Arc<AtomicBool>,
        shutdown_requested: Arc<AtomicBool>,
        account_number: Arc<Mutex<Option<String>>>,
        cache: RecipientCache,
    ) {
        tracing::info!("[RECEIVER] Loop started");
        let mut message_count = 0u64;

        loop {
            if shutdown_requested.load(Ordering::Acquire) {
                break;
            }

            tokio::select! {
                msg = stream.next() => {
                    match msg {
                        Some(Received::Content(c)) => {
                            message_count = message_count.saturating_add(1);
                            tracing::debug!(
                                "[RECEIVER] Message #{} from {}",
                                message_count,
                                c.metadata.sender.raw_uuid()
                            );

                            let body_type = match &c.body { ContentBody::DataMessage(_) => "DataMessage", ContentBody::SynchronizeMessage(_) => "SynchronizeMessage", _ => "Other" }; tracing::info!("[RECEIVER] Content type: {}", body_type); if let Some(m) = Self::process_content(&c, account_number.lock().clone(), &cache) {
                                let _ = message_tx.send(m);
                            }
                        }
                        Some(Received::QueueEmpty) => { tracing::debug!("[RECEIVER] Queue empty"); }
                        Some(Received::Contacts) => { tracing::debug!("[RECEIVER] Contacts update"); }
                        Some(Received::DecryptionError(sender)) => { tracing::warn!("[RECEIVER] Decryption error from {sender:?}; a session reset may fix this"); }
                        None => { tracing::warn!("[RECEIVER] Stream closed"); break; }
                    }
                }
                _ = tokio::time::sleep(Duration::from_secs(30)) => { tracing::debug!("[RECEIVER] Keepalive"); }
            }
        }

        receiver_running.store(false, Ordering::Release);
        tracing::info!("[RECEIVER] Loop exited ({} messages)", message_count);
    }

    fn process_content(
        content: &presage::libsignal_service::content::Content,
        account: Option<String>,
        _cache: &RecipientCache,
    ) -> Option<SignalMessage> {
        use presage::libsignal_service::content::ContentBody;

        let uuid = content.metadata.sender.raw_uuid().to_string();
        // Upstream presage moved Metadata.timestamp from a raw u64 to
        // chrono::DateTime<Utc>; Signal wire timestamps are epoch MILLIS, so
        // timestamp_millis() preserves the exact value the old cast carried.
        let ts = content.metadata.timestamp.timestamp_millis();

        let (text, att, quote, group) = match &content.body {
            ContentBody::DataMessage(dm) => {
                let a = if dm.attachments.is_empty() {
                    None
                } else {
                    Some(
                        dm.attachments
                            .iter()
                            .map(|x| SignalAttachment {
                                content_type: x.content_type.clone(),
                                filename: None,
                                size: x.size.map(|s| s as u64),
                                path: None,
                                thumbnail: None,
                            })
                            .collect(),
                    )
                };
                let q = dm.quote.as_ref().map(|x| SignalQuote {
                    id: x.id.map(|i| i as i64),
                    author: x.author_aci.clone(),
                    text: x.text.clone(),
                });
                let g = dm.group_v2.as_ref().map(|x| SignalGroupInfo {
                    group_id: x
                        .master_key
                        .clone()
                        .map(|k| base64::engine::general_purpose::STANDARD.encode(&k)),
                    name: None,
                    revision: x.revision.map(|r| r as i32),
                });
                (dm.body.clone(), a, q, g)
            }
            ContentBody::SynchronizeMessage(sm) => {
                tracing::info!("[SYNC] SynchronizeMessage received");
                if let Some(sent) = sm.sent.as_ref() {
                    tracing::debug!("[SYNC] sent transfer");
                    if let Some(msg) = sent.message.as_ref() {
                        tracing::debug!("[SYNC] message body present");
                        (msg.body.clone(), None, None, None)
                    } else {
                        tracing::warn!("[SYNC] no message in sent");
                        (None, None, None, None)
                    }
                } else {
                    tracing::warn!("[SYNC] no sent in SynchronizeMessage");
                    (None, None, None, None)
                }
            }
            _ => return None,
        };

        if text.is_none() || text.as_ref().is_none_or(|t| t.is_empty()) {
            return None;
        }

        Some(SignalMessage {
            account,
            envelope: SignalEnvelope {
                source: None,
                source_uuid: Some(uuid),
                source_device: Some(0),
                timestamp: Some(ts),
                data_message: Some(SignalDataMessage {
                    message: text,
                    timestamp: Some(ts),
                    attachments: att,
                    group_info: group,
                    quote,
                    mentions: None,
                }),
                sync_message: None,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    /// The anonymity law for OUR OWN number: configured label wins; the
    /// fallback keeps only a two-digit head/tail skeleton.
    #[test]
    fn self_number_presentation_prefers_label_then_masks() {
        // Label wins verbatim — the raw number never appears.
        let labeled = present_self_number("+639171234567", &Some("ghost.42".to_string()));
        assert_eq!(labeled, "ghost.42");

        // Masked fallback: +<head 2>…<tail 2>, nothing else leaks.
        let masked = present_self_number("+639171234567", &None);
        assert_eq!(masked, "+63…67");
        assert!(
            !masked.contains("9171"),
            "middle digits must not leak: {masked}"
        );

        // Degenerate inputs stay safe (no panic, no digits invented).
        let tiny = present_self_number("1", &None);
        assert_eq!(tiny, "+\u{2022}\u{2026}\u{2022}");
        let empty = present_self_number("", &None);
        assert_eq!(empty, "+\u{2022}\u{2026}\u{2022}");
    }

    /// Username-shaped recipients are the lookup path's exact trigger.
    #[test]
    fn username_classifier_matches_gateway_shapes() {
        assert!(RecipientCache::is_username("ghost.42"));
        assert!(!RecipientCache::is_username("+639171234567"));
        assert!(!RecipientCache::is_username(
            "01234567-89ab-cdef-0123-456789abcdef"
        ));
    }
}
