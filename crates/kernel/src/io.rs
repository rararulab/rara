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
//! - **Ingress**: channel adapters publish messages through
//!   [`IngressPipeline`](ingress::IngressPipeline) into the unified
//!   [`EventQueue`](crate::queue::EventQueue).
//! - **Egress**: the kernel event loop delivers outbound envelopes via
//!   [`IOSubsystem::deliver`] to registered adapters.
//! - **Streaming**: ephemeral real-time events (token deltas, tool progress)
//!   flow through the [`StreamHub`](stream::StreamHub) for connected frontends.
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
use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::Instrument;
use uuid::Uuid;

use crate::{
    channel::types::{ChannelType, MessageContent},
    identity::UserId,
    session::{AgentRunLoopResult, SessionKey},
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
/// After ingress resolves identity and session, the raw platform event is
/// converted into this type and published to the
/// [`EventQueue`](crate::queue::EventQueue).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundMessage {
    /// Unique message identifier (ULID).
    pub id:                 MessageId,
    /// Platform source details.
    pub source:             ChannelSource,
    /// Unified user identity (resolved by ingress).
    pub user:               UserId,
    /// Session this message belongs to.
    pub session_key:        SessionKey,
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
            session_key: session_id,
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
            session_key: session_id,
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
            session_key,
            target_session_key: Some(target_id),
            content: MessageContent::Text(text),
            reply_context: None,
            timestamp: jiff::Timestamp::now(),
            metadata: HashMap::new(),
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
    pub id:          MessageId,
    /// The inbound message this is replying to.
    pub in_reply_to: MessageId,
    /// Target user.
    pub user:        UserId,
    /// Session context.
    pub session_key: SessionKey,
    /// How to route this envelope.
    pub routing:     OutboundRouting,
    /// The payload to deliver.
    pub payload:     OutboundPayload,
    /// When this envelope was created.
    pub timestamp:   jiff::Timestamp,
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
        }
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
    /// Parked readers for named pipes (take-once via Mutex<Option>).
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    /// Incremental text output from the LLM.
    TextDelta { text: String },
    /// Incremental reasoning/thinking text.
    ReasoningDelta { text: String },
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
    /// Turn metrics summary (emitted before stream close).
    TurnMetrics {
        duration_ms: u64,
        iterations:  usize,
        tool_calls:  usize,
        model:       String,
    },
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
    streams:  DashMap<StreamId, StreamEntry>,
    capacity: usize,
}

impl StreamHub {
    /// Create a new hub with the given per-stream broadcast capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            streams: DashMap::new(),
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
        StreamHandle { stream_id, tx }
    }

    /// Close a stream by its ID.
    ///
    /// This is precise — only the specified stream is removed, not other
    /// streams on the same session.
    #[tracing::instrument(skip(self))]
    pub fn close(&self, stream_id: &StreamId) { self.streams.remove(stream_id); }

    /// Subscribe to all active streams for a given session.
    ///
    /// Returns a list of `(StreamId, Receiver)` pairs. Multiple streams
    /// may exist if the session has concurrent agent runs.
    pub fn subscribe_session(
        &self,
        session_key: &SessionKey,
    ) -> Vec<(StreamId, broadcast::Receiver<StreamEvent>)> {
        self.streams
            .iter()
            .filter(|entry| &entry.value().session_key == session_key)
            .map(|entry| (entry.key().clone(), entry.value().tx.subscribe()))
            .collect()
    }
}

/// Shared reference to an [`IdentityResolver`] implementation.
pub type IdentityResolverRef = Arc<dyn IdentityResolver>;

/// Shared reference to a [`SessionResolver`] implementation.
pub type SessionResolverRef = Arc<dyn SessionResolver>;

// ---------------------------------------------------------------------------
// RawPlatformMessage
// ---------------------------------------------------------------------------

/// Raw message from a channel adapter before identity/session resolution.
///
/// Adapters construct this from platform-specific events and hand it to
/// [`IngressPipeline::ingest`]. The ingress pipeline then resolves identity
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
/// Implementations may look up a database mapping, create auto-provisioned
/// users, or apply group-chat policies.
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

// ---------------------------------------------------------------------------
// SessionResolver
// ---------------------------------------------------------------------------

/// Resolves or creates a session for a given user + channel context.
///
/// Implementations may support cross-channel session sharing (e.g. the same
/// user on Telegram and Web shares a session) or per-chat isolation.
#[async_trait]
pub trait SessionResolver: Send + Sync + 'static {
    /// Resolve (or create) a session for the given user and channel context.
    async fn resolve(
        &self,
        user: &UserId,
        channel_type: ChannelType,
        platform_chat_id: Option<&str>,
    ) -> Result<SessionKey, IOError>;
}

/// Handle returned from spawn — allows waiting for agent completion.
///
/// Holds the spawned agent's ID and a oneshot receiver that resolves when
/// the agent finishes execution (successfully or with failure).
// TODO: deprecate me
pub struct AgentHandle {
    /// The ID of the spawned agent process.
    pub session_key: SessionKey,
    /// Receiver for the agent's result. Resolves when the agent finishes.
    pub result_rx:   oneshot::Receiver<AgentRunLoopResult>,
}

// ---------------------------------------------------------------------------
// Endpoint / EndpointAddress
// ---------------------------------------------------------------------------

/// A concrete deliverable target (not the coarse [`ChannelType`]).
///
/// An endpoint pairs a channel type with a specific address, enabling
/// precise delivery to individual connections (e.g. a specific Telegram
/// chat, a specific WebSocket connection).
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct Endpoint {
    /// The channel type of this endpoint.
    pub channel_type: ChannelType,
    /// Platform-specific address details.
    pub address:      EndpointAddress,
}

/// Platform-specific addressing for an [`Endpoint`].
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
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

/// Shared reference to a [`ChannelAdapter`](crate::channel::adapter::ChannelAdapter).
pub type ChannelAdapterRef = crate::channel::adapter::ChannelAdapterRef;

/// Shared reference to the [`EndpointRegistry`].
pub type EndpointRegistryRef = Arc<EndpointRegistry>;

// ---------------------------------------------------------------------------
// IOSubsystem
// ---------------------------------------------------------------------------

/// Bundled I/O subsystem for the kernel.
///
/// Owns all ingress/egress components: identity + session resolution,
/// real-time token streaming ([`StreamHub`]), egress adapters, and
/// endpoint registry.
///
/// Constructed at the app/boot layer and injected into
/// [`Kernel::new()`](crate::kernel::Kernel::new) as a single unit.
pub struct IOSubsystem {
    identity_resolver: IdentityResolverRef,
    session_resolver:  SessionResolverRef,
    stream_hub:        StreamHubRef,
    adapters:   HashMap<ChannelType, ChannelAdapterRef>,
    endpoint_registry: EndpointRegistryRef,
}

impl IOSubsystem {
    /// Create a new I/O subsystem.
    ///
    /// Internally creates a [`StreamHub`] and [`EndpointRegistry`].
    /// Call [`register_adapter`](Self::register_adapter) to add egress
    /// adapters before passing to the kernel.
    pub fn new(
        identity_resolver: IdentityResolverRef,
        session_resolver: SessionResolverRef,
    ) -> Self {
        Self {
            identity_resolver,
            session_resolver,
            stream_hub: Arc::new(StreamHub::new(256)),
            adapters: HashMap::new(),
            endpoint_registry: Arc::new(EndpointRegistry::new()),
        }
    }

    // -- Ingress --------------------------------------------------------------

    /// Resolve identity and session for a raw platform message.
    ///
    /// Returns a fully-formed [`InboundMessage`] ready for the event queue.
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

        // 1. Resolve identity
        let user_id = self
            .identity_resolver
            .resolve(
                raw.channel_type,
                &raw.platform_user_id,
                raw.platform_chat_id.as_deref(),
            )
            .await?;
        span.record("user_id", tracing::field::display(&user_id.0));

        // 2. Resolve session
        let session_id = self
            .session_resolver
            .resolve(&user_id, raw.channel_type, raw.platform_chat_id.as_deref())
            .await?;
        span.record("session_id", tracing::field::display(&session_id));

        // 3. Build InboundMessage
        let msg = InboundMessage {
            id:                 MessageId::new(),
            source:             ChannelSource {
                channel_type:        raw.channel_type,
                platform_message_id: raw.platform_message_id,
                platform_user_id:    raw.platform_user_id,
                platform_chat_id:    raw.platform_chat_id,
            },
            user:               user_id,
            session_key:        session_id,
            target_session_key: None,
            content:            raw.content,
            reply_context:      raw.reply_context,
            timestamp:          jiff::Timestamp::now(),
            metadata:           raw.metadata,
        };

        tracing::info!(
            channel = ?msg.source.channel_type,
            user_id = %msg.user.0,
            session_id = %msg.session_key,
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
        let targets = match &envelope.routing {
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
