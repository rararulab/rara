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

//! KernelHandle — the unified "syscall" interface for agents.
//!
//! Each agent process receives a [`KernelHandle`] implementation (typically
//! [`ScopedKernelHandle`]) that provides access to kernel capabilities:
//!
//! - [`ProcessOps`]: spawn/kill/status child agents
//! - [`MemoryOps`]: cross-agent shared memory
//! - [`EventOps`]: event bus publishing
//! - [`GuardOps`]: tool approval requests
//! - [`PipeOps`]: inter-agent streaming pipes
//!
//! The blanket `KernelHandle` trait is automatically implemented for any type
//! that implements all five subsystem traits.

pub mod scoped;
pub mod spawn_tool;

use async_trait::async_trait;
use tokio::sync::oneshot;

use crate::{
    error::Result,
    io::pipe::{PipeReader, PipeWriter},
    process::{AgentId, AgentManifest, AgentResult, ProcessInfo, ProcessMessage},
};

/// Handle returned from spawn — allows waiting for agent completion.
///
/// Holds the spawned agent's ID, a mailbox sender for delivering messages
/// to the process, and a oneshot receiver that resolves when the agent
/// finishes execution (successfully or with failure).
pub struct AgentHandle {
    /// The ID of the spawned agent process.
    pub agent_id:  AgentId,
    /// Mailbox sender for delivering messages to the process.
    pub mailbox:   tokio::sync::mpsc::Sender<ProcessMessage>,
    /// Receiver for the agent's result. Resolves when the agent finishes.
    pub result_rx: oneshot::Receiver<AgentResult>,
}

impl std::fmt::Debug for AgentHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentHandle")
            .field("agent_id", &self.agent_id)
            .field("mailbox", &"<mpsc::Sender>")
            .field("result_rx", &"<oneshot::Receiver>")
            .finish()
    }
}

// ---- Subsystem traits ----

/// Process lifecycle management.
///
/// Provides the core "syscalls" for agent process management:
/// spawning children, querying status, and termination.
#[async_trait]
pub trait ProcessOps: Send + Sync {
    /// Spawn a child agent.
    ///
    /// The child inherits the current principal. The child's tools are the
    /// intersection of the parent's allowed tools and the manifest's tools.
    async fn spawn(&self, manifest: AgentManifest, input: String) -> Result<AgentHandle>;

    /// Send a message to another agent and wait for the response.
    async fn send(&self, agent_id: AgentId, message: String) -> Result<String>;

    /// Query process state.
    fn status(&self, agent_id: AgentId) -> Result<ProcessInfo>;

    /// Kill an agent and its entire subtree.
    fn kill(&self, agent_id: AgentId) -> Result<()>;

    /// Pause an agent process (suspend message processing, buffer incoming).
    fn pause(&self, agent_id: AgentId) -> Result<()>;

    /// Resume a paused agent process (drain buffered messages).
    fn resume(&self, agent_id: AgentId) -> Result<()>;

    /// Interrupt the current LLM turn (cancel in-flight call, process stays alive).
    fn interrupt(&self, agent_id: AgentId) -> Result<()>;

    /// List child processes of the current agent.
    fn children(&self) -> Vec<ProcessInfo>;
}

/// Cross-agent shared memory operations.
///
/// Provides a namespaced key-value store for agents. By default, each agent
/// has its own isolated namespace — `mem_store`/`mem_recall` auto-prefix
/// keys with the agent's ID.
///
/// For explicit cross-agent data sharing, use `shared_store`/`shared_recall`
/// with a [`KvScope`](crate::memory::KvScope) that controls visibility and
/// permission requirements.
pub trait MemoryOps: Send + Sync {
    /// Store a value in this agent's private namespace.
    ///
    /// The key is automatically prefixed with `"agent:{agent_id}:"` so that
    /// different agents cannot accidentally overwrite each other's data.
    fn mem_store(&self, key: &str, value: serde_json::Value) -> Result<()>;

    /// Recall a value from this agent's private namespace.
    ///
    /// Only returns values stored by the same agent (auto-prefixed lookup).
    fn mem_recall(&self, key: &str) -> Result<Option<serde_json::Value>>;

    /// Store a value in an explicit shared scope.
    ///
    /// Permission rules:
    /// - `KvScope::Global` — requires Root or Admin role
    /// - `KvScope::Team(name)` — requires Root or Admin role
    /// - `KvScope::Agent(id)` — regular agents can only write to their own ID;
    ///   Root/Admin can write to any agent's scope
    fn shared_store(
        &self,
        scope: crate::memory::KvScope,
        key: &str,
        value: serde_json::Value,
    ) -> Result<()>;

    /// Recall a value from an explicit shared scope.
    ///
    /// Permission rules are the same as `shared_store`.
    fn shared_recall(
        &self,
        scope: crate::memory::KvScope,
        key: &str,
    ) -> Result<Option<serde_json::Value>>;
}

/// Event bus operations.
///
/// Allows agents to publish events that other components can subscribe to.
#[async_trait]
pub trait EventOps: Send + Sync {
    /// Publish an event to the event bus.
    async fn publish(&self, event_type: &str, payload: serde_json::Value) -> Result<()>;
}

/// Guard / approval operations.
///
/// Provides the interface for tool approval — agents can check whether a
/// tool requires human approval and request it.
#[async_trait]
pub trait GuardOps: Send + Sync {
    /// Check whether a tool requires approval before execution.
    fn requires_approval(&self, tool_name: &str) -> bool;

    /// Request approval for a tool execution. Returns true if approved.
    async fn request_approval(&self, tool_name: &str, summary: &str) -> Result<bool>;
}

/// Inter-agent pipe operations.
///
/// Provides Unix-pipe-like streaming data channels between agent processes.
/// Supports both anonymous pipes (parent creates, gives reader to child)
/// and named pipes (any agent can create and connect by name).
pub trait PipeOps: Send + Sync {
    /// Create an anonymous pipe targeting a specific agent.
    ///
    /// Returns the writer end. The reader end must be delivered to the
    /// target agent through the returned `PipeReader`.
    fn create_pipe(&self, target: AgentId) -> Result<(PipeWriter, PipeReader)>;

    /// Create a named pipe that any agent can connect to.
    ///
    /// The name is globally unique within the kernel. Returns the writer
    /// end; readers connect via [`connect_pipe`](Self::connect_pipe).
    fn create_named_pipe(&self, name: &str) -> Result<(PipeWriter, PipeReader)>;

    /// Connect to a named pipe as a reader.
    ///
    /// Returns `Err` if the named pipe does not exist or has already been
    /// connected by another reader.
    fn connect_pipe(&self, name: &str) -> Result<PipeReader>;
}

/// Unified kernel handle — the single "syscall" interface for agents.
///
/// This is a blanket trait: any type implementing all five subsystem traits
/// automatically implements `KernelHandle`. This allows agents to receive a
/// single handle that provides all kernel capabilities.
pub trait KernelHandle: ProcessOps + MemoryOps + EventOps + GuardOps + PipeOps {}
impl<T: ProcessOps + MemoryOps + EventOps + GuardOps + PipeOps> KernelHandle for T {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_handle_creation() {
        let (result_tx, result_rx) = oneshot::channel();
        let (mailbox_tx, _mailbox_rx) = tokio::sync::mpsc::channel(16);
        let id = AgentId::new();
        let handle = AgentHandle {
            agent_id: id,
            mailbox: mailbox_tx,
            result_rx,
        };
        assert_eq!(handle.agent_id, id);

        // Send a result through the channel
        let result = AgentResult {
            output:     "done".to_string(),
            iterations: 3,
            tool_calls: 1,
        };
        result_tx.send(result).unwrap();

        // We can't easily test the receiver in a sync test without tokio,
        // but we can verify the handle was created correctly
    }

    /// Verify that the KernelHandle blanket impl works at the type level.
    /// This is a compile-time check — if this module compiles, the blanket
    /// impl is correctly defined.
    fn _assert_kernel_handle_blanket<
        T: ProcessOps + MemoryOps + EventOps + GuardOps + PipeOps,
    >() {
        fn _requires_kernel_handle<H: KernelHandle>() {}
        _requires_kernel_handle::<T>();
    }
}
