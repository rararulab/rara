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

//! ScopedKernelHandle — per-process scoped handle to kernel capabilities.
//!
//! Each [`AgentProcess`] receives its own `ScopedKernelHandle` which:
//! - Knows its own `agent_id` (so `spawn` auto-sets `parent_id`)
//! - Carries its `principal` (so children inherit identity)
//! - Enforces tool subset restrictions (children can only use parent's tools)
//! - Limits concurrent children via a per-agent semaphore
//!
//! Trait implementations (ProcessOps, MemoryOps, etc.) will be added in
//! Task 3 when `Kernel.spawn()` is implemented.

use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::process::principal::Principal;
use crate::process::{AgentId, ProcessTable};

/// Per-process scoped handle to kernel capabilities.
///
/// This is the concrete type that agents interact with. Each handle is scoped
/// to a specific agent process, enforcing:
/// - **Identity**: operations run under the agent's principal
/// - **Tool isolation**: child agents can only use a subset of parent's tools
/// - **Concurrency limits**: per-agent child semaphore prevents fork bombs
///
/// # Lifetime
/// Created by `Kernel.spawn()` and lives for the duration of the agent process.
pub struct ScopedKernelHandle {
    /// The agent process this handle belongs to.
    pub(crate) agent_id: AgentId,
    /// The identity under which this agent runs.
    pub(crate) principal: Principal,
    /// Tools this agent is allowed to use (children can only subset these).
    pub(crate) allowed_tools: Vec<String>,
    /// Per-agent semaphore limiting concurrent child processes.
    pub(crate) child_semaphore: Arc<Semaphore>,
    /// Shared kernel internals (process table, global limits, etc.).
    pub(crate) inner: Arc<KernelInner>,
}

/// Shared kernel state that `ScopedKernelHandle` delegates to.
///
/// This struct holds the "real" kernel state shared by all handles via `Arc`.
/// It will be extended in Task 3 with LLM provider, tool registry, memory,
/// event bus, and guard components.
pub(crate) struct KernelInner {
    /// The global process table tracking all running agents.
    pub process_table: ProcessTable,
    /// Global semaphore limiting total concurrent agent processes.
    pub global_semaphore: Arc<Semaphore>,
    /// Default maximum number of children per agent.
    pub default_child_limit: usize,
    // More fields will be added in Task 3:
    // pub llm_provider: Arc<dyn LlmProvider>,
    // pub tool_registry: Arc<dyn ToolRegistry>,
    // pub memory: Arc<dyn Memory>,
    // pub event_bus: Arc<dyn EventBus>,
    // pub guard: Arc<dyn Guard>,
}

impl ScopedKernelHandle {
    /// The agent ID this handle belongs to.
    pub fn agent_id(&self) -> AgentId {
        self.agent_id
    }

    /// The principal (identity) of this agent.
    pub fn principal(&self) -> &Principal {
        &self.principal
    }

    /// The tools this agent is allowed to use.
    pub fn allowed_tools(&self) -> &[String] {
        &self.allowed_tools
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::principal::Principal;

    fn make_kernel_inner() -> Arc<KernelInner> {
        Arc::new(KernelInner {
            process_table: ProcessTable::new(),
            global_semaphore: Arc::new(Semaphore::new(10)),
            default_child_limit: 5,
        })
    }

    #[test]
    fn test_scoped_handle_accessors() {
        let agent_id = AgentId::new();
        let principal = Principal::user("test-user");
        let tools = vec!["read_file".to_string(), "grep".to_string()];

        let handle = ScopedKernelHandle {
            agent_id,
            principal: principal.clone(),
            allowed_tools: tools.clone(),
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner: make_kernel_inner(),
        };

        assert_eq!(handle.agent_id(), agent_id);
        assert!(!handle.principal().is_admin());
        assert_eq!(handle.allowed_tools().len(), 2);
        assert_eq!(handle.allowed_tools()[0], "read_file");
    }

    #[test]
    fn test_kernel_inner_shared() {
        let inner = make_kernel_inner();

        // Two handles sharing the same inner
        let handle1 = ScopedKernelHandle {
            agent_id: AgentId::new(),
            principal: Principal::user("user-1"),
            allowed_tools: vec![],
            child_semaphore: Arc::new(Semaphore::new(3)),
            inner: Arc::clone(&inner),
        };

        let handle2 = ScopedKernelHandle {
            agent_id: AgentId::new(),
            principal: Principal::admin("admin-1"),
            allowed_tools: vec!["bash".to_string()],
            child_semaphore: Arc::new(Semaphore::new(3)),
            inner: Arc::clone(&inner),
        };

        // Both share the same process table
        assert_eq!(handle1.inner.process_table.list().len(), 0);
        assert_eq!(handle2.inner.process_table.list().len(), 0);

        // Different agent IDs
        assert_ne!(handle1.agent_id(), handle2.agent_id());

        // Different principals
        assert!(!handle1.principal().is_admin());
        assert!(handle2.principal().is_admin());
    }
}
