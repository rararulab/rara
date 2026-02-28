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
use jiff::Timestamp;
use tokio::sync::Semaphore;
use super::{AgentHandle, EventOps, GuardOps, MemoryOps, ProcessOps};
use crate::{
    error::{KernelError, Result},
    event::KernelEvent,
    guard::GuardContext,
    io::types::InboundMessage,
    kernel::{KernelInner, SpawnPermits},
    process::{
        AgentId, AgentManifest, ProcessInfo, ProcessState, SessionId,
        principal::Principal,
    },
    provider::LlmProviderLoaderRef,
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
    /// The manifest describing this agent (model, system_prompt, etc.).
    pub(crate) manifest:        AgentManifest,
    /// Tools this agent is allowed to use (children can only subset these).
    pub(crate) allowed_tools:   Vec<String>,
    /// Effective tool registry for this agent (filtered from global for children).
    pub(crate) tool_registry:   Arc<ToolRegistry>,
    /// Per-agent semaphore limiting concurrent child processes.
    pub(crate) child_semaphore: Arc<Semaphore>,
    /// Shared kernel internals (process table, global limits, etc.).
    pub(crate) inner:           Arc<KernelInner>,
}

impl ScopedKernelHandle {
    /// The agent ID this handle belongs to.
    pub fn agent_id(&self) -> AgentId { self.agent_id }

    /// The principal (identity) of this agent.
    pub fn principal(&self) -> &Principal { &self.principal }

    /// The tools this agent is allowed to use.
    pub fn allowed_tools(&self) -> &[String] { &self.allowed_tools }

    // ---- Agent execution accessors (used by run_agent_turn) ----

    /// The manifest for this agent.
    pub(crate) fn manifest(&self) -> &AgentManifest { &self.manifest }

    /// The session ID this agent belongs to.
    pub(crate) fn session_id(&self) -> &SessionId { &self.session_id }

    /// The LLM provider loader.
    pub(crate) fn llm_provider(&self) -> &LlmProviderLoaderRef { &self.inner.llm_provider }

    /// The effective tool registry for this agent.
    pub(crate) fn tool_registry(&self) -> &Arc<ToolRegistry> { &self.tool_registry }

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

    /// Compute the effective allowed_tools for a child agent.
    fn effective_child_tools(&self, manifest_tools: &[String]) -> Vec<String> {
        if manifest_tools.is_empty() {
            // Inherit parent's tools
            self.allowed_tools.clone()
        } else if self.allowed_tools.is_empty() {
            // Parent has no restriction
            manifest_tools.to_vec()
        } else {
            // Intersect manifest tools with parent's allowed tools
            manifest_tools
                .iter()
                .filter(|t| self.allowed_tools.iter().any(|a| a == *t))
                .cloned()
                .collect()
        }
    }

    /// Cancel a process via its CancellationToken.
    ///
    /// Child tokens are automatically cancelled by tokio_util's hierarchy,
    /// so there is no need to recurse the process tree.
    fn cancel_process(&self, target_id: AgentId) -> Result<()> {
        if let Some(token) = self.inner.process_table.get_cancellation_token(&target_id) {
            token.cancel();
        }
        // Mark state immediately for callers that check right after kill().
        // The process loop will also set Cancelled when it detects the token,
        // but this ensures immediate visibility.
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
    /// - Wraps input as InboundMessage::synthetic
    /// - Delegates to `KernelInner::spawn_process()`
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

        // 4. Compute effective child tools
        let allowed_tools = self.effective_child_tools(&manifest.tools);
        let child_limit = manifest
            .max_children
            .unwrap_or(self.inner.default_child_limit);

        // 5. Wrap input as InboundMessage
        let inbound = InboundMessage::synthetic(
            input,
            self.principal.user_id.clone(),
            self.session_id.clone(),
        );

        // 6. Delegate to unified spawn
        KernelInner::spawn_process(
            Arc::clone(&self.inner),
            manifest,
            inbound,
            self.principal.clone(),
            self.session_id.clone(),
            Some(self.agent_id),
            child_limit,
            allowed_tools,
            SpawnPermits::Child {
                _child:  child_permit,
                _global: global_permit,
            },
        )
        .await
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

    fn kill(&self, agent_id: AgentId) -> Result<()> { self.cancel_process(agent_id) }

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
    use dashmap::DashMap;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::process::{
        AgentEnv, AgentProcess, ProcessTable,
        manifest_loader::ManifestLoader, principal::Principal,
    };

    fn make_kernel_inner() -> Arc<KernelInner> {
        use crate::{
            defaults::{
                noop::{NoopEventBus, NoopGuard, NoopMemory, NoopSessionRepository},
                noop_user_store::NoopUserStore,
            },
            io::{memory_bus::InMemoryOutboundBus, stream::StreamHub},
            provider::EnvLlmProviderLoader,
            session::SessionRepository,
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
            session_repo:           Arc::new(NoopSessionRepository) as Arc<dyn SessionRepository>,
            stream_hub:             Arc::new(StreamHub::new(1)),
            outbound_bus:           Arc::new(InMemoryOutboundBus::new(1))
                as Arc<dyn crate::io::bus::OutboundBus>,
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
            manifest: test_manifest("test"),
            allowed_tools: tools.clone(),
            tool_registry: Arc::new(ToolRegistry::new()),
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
            manifest:        test_manifest("test-1"),
            allowed_tools:   vec![],
            tool_registry:   Arc::new(ToolRegistry::new()),
            child_semaphore: Arc::new(Semaphore::new(3)),
            inner:           Arc::clone(&inner),
        };

        let handle2 = ScopedKernelHandle {
            agent_id:        AgentId::new(),
            session_id:      SessionId::new("session-2"),
            principal:       Principal::admin("admin-1"),
            manifest:        test_manifest("test-2"),
            allowed_tools:   vec!["bash".to_string()],
            tool_registry:   Arc::new(ToolRegistry::new()),
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
            manifest:        test_manifest("test"),
            allowed_tools:   vec![
                "read_file".to_string(),
                "grep".to_string(),
                "bash".to_string(),
            ],
            tool_registry:   Arc::new(ToolRegistry::new()),
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner:           make_kernel_inner(),
        };

        assert!(
            handle
                .validate_tool_subset(&["read_file".to_string(), "grep".to_string()])
                .is_ok()
        );
    }

    #[test]
    fn test_kill_cancels_token() {
        let inner = make_kernel_inner();
        let agent_id = AgentId::new();

        inner.process_table.insert(AgentProcess {
            agent_id,
            parent_id: None,
            session_id: SessionId::new("test"),
            manifest: test_manifest("token-agent"),
            principal: Principal::user("test"),
            env: AgentEnv::default(),
            state: ProcessState::Running,
            created_at: Timestamp::now(),
            finished_at: None,
            result: None,
        });

        let token = CancellationToken::new();
        inner
            .process_table
            .set_cancellation_token(agent_id, token.clone());

        let handle = ScopedKernelHandle {
            agent_id:        AgentId::new(),
            session_id:      SessionId::new("test"),
            principal:       Principal::user("test"),
            manifest:        test_manifest("test"),
            allowed_tools:   vec![],
            tool_registry:   Arc::new(ToolRegistry::new()),
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner:           Arc::clone(&inner),
        };

        assert!(!token.is_cancelled());
        handle.kill(agent_id).unwrap();
        assert!(token.is_cancelled());
        assert_eq!(
            inner.process_table.get(agent_id).unwrap().state,
            ProcessState::Cancelled
        );
    }

    #[test]
    fn test_validate_tool_subset_denied() {
        let handle = ScopedKernelHandle {
            agent_id:        AgentId::new(),
            session_id:      SessionId::new("test"),
            principal:       Principal::user("test"),
            manifest:        test_manifest("test"),
            allowed_tools:   vec!["read_file".to_string()],
            tool_registry:   Arc::new(ToolRegistry::new()),
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
            manifest:        test_manifest("test"),
            allowed_tools:   vec![], // empty = no restriction
            tool_registry:   Arc::new(ToolRegistry::new()),
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
            manifest:        test_manifest("test"),
            allowed_tools:   vec![],
            tool_registry:   Arc::new(ToolRegistry::new()),
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
            manifest:        test_manifest("test"),
            allowed_tools:   vec![],
            tool_registry:   Arc::new(ToolRegistry::new()),
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner:           Arc::clone(&inner),
        };

        let handle2 = ScopedKernelHandle {
            agent_id:        AgentId::new(),
            session_id:      SessionId::new("test"),
            principal:       Principal::user("user-2"),
            manifest:        test_manifest("test"),
            allowed_tools:   vec![],
            tool_registry:   Arc::new(ToolRegistry::new()),
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
    fn test_kill_cascades_via_cancellation_token() {
        let inner = make_kernel_inner();
        let parent_id = AgentId::new();
        let child1_id = AgentId::new();
        let child2_id = AgentId::new();
        let grandchild_id = AgentId::new();

        // Build token hierarchy: parent → child1/child2, child1 → grandchild
        let parent_token = CancellationToken::new();
        let child1_token = parent_token.child_token();
        let child2_token = parent_token.child_token();
        let grandchild_token = child1_token.child_token();

        inner
            .process_table
            .set_cancellation_token(parent_id, parent_token.clone());
        inner
            .process_table
            .set_cancellation_token(child1_id, child1_token.clone());
        inner
            .process_table
            .set_cancellation_token(child2_id, child2_token.clone());
        inner
            .process_table
            .set_cancellation_token(grandchild_id, grandchild_token.clone());

        // Insert processes
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
            agent_id:        AgentId::new(),
            session_id:      SessionId::new("test"),
            principal:       Principal::user("test"),
            manifest:        test_manifest("test"),
            allowed_tools:   vec![],
            tool_registry:   Arc::new(ToolRegistry::new()),
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner:           Arc::clone(&inner),
        };

        // Kill parent — CancellationToken hierarchy cascades automatically
        handle.kill(parent_id).unwrap();

        // All tokens should be cancelled
        assert!(parent_token.is_cancelled());
        assert!(child1_token.is_cancelled());
        assert!(child2_token.is_cancelled());
        assert!(grandchild_token.is_cancelled());

        // Parent state set immediately by kill()
        assert_eq!(
            inner.process_table.get(parent_id).unwrap().state,
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
            manifest: test_manifest("test"),
            allowed_tools: vec![],
            tool_registry: Arc::new(ToolRegistry::new()),
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
            manifest: test_manifest("test"),
            allowed_tools: vec![],
            tool_registry: Arc::new(ToolRegistry::new()),
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
            manifest: test_manifest("test"),
            allowed_tools: vec![],
            tool_registry: Arc::new(ToolRegistry::new()),
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
            manifest: test_manifest("test"),
            allowed_tools: vec![],
            tool_registry: Arc::new(ToolRegistry::new()),
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
            manifest: test_manifest("test"),
            allowed_tools: vec![],
            tool_registry: Arc::new(ToolRegistry::new()),
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
