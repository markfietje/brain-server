//! Signal commands for the command pattern

use anyhow::Result;
use tokio::sync::oneshot;

pub enum SignalCommand {
    LoadRegistered {
        reply: oneshot::Sender<Result<bool>>,
    },
    LinkDevice {
        device_name: String,
        reply: oneshot::Sender<Result<String>>,
    },
    SendMessage {
        recipient: String,
        text: String,
        reply: oneshot::Sender<Result<String>>,
    },
    GetProfile {
        reply: oneshot::Sender<Result<Option<String>>>,
    },
    SendTyping {
        recipient: String,
        stop: bool,
        reply: oneshot::Sender<Result<()>>,
    },
    SendReaction {
        recipient: String,
        target_timestamp: u64,
        emoji: String,
        remove: bool,
        reply: oneshot::Sender<Result<()>>,
    },
    StartReceiver {
        reply: oneshot::Sender<Result<()>>,
    },
    StopReceiver {
        reply: oneshot::Sender<Result<()>>,
    },
    #[allow(dead_code)]
    Shutdown {
        reply: oneshot::Sender<()>,
    },
    /// Cache phone -> UUID mapping (from incoming messages)
    #[allow(dead_code)]
    CacheRecipient {
        phone: String,
        uuid: String,
        reply: oneshot::Sender<Result<()>>,
    },
}
