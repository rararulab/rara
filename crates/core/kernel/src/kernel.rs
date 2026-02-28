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

//! Kernel — the unified OS-inspired orchestrator for agent lifecycle.
//!
//! The [`Kernel`] is the single entry point for all agent operations.
//! It manages a [`ProcessTable`] of running agents, enforces concurrency
//! limits via dual semaphores (global + per-agent), and provides
//! [`spawn`](Kernel::spawn) as the primary API for creating agent processes.
//!
//! # Architecture
//!
//! ```text
//! Kernel (top-level)
//!   ├── ProcessTable  (all running agents)
//!   ├── global_semaphore (max total concurrent agents)
//!   ├── ManifestLoader  (named agent definitions)
//!   └── KernelInner (shared state via Arc)
//!         ├── LlmProviderLoader
//!         ├── ToolRegistry
//!         ├── Memory
//!         ├── EventBus
//!         ├── Guard
//!         └── shared_kv (cross-agent KV)
//! ```
//!
//! Each spawned agent receives a [`ScopedKernelHandle`] providing syscall-like
//! access to kernel capabilities (ProcessOps, MemoryOps, EventOps, GuardOps).

use std::sync::Arc;

use dashmap::DashMap;
use jiff::Timestamp;
use tokio::sync::{oneshot, Semaphore};
use tracing::info;

use crate::error::{KernelError, Result};
use crate::event::EventBus;
use crate::guard::Guard;
use crate::handle::scoped::{KernelInner, ScopedKernelHandle};
use crate::handle::AgentHandle;
use crate::memory::Memory;
use crate::process::manifest_loader::ManifestLoader;
use crate::process::principal::Principal;
use crate::process::user::UserStore;
use crate::process::{
    AgentEnv, AgentId, AgentManifest, AgentProcess, AgentResult, ProcessState,
    ProcessTable, SessionId,
};
use crate::provider::LlmProviderLoaderRef;
use crate::runner::{AgentRunner, UserContent};
use crate::tool::ToolRegistry;

/// Kernel configuration.
#[derive(Debug, Clone)]
pub struct KernelConfig {
    /// Maximum number of concurrent agent processes globally.
    pub max_concurrency: usize,
    /// Default maximum number of children per agent.
    pub default_child_limit: usize,
    /// Default max LLM iterations for spawned agents.
    pub default_max_iterations: usize,
}

impl Default for KernelConfig {
    fn default() -> Self {
        Self {
            max_concurrency: 16,
            default_child_limit: 8,
            default_max_iterations: 25,
        }
    }
}

/// The unified agent orchestrator.
///
/// Acts as an OS kernel for agents: manages the process table, enforces
/// concurrency limits, and provides `spawn()` as the primary entry point.
pub struct Kernel {
    /// Shared kernel internals (process table, components, etc.).
    inner: Arc<KernelInner>,
    /// Kernel configuration.
    config: KernelConfig,
}

impl Kernel {
    /// Create a new Kernel with the given configuration and components.
    pub fn new(
        config: KernelConfig,
        llm_provider: LlmProviderLoaderRef,
        tool_registry: Arc<ToolRegistry>,
        memory: Arc<dyn Memory>,
        event_bus: Arc<dyn EventBus>,
        guard: Arc<dyn Guard>,
        manifest_loader: ManifestLoader,
        user_store: Arc<dyn UserStore>,
    ) -> Self {
        info!(
            max_concurrency = config.max_concurrency,
            default_child_limit = config.default_child_limit,
            default_max_iterations = config.default_max_iterations,
            "booting kernel"
        );

        let inner = Arc::new(KernelInner {
            process_table: ProcessTable::new(),
            global_semaphore: Arc::new(Semaphore::new(config.max_concurrency)),
            default_child_limit: config.default_child_limit,
            default_max_iterations: config.default_max_iterations,
            llm_provider,
            tool_registry,
            memory,
            event_bus,
            guard,
            manifest_loader,
            shared_kv: DashMap::new(),
            user_store,
        });

        Self { inner, config }
    }

    /// Spawn a top-level agent process (no parent).
    ///
    /// This is the primary entry point for external callers (workers, channel
    /// adapters, HTTP handlers) to create a new agent.
    ///
    /// # Arguments
    /// - `manifest` — the agent definition to run
    /// - `input` — the user message / task description
    /// - `principal` — the identity under which the agent runs
    /// - `session_id` — the session this agent belongs to
    /// - `parent_id` — optional parent agent ID (for process tree)
    pub async fn spawn(
        &self,
        manifest: AgentManifest,
        input: String,
        principal: Principal,
        session_id: SessionId,
        parent_id: Option<AgentId>,
    ) -> Result<AgentHandle> {
        // Acquire global semaphore
        let _global_permit =
            self.inner
                .global_semaphore
                .clone()
                .try_acquire_owned()
                .map_err(|_| KernelError::SpawnLimitReached {
                    message: "global concurrency limit reached".to_string(),
                })?;

        // Create AgentProcess
        let agent_id = AgentId::new();
        let max_iterations = manifest
            .max_iterations
            .unwrap_or(self.config.default_max_iterations);
        let child_limit = manifest
            .max_children
            .unwrap_or(self.config.default_child_limit);

        let process = AgentProcess {
            agent_id,
            parent_id,
            session_id: session_id.clone(),
            manifest: manifest.clone(),
            principal: principal.clone(),
            env: AgentEnv::default(),
            state: ProcessState::Running,
            created_at: Timestamp::now(),
            finished_at: None,
            result: None,
        };
        self.inner.process_table.insert(process);

        // Build tool registry for this agent
        let agent_tools = self.inner.tool_registry.filtered(&manifest.tools);
        let tool_names: Vec<String> = agent_tools.tool_names();

        // Create ScopedKernelHandle for the agent
        let scoped_handle = Arc::new(ScopedKernelHandle {
            agent_id,
            session_id,
            principal,
            allowed_tools: tool_names,
            child_semaphore: Arc::new(Semaphore::new(child_limit)),
            inner: Arc::clone(&self.inner),
        });

        // Build AgentRunner
        let model_name = manifest.model.clone();
        let system_prompt = manifest.system_prompt.clone();
        let provider_hint = manifest.provider_hint.clone().unwrap_or_default();
        let llm_provider = Arc::clone(&self.inner.llm_provider);

        let runner = AgentRunner::builder()
            .llm_provider(llm_provider)
            .provider_hint(provider_hint)
            .model_name(model_name)
            .system_prompt(system_prompt)
            .user_content(UserContent::Text(input))
            .max_iterations(max_iterations)
            .build();

        // Create oneshot channel for result
        let (result_tx, result_rx) = oneshot::channel();
        let inner_ref = Arc::clone(&self.inner);

        // Spawn tokio task
        tokio::spawn(async move {
            let _global_permit = _global_permit;
            let _scoped_handle = scoped_handle;

            let run_result = runner.run(&agent_tools, None).await;

            match run_result {
                Ok(response) => {
                    let agent_result = AgentResult {
                        output: response.response_text(),
                        iterations: response.iterations,
                        tool_calls: response.tool_calls_made,
                    };

                    let _ = inner_ref
                        .process_table
                        .set_state(agent_id, ProcessState::Completed);
                    let _ = inner_ref
                        .process_table
                        .set_result(agent_id, agent_result.clone());

                    info!(
                        agent_id = %agent_id,
                        iterations = agent_result.iterations,
                        tool_calls = agent_result.tool_calls,
                        "agent process completed"
                    );

                    let _ = result_tx.send(agent_result);
                }
                Err(err) => {
                    tracing::warn!(
                        agent_id = %agent_id,
                        error = %err,
                        "agent process failed"
                    );

                    let _ = inner_ref
                        .process_table
                        .set_state(agent_id, ProcessState::Failed);

                    let error_result = AgentResult {
                        output: format!("Error: {err}"),
                        iterations: 0,
                        tool_calls: 0,
                    };
                    let _ = inner_ref
                        .process_table
                        .set_result(agent_id, error_result.clone());
                    let _ = result_tx.send(error_result);
                }
            }
        });

        Ok(AgentHandle {
            agent_id,
            result_rx,
        })
    }

    /// Spawn a named agent by looking up its manifest.
    ///
    /// Convenience method that resolves a manifest name to an `AgentManifest`
    /// and delegates to [`spawn`](Self::spawn).
    pub async fn spawn_named(
        &self,
        agent_name: &str,
        input: String,
        principal: Principal,
        session_id: SessionId,
        parent_id: Option<AgentId>,
    ) -> Result<AgentHandle> {
        let manifest = self
            .inner
            .manifest_loader
            .get(agent_name)
            .ok_or(KernelError::ManifestNotFound {
                name: agent_name.to_string(),
            })?
            .clone();

        self.spawn(manifest, input, principal, session_id, parent_id)
            .await
    }

    /// Access the process table for querying.
    pub fn process_table(&self) -> &ProcessTable {
        &self.inner.process_table
    }

    /// Access the manifest loader for looking up named manifests.
    pub fn manifest_loader(&self) -> &ManifestLoader {
        &self.inner.manifest_loader
    }

    /// Access the tool registry.
    pub fn tool_registry(&self) -> &Arc<ToolRegistry> {
        &self.inner.tool_registry
    }

    /// Access the event bus.
    pub fn event_bus(&self) -> &Arc<dyn EventBus> {
        &self.inner.event_bus
    }

    /// Access the memory subsystem.
    pub fn memory(&self) -> &Arc<dyn Memory> {
        &self.inner.memory
    }

    /// Access the kernel config.
    pub fn config(&self) -> &KernelConfig {
        &self.config
    }

    /// Access the shared KernelInner (for constructing ScopedKernelHandles externally).
    pub(crate) fn inner(&self) -> &Arc<KernelInner> {
        &self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defaults::noop::{NoopEventBus, NoopGuard, NoopMemory};
    use crate::defaults::noop_user_store::NoopUserStore;
    use crate::process::principal::Principal;
    use crate::provider::EnvLlmProviderLoader;

    fn make_test_kernel(max_concurrency: usize, child_limit: usize) -> Kernel {
        let config = KernelConfig {
            max_concurrency,
            default_child_limit: child_limit,
            default_max_iterations: 5,
        };

        let mut loader = ManifestLoader::new();
        loader.load_bundled();

        Kernel::new(
            config,
            Arc::new(EnvLlmProviderLoader::default()) as LlmProviderLoaderRef,
            Arc::new(ToolRegistry::new()),
            Arc::new(NoopMemory),
            Arc::new(NoopEventBus),
            Arc::new(NoopGuard),
            loader,
            Arc::new(NoopUserStore),
        )
    }

    fn test_manifest(name: &str) -> AgentManifest {
        AgentManifest {
            name: name.to_string(),
            description: format!("Test agent: {name}"),
            model: "test-model".to_string(),
            system_prompt: "You are a test agent.".to_string(),
            provider_hint: None,
            max_iterations: Some(5),
            tools: vec![],
            max_children: None,
            metadata: serde_json::Value::Null,
        }
    }

    #[test]
    fn test_kernel_creation() {
        let kernel = make_test_kernel(10, 5);
        assert_eq!(kernel.config().max_concurrency, 10);
        assert_eq!(kernel.config().default_child_limit, 5);
        assert_eq!(kernel.process_table().list().len(), 0);
    }

    #[test]
    fn test_kernel_manifest_loader() {
        let kernel = make_test_kernel(10, 5);
        assert!(kernel.manifest_loader().get("scout").is_some());
        assert!(kernel.manifest_loader().get("planner").is_some());
        assert!(kernel.manifest_loader().get("worker").is_some());
        assert!(kernel.manifest_loader().get("nonexistent").is_none());
    }

    #[test]
    fn test_kernel_default_config() {
        let config = KernelConfig::default();
        assert_eq!(config.max_concurrency, 16);
        assert_eq!(config.default_child_limit, 8);
        assert_eq!(config.default_max_iterations, 25);
    }

    #[tokio::test]
    async fn test_kernel_spawn_creates_process() {
        let kernel = make_test_kernel(10, 5);
        let manifest = test_manifest("test-agent");
        let principal = Principal::user("test-user");
        let session_id = SessionId::new("test-session");

        // spawn will fail because there's no real LLM provider,
        // but the process should be created in the table
        let handle = kernel
            .spawn(manifest, "hello".to_string(), principal, session_id, None)
            .await;

        // The spawn itself should succeed (it just launches a task)
        assert!(handle.is_ok());
        let handle = handle.unwrap();

        // Process should appear in the table
        let process = kernel.process_table().get(handle.agent_id);
        assert!(process.is_some());
        let process = process.unwrap();
        assert_eq!(process.manifest.name, "test-agent");
        // It's running (or may have already failed due to no LLM, but was created as Running)
    }

    #[tokio::test]
    async fn test_kernel_spawn_global_limit() {
        let kernel = make_test_kernel(2, 5);
        let principal = Principal::user("test-user");
        let session_id = SessionId::new("test-session");

        // Spawn 2 agents (global limit is 2)
        let h1 = kernel
            .spawn(
                test_manifest("a1"),
                "task 1".to_string(),
                principal.clone(),
                session_id.clone(),
                None,
            )
            .await;
        assert!(h1.is_ok());

        let h2 = kernel
            .spawn(
                test_manifest("a2"),
                "task 2".to_string(),
                principal.clone(),
                session_id.clone(),
                None,
            )
            .await;
        assert!(h2.is_ok());

        // Third spawn should fail (global limit reached)
        let h3 = kernel
            .spawn(
                test_manifest("a3"),
                "task 3".to_string(),
                principal,
                session_id,
                None,
            )
            .await;
        assert!(h3.is_err());
        let err = h3.unwrap_err();
        assert!(
            err.to_string().contains("global concurrency limit"),
            "expected global limit error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_kernel_spawn_named_success() {
        let kernel = make_test_kernel(10, 5);
        let principal = Principal::user("test-user");
        let session_id = SessionId::new("test-session");

        let handle = kernel
            .spawn_named(
                "scout",
                "find something".to_string(),
                principal,
                session_id,
                None,
            )
            .await;
        assert!(handle.is_ok());
    }

    #[tokio::test]
    async fn test_kernel_spawn_named_not_found() {
        let kernel = make_test_kernel(10, 5);
        let principal = Principal::user("test-user");
        let session_id = SessionId::new("test-session");

        let result = kernel
            .spawn_named(
                "nonexistent",
                "task".to_string(),
                principal,
                session_id,
                None,
            )
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("manifest not found"));
    }

    #[tokio::test]
    async fn test_kernel_spawn_with_parent() {
        let kernel = make_test_kernel(10, 5);
        let principal = Principal::user("test-user");
        let session_id = SessionId::new("test-session");

        let parent_handle = kernel
            .spawn(
                test_manifest("parent"),
                "parent task".to_string(),
                principal.clone(),
                session_id.clone(),
                None,
            )
            .await
            .unwrap();

        let child_handle = kernel
            .spawn(
                test_manifest("child"),
                "child task".to_string(),
                principal,
                session_id,
                Some(parent_handle.agent_id),
            )
            .await
            .unwrap();

        let child_process = kernel.process_table().get(child_handle.agent_id).unwrap();
        assert_eq!(child_process.parent_id, Some(parent_handle.agent_id));

        let children = kernel
            .process_table()
            .children_of(parent_handle.agent_id);
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].agent_id, child_handle.agent_id);
    }
}
