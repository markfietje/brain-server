//! Signal message types (the gateway's wire vocabulary)

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Incoming Signal message
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignalMessage {
    pub account: Option<String>,
    pub envelope: SignalEnvelope,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignalEnvelope {
    #[serde(rename = "sourceNumber", skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(rename = "sourceUuid", skip_serializing_if = "Option::is_none")]
    pub source_uuid: Option<String>,
    #[serde(rename = "sourceDevice", skip_serializing_if = "Option::is_none")]
    pub source_device: Option<i32>,
    pub timestamp: Option<i64>,
    #[serde(rename = "dataMessage", skip_serializing_if = "Option::is_none")]
    pub data_message: Option<SignalDataMessage>,
    #[serde(rename = "syncMessage", skip_serializing_if = "Option::is_none")]
    pub sync_message: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignalDataMessage {
    pub message: Option<String>,
    pub timestamp: Option<i64>,
    pub attachments: Option<Vec<SignalAttachment>>,
    #[serde(rename = "groupInfo", skip_serializing_if = "Option::is_none")]
    pub group_info: Option<SignalGroupInfo>,
    pub quote: Option<SignalQuote>,
    pub mentions: Option<Vec<SignalMention>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignalAttachment {
    #[serde(rename = "contentType", skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    pub filename: Option<String>,
    pub size: Option<u64>,
    pub path: Option<PathBuf>,
    pub thumbnail: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignalGroupInfo {
    #[serde(rename = "groupId", skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    #[serde(rename = "groupName", skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub revision: Option<i32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignalQuote {
    pub id: Option<i64>,
    pub author: Option<String>,
    pub text: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignalMention {
    pub uuid: Option<String>,
    pub start: Option<i32>,
    pub length: Option<i32>,
}

/// Configuration for the Signal manager
#[derive(Clone, Debug)]
pub struct ManagerConfig {
    pub db_path: String,
    pub command_channel_capacity: usize,
    pub message_broadcast_capacity: usize,
    pub command_timeout_ms: u64,
    pub max_sends_per_second: usize,
    /// Public presentation of OUR account in API/log/payload surfaces — set
    /// it to your Signal username. None → masked digits (+63…78 style).
    pub display_name: Option<String>,
}

impl Default for ManagerConfig {
    fn default() -> Self {
        Self {
            db_path: "signal.db".to_string(),
            command_channel_capacity: 64,
            message_broadcast_capacity: 256,
            command_timeout_ms: 30_000,
            max_sends_per_second: 5,
            display_name: None,
        }
    }
}

impl ManagerConfig {
    #[allow(dead_code)]
    pub fn new(db_path: String, _attachments_dir: String) -> Self {
        Self {
            db_path,
            ..Default::default()
        }
    }
}
