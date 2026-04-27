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
use serde::Serialize;
use tokio::sync::oneshot;

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
        tags:     Vec<String>,
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

    /// List all scheduled jobs across sessions.
    ListJobs {
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx: oneshot::Sender<crate::error::Result<Vec<crate::schedule::JobEntry>>>,
    },

    /// List every scheduled job across all sessions — admin-only surface.
    ///
    /// Semantically distinct from [`Syscall::ListJobs`]: this variant exists
    /// so the backend admin route has an unambiguous, auth-gated entry point
    /// that cannot be reused by unprivileged session tools. The kernel does
    /// no auth check itself — the HTTP layer is responsible for gating.
    ListAllJobs {
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx: oneshot::Sender<crate::error::Result<Vec<crate::schedule::JobEntry>>>,
    },

    /// Immediately fire a scheduled job without advancing its `next_at`.
    ///
    /// The job is cloned from the wheel (not removed), inserted into the
    /// in-flight ledger with the standard lease, and dispatched through the
    /// same `ScheduledTask` path that [`JobWheel::drain_expired`] uses. The
    /// wheel's original schedule is untouched — recurring jobs still fire at
    /// their next regular `next_at`.
    ///
    /// Reply shape mirrors [`crate::schedule::TriggerOutcome`] so callers can
    /// distinguish a fresh dispatch from a deduplicated no-op (job already
    /// in-flight) and from a missing job. `NotFound` is the only error; the
    /// other two outcomes are both `Ok`.
    ///
    /// [`JobWheel::drain_expired`]: crate::schedule::JobWheel::drain_expired
    TriggerJob {
        job_id:   crate::schedule::JobId,
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx: oneshot::Sender<crate::error::Result<TriggerJobReply>>,
    },

    // -- Task Report & Subscription --
    /// Register a notification subscription for the calling session.
    Subscribe {
        match_tags: Vec<String>,
        on_receive: crate::notification::NotifyAction,
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx:   oneshot::Sender<crate::error::Result<uuid::Uuid>>,
    },

    /// Remove a subscription by ID.
    Unsubscribe {
        subscription_id: uuid::Uuid,
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx:        oneshot::Sender<crate::error::Result<bool>>,
    },

    /// Publish a TaskReport and broadcast notification to matching subscribers.
    PublishTaskReport {
        report:   crate::task_report::TaskReport,
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx: oneshot::Sender<crate::error::Result<()>>,
    },
}

/// Reply for [`Syscall::TriggerJob`] distinguishing a fresh dispatch from a
/// deduplicated no-op. Mirrors the success arms of
/// [`crate::schedule::TriggerOutcome`] — the `NotFound` case is surfaced as
/// the `Err` half of `Result<TriggerJobReply, KernelError>`.
///
/// Each arm carries the wheel's [`JobEntry`] so
/// HTTP callers can shape a response view without a follow-up `ListAllJobs`
/// query. That second lookup used to race against `complete_in_flight` on
/// `Trigger::Once` jobs — a caller would see the syscall succeed and then
/// get a 404 when the job had already finished and been removed from the
/// wheel. Threading the entry through the reply eliminates the race.
#[derive(Debug, Clone)]
pub enum TriggerJobReply {
    /// The job was cloned into the in-flight ledger and a `ScheduledTask`
    /// event has been published. Carries the wheel's job snapshot taken at
    /// dispatch time.
    Fired(crate::schedule::JobEntry),
    /// A prior trigger is still executing. No new dispatch happened; the
    /// caller can treat this as a successful idempotent operation and the
    /// HTTP layer maps it to a `triggered: false` discriminator. Carries
    /// the wheel's current entry for the job.
    AlreadyInFlight(crate::schedule::JobEntry),
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
    #[debug("UserMessage(session={:?})", _0.session_key_opt())]
    UserMessage(InboundMessage),

    /// A group-chat message where the bot was **not** directly mentioned.
    ///
    /// Handled separately from `UserMessage`: the kernel records the message
    /// to the session tape, runs a lightweight LLM judgment to decide whether
    /// to reply, and only promotes to a full `UserMessage` turn on approval.
    #[debug("GroupMessage(session={:?})", _0.session_key_opt())]
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
        principal:           Principal<crate::identity::Lookup>,
        parent_id:           Option<SessionKey>,
        desired_session_key: Option<SessionKey>,
        #[debug(skip)]
        #[serde(skip_serializing)]
        reply_tx:            oneshot::Sender<crate::error::Result<SessionKey>>,
        /// Pre-created channel for streaming `AgentEvent`s back to the
        /// spawner.  Passed through so that `handle_spawn_agent` can store
        /// it on the `Session` atomically at creation time, closing the
        /// race window where a fast-completing child turn could miss a
        /// late-set `result_tx`.
        #[debug(skip)]
        #[serde(skip_serializing)]
        child_result_tx:     Option<tokio::sync::mpsc::Sender<crate::io::AgentEvent>>,
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
        result:          Result<AgentTurnResult, crate::error::OutboundError>,
        in_reply_to:     MessageId,
        user:            UserId,
        /// Origin endpoint from the inbound message for session-scoped routing.
        origin_endpoint: Option<crate::io::Endpoint>,
        /// Channel type of the inbound message that triggered this turn.
        /// `None` for abnormal exits where source info is unavailable.
        source_channel:  Option<crate::channel::types::ChannelType>,
        /// Whether this turn was interrupted by a user /stop command.
        interrupted:     bool,
    },

    /// A child session completed its work.
    /// The parent session is in [`EventBase::session_key`].
    ChildSessionDone {
        child_id:          SessionKey,
        result:            AgentRunLoopResult,
        /// When `true`, the child's result should NOT be appended to the
        /// parent's tape as a system message.  Set for fold-branch children
        /// whose output is already returned inline as a `ToolResult`.
        skip_tape_persist: bool,
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
    MitaDirective { instruction: String },

    /// Periodic Mita heartbeat — ensures the Mita session exists and
    /// delivers a heartbeat message to it.
    MitaHeartbeat,

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
            | Self::MitaHeartbeat
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
        let base_key = msg.session_key_opt().copied().unwrap_or_default();
        Self {
            base: EventBase::from(base_key),
            kind: KernelEvent::UserMessage(msg),
        }
    }

    /// Create a `GroupMessage` event.
    pub fn group_message(msg: InboundMessage) -> Self {
        let base_key = msg.session_key_opt().copied().unwrap_or_default();
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
        principal: Principal<crate::identity::Lookup>,
        parent_id: Option<SessionKey>,
        desired_session_key: Option<SessionKey>,
        reply_tx: oneshot::Sender<crate::error::Result<SessionKey>>,
        child_result_tx: Option<tokio::sync::mpsc::Sender<crate::io::AgentEvent>>,
    ) -> Self {
        Self {
            base: EventBase::from(desired_session_key.unwrap_or_default()),
            kind: KernelEvent::CreateSession {
                manifest,
                input,
                principal,
                parent_id,
                desired_session_key,
                reply_tx,
                child_result_tx,
            },
        }
    }

    /// Backwards-compatible alias for `create_session`.
    pub fn spawn_agent(
        manifest: AgentManifest,
        input: String,
        principal: Principal<crate::identity::Lookup>,
        parent_id: Option<SessionKey>,
        desired_session_key: Option<SessionKey>,
        reply_tx: oneshot::Sender<crate::error::Result<SessionKey>>,
        child_result_tx: Option<tokio::sync::mpsc::Sender<crate::io::AgentEvent>>,
    ) -> Self {
        Self::create_session(
            manifest,
            input,
            principal,
            parent_id,
            desired_session_key,
            reply_tx,
            child_result_tx,
        )
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
        result: Result<AgentTurnResult, crate::error::OutboundError>,
        in_reply_to: MessageId,
        user: UserId,
        origin_endpoint: Option<crate::io::Endpoint>,
        source_channel: Option<crate::channel::types::ChannelType>,
        interrupted: bool,
    ) -> Self {
        Self {
            base: EventBase::from(session_key),
            kind: KernelEvent::TurnCompleted {
                result,
                in_reply_to,
                user,
                origin_endpoint,
                source_channel,
                interrupted,
            },
        }
    }

    /// Create a `ChildSessionDone` event.
    pub fn child_session_done(
        parent_id: SessionKey,
        child_id: SessionKey,
        result: AgentRunLoopResult,
        skip_tape_persist: bool,
    ) -> Self {
        Self {
            base: EventBase::from(parent_id),
            kind: KernelEvent::ChildSessionDone {
                child_id,
                result,
                skip_tape_persist,
            },
        }
    }

    /// Backwards-compatible alias for `child_session_done`.
    pub fn child_completed(
        parent_id: SessionKey,
        child_id: SessionKey,
        result: AgentRunLoopResult,
    ) -> Self {
        Self::child_session_done(parent_id, child_id, result, false)
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

    /// Create a `MitaHeartbeat` event.
    pub fn mita_heartbeat() -> Self {
        Self {
            base: EventBase::from(SessionKey::new()),
            kind: KernelEvent::MitaHeartbeat,
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
            KernelEvent::UserMessage(msg) => match msg.session_key_opt() {
                Some(key) => format!("user message queued for session {key}"),
                None => "user message queued (no session yet)".to_string(),
            },
            KernelEvent::GroupMessage(msg) => match msg.session_key_opt() {
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
            KernelEvent::MitaHeartbeat => "periodic mita heartbeat".to_string(),
            KernelEvent::IdleCheck => "periodic idle check".to_string(),
            KernelEvent::Shutdown => "shutdown requested".to_string(),
        }
    }

    /// Returns the session key used for shard routing, or `None` for global
    /// events.
    ///
    /// - **Global** (returns `None`): `UserMessage`, `GroupMessage`,
    ///   `CreateSession`, `ScheduledTask`, `SendNotification`, `MitaHeartbeat`,
    ///   `IdleCheck`, `Shutdown`
    /// - **Sharded** (returns `Some`): `SendSignal`, `TurnCompleted`,
    ///   `ChildSessionDone`, `SessionCommand`, `MitaDirective`, `Deliver`
    pub fn shard_key(&self) -> Option<SessionKey> {
        match &self.kind {
            KernelEvent::SendSignal { .. }
            | KernelEvent::TurnCompleted { .. }
            | KernelEvent::ChildSessionDone { .. }
            | KernelEvent::SessionCommand(_)
            | KernelEvent::MitaDirective { .. }
            | KernelEvent::Deliver(_) => Some(self.base.session_key),
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
