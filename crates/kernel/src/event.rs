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
    /// UserMessage, SpawnAgent — processed last.
    Low = 2,
}

// ---------------------------------------------------------------------------
// Syscall — process handle requests routed through the event queue
// ---------------------------------------------------------------------------

/// Syscall variants — all interactions that a `ProcessHandle` routes through
/// the kernel event queue. Each variant carries identity fields plus a oneshot
/// reply channel for the kernel event loop to respond on.
#[derive(derive_more::Debug, strum::IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum Syscall {
    // -- Process queries --
    /// Query the status of a target agent process.
    QueryStatus {
        target:   AgentId,
        #[debug(skip)]
        reply_tx: oneshot::Sender<crate::error::Result<ProcessInfo>>,
    },

    /// Query children of a parent agent process.
    QueryChildren {
        parent:   AgentId,
        #[debug(skip)]
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
        reply_tx:   oneshot::Sender<crate::error::Result<()>>,
    },

    /// Recall a value from the agent's private memory namespace.
    MemRecall {
        agent_id: AgentId,
        key:      String,
        #[debug(skip)]
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
        reply_tx:  oneshot::Sender<crate::error::Result<()>>,
    },

    /// Recall a value from a shared (scoped) memory namespace.
    SharedRecall {
        agent_id:  AgentId,
        principal: Principal,
        scope:     KvScope,
        key:       String,
        #[debug(skip)]
        reply_tx:  oneshot::Sender<crate::error::Result<Option<serde_json::Value>>>,
    },

    // -- Pipe --
    /// Create an anonymous pipe between two agents.
    CreatePipe {
        owner:    AgentId,
        target:   AgentId,
        #[debug(skip)]
        reply_tx: oneshot::Sender<crate::error::Result<(PipeWriter, PipeReader)>>,
    },

    /// Create a named pipe.
    CreateNamedPipe {
        owner:    AgentId,
        name:     String,
        #[debug(skip)]
        reply_tx: oneshot::Sender<crate::error::Result<(PipeWriter, PipeReader)>>,
    },

    /// Connect to a named pipe as a reader.
    ConnectPipe {
        connector: AgentId,
        name:      String,
        #[debug(skip)]
        reply_tx:  oneshot::Sender<crate::error::Result<PipeReader>>,
    },

    // -- Guard --
    /// Check whether a tool requires approval before execution.
    RequiresApproval {
        tool_name: String,
        #[debug(skip)]
        reply_tx:  oneshot::Sender<bool>,
    },

    /// Request approval for a tool execution.
    RequestApproval {
        agent_id:  AgentId,
        principal: Principal,
        tool_name: String,
        summary:   String,
        #[debug(skip)]
        reply_tx:  oneshot::Sender<crate::error::Result<bool>>,
    },

    /// Check guard verdict for a batch of tool calls before execution.
    CheckGuardBatch {
        agent_id:   AgentId,
        session_id: SessionId,
        #[debug("{} checks", checks.len())]
        checks:     Vec<(String, serde_json::Value)>,
        #[debug(skip)]
        reply_tx:   oneshot::Sender<Vec<crate::guard::Verdict>>,
    },

    // -- Context queries (used by agent_turn) --
    /// Get the manifest for an agent process.
    GetManifest {
        agent_id: AgentId,
        #[debug(skip)]
        reply_tx: oneshot::Sender<crate::error::Result<AgentManifest>>,
    },

    /// Get the tool registry, enriched with per-process tools (e.g.
    /// SyscallTool).
    GetToolRegistry {
        agent_id: AgentId,
        #[debug(skip)]
        reply_tx: oneshot::Sender<Arc<ToolRegistry>>,
    },

    /// Resolve an [`LlmDriver`](crate::llm::LlmDriver) + model for a
    /// specific agent via `DriverRegistry::resolve()`.
    ResolveDriver {
        agent_id: AgentId,
        #[debug(skip)]
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

// ---------------------------------------------------------------------------
// KernelEvent
// ---------------------------------------------------------------------------

/// Unified event type for all kernel interactions.
///
/// Every interaction with the kernel — user messages, process control,
/// internal callbacks, output delivery — is represented as a `KernelEvent`
/// and processed by the single `Kernel::run()` event loop.
#[derive(derive_more::Debug, strum::IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum KernelEvent {
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
        reply_tx:  oneshot::Sender<crate::error::Result<AgentId>>,
    },

    /// Send a control signal to an agent process.
    SendSignal { target: AgentId, signal: Signal },

    // === Internal callbacks: from async task completion ===
    /// An LLM turn completed (success or failure).
    TurnCompleted {
        agent_id:    AgentId,
        session_id:  SessionId,
        #[debug("{}", if result.is_ok() { "Ok(..)" } else { "Err(..)" })]
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
            Self::UserMessage(_) | Self::SpawnAgent { .. } | Self::Deliver(_) | Self::Shutdown => {
                None
            }
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
            Self::UserMessage(_) | Self::SpawnAgent { .. } => EventPriority::Low,
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
            session_id:      SessionId::new(),
            target_agent_id: None,
            target_agent:    None,
            content:         MessageContent::Text(text.to_string()),
            reply_context:   None,
            timestamp:       jiff::Timestamp::now(),
            metadata:        HashMap::new(),
        }
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
            session_id: SessionId::new(),
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
