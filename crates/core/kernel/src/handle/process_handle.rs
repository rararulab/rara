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

//! ProcessHandle — thin per-process handle that routes all interactions
//! through `KernelEvent::Syscall` variants via the unified event queue.
//!
//! `ProcessHandle` only holds identity fields and a reference to the event
//! queue. All business logic lives in the kernel event loop's
//! `handle_syscall()` method.

use std::sync::Arc;

use super::AgentHandle;
use crate::{
    error::{KernelError, Result},
    event_queue::EventQueue,
    io::pipe::{PipeReader, PipeWriter},
    memory::KvScope,
    process::{
        AgentId, AgentManifest, ProcessInfo, SessionId, Signal,
        principal::Principal,
    },
    provider::LlmProvider,
    tool::ToolRegistry,
    unified_event::{KernelEvent, Syscall},
};

/// Thin per-process handle to kernel capabilities.
///
/// All methods create a oneshot channel, push a `KernelEvent::Syscall`
/// variant into the event queue, and await the reply. The kernel event
/// loop handles the actual logic.
///
/// # Lifetime
/// Created by `Kernel::handle_spawn_agent()` and lives for the duration
/// of the agent process.
pub struct ProcessHandle {
    /// The agent process this handle belongs to.
    agent_id:    AgentId,
    /// The session this agent belongs to.
    session_id:  SessionId,
    /// The identity under which this agent runs.
    principal:   Principal,
    /// Reference to the unified event queue for pushing syscalls.
    event_queue: Arc<EventQueue>,
}

impl ProcessHandle {
    /// Create a new ProcessHandle.
    pub(crate) fn new(
        agent_id: AgentId,
        session_id: SessionId,
        principal: Principal,
        event_queue: Arc<EventQueue>,
    ) -> Self {
        Self {
            agent_id,
            session_id,
            principal,
            event_queue,
        }
    }

    /// The agent ID this handle belongs to.
    pub fn agent_id(&self) -> AgentId { self.agent_id }

    /// The principal (identity) of this agent.
    pub fn principal(&self) -> &Principal { &self.principal }

    /// The session ID this agent belongs to.
    pub fn session_id(&self) -> &SessionId { &self.session_id }

    // ---- Helper: push syscall and await reply ----

    /// Push a syscall event and await the reply.
    async fn syscall_push(&self, event: KernelEvent) -> Result<()> {
        self.event_queue.push(event).await.map_err(|_| {
            KernelError::Other {
                message: "event queue full for syscall".into(),
            }
        })
    }

    // ========================================================================
    // Process operations
    // ========================================================================

    /// Spawn a child agent via the unified event queue.
    ///
    /// The kernel generates a fresh isolated session for the child — it does
    /// NOT inherit this process's session.
    pub async fn spawn(&self, manifest: AgentManifest, input: String) -> Result<AgentHandle> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let event = KernelEvent::SpawnAgent {
            manifest,
            input,
            principal: self.principal.clone(),
            parent_id: Some(self.agent_id),
            reply_tx,
        };
        self.syscall_push(event).await?;

        let agent_id = reply_rx
            .await
            .map_err(|_| KernelError::SpawnFailed {
                message: "spawn reply channel closed".to_string(),
            })??;

        let (_result_tx, result_rx) = tokio::sync::oneshot::channel();
        Ok(AgentHandle {
            agent_id,
            result_rx,
        })
    }

    /// Send a message to another agent and wait for the response.
    pub async fn send(&self, _agent_id: AgentId, _message: String) -> Result<String> {
        Err(KernelError::Other {
            message: "send not yet implemented".into(),
        })
    }

    /// Query process state.
    pub async fn status(&self, target: AgentId) -> Result<ProcessInfo> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.syscall_push(KernelEvent::Syscall(Syscall::QueryStatus {
            target,
            reply_tx,
        }))
        .await?;
        reply_rx.await.map_err(|_| KernelError::Other {
            message: "syscall reply channel closed".into(),
        })?
    }

    /// Kill an agent and its entire subtree.
    pub async fn kill(&self, target: AgentId) -> Result<()> {
        self.event_queue
            .try_push(KernelEvent::SendSignal {
                target,
                signal: Signal::Kill,
            })
            .map_err(|_| KernelError::Other {
                message: "event queue full for kill signal".into(),
            })
    }

    /// Pause an agent process.
    pub async fn pause(&self, target: AgentId) -> Result<()> {
        self.event_queue
            .try_push(KernelEvent::SendSignal {
                target,
                signal: Signal::Pause,
            })
            .map_err(|_| KernelError::Other {
                message: "event queue full for pause signal".into(),
            })
    }

    /// Resume a paused agent process.
    pub async fn resume(&self, target: AgentId) -> Result<()> {
        self.event_queue
            .try_push(KernelEvent::SendSignal {
                target,
                signal: Signal::Resume,
            })
            .map_err(|_| KernelError::Other {
                message: "event queue full for resume signal".into(),
            })
    }

    /// Interrupt the current LLM turn.
    pub async fn interrupt(&self, target: AgentId) -> Result<()> {
        self.event_queue
            .try_push(KernelEvent::SendSignal {
                target,
                signal: Signal::Interrupt,
            })
            .map_err(|_| KernelError::Other {
                message: "event queue full for interrupt signal".into(),
            })
    }

    /// List child processes of the current agent.
    pub async fn children(&self) -> Vec<ProcessInfo> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        if self
            .syscall_push(KernelEvent::Syscall(Syscall::QueryChildren {
                parent: self.agent_id,
                reply_tx,
            }))
            .await
            .is_err()
        {
            return vec![];
        }
        reply_rx.await.unwrap_or_default()
    }

    // ========================================================================
    // Memory operations
    // ========================================================================

    /// Store a value in this agent's private namespace.
    pub async fn mem_store(&self, key: &str, value: serde_json::Value) -> Result<()> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.syscall_push(KernelEvent::Syscall(Syscall::MemStore {
            agent_id: self.agent_id,
            session_id: self.session_id.clone(),
            principal: self.principal.clone(),
            key: key.to_string(),
            value,
            reply_tx,
        }))
        .await?;
        reply_rx.await.map_err(|_| KernelError::Other {
            message: "syscall reply channel closed".into(),
        })?
    }

    /// Recall a value from this agent's private namespace.
    pub async fn mem_recall(&self, key: &str) -> Result<Option<serde_json::Value>> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.syscall_push(KernelEvent::Syscall(Syscall::MemRecall {
            agent_id: self.agent_id,
            key: key.to_string(),
            reply_tx,
        }))
        .await?;
        reply_rx.await.map_err(|_| KernelError::Other {
            message: "syscall reply channel closed".into(),
        })?
    }

    /// Store a value in an explicit shared scope.
    pub async fn shared_store(
        &self,
        scope: KvScope,
        key: &str,
        value: serde_json::Value,
    ) -> Result<()> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.syscall_push(KernelEvent::Syscall(Syscall::SharedStore {
            agent_id: self.agent_id,
            principal: self.principal.clone(),
            scope,
            key: key.to_string(),
            value,
            reply_tx,
        }))
        .await?;
        reply_rx.await.map_err(|_| KernelError::Other {
            message: "syscall reply channel closed".into(),
        })?
    }

    /// Recall a value from an explicit shared scope.
    pub async fn shared_recall(
        &self,
        scope: KvScope,
        key: &str,
    ) -> Result<Option<serde_json::Value>> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.syscall_push(KernelEvent::Syscall(Syscall::SharedRecall {
            agent_id: self.agent_id,
            principal: self.principal.clone(),
            scope,
            key: key.to_string(),
            reply_tx,
        }))
        .await?;
        reply_rx.await.map_err(|_| KernelError::Other {
            message: "syscall reply channel closed".into(),
        })?
    }

    // ========================================================================
    // Pipe operations
    // ========================================================================

    /// Create an anonymous pipe targeting a specific agent.
    pub async fn create_pipe(&self, target: AgentId) -> Result<(PipeWriter, PipeReader)> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.syscall_push(KernelEvent::Syscall(Syscall::CreatePipe {
            owner: self.agent_id,
            target,
            reply_tx,
        }))
        .await?;
        reply_rx.await.map_err(|_| KernelError::Other {
            message: "syscall reply channel closed".into(),
        })?
    }

    /// Create a named pipe that any agent can connect to.
    pub async fn create_named_pipe(
        &self,
        name: &str,
    ) -> Result<(PipeWriter, PipeReader)> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.syscall_push(KernelEvent::Syscall(Syscall::CreateNamedPipe {
            owner: self.agent_id,
            name: name.to_string(),
            reply_tx,
        }))
        .await?;
        reply_rx.await.map_err(|_| KernelError::Other {
            message: "syscall reply channel closed".into(),
        })?
    }

    /// Connect to a named pipe as a reader.
    pub async fn connect_pipe(&self, name: &str) -> Result<PipeReader> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.syscall_push(KernelEvent::Syscall(Syscall::ConnectPipe {
            connector: self.agent_id,
            name: name.to_string(),
            reply_tx,
        }))
        .await?;
        reply_rx.await.map_err(|_| KernelError::Other {
            message: "syscall reply channel closed".into(),
        })?
    }

    // ========================================================================
    // Guard operations
    // ========================================================================

    /// Check whether a tool requires approval before execution.
    pub async fn requires_approval(&self, tool_name: &str) -> bool {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        if self
            .syscall_push(KernelEvent::Syscall(Syscall::RequiresApproval {
                tool_name: tool_name.to_string(),
                reply_tx,
            }))
            .await
            .is_err()
        {
            return false;
        }
        reply_rx.await.unwrap_or(false)
    }

    /// Request approval for a tool execution.
    pub async fn request_approval(
        &self,
        tool_name: &str,
        summary: &str,
    ) -> Result<bool> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.syscall_push(KernelEvent::Syscall(Syscall::RequestApproval {
            agent_id: self.agent_id,
            principal: self.principal.clone(),
            tool_name: tool_name.to_string(),
            summary: summary.to_string(),
            reply_tx,
        }))
        .await?;
        reply_rx.await.map_err(|_| KernelError::Other {
            message: "syscall reply channel closed".into(),
        })?
    }

    // ========================================================================
    // Context queries (used by agent_turn)
    // ========================================================================

    /// Get the manifest for this agent.
    pub async fn manifest(&self) -> Result<AgentManifest> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.syscall_push(KernelEvent::Syscall(Syscall::GetManifest {
            agent_id: self.agent_id,
            reply_tx,
        }))
        .await?;
        reply_rx.await.map_err(|_| KernelError::Other {
            message: "syscall reply channel closed".into(),
        })?
    }

    /// Get the tool registry.
    pub async fn tool_registry(&self) -> Result<Arc<ToolRegistry>> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.syscall_push(KernelEvent::Syscall(Syscall::GetToolRegistry {
            reply_tx,
        }))
        .await?;
        reply_rx.await.map_err(|_| KernelError::Other {
            message: "syscall reply channel closed".into(),
        })
    }

    /// Acquire an LLM provider instance.
    pub async fn acquire_provider(&self) -> Result<Arc<dyn LlmProvider>> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.syscall_push(KernelEvent::Syscall(Syscall::AcquireProvider {
            reply_tx,
        }))
        .await?;
        reply_rx.await.map_err(|_| KernelError::Other {
            message: "syscall reply channel closed".into(),
        })?
    }

    // ========================================================================
    // Event publishing
    // ========================================================================

    /// Publish an event to the kernel event bus.
    pub async fn publish(
        &self,
        event_type: &str,
        payload: serde_json::Value,
    ) -> Result<()> {
        self.syscall_push(KernelEvent::Syscall(Syscall::PublishEvent {
            agent_id: self.agent_id,
            event_type: event_type.to_string(),
            payload,
        }))
        .await
    }
}

impl std::fmt::Debug for ProcessHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessHandle")
            .field("agent_id", &self.agent_id)
            .field("session_id", &self.session_id)
            .field("principal", &self.principal)
            .finish()
    }
}
