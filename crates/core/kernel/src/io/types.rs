// Copyright 2025 Crrow
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Core message types for the I/O Bus.
//!
//! These types define the unified message model flowing through the inbound and
//! outbound buses. Platform adapters convert raw events into
//! [`InboundMessage`], and the kernel publishes [`OutboundEnvelope`] for egress
//! delivery.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use snafu::Snafu;

use crate::{
    channel::types::{ChannelType, MessageContent},
    process::{SessionId, principal::UserId},
};

// ---------------------------------------------------------------------------
// MessageId
// ---------------------------------------------------------------------------

/// ULID-based message identifier.
///
/// Every inbound and outbound message gets a unique `MessageId` for
/// correlation, deduplication, and reply threading.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId(pub String);

impl MessageId {
    /// Generate a new ULID-based message ID.
    pub fn new() -> Self { Self(ulid::Ulid::new().to_string()) }
}

impl Default for MessageId {
    fn default() -> Self { Self::new() }
}

impl std::fmt::Display for MessageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(&self.0) }
}

// ---------------------------------------------------------------------------
// ChannelSource
// ---------------------------------------------------------------------------

/// First-class platform source fields for an inbound message.
///
/// These fields are extracted from the raw platform event rather than being
/// stuffed into a generic metadata map, enabling type-safe routing and
/// deduplication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelSource {
    /// Which channel this message arrived from.
    pub channel_type:        ChannelType,
    /// Platform-specific message ID (used for dedup and reply mapping).
    pub platform_message_id: Option<String>,
    /// Platform-specific user identifier.
    pub platform_user_id:    String,
    /// Platform-specific chat/thread identifier.
    pub platform_chat_id:    Option<String>,
}

// ---------------------------------------------------------------------------
// ReplyContext / InteractionType
// ---------------------------------------------------------------------------

/// Contextual information for egress to reply correctly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplyContext {
    /// Thread identifier for threaded conversations.
    pub thread_id:                Option<String>,
    /// Platform message ID to reply to.
    pub reply_to_platform_msg_id: Option<String>,
    /// The type of user interaction that generated this message.
    pub interaction_type:         InteractionType,
}

/// The kind of interaction that generated an inbound message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InteractionType {
    /// A regular text message.
    Message,
    /// A slash-command (e.g. `/search`).
    Command(String),
    /// A callback from an interactive element (e.g. inline button).
    Callback(String),
}

// ---------------------------------------------------------------------------
// InboundMessage
// ---------------------------------------------------------------------------

/// A unified inbound message from any channel adapter.
///
/// After ingress resolves identity and session, the raw platform event is
/// converted into this type and published to the
/// [`EventQueue`](crate::event_queue::EventQueue).
#[derive(Debug, Clone)]
pub struct InboundMessage {
    /// Unique message identifier (ULID).
    pub id:            MessageId,
    /// Platform source details.
    pub source:        ChannelSource,
    /// Unified user identity (resolved by ingress).
    pub user:          UserId,
    /// Session this message belongs to.
    pub session_id:    SessionId,
    /// Target agent name. `None` means route to the default root agent ("rara").
    pub target_agent:  Option<String>,
    /// Message content (text or multimodal).
    pub content:       MessageContent,
    /// Optional reply/thread context for egress.
    pub reply_context: Option<ReplyContext>,
    /// When this message was created.
    pub timestamp:     jiff::Timestamp,
    /// Extension metadata (adapter-specific fields only).
    pub metadata:      HashMap<String, Value>,
}

impl InboundMessage {
    /// Create a synthetic internal message (for workers, SpawnTool, etc.).
    pub fn synthetic(text: String, user: UserId, session_id: SessionId) -> Self {
        Self {
            id:            MessageId::new(),
            source:        ChannelSource {
                channel_type:        ChannelType::Internal,
                platform_message_id: None,
                platform_user_id:    user.0.clone(),
                platform_chat_id:    None,
            },
            user,
            session_id,
            target_agent:  None,
            content:       MessageContent::Text(text),
            reply_context: None,
            timestamp:     jiff::Timestamp::now(),
            metadata:      HashMap::new(),
        }
    }

    /// Create a synthetic internal message addressed to a specific agent.
    pub fn synthetic_to(
        text: String,
        user: UserId,
        session_id: SessionId,
        target_agent: String,
    ) -> Self {
        Self {
            id:            MessageId::new(),
            source:        ChannelSource {
                channel_type:        ChannelType::Internal,
                platform_message_id: None,
                platform_user_id:    user.0.clone(),
                platform_chat_id:    None,
            },
            user,
            session_id,
            target_agent:  Some(target_agent),
            content:       MessageContent::Text(text),
            reply_context: None,
            timestamp:     jiff::Timestamp::now(),
            metadata:      HashMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Attachment
// ---------------------------------------------------------------------------

/// A binary attachment for outbound messages.
#[derive(Debug, Clone)]
pub struct Attachment {
    /// Raw binary data.
    pub data:      Vec<u8>,
    /// MIME type (e.g. "image/png", "application/pdf").
    pub mime_type: String,
    /// Optional filename hint.
    pub filename:  Option<String>,
}

// ---------------------------------------------------------------------------
// OutboundEnvelope
// ---------------------------------------------------------------------------

/// A message published by the kernel for egress delivery.
///
/// Contains routing information so egress can determine which channels
/// should receive this message.
#[derive(Debug, Clone)]
pub struct OutboundEnvelope {
    /// Unique envelope identifier (ULID).
    pub id:          MessageId,
    /// The inbound message this is replying to.
    pub in_reply_to: MessageId,
    /// Target user.
    pub user:        UserId,
    /// Session context.
    pub session_id:  SessionId,
    /// How to route this envelope.
    pub routing:     OutboundRouting,
    /// The payload to deliver.
    pub payload:     OutboundPayload,
    /// When this envelope was created.
    pub timestamp:   jiff::Timestamp,
}

// ---------------------------------------------------------------------------
// OutboundRouting
// ---------------------------------------------------------------------------

/// Routing strategy for an outbound envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutboundRouting {
    /// Broadcast to all connected endpoints for this user.
    BroadcastAll,
    /// Broadcast but exclude source channel (prevent echo).
    BroadcastExcept { exclude: ChannelType },
    /// Send to specific channels only.
    Targeted { channels: Vec<ChannelType> },
}

// ---------------------------------------------------------------------------
// OutboundPayload
// ---------------------------------------------------------------------------

/// The payload carried by an outbound envelope.
#[derive(Debug, Clone)]
pub enum OutboundPayload {
    /// A complete reply to deliver.
    Reply {
        content:     MessageContent,
        attachments: Vec<Attachment>,
    },
    /// Progress update (ephemeral).
    Progress {
        stage:  String,
        detail: Option<String>,
    },
    /// State change notification.
    StateChange {
        event_type: String,
        data:       Value,
    },
    /// Error response.
    Error { code: String, message: String },
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors from bus operations.
#[derive(Debug, Snafu)]
#[snafu(module)]
pub enum BusError {
    /// Bus is at capacity; message rejected.
    #[snafu(display("bus is full"))]
    Full,
    /// Internal bus error.
    #[snafu(display("bus internal error: {message}"))]
    Internal { message: String },
}

/// Errors from the ingress pipeline.
#[derive(Debug, Snafu)]
#[snafu(module)]
pub enum IngestError {
    /// System is overloaded; try again later.
    #[snafu(display("system busy"))]
    SystemBusy,
    /// Failed to resolve platform identity to a unified user ID.
    #[snafu(display("identity resolution failed: {message}"))]
    IdentityResolutionFailed { message: String },
    /// Internal ingress error.
    #[snafu(display("ingress internal error: {message}"))]
    Internal { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_id_uniqueness() {
        let a = MessageId::new();
        let b = MessageId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn message_id_display() {
        let id = MessageId::new();
        let s = id.to_string();
        // ULID is 26 characters
        assert_eq!(s.len(), 26);
    }

    #[test]
    fn bus_error_display() {
        let err = BusError::Full;
        assert_eq!(err.to_string(), "bus is full");

        let err = BusError::Internal {
            message: "oops".to_string(),
        };
        assert_eq!(err.to_string(), "bus internal error: oops");
    }

    #[test]
    fn ingest_error_display() {
        let err = IngestError::SystemBusy;
        assert_eq!(err.to_string(), "system busy");

        let err = IngestError::IdentityResolutionFailed {
            message: "no mapping".to_string(),
        };
        assert_eq!(err.to_string(), "identity resolution failed: no mapping");
    }

    #[test]
    fn channel_source_construction() {
        let source = ChannelSource {
            channel_type:        ChannelType::Telegram,
            platform_message_id: Some("42".to_string()),
            platform_user_id:    "tg-user-1".to_string(),
            platform_chat_id:    Some("tg-chat-1".to_string()),
        };
        assert_eq!(source.channel_type, ChannelType::Telegram);
        assert_eq!(source.platform_user_id, "tg-user-1");
    }

    #[test]
    fn outbound_routing_variants() {
        let r = OutboundRouting::BroadcastAll;
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("broadcast_all"));

        let r = OutboundRouting::BroadcastExcept {
            exclude: ChannelType::Telegram,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("broadcast_except"));
        assert!(json.contains("telegram"));
    }
}
