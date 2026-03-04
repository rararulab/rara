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
//!   [`Egress::deliver`](egress::Egress::deliver) to registered adapters.
//! - **Streaming**: ephemeral real-time events (token deltas, tool progress)
//!   flow through the [`StreamHub`](stream::StreamHub) for connected frontends.
//!
//! ## Architecture
//!
//! ```text
//! Adapters → IngressPipeline → EventQueue → Kernel Event Loop
//!                                                   ↓
//!                                         Egress::deliver + StreamHub
//!                                                   ↓
//!                                         Channel Adapters (Web, Telegram, ...)
//! ```

use std::{
    collections::HashMap,
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

use crate::session::SessionKey;

/// Well-known progress stage names used by `OutboundPayload::Progress` and
/// `StreamEvent::Progress`.
pub mod stages {
    pub const THINKING: &str = "thinking";
}

use crate::{
    channel::types::{ChannelType, MessageContent},
    process::principal::UserId,
    session::SessionKey,
};

// ---------------------------------------------------------------------------
// MessageId
// ---------------------------------------------------------------------------

/// ULID-based message identifier.
///
/// Every inbound and outbound message gets a unique `MessageId` for
/// correlation, deduplication, and reply threading.
define_id!(MessageId);

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
        target_session_key: SessionKey,
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
    pub session_id:  SessionKey,
    /// How to route this envelope.
    pub routing:     OutboundRouting,
    /// The payload to deliver.
    pub payload:     OutboundPayload,
    /// When this envelope was created.
    pub timestamp:   jiff::Timestamp,
}

impl OutboundEnvelope {
    /// Create an error envelope with `BroadcastAll` routing.
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
            session_id,
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
            session_id,
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
        session_id: SessionKey,
        stage: impl Into<String>,
        detail: Option<String>,
    ) -> Self {
        Self {
            id: MessageId::new(),
            in_reply_to,
            user,
            session_id,
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
            session_id,
            routing: OutboundRouting::BroadcastAll,
            payload: OutboundPayload::StateChange {
                event_type: event_type.into(),
                data,
            },
            timestamp: jiff::Timestamp::now(),
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
    async fn append(&self, envelope: OutboundEnvelope) -> Result<(), BusError>;

    /// Drain up to `max` pending envelopes for re-delivery.
    async fn drain_pending(&self, max: usize) -> Vec<OutboundEnvelope>;

    /// Mark an envelope as successfully delivered (remove from outbox).
    async fn mark_delivered(&self, id: &MessageId) -> Result<(), BusError>;
}

// ---------------------------------------------------------------------------
// PipeId
// ---------------------------------------------------------------------------

/// Unique identifier for a pipe (ULID string).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PipeId(pub String);

impl PipeId {
    /// Generate a new ULID-based pipe ID.
    pub fn new() -> Self { Self(ulid::Ulid::new().to_string()) }
}

impl Default for PipeId {
    fn default() -> Self { Self::new() }
}

impl std::fmt::Display for PipeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(&self.0) }
}

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

impl std::fmt::Debug for PipeWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PipeWriter")
            .field("pipe_id", &self.pipe_id)
            .field("tx", &"<mpsc::Sender>")
            .finish()
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

impl std::fmt::Debug for PipeReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PipeReader")
            .field("pipe_id", &self.pipe_id)
            .field("rx", &"<mpsc::Receiver>")
            .finish()
    }
}

// ---------------------------------------------------------------------------
// PipeSendError
// ---------------------------------------------------------------------------

/// Error returned when writing to a pipe whose reader has been dropped.
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

/// Unique identifier for a stream (ULID string).
///
/// Each agent execution run gets its own `StreamId`, allowing multiple
/// concurrent streams on the same session.
define_id!(StreamId);

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
        tracing::Span::current().record("stream_id", stream_id.0.as_str());
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
            .filter(|entry| &entry.value().SessionKey == session_key)
            .map(|entry| (entry.key().clone(), entry.value().tx.subscribe()))
            .collect()
    }
}

/// Shared reference to an [`IdentityResolver`] implementation.
pub type IdentityResolverRef = Arc<dyn IdentityResolver>;

/// Shared reference to a [`SessionResolver`] implementation.
pub type SessionResolverRef = Arc<dyn SessionResolver>;

/// Shared reference to the [`IngressPipeline`].
pub type IngressPipelineRef = Arc<IngressPipeline>;

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
    ) -> Result<UserId, IngestError>;
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
    ) -> Result<SessionKey, IngestError>;
}

// ---------------------------------------------------------------------------
// IngressPipeline
// ---------------------------------------------------------------------------

/// Resolves identity and session for raw platform messages.
///
/// This is a pure resolution layer — it does not push events or interact
/// with the event queue.
/// [`KernelHandle::ingest`](crate::handle::KernelHandle::ingest)
/// calls [`resolve`](Self::resolve) and then pushes the resulting
/// [`InboundMessage`] through the event queue.
pub struct IngressPipeline {
    identity_resolver: Arc<dyn IdentityResolver>,
    session_resolver:  Arc<dyn SessionResolver>,
}

impl IngressPipeline {
    /// Create a new ingress pipeline.
    pub fn new(
        identity_resolver: Arc<dyn IdentityResolver>,
        session_resolver: Arc<dyn SessionResolver>,
    ) -> Self {
        Self {
            identity_resolver,
            session_resolver,
        }
    }

    /// Resolve identity and session for a raw platform message.
    ///
    /// Returns a fully-formed [`InboundMessage`] ready for the event queue.
    pub async fn resolve(&self, raw: RawPlatformMessage) -> Result<InboundMessage, IngestError> {
        let span = tracing::info_span!(
            "ingress",
            channel = ?raw.channel_type,
            platform_user = %raw.platform_user_id,
            session_id = tracing::field::Empty,
            user_id = tracing::field::Empty,
        );
        let _guard = span.enter();

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
            session_key:        None,
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
}

/// Manages egress adapters and the endpoint registry for outbound message
/// delivery.
///
/// Owns the `egress_adapters` map (previously on `Kernel`) and the
/// `endpoint_registry`. Provides `deliver()` for fire-and-forget outbound
/// delivery and `register_endpoint()` for stateless channel registration.
pub(crate) struct DeliverySubsystem {
    /// Registered egress adapters keyed by channel type.
    egress_adapters:   HashMap<ChannelType, EgressAdapterRef>,
    /// Per-user endpoint registry (tracks connected channels).
    endpoint_registry: EndpointRegistryRef,
}

impl DeliverySubsystem {
    /// Create a new delivery subsystem.
    pub fn new(endpoint_registry: EndpointRegistryRef) -> Self {
        Self {
            egress_adapters: HashMap::new(),
            endpoint_registry,
        }
    }

    /// Register an egress adapter for a channel type.
    ///
    /// Must be called **before** the kernel event loop starts.
    pub fn register_adapter(&mut self, channel_type: ChannelType, adapter: EgressAdapterRef) {
        self.egress_adapters.insert(channel_type, adapter);
    }

    /// Access the egress adapters map.
    pub fn egress_adapters(&self) -> &HashMap<ChannelType, EgressAdapterRef> {
        &self.egress_adapters
    }

    /// Access the endpoint registry.
    pub fn endpoint_registry(&self) -> &EndpointRegistryRef { &self.endpoint_registry }

    /// Spawn a Deliver event as an independent task so that egress I/O
    /// (Telegram API, WebSocket send, etc.) does not block the event loop.
    pub fn deliver(&self, envelope: OutboundEnvelope) {
        let adapters = self.egress_adapters.clone();
        let endpoints = Arc::clone(&self.endpoint_registry);
        let payload_type = match &envelope.payload {
            OutboundPayload::Reply { .. } => "reply",
            OutboundPayload::Progress { .. } => "progress",
            OutboundPayload::StateChange { .. } => "state_change",
            OutboundPayload::Error { .. } => "error",
        };
        let span = tracing::info_span!(
            "deliver",
            session_id = %envelope.session_id,
            payload_type,
        );

        tokio::spawn(
            async move {
                crate::io::egress::Egress::deliver(&adapters, &endpoints, envelope).await;
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
            crate::io::egress::Endpoint {
                channel_type: ChannelType::Telegram,
                address:      crate::io::egress::EndpointAddress::Telegram {
                    chat_id,
                    thread_id: None,
                },
            },
        );
    }
}
