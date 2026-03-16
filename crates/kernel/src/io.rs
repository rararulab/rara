// Copyright 2025 Rararulab
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

//! I/O — transport primitives for inbound and outbound communication.
//!
//! This module implements the kernel's I/O transport layer:
//!
//! - **Ingress**: channel adapters publish messages through `IngressPipeline`
//!   into the unified [`EventQueue`](crate::queue::EventQueue).
//! - **Egress**: the kernel event loop delivers outbound envelopes via
//!   [`IOSubsystem::deliver`] to registered adapters.
//! - **Streaming**: ephemeral real-time events (token deltas, tool progress)
//!   flow through the `StreamHub` for connected frontends.
//!
//! ## Architecture
//!
//! ```text
//! Adapters → IngressPipeline → EventQueue → Kernel Event Loop
//!                                                   ↓
//!                                         IOSubsystem::deliver + StreamHub
//!                                                   ↓
//!                                         Channel Adapters (Web, Telegram, ...)
//! ```

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use base::define_id;
use dashmap::DashMap;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use snafu::Snafu;
use tokio::sync::{broadcast, mpsc};
use tracing::Instrument;
use uuid::Uuid;

use crate::{
    channel::types::{ChannelType, MessageContent},
    identity::UserId,
    session::{AgentRunLoopResult, SessionIndex, SessionKey},
};

/// Well-known progress stage names used by `OutboundPayload::Progress` and
/// `StreamEvent::Progress`.
pub mod stages {
    pub const THINKING: &str = "thinking";
}

// ---------------------------------------------------------------------------
// MessageId
// ---------------------------------------------------------------------------

define_id!(
    /// ULID-based message identifier.
    ///
    /// Every inbound and outbound message gets a unique `MessageId` for
    /// correlation, deduplication, and reply threading.
    MessageId
);

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
/// Produced by [`IOSubsystem::resolve()`] and published to the
/// [`EventQueue`](crate::queue::EventQueue).
///
/// ## `session_key` lifecycle
///
/// `session_key` starts as `Option<SessionKey>`:
/// - **`Some(key)`** — a channel binding already maps this chat to a session.
/// - **`None`** — first message from a new chat; no binding exists yet.
///
/// Before routing, `Kernel::handle_user_message()` resolves `None` by
/// creating a new session + writing a channel binding, then patches
/// `session_key` to `Some`. All downstream code (routing, LLM turn,
/// stream forwarder) therefore always sees `Some`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundMessage {
    /// Unique message identifier (ULID).
    pub id:                 MessageId,
    /// Platform source details.
    pub source:             ChannelSource,
    /// Unified user identity (resolved by ingress).
    pub user:               UserId,
    /// Session this message belongs to.
    ///
    /// `None` on ingress when no channel binding exists (first message).
    /// Patched to `Some` by the kernel before routing — see struct-level docs.
    pub session_key:        Option<SessionKey>,
    /// Direct process targeting (agent-to-agent communication).
    /// When set, routing bypasses session/name resolution entirely.
    pub target_session_key: Option<SessionKey>,

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
    /// Create a synthetic internal message (for workers, SyscallTool, etc.).
    pub fn synthetic(text: String, user: UserId, session_id: SessionKey) -> Self {
        Self {
            id: MessageId::new(),
            source: ChannelSource {
                channel_type:        ChannelType::Internal,
                platform_message_id: None,
                platform_user_id:    user.0.clone(),
                platform_chat_id:    None,
            },
            user,
            session_key: Some(session_id),
            target_session_key: None,
            content: MessageContent::Text(text),
            reply_context: None,
            timestamp: jiff::Timestamp::now(),
            metadata: HashMap::new(),
        }
    }

    /// Create a synthetic internal message addressed to a specific agent by
    /// name.
    pub fn synthetic_to(
        text: String,
        user: UserId,
        session_id: SessionKey,
        _target_session_key: SessionKey,
    ) -> Self {
        Self {
            id: MessageId::new(),
            source: ChannelSource {
                channel_type:        ChannelType::Internal,
                platform_message_id: None,
                platform_user_id:    user.0.clone(),
                platform_chat_id:    None,
            },
            user,
            session_key: Some(session_id),
            target_session_key: None,
            content: MessageContent::Text(text),
            reply_context: None,
            timestamp: jiff::Timestamp::now(),
            metadata: HashMap::new(),
        }
    }

    /// Create a synthetic internal message addressed to a specific agent by ID
    /// (agent-to-agent communication).
    pub fn synthetic_to_id(
        text: String,
        user: UserId,
        session_key: SessionKey,
        target_id: SessionKey,
    ) -> Self {
        Self {
            id: MessageId::new(),
            source: ChannelSource {
                channel_type:        ChannelType::Internal,
                platform_message_id: None,
                platform_user_id:    user.0.clone(),
                platform_chat_id:    None,
            },
            user,
            session_key: Some(session_key),
            target_session_key: Some(target_id),
            content: MessageContent::Text(text),
            reply_context: None,
            timestamp: jiff::Timestamp::now(),
            metadata: HashMap::new(),
        }
    }

    /// Build the originating endpoint for session-scoped reply routing.
    ///
    /// Returns `Some(Endpoint)` for channel types that support multiple
    /// chat destinations per user (e.g. Telegram private vs group chats).
    /// Returns `None` for internal/synthetic messages.
    pub fn origin_endpoint(&self) -> Option<Endpoint> {
        match self.source.channel_type {
            ChannelType::Telegram => {
                let chat_id = self.source.platform_chat_id.as_ref()?.parse::<i64>().ok()?;
                Some(Endpoint {
                    channel_type: ChannelType::Telegram,
                    address:      EndpointAddress::Telegram {
                        chat_id,
                        thread_id: None,
                    },
                })
            }
            // Web endpoints are already per-connection; CLI/Internal don't need scoping.
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Attachment
// ---------------------------------------------------------------------------

/// A binary attachment for outbound messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundEnvelope {
    /// Unique envelope identifier (ULID).
    pub id:              MessageId,
    /// The inbound message this is replying to.
    pub in_reply_to:     MessageId,
    /// Target user.
    pub user:            UserId,
    /// Session context.
    pub session_key:     SessionKey,
    /// How to route this envelope.
    pub routing:         OutboundRouting,
    /// The payload to deliver.
    pub payload:         OutboundPayload,
    /// When this envelope was created.
    pub timestamp:       jiff::Timestamp,
    /// When set, deliver ONLY to this endpoint (session-scoped routing).
    /// Takes priority over `routing` for same-type endpoints.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub origin_endpoint: Option<Endpoint>,
}

impl OutboundEnvelope {
    /// Create an error envelope with `BroadcastAll` routing.
    /// TODO: what is used for ?
    pub fn error(
        in_reply_to: MessageId,
        user: UserId,
        session_id: SessionKey,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            id: MessageId::new(),
            in_reply_to,
            user,
            session_key: session_id,
            routing: OutboundRouting::BroadcastAll,
            payload: OutboundPayload::Error {
                code:    code.into(),
                message: message.into(),
            },
            timestamp: jiff::Timestamp::now(),
            origin_endpoint: None,
        }
    }

    /// Create a reply envelope with `BroadcastAll` routing.
    pub fn reply(
        in_reply_to: MessageId,
        user: UserId,
        session_id: SessionKey,
        content: crate::channel::types::MessageContent,
        attachments: Vec<Attachment>,
    ) -> Self {
        Self {
            id: MessageId::new(),
            in_reply_to,
            user,
            session_key: session_id,
            routing: OutboundRouting::BroadcastAll,
            payload: OutboundPayload::Reply {
                content,
                attachments,
            },
            timestamp: jiff::Timestamp::now(),
            origin_endpoint: None,
        }
    }

    /// Create a progress envelope with `BroadcastAll` routing.
    pub fn progress(
        in_reply_to: MessageId,
        user: UserId,
        session_key: SessionKey,
        stage: impl Into<String>,
        detail: Option<String>,
    ) -> Self {
        Self {
            id: MessageId::new(),
            in_reply_to,
            user,
            session_key,
            routing: OutboundRouting::BroadcastAll,
            payload: OutboundPayload::Progress {
                stage: stage.into(),
                detail,
            },
            timestamp: jiff::Timestamp::now(),
            origin_endpoint: None,
        }
    }

    /// Create a state-change envelope with `BroadcastAll` routing.
    pub fn state_change(
        in_reply_to: MessageId,
        user: UserId,
        session_id: SessionKey,
        event_type: impl Into<String>,
        data: serde_json::Value,
    ) -> Self {
        Self {
            id: MessageId::new(),
            in_reply_to,
            user,
            session_key: session_id,
            routing: OutboundRouting::BroadcastAll,
            payload: OutboundPayload::StateChange {
                event_type: event_type.into(),
                data,
            },
            timestamp: jiff::Timestamp::now(),
            origin_endpoint: None,
        }
    }

    /// Set the origin endpoint for session-scoped routing.
    #[must_use]
    pub fn with_origin(mut self, endpoint: Option<Endpoint>) -> Self {
        self.origin_endpoint = endpoint;
        self
    }

    /// Format this envelope as a [`PlatformOutbound`] for delivery.
    pub fn to_platform_outbound(&self) -> PlatformOutbound {
        match &self.payload {
            OutboundPayload::Reply {
                content,
                attachments,
            } => PlatformOutbound::Reply {
                content:       content.as_text(),
                attachments:   attachments.clone(),
                reply_context: None,
            },
            OutboundPayload::Progress { stage, detail } => PlatformOutbound::Progress {
                text: detail.as_deref().unwrap_or(stage).to_string(),
            },
            OutboundPayload::Error { code, message } => PlatformOutbound::Reply {
                content:       format!("Error [{}]: {}", code, message),
                attachments:   vec![],
                reply_context: None,
            },
            OutboundPayload::StateChange { .. } => PlatformOutbound::Progress {
                text: String::new(),
            },
        }
    }
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
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

#[derive(Debug, Snafu)]
#[snafu(module)]
pub enum IOError {
    /// Bus is at capacity; message rejected.
    #[snafu(display("bus is full"))]
    Full,
    /// Internal bus error.
    #[snafu(display("bus internal error: {message}"))]
    Internal { message: String },

    /// System is overloaded; try again later.
    #[snafu(display("system busy"))]
    SystemBusy,
    /// Failed to resolve platform identity to a unified user ID.
    #[snafu(display("identity resolution failed: {message}"))]
    IdentityResolutionFailed { message: String },
    /// Ingress rate limit exceeded for this user/channel.
    #[snafu(display("Rate limited: {message}"))]
    RateLimited { message: String },
}

// ---------------------------------------------------------------------------
// OutboxStore
// ---------------------------------------------------------------------------

/// Durable outbox for messages that could not be delivered immediately.
///
/// When egress detects a user is offline, the envelope is appended here.
/// A background drainer periodically re-publishes pending envelopes.
#[async_trait]
pub trait OutboxStore: Send + Sync + 'static {
    /// Append an envelope to the durable outbox.
    async fn append(&self, envelope: OutboundEnvelope) -> Result<(), IOError>;

    /// Drain up to `max` pending envelopes for re-delivery.
    async fn drain_pending(&self, max: usize) -> Vec<OutboundEnvelope>;

    /// Mark an envelope as successfully delivered (remove from outbox).
    async fn mark_delivered(&self, id: &MessageId) -> Result<(), IOError>;
}

// ---------------------------------------------------------------------------
// PipeId
// ---------------------------------------------------------------------------

define_id!(PipeId);

// ---------------------------------------------------------------------------
// PipeMessage
// ---------------------------------------------------------------------------

/// A single message transmitted through a pipe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipeMessage {
    /// A data chunk (text payload).
    Data(String),
    /// An error message — the writer encountered a problem.
    Error(String),
    /// End-of-file marker — no more data will be sent.
    Eof,
}

// ---------------------------------------------------------------------------
// PipeWriter / PipeReader
// ---------------------------------------------------------------------------

/// Write end of a pipe.
///
/// Dropping the writer will cause the reader's [`PipeReader::recv`] to
/// eventually return `None`, signalling end-of-stream.
pub struct PipeWriter {
    pipe_id: PipeId,
    tx:      mpsc::Sender<PipeMessage>,
}

impl PipeWriter {
    /// The pipe this writer belongs to.
    pub fn pipe_id(&self) -> &PipeId { &self.pipe_id }

    /// Send a data message through the pipe.
    ///
    /// Returns `Err` if the reader has been dropped.
    pub async fn send(&self, data: String) -> Result<(), PipeSendError> {
        self.tx
            .send(PipeMessage::Data(data))
            .await
            .map_err(|_| PipeSendError)
    }

    /// Send an error message through the pipe.
    pub async fn send_error(&self, msg: String) -> Result<(), PipeSendError> {
        self.tx
            .send(PipeMessage::Error(msg))
            .await
            .map_err(|_| PipeSendError)
    }

    /// Send an explicit EOF and close the writer.
    ///
    /// After calling this the writer should be dropped.
    pub async fn send_eof(self) -> Result<(), PipeSendError> {
        self.tx
            .send(PipeMessage::Eof)
            .await
            .map_err(|_| PipeSendError)
    }
}

/// Read end of a pipe.
///
/// When the writer is dropped and all buffered messages are consumed,
/// [`recv`](Self::recv) returns `None`.
pub struct PipeReader {
    pipe_id: PipeId,
    rx:      mpsc::Receiver<PipeMessage>,
}

impl PipeReader {
    /// The pipe this reader belongs to.
    pub fn pipe_id(&self) -> &PipeId { &self.pipe_id }

    /// Receive the next message from the pipe.
    ///
    /// Returns `None` when the writer has been dropped and the buffer is
    /// exhausted (i.e., end-of-stream).
    pub async fn recv(&mut self) -> Option<PipeMessage> { self.rx.recv().await }
}

// ---------------------------------------------------------------------------
// PipeSendError
// ---------------------------------------------------------------------------

/// Error returned when writing to a pipe whose reader has been dropped.
/// TODO: use a better way
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipeSendError;

impl std::fmt::Display for PipeSendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "pipe closed: reader dropped")
    }
}

impl std::error::Error for PipeSendError {}

// ---------------------------------------------------------------------------
// pipe() — constructor
// ---------------------------------------------------------------------------

/// Create an anonymous pipe pair with the given buffer capacity.
///
/// Returns `(PipeWriter, PipeReader)` sharing the same [`PipeId`].
pub fn pipe(buffer: usize) -> (PipeWriter, PipeReader) {
    let (tx, rx) = mpsc::channel(buffer);
    let id = PipeId::new();
    (
        PipeWriter {
            pipe_id: id.clone(),
            tx,
        },
        PipeReader { pipe_id: id, rx },
    )
}

// ---------------------------------------------------------------------------
// PipeEntry — registry metadata
// ---------------------------------------------------------------------------

/// Metadata about a pipe tracked in the [`PipeRegistry`].
#[derive(Debug, Clone)]
pub struct PipeEntry {
    /// The agent that created (owns) this pipe.
    pub owner:      SessionKey,
    /// The agent connected as reader (if any).
    pub reader:     Option<SessionKey>,
    /// When the pipe was created.
    pub created_at: Timestamp,
}

// ---------------------------------------------------------------------------
// PipeRegistry
// ---------------------------------------------------------------------------

/// Central registry tracking active pipes and their ownership.
///
/// Supports both anonymous pipes (tracked by [`PipeId`]) and named pipes
/// (tracked by an additional string key).
///
/// Named pipes support a "parking" mechanism: the creator parks the reader
/// end in the registry, and a connecting agent retrieves it via
/// [`take_parked_reader`](Self::take_parked_reader).
pub struct PipeRegistry {
    /// All active pipes keyed by PipeId.
    pipes:          DashMap<PipeId, PipeEntry>,
    /// Named pipe index: name -> PipeId.
    named:          DashMap<String, PipeId>,
    /// Parked readers for named pipes (take-once via `Mutex<Option>`).
    parked_readers: DashMap<PipeId, Mutex<Option<PipeReader>>>,
}

impl PipeRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            pipes:          DashMap::new(),
            named:          DashMap::new(),
            parked_readers: DashMap::new(),
        }
    }

    /// Register a pipe entry.
    pub fn register(&self, pipe_id: PipeId, entry: PipeEntry) { self.pipes.insert(pipe_id, entry); }

    /// Register a named pipe (also adds to the pipe table).
    pub fn register_named(&self, name: String, pipe_id: PipeId, entry: PipeEntry) {
        self.pipes.insert(pipe_id.clone(), entry);
        self.named.insert(name, pipe_id);
    }

    /// Park a reader end for a named pipe, so a connecting agent can take it.
    pub fn park_reader(&self, pipe_id: PipeId, reader: PipeReader) {
        self.parked_readers
            .insert(pipe_id, Mutex::new(Some(reader)));
    }

    /// Take the parked reader for a named pipe (one-shot).
    ///
    /// Returns `None` if no reader was parked or it has already been taken.
    pub fn take_parked_reader(&self, pipe_id: &PipeId) -> Option<PipeReader> {
        self.parked_readers
            .get(pipe_id)
            .and_then(|slot| slot.value().lock().ok()?.take())
    }

    /// Look up the PipeId for a named pipe.
    pub fn resolve_name(&self, name: &str) -> Option<PipeId> {
        self.named.get(name).map(|r| r.value().clone())
    }

    /// Set the reader agent on a pipe entry.
    pub fn set_reader(&self, pipe_id: &PipeId, reader: SessionKey) -> bool {
        if let Some(mut entry) = self.pipes.get_mut(pipe_id) {
            entry.reader = Some(reader);
            true
        } else {
            false
        }
    }

    /// Get metadata for a pipe.
    pub fn get(&self, pipe_id: &PipeId) -> Option<PipeEntry> {
        self.pipes.get(pipe_id).map(|r| r.value().clone())
    }

    /// Remove a pipe from the registry (including its named entry and parked
    /// reader if any).
    pub fn remove(&self, pipe_id: &PipeId) {
        self.pipes.remove(pipe_id);
        self.parked_readers.remove(pipe_id);
        // Clean up named reference if any
        self.named.retain(|_, v| v != pipe_id);
    }

    /// List all pipes owned by an agent.
    pub fn pipes_by_owner(&self, owner: SessionKey) -> Vec<PipeId> {
        self.pipes
            .iter()
            .filter(|entry| entry.value().owner == owner)
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Count of active pipes.
    pub fn len(&self) -> usize { self.pipes.len() }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool { self.pipes.is_empty() }

    /// Count of named pipes.
    pub fn named_count(&self) -> usize { self.named.len() }
}

impl Default for PipeRegistry {
    fn default() -> Self { Self::new() }
}

/// Shared reference to the [`StreamHub`].
pub type StreamHubRef = Arc<StreamHub>;

// ---------------------------------------------------------------------------
// StreamId
// ---------------------------------------------------------------------------

define_id!(
    /// Unique identifier for a stream (ULID string).
    ///
    /// Each agent execution run gets its own `StreamId`, allowing multiple
    /// concurrent streams on the same session.
    StreamId
);

// ---------------------------------------------------------------------------
// StreamEvent
// ---------------------------------------------------------------------------

/// Incremental events emitted during agent execution.
///
/// These are ephemeral — not stored durably. Final results and errors
/// are published through the `OutboundBus`.
/// Terminal status of a background agent task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundTaskStatus {
    Completed,
    Failed,
    Cancelled,
}

/// Incremental events emitted during agent execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    /// Incremental text output from the LLM.
    TextDelta { text: String },
    /// Incremental reasoning/thinking text.
    ReasoningDelta { text: String },
    /// Signal to discard accumulated text from an intermediate iteration.
    ///
    /// Emitted by the agent when an iteration ends with tool calls — the
    /// narration text is noise and should not be shown to the user.
    TextClear,
    /// A tool call has started executing.
    ToolCallStart {
        name:      String,
        id:        String,
        arguments: serde_json::Value,
    },
    /// A tool call has finished.
    ToolCallEnd {
        id:             String,
        result_preview: String,
        success:        bool,
        error:          Option<String>,
    },
    /// Progress stage update.
    Progress { stage: String },
    /// A background agent has been spawned. Client should display an
    /// ongoing status indicator with elapsed timer until
    /// `BackgroundTaskDone` arrives.
    BackgroundTaskStarted {
        task_id:     String,
        agent_name:  String,
        description: String,
    },
    /// A background agent has finished (completed, failed, or cancelled).
    /// Client should remove the status indicator for this task.
    BackgroundTaskDone {
        task_id: String,
        status:  BackgroundTaskStatus,
    },
    /// Cumulative token usage update (emitted after each LLM iteration).
    ///
    /// - `input_tokens`: the *latest* iteration's prompt_tokens (= current
    ///   context size), NOT cumulative — each iteration re-sends full context.
    /// - `output_tokens`: cumulative sum of completion_tokens across all
    ///   iterations in this turn.
    /// - `thinking_ms`: cumulative extended-thinking duration (reasoning phase
    ///   before content generation), 0 if model doesn't support it.
    ///
    /// Consumed by: Telegram progress footer, collapsible trace (#303/#305).
    UsageUpdate {
        input_tokens:  u32,
        output_tokens: u32,
        thinking_ms:   u64,
    },
    /// Turn metrics summary (emitted before stream close).
    TurnMetrics {
        duration_ms: u64,
        iterations:  usize,
        tool_calls:  usize,
        model:       String,
        rara_message_id: String,
    },
    /// A plan has been created with a goal and ordered steps.
    PlanCreated {
        goal:                    String,
        total_steps:             usize,
        compact_summary:         String,
        estimated_duration_secs: Option<u32>,
    },
    /// Incremental plan progress update (replaces PlanStepStart + PlanStepEnd).
    PlanProgress {
        current_step: usize,
        total_steps:  usize,
        status_text:  String,
    },
    /// The plan has been revised.
    PlanReplan { reason: String },
    /// The plan has completed successfully.
    PlanCompleted { summary: String },
}

// ---------------------------------------------------------------------------
// StreamEntry (internal)
// ---------------------------------------------------------------------------

/// Internal entry in the stream table.
struct StreamEntry {
    session_key: SessionKey,
    tx:          broadcast::Sender<StreamEvent>,
}

// ---------------------------------------------------------------------------
// StreamHandle
// ---------------------------------------------------------------------------

/// Handle held by the agent executor to emit stream events.
///
/// Created by [`StreamHub::open`]. The agent emits events via
/// [`emit`](Self::emit).
pub struct StreamHandle {
    stream_id: StreamId,
    tx:        broadcast::Sender<StreamEvent>,
}

impl StreamHandle {
    /// Get the stream ID.
    pub fn stream_id(&self) -> &StreamId { &self.stream_id }

    /// Emit a stream event. Silently drops if no subscribers.
    pub fn emit(&self, event: StreamEvent) { let _ = self.tx.send(event); }
}

// ---------------------------------------------------------------------------
// StreamHub
// ---------------------------------------------------------------------------

/// Central registry for active ephemeral streams.
///
/// Manages the lifecycle of per-execution streams and provides
/// subscription endpoints for egress/frontends.
pub struct StreamHub {
    streams:          DashMap<StreamId, StreamEntry>,
    /// Reverse index: session_key → active stream IDs for O(1) lookup.
    session_streams:  DashMap<SessionKey, Vec<StreamId>>,
    capacity:         usize,
}

impl StreamHub {
    /// Create a new hub with the given per-stream broadcast capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            streams: DashMap::new(),
            session_streams: DashMap::new(),
            capacity,
        }
    }

    /// Open a new stream for an agent execution run.
    ///
    /// Returns a [`StreamHandle`] that the executor uses to emit events.
    #[tracing::instrument(skip(self), fields(stream_id = tracing::field::Empty))]
    pub fn open(&self, session_key: SessionKey) -> StreamHandle {
        let stream_id = StreamId::new();
        tracing::Span::current().record("stream_id", tracing::field::display(&stream_id.0));
        let (tx, _) = broadcast::channel(self.capacity);
        let entry = StreamEntry {
            session_key,
            tx: tx.clone(),
        };
        self.streams.insert(stream_id.clone(), entry);
        self.session_streams
            .entry(session_key)
            .or_default()
            .push(stream_id.clone());
        StreamHandle { stream_id, tx }
    }

    /// Close a stream by its ID.
    ///
    /// This is precise — only the specified stream is removed, not other
    /// streams on the same session.
    #[tracing::instrument(skip(self))]
    pub fn close(&self, stream_id: &StreamId) {
        if let Some((_, entry)) = self.streams.remove(stream_id) {
            if let Some(mut ids) = self.session_streams.get_mut(&entry.session_key) {
                ids.retain(|id| id != stream_id);
                if ids.is_empty() {
                    drop(ids);
                    self.session_streams.remove(&entry.session_key);
                }
            }
        }
    }

    /// Emit a stream event to all active streams for a session.
    ///
    /// Used by background task lifecycle events that need to push to a
    /// session's streams without holding a `StreamHandle`.
    pub fn emit_to_session(&self, session_key: &SessionKey, event: StreamEvent) {
        if let Some(ids) = self.session_streams.get(session_key) {
            for id in ids.iter() {
                if let Some(entry) = self.streams.get(id) {
                    let _ = entry.value().tx.send(event.clone());
                }
            }
        }
    }

    /// Subscribe to all active streams for a given session.
    ///
    /// Returns a list of `(StreamId, Receiver)` pairs. Multiple streams
    /// may exist if the session has concurrent agent runs.
    pub fn subscribe_session(
        &self,
        session_key: &SessionKey,
    ) -> Vec<(StreamId, broadcast::Receiver<StreamEvent>)> {
        let Some(ids) = self.session_streams.get(session_key) else {
            return Vec::new();
        };
        ids.iter()
            .filter_map(|id| {
                self.streams
                    .get(id)
                    .map(|entry| (id.clone(), entry.value().tx.subscribe()))
            })
            .collect()
    }
}

/// Shared reference to an [`IdentityResolver`] implementation.
pub type IdentityResolverRef = Arc<dyn IdentityResolver>;

// ---------------------------------------------------------------------------
// RawPlatformMessage
// ---------------------------------------------------------------------------

/// Raw message from a channel adapter before identity/session resolution.
///
/// Adapters construct this from platform-specific events and hand it to
/// `IngressPipeline::ingest`. The ingress pipeline then resolves identity
/// and session before publishing to the bus.
#[derive(Debug)]
pub struct RawPlatformMessage {
    /// Which channel this message arrived from.
    pub channel_type:        ChannelType,
    /// Platform-specific message ID (for dedup / reply mapping).
    pub platform_message_id: Option<String>,
    /// Platform-specific user identifier.
    pub platform_user_id:    String,
    /// Platform-specific chat/thread identifier.
    pub platform_chat_id:    Option<String>,
    /// Message content (text or multimodal).
    pub content:             MessageContent,
    /// Optional reply/thread context for egress routing.
    pub reply_context:       Option<ReplyContext>,
    /// Arbitrary adapter-specific metadata.
    pub metadata:            HashMap<String, Value>,
}

// ---------------------------------------------------------------------------
// IdentityResolver
// ---------------------------------------------------------------------------

/// Resolves a platform identity to a unified [`UserId`].
///
/// Implementations use an in-memory mapping built from config to translate
/// `(channel_type, platform_user_id)` into a kernel user identity.
#[async_trait]
pub trait IdentityResolver: Send + Sync + 'static {
    /// Map platform coordinates to a kernel-level user identity.
    async fn resolve(
        &self,
        channel_type: ChannelType,
        platform_user_id: &str,
        platform_chat_id: Option<&str>,
    ) -> Result<UserId, IOError>;
}

/// Events sent from a child agent to its parent during execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    /// A key execution milestone (tool call, iteration boundary, etc.).
    Milestone {
        stage:  String,
        detail: Option<String>,
    },
    /// Agent execution completed.
    Done(AgentRunLoopResult),
}

/// Handle returned from spawn — allows waiting for agent completion.
///
/// Holds the spawned agent's ID and an mpsc receiver that carries
/// [`AgentEvent`]s (milestones followed by the final result).
pub struct AgentHandle {
    /// The ID of the spawned agent process.
    pub session_key: SessionKey,
    /// Receiver for agent events. Yields milestones during execution and
    /// a final [`AgentEvent::Done`] when the agent finishes.
    pub result_rx:   mpsc::Receiver<AgentEvent>,
}

// ---------------------------------------------------------------------------
// Endpoint / EndpointAddress
// ---------------------------------------------------------------------------

/// A concrete deliverable target (not the coarse [`ChannelType`]).
///
/// An endpoint pairs a channel type with a specific address, enabling
/// precise delivery to individual connections (e.g. a specific Telegram
/// chat, a specific WebSocket connection).
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct Endpoint {
    /// The channel type of this endpoint.
    pub channel_type: ChannelType,
    /// Platform-specific address details.
    pub address:      EndpointAddress,
}

/// Platform-specific addressing for an [`Endpoint`].
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub enum EndpointAddress {
    /// Telegram chat endpoint.
    Telegram {
        /// Telegram chat ID.
        chat_id:   i64,
        /// Optional thread ID within the chat.
        thread_id: Option<i64>,
    },
    /// Web (SSE / WebSocket) endpoint.
    Web {
        /// Unique connection identifier.
        connection_id: String,
    },
    /// CLI session endpoint.
    Cli {
        /// CLI session identifier.
        session_id: String,
    },
}

// ---------------------------------------------------------------------------
// EndpointRegistry
// ---------------------------------------------------------------------------

/// Tracks per-user active endpoints.
///
/// Thread-safe via `DashMap`. Adapters register endpoints when a user
/// connects and unregister when they disconnect.
#[derive(Default)]
pub struct EndpointRegistry {
    connections: DashMap<UserId, HashSet<Endpoint>>,
}

impl EndpointRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            connections: DashMap::new(),
        }
    }

    /// Register an endpoint for a user.
    pub fn register(&self, user: &UserId, endpoint: Endpoint) {
        self.connections
            .entry(user.clone())
            .or_default()
            .insert(endpoint);
    }

    /// Unregister an endpoint for a user.
    pub fn unregister(&self, user: &UserId, endpoint: &Endpoint) {
        if let Some(mut endpoints) = self.connections.get_mut(user) {
            endpoints.remove(endpoint);
            if endpoints.is_empty() {
                drop(endpoints);
                self.connections.remove(user);
            }
        }
    }

    /// Get all active endpoints for a user.
    fn get_endpoints(&self, user: &UserId) -> Vec<Endpoint> {
        self.connections
            .get(user)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Check whether a user has any active endpoints.
    fn is_online(&self, user: &UserId) -> bool {
        self.connections
            .get(user)
            .map(|set| !set.is_empty())
            .unwrap_or(false)
    }
}

// ---------------------------------------------------------------------------
// PlatformOutbound
// ---------------------------------------------------------------------------

/// What [`ChannelAdapter::send`](crate::channel::adapter::ChannelAdapter::send)
/// receives for delivery.
///
/// This is the adapter-facing message type — already formatted and ready
/// for the specific platform.
#[derive(Debug, Clone)]
pub enum PlatformOutbound {
    /// A complete reply message.
    Reply {
        /// Text content to deliver.
        content:       String,
        /// Binary attachments.
        attachments:   Vec<Attachment>,
        /// Optional reply context for threading.
        reply_context: Option<ReplyContext>,
    },
    /// An incremental streaming chunk.
    StreamChunk {
        /// Incremental text delta.
        delta:       String,
        /// Platform message ID to edit (for progressive updates).
        edit_target: Option<String>,
    },
    /// A progress/status update.
    Progress {
        /// Progress text.
        text: String,
    },
}

// ---------------------------------------------------------------------------
// EgressError
// ---------------------------------------------------------------------------

/// Errors from egress delivery.
#[derive(Debug, Snafu)]
pub enum EgressError {
    /// Delivery to the target endpoint failed.
    #[snafu(display("delivery failed: {message}"))]
    DeliveryFailed { message: String },

    /// Delivery timed out.
    #[snafu(display("delivery timeout"))]
    Timeout,
}

/// Shared reference to a
/// [`ChannelAdapter`](crate::channel::adapter::ChannelAdapter).
pub type ChannelAdapterRef = crate::channel::adapter::ChannelAdapterRef;

/// Shared reference to the [`EndpointRegistry`].
pub type EndpointRegistryRef = Arc<EndpointRegistry>;

// ---------------------------------------------------------------------------
// IngressRateLimiter
// ---------------------------------------------------------------------------

/// Per-key sliding-window rate limiter for ingress messages.
///
/// Uses a 60-second window with a configurable max count per key.
/// Keys are formatted as `"{channel_type}:{platform_user_id}"`.
///
/// Ref: OpenFang `openfang-channels/src/bridge.rs` — `ChannelRateLimiter`.
pub struct IngressRateLimiter {
    /// Per-key timestamps of accepted messages within the current window.
    buckets:        DashMap<String, Vec<std::time::Instant>>,
    /// Maximum messages per key per 60-second window.
    max_per_minute: u32,
    /// Clock function for obtaining the current instant (test-injectable).
    now_fn:         fn() -> std::time::Instant,
}

impl IngressRateLimiter {
    /// Create a new rate limiter with the given per-key limit.
    pub fn new(max_per_minute: u32) -> Self {
        Self {
            buckets: DashMap::new(),
            max_per_minute,
            now_fn: std::time::Instant::now,
        }
    }

    /// Create a rate limiter with a custom clock (for testing).
    #[cfg(test)]
    fn with_clock(max_per_minute: u32, now_fn: fn() -> std::time::Instant) -> Self {
        Self {
            buckets: DashMap::new(),
            max_per_minute,
            now_fn,
        }
    }

    /// Check whether a key is within its rate limit.
    ///
    /// Returns `Ok(())` if allowed, `Err(IOError::RateLimited)` if exceeded.
    pub fn check_rate(&self, key: &str) -> Result<(), IOError> {
        let now = (self.now_fn)();
        let window = std::time::Duration::from_secs(60);

        let mut entry = self.buckets.entry(key.to_string()).or_default();
        entry.retain(|ts| now.duration_since(*ts) < window);

        if entry.len() >= self.max_per_minute as usize {
            return Err(IOError::RateLimited {
                message: format!(
                    "Rate limit exceeded ({} messages/minute). Please wait.",
                    self.max_per_minute
                ),
            });
        }

        entry.push(now);
        Ok(())
    }

    /// Remove keys whose window has fully expired.
    ///
    /// Called by the kernel's unified scheduler (processor 0) on every tick
    /// to reclaim memory from users who have gone idle.
    pub fn gc(&self) {
        let now = (self.now_fn)();
        let window = std::time::Duration::from_secs(60);
        self.buckets.retain(|_, v| {
            v.retain(|ts| now.duration_since(*ts) < window);
            !v.is_empty()
        });
    }
}

// ---------------------------------------------------------------------------
// IOSubsystem
// ---------------------------------------------------------------------------

/// Bundled I/O subsystem for the kernel.
///
/// Owns all ingress/egress components: identity resolution, channel-binding
/// lookup, real-time token streaming ([`StreamHub`]), egress adapters, and
/// endpoint registry.
///
/// ## Ingress pipeline
///
/// [`resolve()`](Self::resolve) is **pure translation** with no side effects:
///
/// ```text
/// RawPlatformMessage
///   → IngressRateLimiter: per-user sliding-window check (rejects spam before DB)
///   → IdentityResolver:   (channel_type, platform_user_id) → UserId
///   → SessionIndex:       (channel_type, chat_id)          → Option<SessionKey>
///   → InboundMessage { session_key: Option<SessionKey> }
/// ```
///
/// When no channel binding exists (first message from a new chat),
/// `session_key` is `None`. The kernel — not the I/O layer — is responsible for
/// creating the session and writing the binding. See
/// `Kernel::handle_user_message()`.
///
/// Constructed at the app/boot layer and injected into
/// [`Kernel::new()`](crate::kernel::Kernel::new) as a single unit.
pub struct IOSubsystem {
    identity_resolver:       IdentityResolverRef,
    session_index:           Arc<dyn SessionIndex>,
    stream_hub:              StreamHubRef,
    adapters:                HashMap<ChannelType, ChannelAdapterRef>,
    endpoint_registry:       EndpointRegistryRef,
    /// Telegram channel ID for agent-initiated notifications.
    notification_channel_id: Option<i64>,
    rate_limiter:            IngressRateLimiter,
}

impl IOSubsystem {
    /// Create a new I/O subsystem.
    ///
    /// Internally creates a [`StreamHub`] and [`EndpointRegistry`].
    /// Call [`register_adapter`](Self::register_adapter) to add egress
    /// adapters before passing to the kernel.
    pub fn new(
        identity_resolver: IdentityResolverRef,
        session_index: Arc<dyn SessionIndex>,
        notification_channel_id: Option<i64>,
        max_ingress_per_minute: u32,
    ) -> Self {
        Self {
            identity_resolver,
            session_index,
            stream_hub: Arc::new(StreamHub::new(256)),
            adapters: HashMap::new(),
            endpoint_registry: Arc::new(EndpointRegistry::new()),
            notification_channel_id,
            rate_limiter: IngressRateLimiter::new(max_ingress_per_minute),
        }
    }

    // -- Maintenance ----------------------------------------------------------

    /// Run garbage collection on the ingress rate limiter, evicting expired
    /// keys to prevent unbounded memory growth.
    pub fn gc_rate_limiter(&self) { self.rate_limiter.gc(); }

    // -- Ingress --------------------------------------------------------------

    /// Resolve identity and channel binding for a raw platform message.
    ///
    /// This is a **read-only** operation — it never creates sessions or writes
    /// bindings. Returns a fully-formed [`InboundMessage`] ready for the event
    /// queue.
    ///
    /// ## Session resolution
    ///
    /// Looks up `(channel_type, platform_chat_id)` in the [`SessionIndex`]
    /// binding table:
    /// - **Found** → `session_key = Some(bound_key)`
    /// - **Not found / no chat_id** → `session_key = None`
    ///
    /// The kernel handles the `None` case by creating a session on demand.
    /// See `Kernel::handle_user_message()`.
    #[tracing::instrument(
        skip(self, raw),
        fields(
            channel = ?raw.channel_type,
            platform_user = %raw.platform_user_id,
            session_id,
            user_id,
        )
    )]
    pub async fn resolve(&self, raw: RawPlatformMessage) -> Result<InboundMessage, IOError> {
        let span = tracing::Span::current();

        // 1. Rate-limit ingress before any expensive operations.
        let rate_key = format!("{}:{}", raw.channel_type, raw.platform_user_id);
        self.rate_limiter.check_rate(&rate_key)?;

        // 2. Resolve identity
        let user_id = self
            .identity_resolver
            .resolve(
                raw.channel_type,
                &raw.platform_user_id,
                raw.platform_chat_id.as_deref(),
            )
            .await?;
        span.record("user_id", tracing::field::display(&user_id.0));

        // 3. Look up channel binding (pure lookup, no creation)
        let session_key = match raw.platform_chat_id.as_deref() {
            Some(chat_id) => {
                match self
                    .session_index
                    .get_channel_binding(&raw.channel_type.to_string(), chat_id)
                    .await
                {
                    Ok(Some(binding)) => {
                        span.record("session_id", tracing::field::display(&binding.session_key));
                        Some(binding.session_key)
                    }
                    Ok(None) => None,
                    Err(e) => {
                        tracing::warn!(error = %e, "channel binding lookup failed");
                        None
                    }
                }
            }
            None => None,
        };

        // 4. Build InboundMessage
        let msg = InboundMessage {
            id: MessageId::new(),
            source: ChannelSource {
                channel_type:        raw.channel_type,
                platform_message_id: raw.platform_message_id,
                platform_user_id:    raw.platform_user_id,
                platform_chat_id:    raw.platform_chat_id,
            },
            user: user_id,
            session_key,
            target_session_key: None,
            content: raw.content,
            reply_context: raw.reply_context,
            timestamp: jiff::Timestamp::now(),
            metadata: raw.metadata,
        };

        tracing::info!(
            channel = ?msg.source.channel_type,
            user_id = %msg.user.0,
            session_id = ?msg.session_key,
            content = %msg.content.as_text(),
            "resolved inbound message",
        );

        Ok(msg)
    }

    // -- Egress ---------------------------------------------------------------

    /// Register an egress adapter for a channel type.
    ///
    /// Must be called **before** passing to the kernel.
    pub fn register_adapter(&mut self, channel_type: ChannelType, adapter: ChannelAdapterRef) {
        self.adapters.insert(channel_type, adapter);
    }

    /// Spawn a deliver task so that egress I/O (Telegram API, WebSocket
    /// send, etc.) does not block the kernel event loop.
    pub fn deliver(self: &Arc<Self>, envelope: OutboundEnvelope) {
        let this = Arc::clone(self);
        let payload_type = match &envelope.payload {
            OutboundPayload::Reply { .. } => "reply",
            OutboundPayload::Progress { .. } => "progress",
            OutboundPayload::StateChange { .. } => "state_change",
            OutboundPayload::Error { .. } => "error",
        };
        let span = tracing::info_span!(
            "deliver",
            session_id = %envelope.session_key,
            payload_type,
        );

        tokio::spawn(
            async move {
                this.deliver_to_endpoints(envelope).await;
            }
            .instrument(span),
        );
    }

    /// Send a notification message to the configured notification channel.
    ///
    /// Delivers via the Telegram adapter to `notification_channel_id`.
    /// If no channel ID is configured or no Telegram adapter is registered,
    /// the notification is silently dropped with a warning.
    pub fn send_notification(self: &Arc<Self>, message: String) {
        let Some(chat_id) = self.notification_channel_id else {
            tracing::warn!("send_notification: no notification_channel_id configured, dropping");
            return;
        };
        let Some(adapter) = self.adapters.get(&ChannelType::Telegram).cloned() else {
            tracing::warn!("send_notification: no Telegram adapter registered, dropping");
            return;
        };

        let endpoint = Endpoint {
            channel_type: ChannelType::Telegram,
            address:      EndpointAddress::Telegram {
                chat_id,
                thread_id: None,
            },
        };

        let span = tracing::info_span!("send_notification", %chat_id);
        tokio::spawn(
            async move {
                let outbound = PlatformOutbound::Reply {
                    content:       message,
                    attachments:   vec![],
                    reply_context: None,
                };
                if let Err(e) = adapter.send(&endpoint, outbound).await {
                    tracing::warn!(%e, "send_notification: delivery failed");
                }
            }
            .instrument(span),
        );
    }

    /// Register egress endpoint for stateless channels (e.g. Telegram).
    ///
    /// Connection-oriented channels (Web) register on WS/SSE connect.
    /// Stateless channels have no persistent connection, so we register on
    /// every inbound message (idempotent — EndpointRegistry uses a HashSet).
    pub fn register_stateless_endpoint(&self, msg: &InboundMessage) {
        if msg.source.channel_type != ChannelType::Telegram {
            return;
        }
        let Some(ref chat_id_str) = msg.source.platform_chat_id else {
            return;
        };
        let Ok(chat_id) = chat_id_str.parse::<i64>() else {
            return;
        };
        self.endpoint_registry.register(
            &msg.user,
            Endpoint {
                channel_type: ChannelType::Telegram,
                address:      EndpointAddress::Telegram {
                    chat_id,
                    thread_id: None,
                },
            },
        );
    }

    // -- Accessors (external consumers) ---------------------------------------

    /// Access the session index.
    pub fn session_index(&self) -> &Arc<dyn SessionIndex> { &self.session_index }

    /// Access the stream hub.
    pub fn stream_hub(&self) -> &StreamHubRef { &self.stream_hub }

    /// Access the endpoint registry.
    pub fn endpoint_registry(&self) -> &EndpointRegistryRef { &self.endpoint_registry }

    /// Deliver a single outbound envelope to all resolved targets.
    #[tracing::instrument(
        skip(self, envelope),
        fields(
            user_id = %envelope.user.0,
            session_id = %envelope.session_key,
        )
    )]
    async fn deliver_to_endpoints(&self, envelope: OutboundEnvelope) {
        let connected = self.endpoint_registry.get_endpoints(&envelope.user);
        let mut targets = match &envelope.routing {
            OutboundRouting::BroadcastAll => connected,
            OutboundRouting::BroadcastExcept { exclude } => connected
                .into_iter()
                .filter(|e| &e.channel_type != exclude)
                .collect(),
            OutboundRouting::Targeted { channels } => connected
                .into_iter()
                .filter(|e| channels.contains(&e.channel_type))
                .collect(),
        };

        // Session-scoped delivery: when origin_endpoint is set, deliver only
        // to that endpoint instead of broadcasting to all same-type endpoints.
        if let Some(ref origin) = envelope.origin_endpoint {
            targets.retain(|e| e == origin);
        }

        let futs = targets.into_iter().map(|endpoint| {
            let adapter = self.adapters.get(&endpoint.channel_type).cloned();
            let outbound = envelope.to_platform_outbound();
            async move {
                if let Some(adapter) = adapter {
                    match tokio::time::timeout(
                        std::time::Duration::from_secs(10),
                        adapter.send(&endpoint, outbound),
                    )
                    .await
                    {
                        Ok(Ok(())) => {
                            crate::metrics::MESSAGE_OUTBOUND
                                .with_label_values(&[&format!("{:?}", endpoint.channel_type)])
                                .inc();
                        }
                        Ok(Err(e)) => {
                            tracing::warn!(?endpoint, %e, "delivery failed");
                        }
                        Err(_) => {
                            tracing::warn!(?endpoint, "delivery timeout");
                        }
                    }
                }
            }
        });
        futures::future::join_all(futs).await;
    }
}

#[cfg(test)]
mod agent_event_tests {
    use super::*;
    use crate::session::AgentRunLoopResult;

    #[test]
    fn milestone_serializes_to_json() {
        let event = AgentEvent::Milestone {
            stage:  "tool_call_start".to_string(),
            detail: Some("mobile_screenshot".to_string()),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "milestone");
        assert_eq!(json["stage"], "tool_call_start");
        assert_eq!(json["detail"], "mobile_screenshot");
    }

    #[test]
    fn done_wraps_result() {
        let result = AgentRunLoopResult {
            output:     "done".to_string(),
            iterations: 3,
            tool_calls: 5,
        };
        let event = AgentEvent::Done(result);
        match event {
            AgentEvent::Done(r) => assert_eq!(r.output, "done"),
            _ => panic!("expected Done"),
        }
    }
}

#[cfg(test)]
mod ingress_rate_limiter_tests {
    use std::{
        sync::atomic::{AtomicU64, Ordering},
        time::{Duration, Instant},
    };

    use super::*;

    /// Fake clock that advances by a controllable offset from a fixed base.
    static FAKE_OFFSET_MS: AtomicU64 = AtomicU64::new(0);

    fn fake_now() -> Instant {
        // SAFETY: Instant::now() is called once as a base; offset simulates time
        // passing.
        static BASE: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        let base = *BASE.get_or_init(Instant::now);
        base + Duration::from_millis(FAKE_OFFSET_MS.load(Ordering::Relaxed))
    }

    fn set_fake_offset(ms: u64) { FAKE_OFFSET_MS.store(ms, Ordering::Relaxed); }

    #[test]
    fn rate_limiter_allows_within_limit() {
        let limiter = IngressRateLimiter::new(3);
        let key = "telegram:user123";
        assert!(limiter.check_rate(key).is_ok());
        assert!(limiter.check_rate(key).is_ok());
        assert!(limiter.check_rate(key).is_ok());
        assert!(limiter.check_rate(key).is_err());
    }

    #[test]
    fn rate_limiter_independent_keys() {
        let limiter = IngressRateLimiter::new(1);
        assert!(limiter.check_rate("user_a").is_ok());
        assert!(limiter.check_rate("user_b").is_ok());
        assert!(limiter.check_rate("user_a").is_err());
    }

    #[test]
    fn rate_limiter_zero_limit_blocks_all() {
        let limiter = IngressRateLimiter::new(0);
        assert!(limiter.check_rate("user").is_err());
    }

    #[test]
    fn rate_limiter_window_expires_via_clock() {
        set_fake_offset(0);
        let limiter = IngressRateLimiter::with_clock(1, fake_now);
        let key = "test:user";

        assert!(limiter.check_rate(key).is_ok());
        assert!(limiter.check_rate(key).is_err());

        // Advance clock past the 60s window.
        set_fake_offset(61_000);
        assert!(
            limiter.check_rate(key).is_ok(),
            "should allow after window expires"
        );
    }

    #[test]
    fn rate_limiter_gc_removes_expired_keys() {
        set_fake_offset(100_000); // reset to a fresh base offset
        let limiter = IngressRateLimiter::with_clock(10, fake_now);
        assert!(limiter.check_rate("active").is_ok());
        assert!(limiter.check_rate("stale").is_ok());
        assert_eq!(limiter.buckets.len(), 2);

        // Advance clock past window so "stale" and "active" entries expire.
        set_fake_offset(200_000);
        // Re-record "active" so it has a fresh timestamp.
        assert!(limiter.check_rate("active").is_ok());

        limiter.gc();

        assert!(limiter.buckets.contains_key("active"));
        assert!(!limiter.buckets.contains_key("stale"));
    }
}
