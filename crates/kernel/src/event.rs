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

use derive_more::Debug;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::{
    agent::{AgentManifest, AgentTurnResult},
    identity::{Principal, UserId},
    io::{InboundMessage, MessageId, OutboundEnvelope, PipeReader, PipeWriter},
    kv::KvScope,
    schedule::JobEntry,
    session::{AgentRunLoopResult, SessionKey, Signal},
    tool::ToolRegistry,
};

base::define_id!(EventId);

/// Common base fields for every kernel event.
///
/// Carries identity, timing, and scope metadata that applies uniformly
/// to all event kinds.
#[derive(Debug, Clone, Serialize)]
pub struct EventBase {
    /// Unique event identifier.
    pub id:          EventId,
    /// When the event was created.
    pub timestamp:   Timestamp,
    /// Session scope
    pub session_key: SessionKey,
}

impl From<SessionKey> for EventBase {
    fn from(key: SessionKey) -> Self {
        Self {
            id:          EventId::new(),
            timestamp:   Timestamp::now(),
            session_key: key,
        }
    }
}

// ---------------------------------------------------------------------------
// EventPriority
// ---------------------------------------------------------------------------

/// Auto-inferred priority tier for event queue ordering.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, strum::Display, strum::EnumString,
)]
pub enum EventPriority {
    /// Signal, Shutdown — processed first.
    Critical = 0,
    /// TurnCompleted, ChildSessionDone, Deliver, SessionCommand — processed
    /// second.
    Normal = 1,
    /// UserMessage, CreateSession, IdleCheck — processed last.
    Low = 2,
}

// ---------------------------------------------------------------------------
// Syscall — process handle requests routed through the event queue
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct SyscallEnvelope {
    pub session_key: SessionKey,
    pub payload:     Syscall,
}

impl SyscallEnvelope {
    pub fn session_key(&self) -> SessionKey { self.session_key }
}

/// Syscall variants — all session-scoped operations routed through the kernel
/// event queue. Each variant carries identity fields plus a oneshot reply
/// channel for the kernel event loop to respond on.
#[derive(derive_more::Debug, Serialize, strum::IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum Syscall {
    // -- Memory --
    /// Store a value in the agent's private memory namespace.
    MemStore {
        session_key: SessionKey,
        principal:   Principal,
        key:         String,
        value:       serde_json::Value,
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx:    oneshot::Sender<crate::error::Result<()>>,
    },

    /// Recall a value from the agent's private memory namespace.
    MemRecall {
        key:      String,
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx: oneshot::Sender<crate::error::Result<Option<serde_json::Value>>>,
    },

    /// Store a value in a shared (scoped) memory namespace.
    SharedStore {
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
        target:   SessionKey,
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx: oneshot::Sender<crate::error::Result<(PipeWriter, PipeReader)>>,
    },

    /// Create a named pipe.
    CreateNamedPipe {
        name:     String,
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx: oneshot::Sender<crate::error::Result<(PipeWriter, PipeReader)>>,
    },

    /// Connect to a named pipe as a reader.
    ConnectPipe {
        name:     String,
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx: oneshot::Sender<crate::error::Result<PipeReader>>,
    },

    // -- Guard --
    /// Request approval for a tool execution.
    RequestApproval {
        principal: Principal,
        tool_name: String,
        summary:   String,
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx:  oneshot::Sender<crate::error::Result<bool>>,
    },

    // -- Context queries (used by agent_turn) --
    /// Get the tool registry, enriched with per-process tools (e.g.
    /// SyscallTool).
    GetToolRegistry {
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx: oneshot::Sender<Arc<ToolRegistry>>,
    },

    // -- Event publishing --
    /// Publish an event to the kernel event bus.
    PublishEvent {
        event_type: String,
        payload:    serde_json::Value,
    },

    // -- Scheduling --
    /// Register a new scheduled job.
    RegisterJob {
        trigger:  crate::schedule::Trigger,
        message:  String,
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx: oneshot::Sender<crate::error::Result<crate::schedule::JobId>>,
    },

    /// Remove a scheduled job by ID.
    RemoveJob {
        job_id:   crate::schedule::JobId,
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx: oneshot::Sender<crate::error::Result<()>>,
    },

    /// List all scheduled jobs for the current session.
    ListJobs {
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx: oneshot::Sender<crate::error::Result<Vec<crate::schedule::JobEntry>>>,
    },
}

impl Syscall {
    /// Extract a stable event type label for observability.
    pub fn event_type(&self) -> String {
        let syscall_type: &'static str = self.into();
        format!("syscall:{syscall_type}")
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
pub enum KernelEvent {
    // === Input: from external sources ===
    /// A new user message from a channel adapter (via IngressPipeline).
    #[debug("UserMessage(session={:?})", _0.session_key)]
    UserMessage(InboundMessage),

    /// A group-chat message where the bot was **not** directly mentioned.
    ///
    /// Handled separately from `UserMessage`: the kernel records the message
    /// to the session tape, runs a lightweight LLM judgment to decide whether
    /// to reply, and only promotes to a full `UserMessage` turn on approval.
    #[debug("GroupMessage(session={:?})", _0.session_key)]
    GroupMessage(InboundMessage),

    // === Session control ===
    /// Request to create (or reactivate) a session.
    ///
    /// The kernel generates a fresh session for the new runtime, or
    /// reactivates an existing suspended session.
    CreateSession {
        #[debug("{}", manifest.name)]
        manifest:            AgentManifest,
        input:               String,
        principal:           Principal,
        parent_id:           Option<SessionKey>,
        desired_session_key: Option<SessionKey>,
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx:            oneshot::Sender<crate::error::Result<SessionKey>>,
    },

    /// Send a control signal to a session.
    /// The target session is in [`EventBase::session_key`].
    SendSignal { signal: Signal },

    // === Internal callbacks: from async task completion ===
    /// An LLM turn completed (success or failure).
    /// Session key is in [`EventBase::session_key`].
    TurnCompleted {
        #[debug("{}", if result.is_ok() { "Ok(..)" } else { "Err(..)" })]
        #[serde(skip_serializing)]
        result:          Result<AgentTurnResult, String>,
        in_reply_to:     MessageId,
        user:            UserId,
        /// Origin endpoint from the inbound message for session-scoped routing.
        origin_endpoint: Option<crate::io::Endpoint>,
    },

    /// A child session completed its work.
    /// The parent session is in [`EventBase::session_key`].
    ChildSessionDone {
        child_id: SessionKey,
        result:   AgentRunLoopResult,
    },

    // === Output ===
    /// Deliver an outbound envelope to egress.
    #[debug("Deliver(session={})", _0.session_key)]
    Deliver(OutboundEnvelope),

    // === SessionCommand: SessionHandle → kernel event loop ===
    /// A command from a SessionHandle. All handle interactions go through
    /// here so that the kernel event loop is the single owner of mutable
    /// state.
    SessionCommand(SyscallEnvelope),

    // === Scheduled ===
    /// A scheduled task has fired. Unlike `UserMessage`, this is a
    /// system-initiated event and is not routed through the ingress pipeline.
    ScheduledTask { job: JobEntry },

    // === Notification ===
    /// Send a notification message to the notification channel.
    SendNotification { message: String },

    // === Mita ===
    /// Internal directive from Mita to a session's agent.
    ///
    /// Unlike `UserMessage`, this does NOT persist to the target session's
    /// tape. The instruction is injected as ephemeral context for one LLM
    /// turn only.
    MitaDirective {
        instruction: String,
    },

    // === System ===
    /// Periodic idle check — transitions Ready sessions to Suspended.
    IdleCheck,

    /// Graceful shutdown request.
    Shutdown,
}

impl KernelEvent {
    /// Determine the priority tier for this event kind.
    ///
    /// Priority is auto-inferred from the variant — callers never
    /// specify it manually.
    pub fn priority(&self) -> EventPriority {
        match self {
            Self::SendSignal { .. } | Self::Shutdown => EventPriority::Critical,
            Self::TurnCompleted { .. }
            | Self::ChildSessionDone { .. }
            | Self::Deliver(_)
            | Self::SessionCommand(_)
            | Self::SendNotification { .. } => EventPriority::Normal,
            Self::UserMessage(_)
            | Self::GroupMessage(_)
            | Self::CreateSession { .. }
            | Self::ScheduledTask { .. }
            | Self::MitaDirective { .. }
            | Self::IdleCheck => EventPriority::Low,
        }
    }

    /// Stable event type label for observability.
    pub fn event_type(&self) -> String {
        match self {
            Self::SessionCommand(envelope) => envelope.payload.event_type(),
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
#[derive(Serialize, Debug)]
pub struct KernelEventEnvelope {
    /// Common metadata for this event.
    pub base: EventBase,
    /// Variant-specific payload.
    pub kind: KernelEvent,
}

// -- Named constructors ---------------------------------------------------

impl KernelEventEnvelope {
    /// Create a `UserMessage` event.
    pub fn user_message(msg: InboundMessage) -> Self {
        let base_key = msg.session_key.clone().unwrap_or_else(SessionKey::new);
        Self {
            base: EventBase::from(base_key),
            kind: KernelEvent::UserMessage(msg),
        }
    }

    /// Create a `GroupMessage` event.
    pub fn group_message(msg: InboundMessage) -> Self {
        let base_key = msg.session_key.clone().unwrap_or_else(SessionKey::new);
        Self {
            base: EventBase::from(base_key),
            kind: KernelEvent::GroupMessage(msg),
        }
    }

    /// Create a `ScheduledTask` event.
    pub fn scheduled_task(job: JobEntry) -> Self {
        let session_key = job.session_key;
        Self {
            base: EventBase::from(session_key),
            kind: KernelEvent::ScheduledTask { job },
        }
    }

    /// Create a `CreateSession` event.
    pub fn create_session(
        manifest: AgentManifest,
        input: String,
        principal: Principal,
        parent_id: Option<SessionKey>,
        desired_session_key: Option<SessionKey>,
        reply_tx: oneshot::Sender<crate::error::Result<SessionKey>>,
    ) -> Self {
        Self {
            base: EventBase::from(desired_session_key.unwrap_or_else(SessionKey::new)),
            kind: KernelEvent::CreateSession {
                manifest,
                input,
                principal,
                parent_id,
                desired_session_key,
                reply_tx,
            },
        }
    }

    /// Backwards-compatible alias for `create_session`.
    pub fn spawn_agent(
        manifest: AgentManifest,
        input: String,
        principal: Principal,
        parent_id: Option<SessionKey>,
        desired_session_key: Option<SessionKey>,
        reply_tx: oneshot::Sender<crate::error::Result<SessionKey>>,
    ) -> Self {
        Self::create_session(manifest, input, principal, parent_id, desired_session_key, reply_tx)
    }

    /// Create a `SendSignal` event.
    pub fn send_signal(target: SessionKey, signal: Signal) -> Self {
        Self {
            base: EventBase::from(target),
            kind: KernelEvent::SendSignal { signal },
        }
    }

    /// Create a `TurnCompleted` event.
    pub fn turn_completed(
        session_key: SessionKey,
        result: Result<AgentTurnResult, String>,
        in_reply_to: MessageId,
        user: UserId,
        origin_endpoint: Option<crate::io::Endpoint>,
    ) -> Self {
        Self {
            base: EventBase::from(session_key),
            kind: KernelEvent::TurnCompleted {
                result,
                in_reply_to,
                user,
                origin_endpoint,
            },
        }
    }

    /// Create a `ChildSessionDone` event.
    pub fn child_session_done(
        parent_id: SessionKey,
        child_id: SessionKey,
        result: AgentRunLoopResult,
    ) -> Self {
        Self {
            base: EventBase::from(parent_id),
            kind: KernelEvent::ChildSessionDone { child_id, result },
        }
    }

    /// Backwards-compatible alias for `child_session_done`.
    pub fn child_completed(
        parent_id: SessionKey,
        child_id: SessionKey,
        result: AgentRunLoopResult,
    ) -> Self {
        Self::child_session_done(parent_id, child_id, result)
    }

    /// Create a `Deliver` event.
    pub fn deliver(envelope: OutboundEnvelope) -> Self {
        let session_key = envelope.session_key.clone();
        Self {
            base: EventBase::from(session_key),
            kind: KernelEvent::Deliver(envelope),
        }
    }

    /// Create a `SessionCommand` event.
    pub fn session_command(session_key: SessionKey, syscall: Syscall) -> Self {
        Self {
            base: EventBase::from(session_key),
            kind: KernelEvent::SessionCommand(SyscallEnvelope {
                session_key,
                payload: syscall,
            }),
        }
    }

    /// Backwards-compatible alias for `session_command`.
    pub fn syscall(session_key: SessionKey, syscall: Syscall) -> Self {
        Self::session_command(session_key, syscall)
    }

    /// Create a `MitaDirective` event.
    pub fn mita_directive(target: SessionKey, instruction: String) -> Self {
        Self {
            base: EventBase::from(target),
            kind: KernelEvent::MitaDirective { instruction },
        }
    }

    /// Create a `SendNotification` event.
    pub fn send_notification(message: String) -> Self {
        Self {
            base: EventBase::from(SessionKey::new()),
            kind: KernelEvent::SendNotification { message },
        }
    }

    /// Create an `IdleCheck` event.
    pub fn idle_check() -> Self {
        Self {
            base: EventBase::from(SessionKey::new()),
            kind: KernelEvent::IdleCheck,
        }
    }

    /// Create a `Shutdown` event.
    pub fn shutdown() -> Self {
        Self {
            base: EventBase::from(SessionKey::new()),
            kind: KernelEvent::Shutdown,
        }
    }
}

// -- Accessor / observability methods --------------------------------------

impl KernelEventEnvelope {
    /// The priority tier for this event.
    pub fn priority(&self) -> EventPriority { self.kind.priority() }

    /// Stable event type label for observability.
    pub fn event_type(&self) -> String { self.kind.event_type() }

    /// Human-readable summary for observability.
    pub fn summary(&self) -> String {
        match &self.kind {
            KernelEvent::UserMessage(msg) => match &msg.session_key {
                Some(key) => format!("user message queued for session {key}"),
                None => "user message queued (no session yet)".to_string(),
            },
            KernelEvent::GroupMessage(msg) => match &msg.session_key {
                Some(key) => format!("group message queued for session {key}"),
                None => "group message queued (no session yet)".to_string(),
            },
            KernelEvent::CreateSession { manifest, .. } => {
                format!("create session for {}", manifest.name)
            }
            KernelEvent::SendSignal { signal } => {
                format!("send {signal:?} to {}", self.base.session_key)
            }
            KernelEvent::TurnCompleted { result, .. } => {
                let status = if result.is_ok() {
                    "completed"
                } else {
                    "failed"
                };
                format!("turn {status} for {}", self.base.session_key)
            }
            KernelEvent::ChildSessionDone { child_id, .. } => {
                format!(
                    "child session {child_id} done for parent {}",
                    self.base.session_key
                )
            }
            KernelEvent::Deliver(envelope) => {
                format!(
                    "deliver outbound message for session {}",
                    envelope.session_key
                )
            }
            KernelEvent::ScheduledTask { job } => {
                format!(
                    "scheduled task {} fired for session {}",
                    job.id, job.session_key
                )
            }
            KernelEvent::SessionCommand(envelope) => envelope.payload.summary(),
            KernelEvent::SendNotification { message } => {
                let preview = if message.len() > 50 {
                    format!("{}...", &message[..50])
                } else {
                    message.clone()
                };
                format!("send notification: {preview}")
            }
            KernelEvent::MitaDirective { .. } => {
                format!("mita directive for session {}", self.base.session_key)
            }
            KernelEvent::IdleCheck => "periodic idle check".to_string(),
            KernelEvent::Shutdown => "shutdown requested".to_string(),
        }
    }

    /// Returns the session key used for shard routing, or `None` for global
    /// events.
    ///
    /// - **Global** (returns `None`): `UserMessage`, `GroupMessage`,
    ///   `CreateSession`, `Deliver`, `IdleCheck`, `Shutdown`
    /// - **Sharded** (returns `Some`): `SendSignal`, `TurnCompleted`,
    ///   `ChildSessionDone`, `SessionCommand`
    pub fn shard_key(&self) -> Option<SessionKey> {
        match &self.kind {
            KernelEvent::SendSignal { .. }
            | KernelEvent::TurnCompleted { .. }
            | KernelEvent::ChildSessionDone { .. }
            | KernelEvent::SessionCommand(_)
            | KernelEvent::MitaDirective { .. } => Some(self.base.session_key),
            _ => None,
        }
    }

    /// Common observability fields derived from the event.
    pub fn common_fields(&self) -> KernelEventCommonFields {
        KernelEventCommonFields {
            id:         self.base.id.clone(),
            timestamp:  self.base.timestamp,
            event_type: self.kind.event_type(),
            priority:   self.kind.priority().to_string(),
            session_id: Some(self.base.session_key.to_string()),
            summary:    self.summary(),
        }
    }
}

// ---------------------------------------------------------------------------
// KernelEventCommonFields — stable observability contract
// ---------------------------------------------------------------------------

/// Stable common fields exposed for any observed kernel event.
// TODO: optimize me
#[derive(Debug, Clone, Serialize)]
pub struct KernelEventCommonFields {
    pub id:         EventId,
    pub timestamp:  Timestamp,
    pub event_type: String,
    pub priority:   String,
    pub session_id: Option<String>,
    pub summary:    String,
}
