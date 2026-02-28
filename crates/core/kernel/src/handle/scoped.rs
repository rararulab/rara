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
//! Trait implementations: ProcessOps, MemoryOps, EventOps, GuardOps.

use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use jiff::Timestamp;
use tokio::sync::{Semaphore, oneshot};
use tracing::{info, warn};

use super::{AgentHandle, EventOps, GuardOps, MemoryOps, ProcessOps};
use crate::{
    error::{KernelError, Result},
    event::{EventBus, KernelEvent},
    guard::{Guard, GuardContext},
    memory::Memory,
    process::{
        AgentEnv, AgentId, AgentManifest, AgentProcess, AgentResult, ProcessInfo, ProcessState,
        ProcessTable, SessionId, manifest_loader::ManifestLoader, principal::Principal,
    },
    provider::LlmProviderLoaderRef,
    runner::{AgentRunner, UserContent},
    tool::ToolRegistry,
};

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
    pub(crate) agent_id:        AgentId,
    /// The session this agent belongs to.
    pub(crate) session_id:      SessionId,
    /// The identity under which this agent runs.
    pub(crate) principal:       Principal,
    /// Tools this agent is allowed to use (children can only subset these).
    pub(crate) allowed_tools:   Vec<String>,
    /// Per-agent semaphore limiting concurrent child processes.
    pub(crate) child_semaphore: Arc<Semaphore>,
    /// Shared kernel internals (process table, global limits, etc.).
    pub(crate) inner:           Arc<KernelInner>,
}

/// Shared kernel state that `ScopedKernelHandle` delegates to.
///
/// This struct holds the "real" kernel state shared by all handles via `Arc`.
pub(crate) struct KernelInner {
    /// The global process table tracking all running agents.
    pub process_table:          Arc<ProcessTable>,
    /// Global semaphore limiting total concurrent agent processes.
    pub global_semaphore:       Arc<Semaphore>,
    /// Default maximum number of children per agent.
    pub default_child_limit:    usize,
    /// Default max LLM iterations for spawned agents.
    pub default_max_iterations: usize,
    /// LLM provider loader for acquiring providers.
    pub llm_provider:           LlmProviderLoaderRef,
    /// Global tool registry (spawned agents get filtered subsets).
    pub tool_registry:          Arc<ToolRegistry>,
    /// 3-layer memory (not used for cross-agent KV — see shared_kv).
    pub memory:                 Arc<dyn Memory>,
    /// Event bus for publishing kernel events.
    pub event_bus:              Arc<dyn EventBus>,
    /// Guard for tool approval checks.
    pub guard:                  Arc<dyn Guard>,
    /// Manifest loader for looking up named agent definitions.
    pub manifest_loader:        ManifestLoader,
    /// Cross-agent shared key-value store (simple DashMap).
    pub shared_kv:              DashMap<String, serde_json::Value>,
    /// User store for user management and permission validation.
    pub user_store:             Arc<dyn crate::process::user::UserStore>,
    /// Session repository for conversation history (used by process_loop).
    pub session_repo:           Option<Arc<dyn crate::session::SessionRepository>>,
    /// Stream hub for real-time streaming events (used by process_loop).
    pub stream_hub:             Option<Arc<crate::io::stream::StreamHub>>,
    /// Outbound bus for publishing final responses (used by process_loop).
    pub outbound_bus:           Option<Arc<dyn crate::io::bus::OutboundBus>>,
}

/// Parameters for spawning an agent process via [`KernelInner::spawn_process`].
pub(crate) struct SpawnParams {
    pub manifest:    AgentManifest,
    pub input:       String,
    pub principal:   Principal,
    pub session_id:  SessionId,
    pub parent_id:   Option<AgentId>,
    /// Pre-filtered tool registry for this agent.
    pub agent_tools: ToolRegistry,
}

impl KernelInner {
    /// Validate that the principal's user exists, is enabled, and has Spawn
    /// permission.
    ///
    /// Called by both `Kernel::spawn()` and `ScopedKernelHandle::spawn()`.
    pub(crate) async fn validate_principal(&self, principal: &Principal) -> Result<()> {
        let user = self
            .user_store
            .get_by_name(&principal.user_id.0)
            .await?
            .ok_or(KernelError::UserNotFound {
                name: principal.user_id.0.clone(),
            })?;
        if !user.enabled {
            return Err(KernelError::UserDisabled { name: user.name });
        }
        if !user.has_permission(&crate::process::user::Permission::Spawn) {
            return Err(KernelError::PermissionDenied {
                reason: format!("user '{}' lacks Spawn permission", user.name),
            });
        }
        Ok(())
    }

    /// Core spawn logic shared by `Kernel::spawn()` and
    /// `ScopedKernelHandle::spawn()`.
    ///
    /// Creates the agent process, builds and runs the AgentRunner in a tokio
    /// task, and returns an `AgentHandle`. The caller is responsible for
    /// acquiring semaphore permits and passing in the pre-filtered tool
    /// registry.
    ///
    /// # Arguments
    /// - `self_ref` — `Arc<KernelInner>` for the spawned task to hold
    /// - `params` — spawn parameters (manifest, input, principal, etc.)
    /// - `child_limit` — max children for the spawned agent
    /// - `permits` — semaphore permits to hold for the lifetime of the task
    pub(crate) fn spawn_process(
        self_ref: Arc<KernelInner>,
        params: SpawnParams,
        child_limit: usize,
        permits: SpawnPermits,
    ) -> AgentHandle {
        let agent_id = AgentId::new();
        let max_iterations = params
            .manifest
            .max_iterations
            .unwrap_or(self_ref.default_max_iterations);

        let process = AgentProcess {
            agent_id,
            parent_id: params.parent_id,
            session_id: params.session_id.clone(),
            manifest: params.manifest.clone(),
            principal: params.principal.clone(),
            env: AgentEnv::default(),
            state: ProcessState::Running,
            created_at: Timestamp::now(),
            finished_at: None,
            result: None,
        };
        self_ref.process_table.insert(process);

        let tool_names: Vec<String> = params.agent_tools.tool_names();

        let _scoped_handle = Arc::new(ScopedKernelHandle {
            agent_id,
            session_id: params.session_id,
            principal: params.principal,
            allowed_tools: tool_names,
            child_semaphore: Arc::new(Semaphore::new(child_limit)),
            inner: Arc::clone(&self_ref),
        });

        let model_name = params.manifest.model.clone();
        let system_prompt = params.manifest.system_prompt.clone();
        let provider_hint = params.manifest.provider_hint.clone().unwrap_or_default();
        let llm_provider = Arc::clone(&self_ref.llm_provider);

        let runner = AgentRunner::builder()
            .llm_provider(llm_provider)
            .provider_hint(provider_hint)
            .model_name(model_name)
            .system_prompt(system_prompt)
            .user_content(UserContent::Text(params.input))
            .max_iterations(max_iterations)
            .build();

        let (result_tx, result_rx) = oneshot::channel();
        // Create a mailbox channel. For child (short-lived) spawns, the
        // receiver is not used — the agent runs once and completes.
        let (mailbox_tx, _mailbox_rx) = tokio::sync::mpsc::channel(16);
        let process_table = Arc::clone(&self_ref.process_table);
        let task_inner = Arc::clone(&self_ref);

        let join_handle = tokio::spawn(async move {
            let _permits = permits;
            let _scoped_handle = _scoped_handle;

            let run_result = runner.run(&params.agent_tools, None).await;

            match run_result {
                Ok(response) => {
                    let agent_result = AgentResult {
                        output:     response.response_text(),
                        iterations: response.iterations,
                        tool_calls: response.tool_calls_made,
                    };

                    let _ = task_inner
                        .process_table
                        .set_state(agent_id, ProcessState::Completed);
                    let _ = task_inner
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
                    warn!(
                        agent_id = %agent_id,
                        error = %err,
                        "agent process failed"
                    );

                    let _ = task_inner
                        .process_table
                        .set_state(agent_id, ProcessState::Failed);

                    let error_result = AgentResult {
                        output:     format!("Error: {err}"),
                        iterations: 0,
                        tool_calls: 0,
                    };
                    let _ = task_inner
                        .process_table
                        .set_result(agent_id, error_result.clone());
                    let _ = result_tx.send(error_result);
                }
            }

            process_table.clear_abort_handle(&agent_id);
        });

        self_ref
            .process_table
            .set_abort_handle(agent_id, join_handle.abort_handle());

        AgentHandle {
            agent_id,
            mailbox: mailbox_tx,
            result_rx,
        }
    }
}

/// Semaphore permits that must be held for the lifetime of the spawned task.
///
/// This enum accommodates both top-level spawns (global permit only) and
/// child spawns (global + child permit).
pub(crate) enum SpawnPermits {
    /// Top-level spawn — only a global permit.
    TopLevel {
        _global: tokio::sync::OwnedSemaphorePermit,
    },
    /// Child spawn — both a child and global permit.
    Child {
        _child:  tokio::sync::OwnedSemaphorePermit,
        _global: tokio::sync::OwnedSemaphorePermit,
    },
}

impl ScopedKernelHandle {
    /// The agent ID this handle belongs to.
    pub fn agent_id(&self) -> AgentId { self.agent_id }

    /// The principal (identity) of this agent.
    pub fn principal(&self) -> &Principal { &self.principal }

    /// The tools this agent is allowed to use.
    pub fn allowed_tools(&self) -> &[String] { &self.allowed_tools }

    /// Validate that all requested child tools are a subset of this agent's
    /// tools.
    fn validate_tool_subset(&self, child_tools: &[String]) -> Result<()> {
        for tool_name in child_tools {
            if !self.allowed_tools.is_empty() && !self.allowed_tools.iter().any(|t| t == tool_name)
            {
                return Err(KernelError::ToolNotAllowed {
                    tool_name: tool_name.clone(),
                });
            }
        }
        Ok(())
    }

    /// Build a filtered ToolRegistry for a child based on manifest tools.
    fn build_child_tool_registry(&self, manifest_tools: &[String]) -> ToolRegistry {
        if manifest_tools.is_empty() {
            // Inherit parent's tools
            self.inner.tool_registry.filtered(&self.allowed_tools)
        } else {
            // Intersect manifest tools with parent's allowed tools
            let effective_tools: Vec<String> = if self.allowed_tools.is_empty() {
                manifest_tools.to_vec()
            } else {
                manifest_tools
                    .iter()
                    .filter(|t| self.allowed_tools.iter().any(|a| a == *t))
                    .cloned()
                    .collect()
            };
            self.inner.tool_registry.filtered(&effective_tools)
        }
    }

    /// Recursively kill a process and all its children.
    fn kill_recursive(&self, target_id: AgentId) -> Result<()> {
        // First kill all children
        let children = self.inner.process_table.children_of(target_id);
        for child in children {
            self.kill_recursive(child.agent_id)?;
        }
        // Prefer a graceful mailbox shutdown for long-lived processes.
        let mut delivered_signal = false;
        if let Some(tx) = self.inner.process_table.get_mailbox(&target_id) {
            match tx.try_send(crate::process::ProcessMessage::Signal(
                crate::process::Signal::Kill,
            )) {
                Ok(()) => {
                    delivered_signal = true;
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_))
                | Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {}
            }
        }

        // Short-lived processes have no mailbox loop, so abort the task directly.
        if !delivered_signal {
            let _ = self.inner.process_table.abort(&target_id);
        }

        // Then mark the target
        self.inner
            .process_table
            .set_state(target_id, ProcessState::Cancelled)?;
        Ok(())
    }
}

#[async_trait]
impl ProcessOps for ScopedKernelHandle {
    /// Spawn a child agent.
    ///
    /// - Validates child tools are a subset of parent's tools
    /// - Acquires per-agent child semaphore (limits concurrent children)
    /// - Acquires global semaphore (limits total concurrent agents)
    /// - Creates AgentProcess in process table
    /// - Builds filtered ToolRegistry for the child
    /// - Spawns tokio task running AgentRunner
    /// - Returns AgentHandle with oneshot receiver
    async fn spawn(&self, manifest: AgentManifest, input: String) -> Result<AgentHandle> {
        // 1. Validate tool subset
        if !manifest.tools.is_empty() {
            self.validate_tool_subset(&manifest.tools)?;
        }

        // 1.5 Validate principal (user may have been disabled after top-level spawn)
        self.inner.validate_principal(&self.principal).await?;

        // 2. Acquire per-agent child semaphore (non-blocking try)
        let child_permit = self
            .child_semaphore
            .clone()
            .try_acquire_owned()
            .map_err(|_| KernelError::SpawnLimitReached {
                message: format!("agent {} reached max child limit", self.agent_id),
            })?;

        // 3. Acquire global semaphore (non-blocking try)
        let global_permit = self
            .inner
            .global_semaphore
            .clone()
            .try_acquire_owned()
            .map_err(|_| KernelError::SpawnLimitReached {
                message: "global concurrency limit reached".to_string(),
            })?;

        // 4. Build filtered ToolRegistry
        let agent_tools = self.build_child_tool_registry(&manifest.tools);
        let child_limit = manifest
            .max_children
            .unwrap_or(self.inner.default_child_limit);

        // 5. Delegate to shared spawn logic
        let handle = KernelInner::spawn_process(
            Arc::clone(&self.inner),
            SpawnParams {
                manifest,
                input,
                principal: self.principal.clone(),
                session_id: self.session_id.clone(),
                parent_id: Some(self.agent_id),
                agent_tools,
            },
            child_limit,
            SpawnPermits::Child {
                _child:  child_permit,
                _global: global_permit,
            },
        );

        Ok(handle)
    }

    async fn send(&self, _agent_id: AgentId, _message: String) -> Result<String> {
        Err(KernelError::Other {
            message: "send not yet implemented".into(),
        })
    }

    fn status(&self, agent_id: AgentId) -> Result<ProcessInfo> {
        self.inner
            .process_table
            .get(agent_id)
            .map(|p| ProcessInfo::from(&p))
            .ok_or(KernelError::ProcessNotFound {
                id: agent_id.to_string(),
            })
    }

    fn kill(&self, agent_id: AgentId) -> Result<()> { self.kill_recursive(agent_id) }

    fn children(&self) -> Vec<ProcessInfo> { self.inner.process_table.children_of(self.agent_id) }
}

impl MemoryOps for ScopedKernelHandle {
    fn mem_store(&self, key: &str, value: serde_json::Value) -> Result<()> {
        self.inner.shared_kv.insert(key.to_string(), value);
        Ok(())
    }

    fn mem_recall(&self, key: &str) -> Result<Option<serde_json::Value>> {
        Ok(self.inner.shared_kv.get(key).map(|v| v.value().clone()))
    }
}

#[async_trait]
impl EventOps for ScopedKernelHandle {
    async fn publish(&self, event_type: &str, _payload: serde_json::Value) -> Result<()> {
        // Map to a KernelEvent. We use ToolExecuted as a generic carrier for now.
        // In a more complete implementation, we would add custom event types.
        self.inner
            .event_bus
            .publish(KernelEvent::ToolExecuted {
                agent_id:  self.agent_id.0,
                tool_name: format!("event:{event_type}"),
                success:   true,
                timestamp: Timestamp::now(),
            })
            .await;
        Ok(())
    }
}

#[async_trait]
impl GuardOps for ScopedKernelHandle {
    fn requires_approval(&self, _tool_name: &str) -> bool {
        // We cannot call async check_tool synchronously, so we use a heuristic:
        // For now, return false (no approval required by default).
        // A more sophisticated implementation would maintain a cached policy.
        false
    }

    async fn request_approval(&self, tool_name: &str, summary: &str) -> Result<bool> {
        let guard_ctx = GuardContext {
            agent_id:   self.agent_id.0,
            user_id:    uuid::Uuid::nil(),
            session_id: uuid::Uuid::nil(),
        };
        let verdict = self
            .inner
            .guard
            .check_tool(
                &guard_ctx,
                tool_name,
                &serde_json::json!({"summary": summary}),
            )
            .await;
        Ok(verdict.is_allow())
    }
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc;

    use super::*;
    use crate::process::{ProcessMessage, Signal, principal::Principal};

    fn make_kernel_inner() -> Arc<KernelInner> {
        use crate::{
            defaults::{
                noop::{NoopEventBus, NoopGuard, NoopMemory},
                noop_user_store::NoopUserStore,
            },
            provider::EnvLlmProviderLoader,
        };

        Arc::new(KernelInner {
            process_table:          Arc::new(ProcessTable::new()),
            global_semaphore:       Arc::new(Semaphore::new(10)),
            default_child_limit:    5,
            default_max_iterations: 25,
            llm_provider:           Arc::new(EnvLlmProviderLoader::default())
                as LlmProviderLoaderRef,
            tool_registry:          Arc::new(ToolRegistry::new()),
            memory:                 Arc::new(NoopMemory),
            event_bus:              Arc::new(NoopEventBus),
            guard:                  Arc::new(NoopGuard),
            manifest_loader:        ManifestLoader::new(),
            shared_kv:              DashMap::new(),
            user_store:             Arc::new(NoopUserStore),
            session_repo:           None,
            stream_hub:             None,
            outbound_bus:           None,
        })
    }

    #[test]
    fn test_scoped_handle_accessors() {
        let agent_id = AgentId::new();
        let principal = Principal::user("test-user");
        let tools = vec!["read_file".to_string(), "grep".to_string()];

        let handle = ScopedKernelHandle {
            agent_id,
            session_id: SessionId::new("test-session"),
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

        let handle1 = ScopedKernelHandle {
            agent_id:        AgentId::new(),
            session_id:      SessionId::new("session-1"),
            principal:       Principal::user("user-1"),
            allowed_tools:   vec![],
            child_semaphore: Arc::new(Semaphore::new(3)),
            inner:           Arc::clone(&inner),
        };

        let handle2 = ScopedKernelHandle {
            agent_id:        AgentId::new(),
            session_id:      SessionId::new("session-2"),
            principal:       Principal::admin("admin-1"),
            allowed_tools:   vec!["bash".to_string()],
            child_semaphore: Arc::new(Semaphore::new(3)),
            inner:           Arc::clone(&inner),
        };

        assert_eq!(handle1.inner.process_table.list().len(), 0);
        assert_eq!(handle2.inner.process_table.list().len(), 0);
        assert_ne!(handle1.agent_id(), handle2.agent_id());
        assert!(!handle1.principal().is_admin());
        assert!(handle2.principal().is_admin());
    }

    #[test]
    fn test_validate_tool_subset_ok() {
        let handle = ScopedKernelHandle {
            agent_id:        AgentId::new(),
            session_id:      SessionId::new("test"),
            principal:       Principal::user("test"),
            allowed_tools:   vec![
                "read_file".to_string(),
                "grep".to_string(),
                "bash".to_string(),
            ],
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner:           make_kernel_inner(),
        };

        assert!(
            handle
                .validate_tool_subset(&["read_file".to_string(), "grep".to_string()])
                .is_ok()
        );
    }

    #[tokio::test]
    async fn test_kill_sends_kill_signal_to_mailbox() {
        let inner = make_kernel_inner();
        let agent_id = AgentId::new();

        inner.process_table.insert(AgentProcess {
            agent_id,
            parent_id: None,
            session_id: SessionId::new("test"),
            manifest: test_manifest("mailbox-agent"),
            principal: Principal::user("test"),
            env: AgentEnv::default(),
            state: ProcessState::Running,
            created_at: Timestamp::now(),
            finished_at: None,
            result: None,
        });

        let (tx, mut rx) = mpsc::channel(1);
        inner.process_table.set_mailbox(agent_id, tx);

        let handle = ScopedKernelHandle {
            agent_id:        AgentId::new(),
            session_id:      SessionId::new("test"),
            principal:       Principal::user("test"),
            allowed_tools:   vec![],
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner:           Arc::clone(&inner),
        };

        handle.kill(agent_id).unwrap();

        let received = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .expect("kill should deliver promptly");
        assert!(matches!(
            received,
            Some(ProcessMessage::Signal(Signal::Kill))
        ));
    }

    #[test]
    fn test_validate_tool_subset_denied() {
        let handle = ScopedKernelHandle {
            agent_id:        AgentId::new(),
            session_id:      SessionId::new("test"),
            principal:       Principal::user("test"),
            allowed_tools:   vec!["read_file".to_string()],
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner:           make_kernel_inner(),
        };

        let result = handle.validate_tool_subset(&["bash".to_string()]);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_tool_subset_empty_parent_allows_all() {
        let handle = ScopedKernelHandle {
            agent_id:        AgentId::new(),
            session_id:      SessionId::new("test"),
            principal:       Principal::user("test"),
            allowed_tools:   vec![], // empty = no restriction
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner:           make_kernel_inner(),
        };

        assert!(
            handle
                .validate_tool_subset(&["anything".to_string()])
                .is_ok()
        );
    }

    #[test]
    fn test_memory_ops_store_recall() {
        let inner = make_kernel_inner();
        let handle = ScopedKernelHandle {
            agent_id:        AgentId::new(),
            session_id:      SessionId::new("test"),
            principal:       Principal::user("test"),
            allowed_tools:   vec![],
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner:           Arc::clone(&inner),
        };

        handle
            .mem_store("key1", serde_json::json!({"data": 42}))
            .unwrap();

        let recalled = handle.mem_recall("key1").unwrap();
        assert!(recalled.is_some());
        assert_eq!(recalled.unwrap()["data"], 42);

        let missing = handle.mem_recall("nonexistent").unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn test_memory_ops_shared_across_handles() {
        let inner = make_kernel_inner();

        let handle1 = ScopedKernelHandle {
            agent_id:        AgentId::new(),
            session_id:      SessionId::new("test"),
            principal:       Principal::user("user-1"),
            allowed_tools:   vec![],
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner:           Arc::clone(&inner),
        };

        let handle2 = ScopedKernelHandle {
            agent_id:        AgentId::new(),
            session_id:      SessionId::new("test"),
            principal:       Principal::user("user-2"),
            allowed_tools:   vec![],
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner:           Arc::clone(&inner),
        };

        handle1
            .mem_store("shared_key", serde_json::json!("hello"))
            .unwrap();
        let recalled = handle2.mem_recall("shared_key").unwrap();
        assert_eq!(recalled.unwrap(), serde_json::json!("hello"));
    }

    #[test]
    fn test_kill_recursive() {
        let inner = make_kernel_inner();
        let parent_id = AgentId::new();
        let child1_id = AgentId::new();
        let child2_id = AgentId::new();
        let grandchild_id = AgentId::new();

        // Insert parent
        inner.process_table.insert(AgentProcess {
            agent_id:    parent_id,
            parent_id:   None,
            session_id:  SessionId::new("test"),
            manifest:    test_manifest("parent"),
            principal:   Principal::user("test"),
            env:         AgentEnv::default(),
            state:       ProcessState::Running,
            created_at:  Timestamp::now(),
            finished_at: None,
            result:      None,
        });

        // Insert children
        inner.process_table.insert(AgentProcess {
            agent_id:    child1_id,
            parent_id:   Some(parent_id),
            session_id:  SessionId::new("test"),
            manifest:    test_manifest("child1"),
            principal:   Principal::user("test"),
            env:         AgentEnv::default(),
            state:       ProcessState::Running,
            created_at:  Timestamp::now(),
            finished_at: None,
            result:      None,
        });

        inner.process_table.insert(AgentProcess {
            agent_id:    child2_id,
            parent_id:   Some(parent_id),
            session_id:  SessionId::new("test"),
            manifest:    test_manifest("child2"),
            principal:   Principal::user("test"),
            env:         AgentEnv::default(),
            state:       ProcessState::Running,
            created_at:  Timestamp::now(),
            finished_at: None,
            result:      None,
        });

        // Insert grandchild
        inner.process_table.insert(AgentProcess {
            agent_id:    grandchild_id,
            parent_id:   Some(child1_id),
            session_id:  SessionId::new("test"),
            manifest:    test_manifest("grandchild"),
            principal:   Principal::user("test"),
            env:         AgentEnv::default(),
            state:       ProcessState::Running,
            created_at:  Timestamp::now(),
            finished_at: None,
            result:      None,
        });

        let handle = ScopedKernelHandle {
            agent_id:        AgentId::new(), // some external caller
            session_id:      SessionId::new("test"),
            principal:       Principal::user("test"),
            allowed_tools:   vec![],
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner:           Arc::clone(&inner),
        };

        // Kill parent — should cascade to children and grandchild
        handle.kill_recursive(parent_id).unwrap();

        assert_eq!(
            inner.process_table.get(parent_id).unwrap().state,
            ProcessState::Cancelled
        );
        assert_eq!(
            inner.process_table.get(child1_id).unwrap().state,
            ProcessState::Cancelled
        );
        assert_eq!(
            inner.process_table.get(child2_id).unwrap().state,
            ProcessState::Cancelled
        );
        assert_eq!(
            inner.process_table.get(grandchild_id).unwrap().state,
            ProcessState::Cancelled
        );
    }

    #[test]
    fn test_status_found() {
        let inner = make_kernel_inner();
        let agent_id = AgentId::new();

        inner.process_table.insert(AgentProcess {
            agent_id,
            parent_id: None,
            session_id: SessionId::new("test"),
            manifest: test_manifest("scout"),
            principal: Principal::user("test"),
            env: AgentEnv::default(),
            state: ProcessState::Running,
            created_at: Timestamp::now(),
            finished_at: None,
            result: None,
        });

        let handle = ScopedKernelHandle {
            agent_id: AgentId::new(),
            session_id: SessionId::new("test"),
            principal: Principal::user("test"),
            allowed_tools: vec![],
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner,
        };

        let info = handle.status(agent_id).unwrap();
        assert_eq!(info.name, "scout");
        assert_eq!(info.state, ProcessState::Running);
    }

    #[test]
    fn test_status_not_found() {
        let inner = make_kernel_inner();
        let handle = ScopedKernelHandle {
            agent_id: AgentId::new(),
            session_id: SessionId::new("test"),
            principal: Principal::user("test"),
            allowed_tools: vec![],
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner,
        };

        assert!(handle.status(AgentId::new()).is_err());
    }

    #[test]
    fn test_children_empty() {
        let inner = make_kernel_inner();
        let handle = ScopedKernelHandle {
            agent_id: AgentId::new(),
            session_id: SessionId::new("test"),
            principal: Principal::user("test"),
            allowed_tools: vec![],
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner,
        };

        assert!(handle.children().is_empty());
    }

    #[tokio::test]
    async fn test_guard_ops_request_approval() {
        let inner = make_kernel_inner();
        let handle = ScopedKernelHandle {
            agent_id: AgentId::new(),
            session_id: SessionId::new("test"),
            principal: Principal::user("test"),
            allowed_tools: vec![],
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner,
        };

        // NoopGuard always allows
        let approved = handle
            .request_approval("bash", "run a command")
            .await
            .unwrap();
        assert!(approved);
    }

    #[tokio::test]
    async fn test_event_ops_publish() {
        let inner = make_kernel_inner();
        let handle = ScopedKernelHandle {
            agent_id: AgentId::new(),
            session_id: SessionId::new("test"),
            principal: Principal::user("test"),
            allowed_tools: vec![],
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner,
        };

        // Should not panic — NoopEventBus just swallows events
        handle
            .publish("test_event", serde_json::json!({"key": "value"}))
            .await
            .unwrap();
    }

    /// Helper to create a test manifest.
    fn test_manifest(name: &str) -> AgentManifest {
        AgentManifest {
            name:           name.to_string(),
            description:    format!("Test agent: {name}"),
            model:          "test-model".to_string(),
            system_prompt:  "You are a test agent.".to_string(),
            provider_hint:  None,
            max_iterations: Some(10),
            tools:          vec!["read_file".to_string()],
            max_children:   None,
            metadata:       serde_json::Value::Null,
        }
    }
}
