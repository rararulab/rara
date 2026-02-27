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
//!
//! The blanket `KernelHandle` trait is automatically implemented for any type
//! that implements all four subsystem traits.

pub mod scoped;

use async_trait::async_trait;
use tokio::sync::oneshot;

use crate::error::Result;
use crate::process::{AgentId, AgentManifest, AgentResult, ProcessInfo};

/// Handle returned from spawn — allows waiting for agent completion.
///
/// Holds the spawned agent's ID and a oneshot receiver that resolves when
/// the agent finishes execution (successfully or with failure).
pub struct AgentHandle {
    /// The ID of the spawned agent process.
    pub agent_id: AgentId,
    /// Receiver for the agent's result. Resolves when the agent finishes.
    pub result_rx: oneshot::Receiver<AgentResult>,
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

    /// List child processes of the current agent.
    fn children(&self) -> Vec<ProcessInfo>;
}

/// Cross-agent shared memory operations.
///
/// Provides a simple key-value store that is shared across all agents
/// in the same session, enabling inter-agent data passing.
pub trait MemoryOps: Send + Sync {
    /// Store a value in shared memory.
    fn mem_store(&self, key: &str, value: serde_json::Value) -> Result<()>;

    /// Recall a value from shared memory.
    fn mem_recall(&self, key: &str) -> Result<Option<serde_json::Value>>;
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

/// Unified kernel handle — the single "syscall" interface for agents.
///
/// This is a blanket trait: any type implementing all four subsystem traits
/// automatically implements `KernelHandle`. This allows agents to receive a
/// single handle that provides all kernel capabilities.
pub trait KernelHandle: ProcessOps + MemoryOps + EventOps + GuardOps {}
impl<T: ProcessOps + MemoryOps + EventOps + GuardOps> KernelHandle for T {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_handle_creation() {
        let (tx, rx) = oneshot::channel();
        let id = AgentId::new();
        let handle = AgentHandle {
            agent_id: id,
            result_rx: rx,
        };
        assert_eq!(handle.agent_id, id);

        // Send a result through the channel
        let result = AgentResult {
            output: "done".to_string(),
            iterations: 3,
            tool_calls: 1,
        };
        tx.send(result).unwrap();

        // We can't easily test the receiver in a sync test without tokio,
        // but we can verify the handle was created correctly
    }

    /// Verify that the KernelHandle blanket impl works at the type level.
    /// This is a compile-time check — if this module compiles, the blanket
    /// impl is correctly defined.
    fn _assert_kernel_handle_blanket<T: ProcessOps + MemoryOps + EventOps + GuardOps>() {
        fn _requires_kernel_handle<H: KernelHandle>() {}
        _requires_kernel_handle::<T>();
    }
}
