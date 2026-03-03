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

//! Unified kernel event — all kernel interactions as a single enum.
//!
//! Replaces the separate message types (InboundMessage via bus,
//! ProcessMessage via mailbox, OutboundEnvelope via outbound bus) with
//! a single unified event type processed by `Kernel::run()`.

use std::sync::Arc;

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
// EventPriority
// ---------------------------------------------------------------------------

/// Auto-inferred priority tier for event queue ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, strum::Display)]
pub enum EventPriority {
    /// Signal, Shutdown — processed first.
    Critical = 0,
    /// TurnCompleted, ChildCompleted, Deliver, Syscall — processed second.
    Normal = 1,
    /// UserMessage, SpawnAgent, Timer — processed last.
    Low = 2,
}

// ---------------------------------------------------------------------------
// Syscall — process handle requests routed through the event queue
// ---------------------------------------------------------------------------

/// Syscall variants — all interactions that a `ProcessHandle` routes through
/// the kernel event queue. Each variant carries identity fields plus a oneshot
/// reply channel for the kernel event loop to respond on.
#[derive(strum::IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum Syscall {
    // -- Process queries --
    /// Query the status of a target agent process.
    QueryStatus {
        target:   AgentId,
        reply_tx: oneshot::Sender<crate::error::Result<ProcessInfo>>,
    },

    /// Query children of a parent agent process.
    QueryChildren {
        parent:   AgentId,
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
        reply_tx:   oneshot::Sender<crate::error::Result<()>>,
    },

    /// Recall a value from the agent's private memory namespace.
    MemRecall {
        agent_id: AgentId,
        key:      String,
        reply_tx: oneshot::Sender<crate::error::Result<Option<serde_json::Value>>>,
    },

    /// Store a value in a shared (scoped) memory namespace.
    SharedStore {
        agent_id:  AgentId,
        principal: Principal,
        scope:     KvScope,
        key:       String,
        value:     serde_json::Value,
        reply_tx:  oneshot::Sender<crate::error::Result<()>>,
    },

    /// Recall a value from a shared (scoped) memory namespace.
    SharedRecall {
        agent_id:  AgentId,
        principal: Principal,
        scope:     KvScope,
        key:       String,
        reply_tx:  oneshot::Sender<crate::error::Result<Option<serde_json::Value>>>,
    },

    // -- Pipe --
    /// Create an anonymous pipe between two agents.
    CreatePipe {
        owner:    AgentId,
        target:   AgentId,
        reply_tx: oneshot::Sender<crate::error::Result<(PipeWriter, PipeReader)>>,
    },

    /// Create a named pipe.
    CreateNamedPipe {
        owner:    AgentId,
        name:     String,
        reply_tx: oneshot::Sender<crate::error::Result<(PipeWriter, PipeReader)>>,
    },

    /// Connect to a named pipe as a reader.
    ConnectPipe {
        connector: AgentId,
        name:      String,
        reply_tx:  oneshot::Sender<crate::error::Result<PipeReader>>,
    },

    // -- Guard --
    /// Check whether a tool requires approval before execution.
    RequiresApproval {
        tool_name: String,
        reply_tx:  oneshot::Sender<bool>,
    },

    /// Request approval for a tool execution.
    RequestApproval {
        agent_id:  AgentId,
        principal: Principal,
        tool_name: String,
        summary:   String,
        reply_tx:  oneshot::Sender<crate::error::Result<bool>>,
    },

    /// Check guard verdict for a batch of tool calls before execution.
    CheckGuardBatch {
        agent_id:   AgentId,
        session_id: SessionId,
        checks:     Vec<(String, serde_json::Value)>,
        reply_tx:   oneshot::Sender<Vec<crate::guard::Verdict>>,
    },

    // -- Context queries (used by agent_turn) --
    /// Get the manifest for an agent process.
    GetManifest {
        agent_id: AgentId,
        reply_tx: oneshot::Sender<crate::error::Result<AgentManifest>>,
    },

    /// Get the tool registry, enriched with per-process tools (e.g.
    /// SyscallTool).
    GetToolRegistry {
        agent_id: AgentId,
        reply_tx: oneshot::Sender<Arc<ToolRegistry>>,
    },

    /// Resolve an [`LlmDriver`](crate::llm::LlmDriver) + model for a
    /// specific agent via `DriverRegistry::resolve()`.
    ResolveDriver {
        agent_id: AgentId,
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
}

impl std::fmt::Debug for Syscall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::QueryStatus { target, .. } => {
                write!(f, "Syscall::QueryStatus(target={})", target)
            }
            Self::QueryChildren { parent, .. } => {
                write!(f, "Syscall::QueryChildren(parent={})", parent)
            }
            Self::MemStore { agent_id, key, .. } => {
                write!(f, "Syscall::MemStore(agent={}, key={})", agent_id, key)
            }
            Self::MemRecall { agent_id, key, .. } => {
                write!(f, "Syscall::MemRecall(agent={}, key={})", agent_id, key)
            }
            Self::SharedStore {
                agent_id,
                scope,
                key,
                ..
            } => {
                write!(
                    f,
                    "Syscall::SharedStore(agent={}, scope={:?}, key={})",
                    agent_id, scope, key
                )
            }
            Self::SharedRecall {
                agent_id,
                scope,
                key,
                ..
            } => {
                write!(
                    f,
                    "Syscall::SharedRecall(agent={}, scope={:?}, key={})",
                    agent_id, scope, key
                )
            }
            Self::CreatePipe { owner, target, .. } => {
                write!(f, "Syscall::CreatePipe(owner={}, target={})", owner, target)
            }
            Self::CreateNamedPipe { owner, name, .. } => {
                write!(
                    f,
                    "Syscall::CreateNamedPipe(owner={}, name={})",
                    owner, name
                )
            }
            Self::ConnectPipe {
                connector, name, ..
            } => {
                write!(
                    f,
                    "Syscall::ConnectPipe(connector={}, name={})",
                    connector, name
                )
            }
            Self::RequiresApproval { tool_name, .. } => {
                write!(f, "Syscall::RequiresApproval(tool={})", tool_name)
            }
            Self::RequestApproval {
                agent_id,
                tool_name,
                ..
            } => {
                write!(
                    f,
                    "Syscall::RequestApproval(agent={}, tool={})",
                    agent_id, tool_name
                )
            }
            Self::CheckGuardBatch {
                agent_id, checks, ..
            } => {
                write!(
                    f,
                    "Syscall::CheckGuardBatch(agent={}, checks={})",
                    agent_id,
                    checks.len()
                )
            }
            Self::GetManifest { agent_id, .. } => {
                write!(f, "Syscall::GetManifest(agent={})", agent_id)
            }
            Self::GetToolRegistry { agent_id, .. } => {
                write!(f, "Syscall::GetToolRegistry(agent={})", agent_id)
            }
            Self::ResolveDriver { agent_id, .. } => {
                write!(f, "Syscall::ResolveDriver(agent={})", agent_id)
            }
            Self::PublishEvent {
                agent_id,
                event_type,
                ..
            } => {
                write!(
                    f,
                    "Syscall::PublishEvent(agent={}, type={})",
                    agent_id, event_type
                )
            }
            Self::RecordToolCall {
                agent_id,
                tool_name,
                success,
                ..
            } => {
                write!(
                    f,
                    "Syscall::RecordToolCall(agent={}, tool={}, success={})",
                    agent_id, tool_name, success
                )
            }
        }
    }
}

// ---------------------------------------------------------------------------
// KernelEvent
// ---------------------------------------------------------------------------

/// Unified event type for all kernel interactions.
///
/// Every interaction with the kernel — user messages, process control,
/// internal callbacks, output delivery — is represented as a `KernelEvent`
/// and processed by the single `Kernel::run()` event loop.
#[derive(strum::IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum KernelEvent {
    // === Input: from external sources ===
    /// A new user message from a channel adapter (via IngressPipeline).
    UserMessage(InboundMessage),

    // === Process control ===
    /// Request to spawn a new agent process.
    ///
    /// The kernel generates a fresh `agent:{id}` session for the new process.
    /// Callers no longer specify a session — this ensures each process gets
    /// context isolation (subagent session isolation).
    SpawnAgent {
        manifest:  AgentManifest,
        input:     String,
        principal: Principal,
        parent_id: Option<AgentId>,
        reply_tx:  oneshot::Sender<crate::error::Result<AgentId>>,
    },

    /// Send a control signal to an agent process.
    SendSignal { target: AgentId, signal: Signal },

    // === Internal callbacks: from async task completion ===
    /// An LLM turn completed (success or failure).
    TurnCompleted {
        agent_id:    AgentId,
        session_id:  SessionId,
        result:      Result<AgentTurnResult, String>,
        in_reply_to: MessageId,
        user:        UserId,
    },

    /// A child agent process completed.
    ChildCompleted {
        parent_id: AgentId,
        child_id:  AgentId,
        result:    AgentResult,
    },

    // === Output ===
    /// Deliver an outbound envelope to egress.
    Deliver(OutboundEnvelope),

    // === Syscall: ProcessHandle → kernel event loop ===
    /// A syscall from a ProcessHandle. All handle interactions go through
    /// here so that the kernel event loop is the single owner of mutable
    /// state.
    Syscall(Syscall),

    // === System ===
    /// Timer event (reserved for future use).
    Timer {
        name:    String,
        payload: serde_json::Value,
    },

    /// Graceful shutdown request.
    Shutdown,
}

impl KernelEvent {
    /// Extract the primary `AgentId` associated with this event, if any.
    ///
    /// Returns `None` for global events that are not agent-scoped
    /// (UserMessage, SpawnAgent, Timer, Shutdown, Deliver).
    /// Returns `Some(agent_id)` for events that target a specific agent.
    pub fn agent_id(&self) -> Option<AgentId> {
        match self {
            Self::SendSignal { target, .. } => Some(*target),
            Self::TurnCompleted { agent_id, .. } => Some(*agent_id),
            Self::ChildCompleted { parent_id, .. } => Some(*parent_id),
            Self::Syscall(syscall) => Some(syscall.agent_id()),
            // Global events — no specific agent affinity.
            Self::UserMessage(_)
            | Self::SpawnAgent { .. }
            | Self::Timer { .. }
            | Self::Deliver(_)
            | Self::Shutdown => None,
        }
    }

    /// Determine the priority tier for this event.
    ///
    /// Priority is auto-inferred from the event variant — callers never
    /// specify it manually.
    pub fn priority(&self) -> EventPriority {
        match self {
            Self::SendSignal { .. } | Self::Shutdown => EventPriority::Critical,
            Self::TurnCompleted { .. }
            | Self::ChildCompleted { .. }
            | Self::Deliver(_)
            | Self::Syscall(_) => EventPriority::Normal,
            Self::UserMessage(_) | Self::SpawnAgent { .. } | Self::Timer { .. } => {
                EventPriority::Low
            }
        }
    }
}

impl std::fmt::Debug for KernelEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UserMessage(msg) => write!(f, "UserMessage(session={})", msg.session_id),
            Self::SpawnAgent { manifest, .. } => {
                write!(f, "SpawnAgent(name={})", manifest.name)
            }
            Self::SendSignal { target, signal } => {
                write!(f, "SendSignal(target={}, signal={:?})", target, signal)
            }
            Self::TurnCompleted { agent_id, .. } => {
                write!(f, "TurnCompleted(agent={})", agent_id)
            }
            Self::ChildCompleted {
                parent_id,
                child_id,
                ..
            } => {
                write!(
                    f,
                    "ChildCompleted(parent={}, child={})",
                    parent_id, child_id
                )
            }
            Self::Deliver(env) => write!(f, "Deliver(session={})", env.session_id),
            Self::Syscall(syscall) => write!(f, "{:?}", syscall),
            Self::Timer { name, .. } => write!(f, "Timer(name={})", name),
            Self::Shutdown => write!(f, "Shutdown"),
        }
    }
}

// ---------------------------------------------------------------------------
// PersistableEvent — serializable subset of KernelEvent for WAL persistence
// ---------------------------------------------------------------------------

/// Serializable subset of [`KernelEvent`] for WAL persistence.
///
/// Only events that must survive a crash are persisted:
/// - **UserMessage** — unprocessed user input would be lost.
/// - **SpawnAgent** — the caller is blocked on a reply; on recovery the kernel
///   can re-spawn (without the oneshot channel — the original caller is gone).
/// - **Timer** — scheduled work that should not be silently dropped.
///
/// Transient events (signals, syscalls, turn completions) are intentionally
/// excluded — they are either idempotent or the originating task has already
/// been lost on crash.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PersistableEvent {
    /// A user message from a channel adapter.
    UserMessage(InboundMessage),

    /// A spawn-agent request (without the oneshot reply channel).
    SpawnAgent {
        manifest:  AgentManifest,
        input:     String,
        principal: Principal,
        parent_id: Option<AgentId>,
    },

    /// A timer event.
    Timer {
        name:    String,
        payload: serde_json::Value,
    },
}

impl PersistableEvent {
    /// Try to convert a [`KernelEvent`] into a persistable form.
    ///
    /// Returns `None` for events that are transient and should not be
    /// persisted (signals, syscalls, turn completions, etc.).
    pub fn from_kernel_event(event: &KernelEvent) -> Option<Self> {
        match event {
            KernelEvent::UserMessage(msg) => Some(Self::UserMessage(msg.clone())),
            KernelEvent::SpawnAgent {
                manifest,
                input,
                principal,
                parent_id,
                ..
            } => Some(Self::SpawnAgent {
                manifest:  manifest.clone(),
                input:     input.clone(),
                principal: principal.clone(),
                parent_id: *parent_id,
            }),
            KernelEvent::Timer { name, payload } => Some(Self::Timer {
                name:    name.clone(),
                payload: payload.clone(),
            }),
            // Transient events — not persisted.
            KernelEvent::SendSignal { .. }
            | KernelEvent::TurnCompleted { .. }
            | KernelEvent::ChildCompleted { .. }
            | KernelEvent::Deliver(_)
            | KernelEvent::Syscall(_)
            | KernelEvent::Shutdown => None,
        }
    }

    /// Convert back to a [`KernelEvent`].
    ///
    /// For `SpawnAgent`, a fresh oneshot channel is created. The caller
    /// receives the `Receiver` to await the spawn result.
    pub fn into_kernel_event(
        self,
    ) -> (
        KernelEvent,
        Option<oneshot::Receiver<crate::error::Result<AgentId>>>,
    ) {
        match self {
            Self::UserMessage(msg) => (KernelEvent::UserMessage(msg), None),
            Self::SpawnAgent {
                manifest,
                input,
                principal,
                parent_id,
            } => {
                let (tx, rx) = oneshot::channel();
                let event = KernelEvent::SpawnAgent {
                    manifest,
                    input,
                    principal,
                    parent_id,
                    reply_tx: tx,
                };
                (event, Some(rx))
            }
            Self::Timer { name, payload } => (KernelEvent::Timer { name, payload }, None),
        }
    }
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
            session_id:      SessionId::new("s1"),
            target_agent_id: None,
            target_agent:    None,
            content:         MessageContent::Text(text.to_string()),
            reply_context:   None,
            timestamp:       jiff::Timestamp::now(),
            metadata:        HashMap::new(),
        }
    }

    #[test]
    fn persistable_event_roundtrip_user_message() {
        let msg = test_inbound("hello");
        let event = KernelEvent::UserMessage(msg);
        let persistable = PersistableEvent::from_kernel_event(&event).unwrap();

        // Serialize and deserialize
        let json = serde_json::to_string(&persistable).unwrap();
        let restored: PersistableEvent = serde_json::from_str(&json).unwrap();

        let (kernel_event, rx) = restored.into_kernel_event();
        assert!(rx.is_none());
        assert!(matches!(kernel_event, KernelEvent::UserMessage(_)));
    }

    #[test]
    fn persistable_event_roundtrip_spawn_agent() {
        let manifest = AgentManifest {
            name:               "test-agent".to_string(),
            role:               None,
            description:        "test".to_string(),
            model:              Some("gpt-4o-mini".to_string()),
            system_prompt:      "hello".to_string(),
            soul_prompt:        None,
            provider_hint:      None,
            max_iterations:     Some(10),
            tools:              vec![],
            max_children:       None,
            max_context_tokens: None,
            priority:           Default::default(),
            metadata:           Default::default(),
            sandbox:            None,
        };
        let (tx, _rx) = oneshot::channel();
        let event = KernelEvent::SpawnAgent {
            manifest:  manifest.clone(),
            input:     "do something".to_string(),
            principal: Principal::user("test-user"),
            parent_id: None,
            reply_tx:  tx,
        };

        let persistable = PersistableEvent::from_kernel_event(&event).unwrap();
        let json = serde_json::to_string(&persistable).unwrap();
        let restored: PersistableEvent = serde_json::from_str(&json).unwrap();

        let (kernel_event, rx) = restored.into_kernel_event();
        assert!(rx.is_some()); // fresh oneshot created
        match kernel_event {
            KernelEvent::SpawnAgent {
                manifest: m, input, ..
            } => {
                assert_eq!(m.name, "test-agent");
                assert_eq!(input, "do something");
            }
            _ => panic!("expected SpawnAgent"),
        }
    }

    #[test]
    fn persistable_event_roundtrip_timer() {
        let event = KernelEvent::Timer {
            name:    "heartbeat".to_string(),
            payload: serde_json::json!({"interval": 60}),
        };
        let persistable = PersistableEvent::from_kernel_event(&event).unwrap();
        let json = serde_json::to_string(&persistable).unwrap();
        let restored: PersistableEvent = serde_json::from_str(&json).unwrap();

        let (kernel_event, rx) = restored.into_kernel_event();
        assert!(rx.is_none());
        match kernel_event {
            KernelEvent::Timer { name, payload } => {
                assert_eq!(name, "heartbeat");
                assert_eq!(payload, serde_json::json!({"interval": 60}));
            }
            _ => panic!("expected Timer"),
        }
    }

    #[test]
    fn transient_events_are_not_persistable() {
        // SendSignal
        assert!(
            PersistableEvent::from_kernel_event(&KernelEvent::SendSignal {
                target: AgentId::new(),
                signal: Signal::Kill,
            })
            .is_none()
        );

        // Shutdown
        assert!(PersistableEvent::from_kernel_event(&KernelEvent::Shutdown).is_none());
    }

    // -- KernelEvent::agent_id() tests ------------------------------------

    #[test]
    fn agent_id_for_user_message_is_none() {
        let event = KernelEvent::UserMessage(test_inbound("hello"));
        assert!(event.agent_id().is_none());
    }

    #[test]
    fn agent_id_for_spawn_agent_is_none() {
        let (tx, _rx) = oneshot::channel();
        let event = KernelEvent::SpawnAgent {
            manifest:  AgentManifest {
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
            input:     "hello".to_string(),
            principal: Principal::user("test"),
            parent_id: None,
            reply_tx:  tx,
        };
        assert!(event.agent_id().is_none());
    }

    #[test]
    fn agent_id_for_timer_is_none() {
        let event = KernelEvent::Timer {
            name:    "tick".to_string(),
            payload: serde_json::Value::Null,
        };
        assert!(event.agent_id().is_none());
    }

    #[test]
    fn agent_id_for_shutdown_is_none() {
        assert!(KernelEvent::Shutdown.agent_id().is_none());
    }

    #[test]
    fn agent_id_for_send_signal_is_target() {
        let target = AgentId::new();
        let event = KernelEvent::SendSignal {
            target,
            signal: Signal::Interrupt,
        };
        assert_eq!(event.agent_id(), Some(target));
    }

    #[test]
    fn agent_id_for_turn_completed_is_agent_id() {
        let agent_id = AgentId::new();
        let event = KernelEvent::TurnCompleted {
            agent_id,
            session_id: SessionId::new("s1"),
            result: Ok(crate::agent_turn::AgentTurnResult {
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
            in_reply_to: MessageId::new(),
            user: UserId("u1".to_string()),
        };
        assert_eq!(event.agent_id(), Some(agent_id));
    }

    #[test]
    fn agent_id_for_child_completed_is_parent_id() {
        let parent_id = AgentId::new();
        let event = KernelEvent::ChildCompleted {
            parent_id,
            child_id: AgentId::new(),
            result: crate::process::AgentResult {
                output:     "done".to_string(),
                iterations: 1,
                tool_calls: 0,
            },
        };
        assert_eq!(event.agent_id(), Some(parent_id));
    }

    #[test]
    fn agent_id_for_deliver_is_none() {
        let event = KernelEvent::Deliver(crate::io::types::OutboundEnvelope {
            id:          MessageId::new(),
            in_reply_to: MessageId::new(),
            user:        UserId("u1".to_string()),
            session_id:  SessionId::new("s1"),
            routing:     crate::io::types::OutboundRouting::BroadcastAll,
            payload:     crate::io::types::OutboundPayload::Reply {
                content:     crate::channel::types::MessageContent::Text("reply".to_string()),
                attachments: vec![],
            },
            timestamp:   jiff::Timestamp::now(),
        });
        assert!(event.agent_id().is_none());
    }

    // -- Syscall::agent_id() tests ----------------------------------------

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
            session_id: SessionId::new("s1"),
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
