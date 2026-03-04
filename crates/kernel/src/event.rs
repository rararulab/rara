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

//! Unified kernel event — struct with base metadata and a kind discriminator.
//!
//! Every interaction with the kernel is represented as a [`KernelEvent`]
//! (`EventBase` + `EventKind`) and processed by `Kernel::run()`.

use std::sync::Arc;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use crate::{
    agent_turn::AgentTurnResult,
    io::{
        pipe::{PipeReader, PipeWriter},
        types::{InboundMessage, MessageId, OutboundEnvelope},
    },
    memory::KvScope,
    process::{
        AgentId, AgentManifest, AgentResult, ProcessInfo, SessionId, Signal,
        principal::{Principal, UserId},
    },
    tool::ToolRegistry,
};

// ---------------------------------------------------------------------------
// EventId
// ---------------------------------------------------------------------------

/// ULID-based event identifier (time-sortable, unique).
///
/// Every kernel event gets a unique `EventId` for correlation, tracing,
/// and deduplication.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EventId(pub String);

impl EventId {
    /// Generate a new ULID-based event ID.
    pub fn new() -> Self { Self(ulid::Ulid::new().to_string()) }
}

impl Default for EventId {
    fn default() -> Self { Self::new() }
}

impl std::fmt::Display for EventId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(&self.0) }
}

// ---------------------------------------------------------------------------
// EventBase
// ---------------------------------------------------------------------------

/// Common base fields for every kernel event.
///
/// Carries identity, timing, and scope metadata that applies uniformly
/// to all event kinds.
#[derive(Debug, Clone, Serialize)]
pub struct EventBase {
    /// Unique event identifier.
    pub id:         EventId,
    /// When the event was created.
    pub timestamp:  Timestamp,
    /// Primary agent scope (for routing and observability).
    pub agent_id:   Option<AgentId>,
    /// Session scope (when the event is session-bound).
    pub session_id: Option<SessionId>,
}

impl EventBase {
    /// Create a new event base with the given agent and session context.
    pub fn new(agent_id: Option<AgentId>, session_id: Option<SessionId>) -> Self {
        Self {
            id: EventId::new(),
            timestamp: Timestamp::now(),
            agent_id,
            session_id,
        }
    }
}

// ---------------------------------------------------------------------------
// EventPriority
// ---------------------------------------------------------------------------

/// Auto-inferred priority tier for event queue ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, strum::Display)]
pub enum EventPriority {
    /// Signal, Shutdown — processed first.
    Critical = 0,
    /// TurnCompleted, ChildCompleted, Deliver, Syscall — processed second.
    Normal = 1,
    /// UserMessage, SpawnAgent — processed last.
    Low = 2,
}

// ---------------------------------------------------------------------------
// Syscall — process handle requests routed through the event queue
// ---------------------------------------------------------------------------

/// Syscall variants — all interactions that a `ProcessHandle` routes through
/// the kernel event queue. Each variant carries identity fields plus a oneshot
/// reply channel for the kernel event loop to respond on.
#[derive(derive_more::Debug, Serialize, strum::IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum Syscall {
    // -- Process queries --
    /// Query the status of a target agent process.
    QueryStatus {
        target:   AgentId,
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx: oneshot::Sender<crate::error::Result<ProcessInfo>>,
    },

    /// Query children of a parent agent process.
    QueryChildren {
        parent:   AgentId,
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx: oneshot::Sender<Vec<ProcessInfo>>,
    },

    // -- Memory --
    /// Store a value in the agent's private memory namespace.
    MemStore {
        agent_id:   AgentId,
        session_id: SessionId,
        principal:  Principal,
        key:        String,
        value:      serde_json::Value,
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx:   oneshot::Sender<crate::error::Result<()>>,
    },

    /// Recall a value from the agent's private memory namespace.
    MemRecall {
        agent_id: AgentId,
        key:      String,
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx: oneshot::Sender<crate::error::Result<Option<serde_json::Value>>>,
    },

    /// Store a value in a shared (scoped) memory namespace.
    SharedStore {
        agent_id:  AgentId,
        principal: Principal,
        scope:     KvScope,
        key:       String,
        value:     serde_json::Value,
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx:  oneshot::Sender<crate::error::Result<()>>,
    },

    /// Recall a value from a shared (scoped) memory namespace.
    SharedRecall {
        agent_id:  AgentId,
        principal: Principal,
        scope:     KvScope,
        key:       String,
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx:  oneshot::Sender<crate::error::Result<Option<serde_json::Value>>>,
    },

    // -- Pipe --
    /// Create an anonymous pipe between two agents.
    CreatePipe {
        owner:    AgentId,
        target:   AgentId,
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx: oneshot::Sender<crate::error::Result<(PipeWriter, PipeReader)>>,
    },

    /// Create a named pipe.
    CreateNamedPipe {
        owner:    AgentId,
        name:     String,
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx: oneshot::Sender<crate::error::Result<(PipeWriter, PipeReader)>>,
    },

    /// Connect to a named pipe as a reader.
    ConnectPipe {
        connector: AgentId,
        name:      String,
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx:  oneshot::Sender<crate::error::Result<PipeReader>>,
    },

    // -- Guard --
    /// Check whether a tool requires approval before execution.
    RequiresApproval {
        tool_name: String,
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx:  oneshot::Sender<bool>,
    },

    /// Request approval for a tool execution.
    RequestApproval {
        agent_id:  AgentId,
        principal: Principal,
        tool_name: String,
        summary:   String,
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx:  oneshot::Sender<crate::error::Result<bool>>,
    },

    /// Check guard verdict for a batch of tool calls before execution.
    CheckGuardBatch {
        agent_id:   AgentId,
        session_id: SessionId,
        #[debug("{} checks", checks.len())]
        checks:     Vec<(String, serde_json::Value)>,
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx:   oneshot::Sender<Vec<crate::guard::Verdict>>,
    },

    // -- Context queries (used by agent_turn) --
    /// Get the manifest for an agent process.
    GetManifest {
        agent_id: AgentId,
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx: oneshot::Sender<crate::error::Result<AgentManifest>>,
    },

    /// Get the tool registry, enriched with per-process tools (e.g.
    /// SyscallTool).
    GetToolRegistry {
        agent_id: AgentId,
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx: oneshot::Sender<Arc<ToolRegistry>>,
    },

    /// Resolve an [`LlmDriver`](crate::llm::LlmDriver) + model for a
    /// specific agent via `DriverRegistry::resolve()`.
    ResolveDriver {
        agent_id: AgentId,
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx: oneshot::Sender<crate::error::Result<(crate::llm::LlmDriverRef, String)>>,
    },

    // -- Event publishing --
    /// Publish an event to the kernel event bus.
    PublishEvent {
        agent_id:   AgentId,
        event_type: String,
        payload:    serde_json::Value,
    },

    /// Record a tool call for audit trail (fire-and-forget, no reply channel).
    RecordToolCall {
        agent_id:    AgentId,
        tool_name:   String,
        args:        serde_json::Value,
        result:      serde_json::Value,
        success:     bool,
        duration_ms: u64,
    },
}

impl Syscall {
    /// Extract the primary `AgentId` from this syscall variant.
    ///
    /// For agent-less syscalls (`RequiresApproval`, `GetToolRegistry`),
    /// returns a fixed nil UUID-based `AgentId` so they always hash to the
    /// same shard.
    pub fn agent_id(&self) -> AgentId {
        match self {
            Self::QueryStatus { target, .. } => *target,
            Self::QueryChildren { parent, .. } => *parent,
            Self::MemStore { agent_id, .. }
            | Self::MemRecall { agent_id, .. }
            | Self::SharedStore { agent_id, .. }
            | Self::SharedRecall { agent_id, .. }
            | Self::PublishEvent { agent_id, .. }
            | Self::RecordToolCall { agent_id, .. }
            | Self::RequestApproval { agent_id, .. }
            | Self::CheckGuardBatch { agent_id, .. }
            | Self::GetManifest { agent_id, .. }
            | Self::ResolveDriver { agent_id, .. }
            | Self::GetToolRegistry { agent_id, .. } => *agent_id,
            Self::CreatePipe { owner, .. } | Self::CreateNamedPipe { owner, .. } => *owner,
            Self::ConnectPipe { connector, .. } => *connector,
            // Agent-less syscalls — route to a fixed nil shard.
            Self::RequiresApproval { .. } => AgentId(uuid::Uuid::nil()),
        }
    }

    /// Extract a stable event type label for observability.
    pub fn event_type(&self) -> String {
        let syscall_type: &'static str = self.into();
        format!("syscall:{syscall_type}")
    }

    /// Agent id for observability; hides the nil sentinel used internally.
    pub fn observable_agent_id(&self) -> Option<AgentId> {
        let agent_id = self.agent_id();
        if agent_id.0.is_nil() {
            None
        } else {
            Some(agent_id)
        }
    }

    /// Human-readable summary for observability.
    pub fn summary(&self) -> String {
        let syscall_type: &'static str = self.into();
        format!("handle syscall {syscall_type}")
    }
}

// ---------------------------------------------------------------------------
// EventKind — variant discriminator
// ---------------------------------------------------------------------------

/// Discriminator for kernel event variants.
///
/// Each variant carries only its unique business fields. Common metadata
/// (event id, timestamp, agent id, session id) lives in [`EventBase`].
#[derive(derive_more::Debug, Serialize, strum::IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum EventKind {
    // === Input: from external sources ===
    /// A new user message from a channel adapter (via IngressPipeline).
    #[debug("UserMessage(session={})", _0.session_id)]
    UserMessage(InboundMessage),

    // === Process control ===
    /// Request to spawn a new agent process.
    ///
    /// The kernel generates a fresh `agent:{id}` session for the new process.
    /// Callers no longer specify a session — this ensures each process gets
    /// context isolation (subagent session isolation).
    SpawnAgent {
        #[debug("{}", manifest.name)]
        manifest:  AgentManifest,
        input:     String,
        principal: Principal,
        parent_id: Option<AgentId>,
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx:  oneshot::Sender<crate::error::Result<AgentId>>,
    },

    /// Send a control signal to an agent process.
    /// The target agent is in [`EventBase::agent_id`].
    SendSignal { signal: Signal },

    // === Internal callbacks: from async task completion ===
    /// An LLM turn completed (success or failure).
    /// Agent and session are in [`EventBase`].
    TurnCompleted {
        #[debug("{}", if result.is_ok() { "Ok(..)" } else { "Err(..)" })]
        #[serde(skip_serializing)]
        result:      Result<AgentTurnResult, String>,
        in_reply_to: MessageId,
        user:        UserId,
    },

    /// A child agent process completed.
    /// The parent agent is in [`EventBase::agent_id`].
    ChildCompleted {
        child_id: AgentId,
        result:   AgentResult,
    },

    // === Output ===
    /// Deliver an outbound envelope to egress.
    #[debug("Deliver(session={})", _0.session_id)]
    Deliver(OutboundEnvelope),

    // === Syscall: ProcessHandle → kernel event loop ===
    /// A syscall from a ProcessHandle. All handle interactions go through
    /// here so that the kernel event loop is the single owner of mutable
    /// state.
    Syscall(Syscall),

    // === System ===
    /// Graceful shutdown request.
    Shutdown,
}

impl EventKind {
    /// Determine the priority tier for this event kind.
    ///
    /// Priority is auto-inferred from the variant — callers never
    /// specify it manually.
    pub fn priority(&self) -> EventPriority {
        match self {
            Self::SendSignal { .. } | Self::Shutdown => EventPriority::Critical,
            Self::TurnCompleted { .. }
            | Self::ChildCompleted { .. }
            | Self::Deliver(_)
            | Self::Syscall(_) => EventPriority::Normal,
            Self::UserMessage(_) | Self::SpawnAgent { .. } => EventPriority::Low,
        }
    }

    /// Stable event type label for observability.
    pub fn event_type(&self) -> String {
        match self {
            Self::Syscall(syscall) => syscall.event_type(),
            _ => {
                let kind: &'static str = self.into();
                kind.to_string()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// KernelEvent — unified event struct
// ---------------------------------------------------------------------------

/// Unified event type for all kernel interactions.
///
/// Every interaction with the kernel — user messages, process control,
/// internal callbacks, output delivery — is represented as a `KernelEvent`
/// and processed by the single `Kernel::run()` event loop.
///
/// `base` carries common metadata (id, timestamp, agent scope, session scope).
/// `kind` carries the variant-specific payload.
#[derive(Serialize)]
pub struct KernelEvent {
    /// Common metadata for this event.
    pub base: EventBase,
    /// Variant-specific payload.
    pub kind: EventKind,
}

// -- Named constructors ---------------------------------------------------

impl KernelEvent {
    /// Create a `UserMessage` event.
    pub fn user_message(msg: InboundMessage) -> Self {
        let session_id = Some(msg.session_id.clone());
        Self {
            base: EventBase::new(None, session_id),
            kind: EventKind::UserMessage(msg),
        }
    }

    /// Create a `SpawnAgent` event.
    pub fn spawn_agent(
        manifest: AgentManifest,
        input: String,
        principal: Principal,
        parent_id: Option<AgentId>,
        reply_tx: oneshot::Sender<crate::error::Result<AgentId>>,
    ) -> Self {
        Self {
            base: EventBase::new(None, None),
            kind: EventKind::SpawnAgent {
                manifest,
                input,
                principal,
                parent_id,
                reply_tx,
            },
        }
    }

    /// Create a `SendSignal` event.
    pub fn send_signal(target: AgentId, signal: Signal) -> Self {
        Self {
            base: EventBase::new(Some(target), None),
            kind: EventKind::SendSignal { signal },
        }
    }

    /// Create a `TurnCompleted` event.
    pub fn turn_completed(
        agent_id: AgentId,
        session_id: SessionId,
        result: Result<AgentTurnResult, String>,
        in_reply_to: MessageId,
        user: UserId,
    ) -> Self {
        Self {
            base: EventBase::new(Some(agent_id), Some(session_id)),
            kind: EventKind::TurnCompleted {
                result,
                in_reply_to,
                user,
            },
        }
    }

    /// Create a `ChildCompleted` event.
    pub fn child_completed(parent_id: AgentId, child_id: AgentId, result: AgentResult) -> Self {
        Self {
            base: EventBase::new(Some(parent_id), None),
            kind: EventKind::ChildCompleted { child_id, result },
        }
    }

    /// Create a `Deliver` event.
    pub fn deliver(envelope: OutboundEnvelope) -> Self {
        let session_id = Some(envelope.session_id.clone());
        Self {
            base: EventBase::new(None, session_id),
            kind: EventKind::Deliver(envelope),
        }
    }

    /// Create a `Syscall` event.
    pub fn syscall(syscall: Syscall) -> Self {
        let agent_id = Some(syscall.agent_id());
        Self {
            base: EventBase::new(agent_id, None),
            kind: EventKind::Syscall(syscall),
        }
    }

    /// Create a `Shutdown` event.
    pub fn shutdown() -> Self {
        Self {
            base: EventBase::new(None, None),
            kind: EventKind::Shutdown,
        }
    }
}

// -- Accessor / observability methods --------------------------------------

impl KernelEvent {
    /// The primary agent scope for this event.
    pub fn agent_id(&self) -> Option<AgentId> { self.base.agent_id }

    /// The session scope for this event.
    pub fn session_id(&self) -> Option<&SessionId> { self.base.session_id.as_ref() }

    /// The priority tier for this event.
    pub fn priority(&self) -> EventPriority { self.kind.priority() }

    /// Stable event type label for observability.
    pub fn event_type(&self) -> String { self.kind.event_type() }

    /// Human-readable summary for observability.
    pub fn summary(&self) -> String {
        match &self.kind {
            EventKind::UserMessage(msg) => {
                format!("user message queued for session {}", msg.session_id)
            }
            EventKind::SpawnAgent { manifest, .. } => format!("spawn agent {}", manifest.name),
            EventKind::SendSignal { signal } => match self.base.agent_id {
                Some(target) => format!("send {signal:?} to {target}"),
                None => format!("send {signal:?}"),
            },
            EventKind::TurnCompleted { result, .. } => {
                let status = if result.is_ok() {
                    "completed"
                } else {
                    "failed"
                };
                match self.base.agent_id {
                    Some(agent_id) => format!("turn {status} for {agent_id}"),
                    None => format!("turn {status}"),
                }
            }
            EventKind::ChildCompleted { child_id, .. } => match self.base.agent_id {
                Some(parent_id) => {
                    format!("child {child_id} completed for parent {parent_id}")
                }
                None => format!("child {child_id} completed"),
            },
            EventKind::Deliver(envelope) => {
                format!(
                    "deliver outbound message for session {}",
                    envelope.session_id
                )
            }
            EventKind::Syscall(syscall) => syscall.summary(),
            EventKind::Shutdown => "shutdown requested".to_string(),
        }
    }

    /// Common observability fields derived from the event.
    pub fn common_fields(&self) -> KernelEventCommonFields {
        // For syscalls, hide the nil sentinel in observability.
        let observable_agent_id = match &self.kind {
            EventKind::Syscall(syscall) => syscall.observable_agent_id(),
            _ => self.base.agent_id,
        };

        KernelEventCommonFields {
            id:         self.base.id.clone(),
            timestamp:  self.base.timestamp,
            event_type: self.kind.event_type(),
            priority:   match self.kind.priority() {
                EventPriority::Critical => "critical".to_string(),
                EventPriority::Normal => "normal".to_string(),
                EventPriority::Low => "low".to_string(),
            },
            agent_id:   observable_agent_id.map(|id| id.to_string()),
            summary:    self.summary(),
        }
    }
}

/// Allow `&KernelEvent` → `&'static str` for metrics labels.
impl<'a> From<&'a KernelEvent> for &'static str {
    fn from(event: &'a KernelEvent) -> Self { (&event.kind).into() }
}

impl std::fmt::Debug for KernelEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "KernelEvent({}, {:?})", self.base.id, self.kind)
    }
}

// ---------------------------------------------------------------------------
// KernelEventCommonFields — stable observability contract
// ---------------------------------------------------------------------------

/// Stable common fields exposed for any observed kernel event.
#[derive(Debug, Clone, Serialize)]
pub struct KernelEventCommonFields {
    pub id:         EventId,
    pub timestamp:  Timestamp,
    pub event_type: String,
    pub priority:   String,
    pub agent_id:   Option<String>,
    pub summary:    String,
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::{
        channel::types::{ChannelType, MessageContent},
        io::types::{ChannelSource, MessageId},
        process::principal::UserId,
    };

    fn test_inbound(text: &str) -> InboundMessage {
        InboundMessage {
            id:              MessageId::new(),
            source:          ChannelSource {
                channel_type:        ChannelType::Internal,
                platform_message_id: None,
                platform_user_id:    "test".to_string(),
                platform_chat_id:    None,
            },
            user:            UserId("u1".to_string()),
            session_id:      SessionId::new(),
            target_agent_id: None,
            target_agent:    None,
            content:         MessageContent::Text(text.to_string()),
            reply_context:   None,
            timestamp:       jiff::Timestamp::now(),
            metadata:        HashMap::new(),
        }
    }

    // -- EventId tests ------------------------------------------------------

    #[test]
    fn event_id_is_unique() {
        let a = EventId::new();
        let b = EventId::new();
        assert_ne!(a, b);
    }

    // -- KernelEvent::agent_id() tests --------------------------------------

    #[test]
    fn agent_id_for_user_message_is_none() {
        let event = KernelEvent::user_message(test_inbound("hello"));
        assert!(event.agent_id().is_none());
    }

    #[test]
    fn agent_id_for_spawn_agent_is_none() {
        let (tx, _rx) = oneshot::channel();
        let event = KernelEvent::spawn_agent(
            AgentManifest {
                name:               "test".to_string(),
                role:               None,
                description:        "test".to_string(),
                model:              None,
                system_prompt:      "test".to_string(),
                soul_prompt:        None,
                provider_hint:      None,
                max_iterations:     None,
                tools:              vec![],
                max_children:       None,
                max_context_tokens: None,
                priority:           Default::default(),
                metadata:           Default::default(),
                sandbox:            None,
            },
            "hello".to_string(),
            Principal::user("test"),
            None,
            tx,
        );
        assert!(event.agent_id().is_none());
    }

    #[test]
    fn agent_id_for_shutdown_is_none() {
        assert!(KernelEvent::shutdown().agent_id().is_none());
    }

    #[test]
    fn agent_id_for_send_signal_is_target() {
        let target = AgentId::new();
        let event = KernelEvent::send_signal(target, Signal::Interrupt);
        assert_eq!(event.agent_id(), Some(target));
    }

    #[test]
    fn agent_id_for_turn_completed_is_agent_id() {
        let agent_id = AgentId::new();
        let event = KernelEvent::turn_completed(
            agent_id,
            SessionId::new(),
            Ok(crate::agent_turn::AgentTurnResult {
                text:       "done".to_string(),
                iterations: 1,
                tool_calls: 0,
                model:      "test".to_string(),
                trace:      crate::agent_turn::TurnTrace {
                    duration_ms:      0,
                    model:            "test".to_string(),
                    input_text:       None,
                    iterations:       vec![],
                    final_text_len:   4,
                    total_tool_calls: 0,
                    success:          true,
                    error:            None,
                },
            }),
            MessageId::new(),
            UserId("u1".to_string()),
        );
        assert_eq!(event.agent_id(), Some(agent_id));
    }

    #[test]
    fn agent_id_for_child_completed_is_parent_id() {
        let parent_id = AgentId::new();
        let event = KernelEvent::child_completed(
            parent_id,
            AgentId::new(),
            crate::process::AgentResult {
                output:     "done".to_string(),
                iterations: 1,
                tool_calls: 0,
            },
        );
        assert_eq!(event.agent_id(), Some(parent_id));
    }

    #[test]
    fn agent_id_for_deliver_is_none() {
        let event = KernelEvent::deliver(crate::io::types::OutboundEnvelope {
            id:          MessageId::new(),
            in_reply_to: MessageId::new(),
            user:        UserId("u1".to_string()),
            session_id:  SessionId::new(),
            routing:     crate::io::types::OutboundRouting::BroadcastAll,
            payload:     crate::io::types::OutboundPayload::Reply {
                content:     crate::channel::types::MessageContent::Text("reply".to_string()),
                attachments: vec![],
            },
            timestamp:   jiff::Timestamp::now(),
        });
        assert!(event.agent_id().is_none());
    }

    // -- common_fields tests ------------------------------------------------

    #[test]
    fn common_fields_include_event_id_and_timestamp() {
        let agent_id = AgentId::new();
        let event = KernelEvent::send_signal(agent_id, Signal::Pause);

        let fields = event.common_fields();

        assert_eq!(fields.id, event.base.id);
        assert_eq!(fields.event_type, "send_signal");
        assert_eq!(fields.priority, "critical");
        assert_eq!(fields.agent_id, Some(agent_id.to_string()));
        assert!(fields.summary.contains("Pause"));
    }

    #[test]
    fn event_base_has_unique_id_and_timestamp() {
        let e1 = KernelEvent::shutdown();
        let e2 = KernelEvent::shutdown();
        assert_ne!(e1.base.id, e2.base.id);
    }

    // -- Syscall::agent_id() tests ------------------------------------------

    #[test]
    fn syscall_agent_id_for_query_status() {
        let target = AgentId::new();
        let (tx, _rx) = oneshot::channel();
        let syscall = Syscall::QueryStatus {
            target,
            reply_tx: tx,
        };
        assert_eq!(syscall.agent_id(), target);
    }

    #[test]
    fn syscall_agent_id_for_mem_store() {
        let agent_id = AgentId::new();
        let (tx, _rx) = oneshot::channel();
        let syscall = Syscall::MemStore {
            agent_id,
            session_id: SessionId::new(),
            principal: Principal::user("test"),
            key: "k".to_string(),
            value: serde_json::Value::Null,
            reply_tx: tx,
        };
        assert_eq!(syscall.agent_id(), agent_id);
    }

    #[test]
    fn syscall_agent_id_for_requires_approval_is_nil() {
        let (tx, _rx) = oneshot::channel();
        let syscall = Syscall::RequiresApproval {
            tool_name: "test".to_string(),
            reply_tx:  tx,
        };
        assert_eq!(syscall.agent_id(), AgentId(uuid::Uuid::nil()));
    }

    #[test]
    fn syscall_agent_id_for_get_tool_registry() {
        let agent_id = AgentId::new();
        let (tx, _rx) = oneshot::channel();
        let syscall = Syscall::GetToolRegistry {
            agent_id,
            reply_tx: tx,
        };
        assert_eq!(syscall.agent_id(), agent_id);
    }

    #[test]
    fn syscall_agent_id_for_create_pipe() {
        let owner = AgentId::new();
        let target = AgentId::new();
        let (tx, _rx) = oneshot::channel();
        let syscall = Syscall::CreatePipe {
            owner,
            target,
            reply_tx: tx,
        };
        assert_eq!(syscall.agent_id(), owner);
    }

    #[test]
    fn syscall_agent_id_for_connect_pipe() {
        let connector = AgentId::new();
        let (tx, _rx) = oneshot::channel();
        let syscall = Syscall::ConnectPipe {
            connector,
            name: "test".to_string(),
            reply_tx: tx,
        };
        assert_eq!(syscall.agent_id(), connector);
    }
}
