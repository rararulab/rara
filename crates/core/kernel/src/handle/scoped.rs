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
use super::{AgentHandle, EventOps, GuardOps, MemoryOps, PipeOps, ProcessOps};
use crate::{
    audit::{AuditEvent, AuditEventType, MemoryOp},
    error::{KernelError, Result},
    event::KernelEvent,
    guard::GuardContext,
    io::pipe::{self, PipeEntry, PipeReader, PipeWriter},
    kernel::KernelInner,
    process::{
        AgentId, AgentManifest, ProcessInfo, ProcessState, SessionId, Signal,
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

        // Audit: ProcessKilled
        crate::audit::record_async(
            &self.inner.audit_log,
            AuditEvent {
                timestamp:  Timestamp::now(),
                agent_id:   target_id,
                session_id: self.session_id.clone(),
                user_id:    self.principal.user_id.clone(),
                event_type: AuditEventType::ProcessKilled { by: self.agent_id },
                details:    serde_json::Value::Null,
            },
        );

        Ok(())
    }

    /// Send a signal to a process via the EventQueue.
    ///
    /// Uses `try_push` (fire-and-forget) since signals are critical-priority
    /// and should not block the caller.
    fn send_signal(&self, target_id: AgentId, signal: Signal) -> Result<()> {
        // Verify the process exists first.
        self.inner
            .process_table
            .get(target_id)
            .ok_or(KernelError::ProcessNotFound {
                id: format!("agent {target_id} not found"),
            })?;
        self.inner
            .event_queue
            .try_push(crate::unified_event::KernelEvent::SendSignal {
                target: target_id,
                signal,
            })
            .map_err(|_| KernelError::ProcessNotFound {
                id: format!("event queue full for signal to agent {target_id}"),
            })
    }
}

#[async_trait]
impl ProcessOps for ScopedKernelHandle {
    /// Spawn a child agent via the unified event queue.
    ///
    /// - Validates child tools are a subset of parent's tools
    /// - Validates the principal is still active
    /// - Pushes a `KernelEvent::SpawnAgent` into the event queue
    /// - Waits for the reply from the event loop
    async fn spawn(&self, manifest: AgentManifest, input: String) -> Result<AgentHandle> {
        // 1. Validate tool subset
        if !manifest.tools.is_empty() {
            self.validate_tool_subset(&manifest.tools)?;
        }

        // 1.5 Validate principal (user may have been disabled after top-level spawn)
        self.inner.validate_principal(&self.principal).await?;

        // 2. Push SpawnAgent event and wait for reply.
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let event = crate::unified_event::KernelEvent::SpawnAgent {
            manifest,
            input,
            principal: self.principal.clone(),
            session_id: self.session_id.clone(),
            parent_id: Some(self.agent_id),
            reply_tx,
        };
        self.inner
            .event_queue
            .push(event)
            .await
            .map_err(|_| KernelError::SpawnFailed {
                message: "event queue full".to_string(),
            })?;

        let agent_id = reply_rx
            .await
            .map_err(|_| KernelError::SpawnFailed {
                message: "spawn reply channel closed".to_string(),
            })??;

        // Build a handle with a oneshot for the result (placeholder — the
        // event loop manages the actual lifecycle).
        let (_result_tx, result_rx) = tokio::sync::oneshot::channel();
        Ok(AgentHandle {
            agent_id,
            result_rx,
        })
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

    fn pause(&self, agent_id: AgentId) -> Result<()> {
        // Verify the process exists.
        self.inner
            .process_table
            .get(agent_id)
            .ok_or(KernelError::ProcessNotFound {
                id: agent_id.to_string(),
            })?;
        self.send_signal(agent_id, Signal::Pause)
    }

    fn resume(&self, agent_id: AgentId) -> Result<()> {
        // Verify the process exists.
        self.inner
            .process_table
            .get(agent_id)
            .ok_or(KernelError::ProcessNotFound {
                id: agent_id.to_string(),
            })?;
        self.send_signal(agent_id, Signal::Resume)
    }

    fn interrupt(&self, agent_id: AgentId) -> Result<()> {
        // Verify the process exists.
        self.inner
            .process_table
            .get(agent_id)
            .ok_or(KernelError::ProcessNotFound {
                id: agent_id.to_string(),
            })?;
        self.send_signal(agent_id, Signal::Interrupt)
    }

    fn children(&self) -> Vec<ProcessInfo> { self.inner.process_table.children_of(self.agent_id) }
}

impl ScopedKernelHandle {
    /// Build the namespaced key for this agent's private KV scope.
    fn namespaced_key(&self, key: &str) -> String {
        format!("agent:{}:{}", self.agent_id.0, key)
    }

    /// Build a scoped key from a [`KvScope`].
    fn scoped_key(scope: &crate::memory::KvScope, key: &str) -> String {
        match scope {
            crate::memory::KvScope::Global => key.to_string(),
            crate::memory::KvScope::Team(name) => format!("team:{name}:{key}"),
            crate::memory::KvScope::Agent(id) => format!("agent:{id}:{key}"),
        }
    }

    /// Validate that the current principal is allowed to access the given scope.
    fn check_scope_permission(&self, scope: &crate::memory::KvScope) -> Result<()> {
        match scope {
            crate::memory::KvScope::Global | crate::memory::KvScope::Team(_) => {
                // Global and Team scopes require Root or Admin role.
                if !self.principal.is_admin() {
                    return Err(KernelError::MemoryScopeDenied {
                        reason: format!(
                            "agent {} (role {:?}) cannot access {:?} scope — requires Root or Admin",
                            self.agent_id, self.principal.role, scope,
                        ),
                    });
                }
            }
            crate::memory::KvScope::Agent(target_id) => {
                // Regular agents can only access their own agent scope.
                if *target_id != self.agent_id.0 && !self.principal.is_admin() {
                    return Err(KernelError::MemoryScopeDenied {
                        reason: format!(
                            "agent {} cannot access agent {}'s scope — not admin",
                            self.agent_id, target_id,
                        ),
                    });
                }
            }
        }
        Ok(())
    }

    /// Check memory quota for the current agent's namespace.
    ///
    /// Counts all keys with the prefix `"agent:{agent_id}:"` and compares
    /// against the configured quota. Returns an error if the quota would be
    /// exceeded.
    fn check_quota(&self) -> Result<()> {
        let max = self.inner.memory_quota_per_agent;
        if max == 0 {
            return Ok(()); // unlimited
        }
        let prefix = format!("agent:{}:", self.agent_id.0);
        let count = self
            .inner
            .shared_kv
            .iter()
            .filter(|entry| entry.key().starts_with(&prefix))
            .count();
        if count >= max {
            return Err(KernelError::MemoryQuotaExceeded {
                agent_id: self.agent_id.to_string(),
                current:  count,
                max,
            });
        }
        Ok(())
    }
}

impl MemoryOps for ScopedKernelHandle {
    fn mem_store(&self, key: &str, value: serde_json::Value) -> Result<()> {
        let namespaced = self.namespaced_key(key);
        // Check quota before inserting — only if this is a new key.
        if !self.inner.shared_kv.contains_key(&namespaced) {
            self.check_quota()?;
        }
        self.inner.shared_kv.insert(namespaced, value);

        // Audit: MemoryAccess (Store)
        crate::audit::record_async(
            &self.inner.audit_log,
            AuditEvent {
                timestamp:  Timestamp::now(),
                agent_id:   self.agent_id,
                session_id: self.session_id.clone(),
                user_id:    self.principal.user_id.clone(),
                event_type: AuditEventType::MemoryAccess {
                    operation: MemoryOp::Store,
                    key:       key.to_string(),
                },
                details:    serde_json::Value::Null,
            },
        );

        Ok(())
    }

    fn mem_recall(&self, key: &str) -> Result<Option<serde_json::Value>> {
        let namespaced = self.namespaced_key(key);
        Ok(self
            .inner
            .shared_kv
            .get(&namespaced)
            .map(|v| v.value().clone()))
    }

    fn shared_store(
        &self,
        scope: crate::memory::KvScope,
        key: &str,
        value: serde_json::Value,
    ) -> Result<()> {
        self.check_scope_permission(&scope)?;
        let scoped = Self::scoped_key(&scope, key);
        self.inner.shared_kv.insert(scoped, value);
        Ok(())
    }

    fn shared_recall(
        &self,
        scope: crate::memory::KvScope,
        key: &str,
    ) -> Result<Option<serde_json::Value>> {
        self.check_scope_permission(&scope)?;
        let scoped = Self::scoped_key(&scope, key);
        Ok(self
            .inner
            .shared_kv
            .get(&scoped)
            .map(|v| v.value().clone()))
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

impl PipeOps for ScopedKernelHandle {
    fn create_pipe(&self, target: AgentId) -> Result<(PipeWriter, PipeReader)> {
        let (writer, reader) = pipe::pipe(64);
        self.inner.pipe_registry.register(
            writer.pipe_id().clone(),
            PipeEntry {
                owner:      self.agent_id,
                reader:     Some(target),
                created_at: Timestamp::now(),
            },
        );
        Ok((writer, reader))
    }

    fn create_named_pipe(&self, name: &str) -> Result<(PipeWriter, PipeReader)> {
        // Check if name is already taken
        if self.inner.pipe_registry.resolve_name(name).is_some() {
            return Err(KernelError::Other {
                message: format!("named pipe already exists: {name}").into(),
            });
        }
        let (writer, reader) = pipe::pipe(64);
        let pipe_id = writer.pipe_id().clone();
        self.inner.pipe_registry.register_named(
            name.to_string(),
            pipe_id.clone(),
            PipeEntry {
                owner:      self.agent_id,
                reader:     None,
                created_at: Timestamp::now(),
            },
        );
        // Park the reader so that connect_pipe() can hand it out.
        // The caller also receives a copy-less reference via the return value,
        // but for named pipes the typical pattern is:
        //   creator:   create_named_pipe("feed") -> keeps writer, ignores reader
        //   connector: connect_pipe("feed") -> gets the parked reader
        // We return both so the creator has the option of handing the reader
        // directly to a child too (anonymous-pipe style).
        //
        // Because PipeReader is not Clone, we do NOT park it here — the caller
        // decides. If the caller wants rendezvous semantics they should call
        // `pipe_registry.park_reader()` with the returned reader.
        Ok((writer, reader))
    }

    fn connect_pipe(&self, name: &str) -> Result<PipeReader> {
        let pipe_id = self
            .inner
            .pipe_registry
            .resolve_name(name)
            .ok_or_else(|| KernelError::Other {
                message: format!("named pipe not found: {name}").into(),
            })?;

        // Take the parked reader (one-shot — only the first connector gets it).
        let reader = self
            .inner
            .pipe_registry
            .take_parked_reader(&pipe_id)
            .ok_or_else(|| KernelError::Other {
                message: format!(
                    "named pipe '{name}' has no parked reader \
                     (already taken or not parked)"
                )
                .into(),
            })?;

        // Record who connected.
        self.inner
            .pipe_registry
            .set_reader(&pipe_id, self.agent_id);

        Ok(reader)
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
        make_kernel_inner_with_quota(1000)
    }

    fn make_kernel_inner_with_quota(quota: usize) -> Arc<KernelInner> {
        use crate::{
            audit::InMemoryAuditLog,
            defaults::{
                noop::{
                    NoopEventBus, NoopGuard, NoopMemory, NoopModelRepo, NoopSessionRepository,
                },
                noop_user_store::NoopUserStore,
            },
            event_queue::EventQueue,
            io::{pipe::PipeRegistry, stream::StreamHub},
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
            memory_quota_per_agent: quota,
            user_store:             Arc::new(NoopUserStore),
            session_repo:           Arc::new(NoopSessionRepository) as Arc<dyn SessionRepository>,
            model_repo:             Arc::new(NoopModelRepo)
                as Arc<dyn crate::model_repo::ModelRepo>,
            stream_hub:             Arc::new(StreamHub::new(1)),
            pipe_registry:          Arc::new(PipeRegistry::new()),
            device_registry:        Arc::new(crate::device_registry::DeviceRegistry::new()),
            audit_log:              Arc::new(InMemoryAuditLog::default())
                as Arc<dyn crate::audit::AuditLog>,
            event_queue:            Arc::new(EventQueue::new(4096)),
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
            created_files: vec![],
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
    fn test_memory_namespace_isolation() {
        // Agent A cannot read Agent B's data via mem_store/mem_recall.
        let inner = make_kernel_inner();

        let handle_a = ScopedKernelHandle {
            agent_id:        AgentId::new(),
            session_id:      SessionId::new("test"),
            principal:       Principal::user("user-a"),
            manifest:        test_manifest("agent-a"),
            allowed_tools:   vec![],
            tool_registry:   Arc::new(ToolRegistry::new()),
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner:           Arc::clone(&inner),
        };

        let handle_b = ScopedKernelHandle {
            agent_id:        AgentId::new(),
            session_id:      SessionId::new("test"),
            principal:       Principal::user("user-b"),
            manifest:        test_manifest("agent-b"),
            allowed_tools:   vec![],
            tool_registry:   Arc::new(ToolRegistry::new()),
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner:           Arc::clone(&inner),
        };

        // Agent A stores a value.
        handle_a
            .mem_store("secret", serde_json::json!("only-for-a"))
            .unwrap();

        // Agent A can recall it.
        let recalled_a = handle_a.mem_recall("secret").unwrap();
        assert_eq!(recalled_a.unwrap(), serde_json::json!("only-for-a"));

        // Agent B cannot see it via mem_recall (different namespace).
        let recalled_b = handle_b.mem_recall("secret").unwrap();
        assert!(
            recalled_b.is_none(),
            "Agent B should NOT be able to read Agent A's data"
        );

        // Agent B stores the same key — should not overwrite Agent A's.
        handle_b
            .mem_store("secret", serde_json::json!("only-for-b"))
            .unwrap();

        // Both see their own value.
        let a_val = handle_a.mem_recall("secret").unwrap().unwrap();
        let b_val = handle_b.mem_recall("secret").unwrap().unwrap();
        assert_eq!(a_val, serde_json::json!("only-for-a"));
        assert_eq!(b_val, serde_json::json!("only-for-b"));
    }

    #[test]
    fn test_shared_store_global_requires_admin() {
        use crate::memory::KvScope;

        let inner = make_kernel_inner();

        // Regular user handle.
        let handle_user = ScopedKernelHandle {
            agent_id:        AgentId::new(),
            session_id:      SessionId::new("test"),
            principal:       Principal::user("alice"),
            manifest:        test_manifest("test"),
            allowed_tools:   vec![],
            tool_registry:   Arc::new(ToolRegistry::new()),
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner:           Arc::clone(&inner),
        };

        // Should fail — user cannot access Global scope.
        let result = handle_user.shared_store(
            KvScope::Global,
            "global-key",
            serde_json::json!("value"),
        );
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("scope denied"),
            "expected scope denied error"
        );

        // Admin handle.
        let handle_admin = ScopedKernelHandle {
            agent_id:        AgentId::new(),
            session_id:      SessionId::new("test"),
            principal:       Principal::admin("admin-1"),
            manifest:        test_manifest("test"),
            allowed_tools:   vec![],
            tool_registry:   Arc::new(ToolRegistry::new()),
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner:           Arc::clone(&inner),
        };

        // Should succeed.
        handle_admin
            .shared_store(KvScope::Global, "global-key", serde_json::json!("global-val"))
            .unwrap();

        let recalled = handle_admin
            .shared_recall(KvScope::Global, "global-key")
            .unwrap();
        assert_eq!(recalled.unwrap(), serde_json::json!("global-val"));

        // Regular user still cannot read Global scope.
        let result = handle_user.shared_recall(KvScope::Global, "global-key");
        assert!(result.is_err());
    }

    #[test]
    fn test_shared_store_team_requires_admin() {
        use crate::memory::KvScope;

        let inner = make_kernel_inner();

        let handle_user = ScopedKernelHandle {
            agent_id:        AgentId::new(),
            session_id:      SessionId::new("test"),
            principal:       Principal::user("bob"),
            manifest:        test_manifest("test"),
            allowed_tools:   vec![],
            tool_registry:   Arc::new(ToolRegistry::new()),
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner:           Arc::clone(&inner),
        };

        let result = handle_user.shared_store(
            KvScope::Team("team-alpha".to_string()),
            "team-key",
            serde_json::json!("value"),
        );
        assert!(result.is_err());

        let handle_admin = ScopedKernelHandle {
            agent_id:        AgentId::new(),
            session_id:      SessionId::new("test"),
            principal:       Principal::admin("admin"),
            manifest:        test_manifest("test"),
            allowed_tools:   vec![],
            tool_registry:   Arc::new(ToolRegistry::new()),
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner:           Arc::clone(&inner),
        };

        handle_admin
            .shared_store(
                KvScope::Team("team-alpha".to_string()),
                "team-key",
                serde_json::json!("team-val"),
            )
            .unwrap();

        let recalled = handle_admin
            .shared_recall(KvScope::Team("team-alpha".to_string()), "team-key")
            .unwrap();
        assert_eq!(recalled.unwrap(), serde_json::json!("team-val"));
    }

    #[test]
    fn test_shared_store_agent_scope_own_access() {
        use crate::memory::KvScope;

        let inner = make_kernel_inner();
        let agent_id = AgentId::new();

        let handle = ScopedKernelHandle {
            agent_id,
            session_id:      SessionId::new("test"),
            principal:       Principal::user("alice"),
            manifest:        test_manifest("test"),
            allowed_tools:   vec![],
            tool_registry:   Arc::new(ToolRegistry::new()),
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner:           Arc::clone(&inner),
        };

        // Agent can access its own scope.
        handle
            .shared_store(
                KvScope::Agent(agent_id.0),
                "my-key",
                serde_json::json!("my-val"),
            )
            .unwrap();

        let recalled = handle
            .shared_recall(KvScope::Agent(agent_id.0), "my-key")
            .unwrap();
        assert_eq!(recalled.unwrap(), serde_json::json!("my-val"));

        // Agent cannot access another agent's scope.
        let other_id = AgentId::new();
        let result = handle.shared_store(
            KvScope::Agent(other_id.0),
            "other-key",
            serde_json::json!("nope"),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_root_can_access_any_agent_scope() {
        use crate::memory::KvScope;
        use crate::process::principal::Role;

        let inner = make_kernel_inner();
        let target_agent_id = AgentId::new();

        // Root principal.
        let root_principal = Principal {
            user_id:     crate::process::principal::UserId("root".to_string()),
            role:        Role::Root,
            permissions: vec![],
        };

        let handle_root = ScopedKernelHandle {
            agent_id:        AgentId::new(), // different from target
            session_id:      SessionId::new("test"),
            principal:       root_principal,
            manifest:        test_manifest("test"),
            allowed_tools:   vec![],
            tool_registry:   Arc::new(ToolRegistry::new()),
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner:           Arc::clone(&inner),
        };

        // Root can write to any agent's scope.
        handle_root
            .shared_store(
                KvScope::Agent(target_agent_id.0),
                "injected",
                serde_json::json!("by-root"),
            )
            .unwrap();

        let recalled = handle_root
            .shared_recall(KvScope::Agent(target_agent_id.0), "injected")
            .unwrap();
        assert_eq!(recalled.unwrap(), serde_json::json!("by-root"));

        // Root can also access Global.
        handle_root
            .shared_store(KvScope::Global, "g", serde_json::json!(1))
            .unwrap();
        assert!(
            handle_root
                .shared_recall(KvScope::Global, "g")
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn test_memory_quota_enforcement() {
        let small_inner = make_kernel_inner_with_quota(3);

        let handle = ScopedKernelHandle {
            agent_id:        AgentId::new(),
            session_id:      SessionId::new("test"),
            principal:       Principal::user("test"),
            manifest:        test_manifest("test"),
            allowed_tools:   vec![],
            tool_registry:   Arc::new(ToolRegistry::new()),
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner:           Arc::clone(&small_inner),
        };

        // Store 3 entries (at the limit).
        handle.mem_store("k1", serde_json::json!(1)).unwrap();
        handle.mem_store("k2", serde_json::json!(2)).unwrap();
        handle.mem_store("k3", serde_json::json!(3)).unwrap();

        // Fourth entry should fail.
        let result = handle.mem_store("k4", serde_json::json!(4));
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("quota exceeded"),
            "expected quota exceeded error"
        );

        // Updating an existing key should succeed (not a new entry).
        handle
            .mem_store("k1", serde_json::json!("updated"))
            .unwrap();
        let val = handle.mem_recall("k1").unwrap().unwrap();
        assert_eq!(val, serde_json::json!("updated"));
    }

    #[test]
    fn test_memory_quota_zero_means_unlimited() {
        let unlimited_inner = make_kernel_inner_with_quota(0);

        let handle = ScopedKernelHandle {
            agent_id:        AgentId::new(),
            session_id:      SessionId::new("test"),
            principal:       Principal::user("test"),
            manifest:        test_manifest("test"),
            allowed_tools:   vec![],
            tool_registry:   Arc::new(ToolRegistry::new()),
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner:           unlimited_inner,
        };

        // Should be able to store many entries without quota error.
        for i in 0..50 {
            handle
                .mem_store(&format!("key-{i}"), serde_json::json!(i))
                .unwrap();
        }
    }

    #[test]
    fn test_shared_store_cross_agent_via_explicit_scope() {
        // Two agents sharing data through explicit KvScope::Agent.
        use crate::memory::KvScope;

        let inner = make_kernel_inner();
        let agent_a_id = AgentId::new();
        let agent_b_id = AgentId::new();

        // Admin handle for Agent A — can write to any scope.
        let handle_a = ScopedKernelHandle {
            agent_id:        agent_a_id,
            session_id:      SessionId::new("test"),
            principal:       Principal::admin("admin"),
            manifest:        test_manifest("agent-a"),
            allowed_tools:   vec![],
            tool_registry:   Arc::new(ToolRegistry::new()),
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner:           Arc::clone(&inner),
        };

        // Admin handle for Agent B.
        let handle_b = ScopedKernelHandle {
            agent_id:        agent_b_id,
            session_id:      SessionId::new("test"),
            principal:       Principal::admin("admin"),
            manifest:        test_manifest("agent-b"),
            allowed_tools:   vec![],
            tool_registry:   Arc::new(ToolRegistry::new()),
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner:           Arc::clone(&inner),
        };

        // Agent A writes to Agent B's scope.
        handle_a
            .shared_store(
                KvScope::Agent(agent_b_id.0),
                "shared-data",
                serde_json::json!("from-a-to-b"),
            )
            .unwrap();

        // Agent B reads from its own scope.
        let val = handle_b
            .shared_recall(KvScope::Agent(agent_b_id.0), "shared-data")
            .unwrap();
        assert_eq!(val.unwrap(), serde_json::json!("from-a-to-b"));
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
            env:           AgentEnv::default(),
            state:         ProcessState::Running,
            created_at:    Timestamp::now(),
            finished_at:   None,
            result:        None,
            created_files: vec![],
        });

        inner.process_table.insert(AgentProcess {
            agent_id:    child1_id,
            parent_id:   Some(parent_id),
            session_id:  SessionId::new("test"),
            manifest:    test_manifest("child1"),
            principal:   Principal::user("test"),
            env:           AgentEnv::default(),
            state:         ProcessState::Running,
            created_at:    Timestamp::now(),
            finished_at:   None,
            result:        None,
            created_files: vec![],
        });

        inner.process_table.insert(AgentProcess {
            agent_id:    child2_id,
            parent_id:   Some(parent_id),
            session_id:  SessionId::new("test"),
            manifest:    test_manifest("child2"),
            principal:   Principal::user("test"),
            env:           AgentEnv::default(),
            state:         ProcessState::Running,
            created_at:    Timestamp::now(),
            finished_at:   None,
            result:        None,
            created_files: vec![],
        });

        inner.process_table.insert(AgentProcess {
            agent_id:    grandchild_id,
            parent_id:   Some(child1_id),
            session_id:  SessionId::new("test"),
            manifest:    test_manifest("grandchild"),
            principal:   Principal::user("test"),
            env:           AgentEnv::default(),
            state:         ProcessState::Running,
            created_at:    Timestamp::now(),
            finished_at:   None,
            result:        None,
            created_files: vec![],
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
            created_files: vec![],
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

    #[tokio::test]
    async fn test_pause_sends_signal() {
        let inner = make_kernel_inner();
        let target_id = AgentId::new();

        inner.process_table.insert(AgentProcess {
            agent_id:      target_id,
            parent_id:     None,
            session_id:    SessionId::new("test"),
            manifest:      test_manifest("target"),
            principal:     Principal::user("test"),
            env:           AgentEnv::default(),
            state:         ProcessState::Running,
            created_at:    Timestamp::now(),
            finished_at:   None,
            result:        None,
            created_files: vec![],
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

        handle.pause(target_id).unwrap();

        // Verify the signal was pushed to the event queue.
        let events = inner.event_queue.drain(10).await;
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], crate::unified_event::KernelEvent::SendSignal { target, signal }
                if *target == target_id && *signal == Signal::Pause),
            "expected Pause signal event, got: {:?}",
            events[0]
        );
    }

    #[tokio::test]
    async fn test_resume_sends_signal() {
        let inner = make_kernel_inner();
        let target_id = AgentId::new();

        inner.process_table.insert(AgentProcess {
            agent_id:      target_id,
            parent_id:     None,
            session_id:    SessionId::new("test"),
            manifest:      test_manifest("target"),
            principal:     Principal::user("test"),
            env:           AgentEnv::default(),
            state:         ProcessState::Paused,
            created_at:    Timestamp::now(),
            finished_at:   None,
            result:        None,
            created_files: vec![],
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

        handle.resume(target_id).unwrap();

        // Verify the signal was pushed to the event queue.
        let events = inner.event_queue.drain(10).await;
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], crate::unified_event::KernelEvent::SendSignal { target, signal }
                if *target == target_id && *signal == Signal::Resume),
            "expected Resume signal event, got: {:?}",
            events[0]
        );
    }

    #[tokio::test]
    async fn test_interrupt_sends_signal() {
        let inner = make_kernel_inner();
        let target_id = AgentId::new();

        inner.process_table.insert(AgentProcess {
            agent_id:      target_id,
            parent_id:     None,
            session_id:    SessionId::new("test"),
            manifest:      test_manifest("target"),
            principal:     Principal::user("test"),
            env:           AgentEnv::default(),
            state:         ProcessState::Running,
            created_at:    Timestamp::now(),
            finished_at:   None,
            result:        None,
            created_files: vec![],
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

        handle.interrupt(target_id).unwrap();

        // Verify the signal was pushed to the event queue.
        let events = inner.event_queue.drain(10).await;
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], crate::unified_event::KernelEvent::SendSignal { target, signal }
                if *target == target_id && *signal == Signal::Interrupt),
            "expected Interrupt signal event, got: {:?}",
            events[0]
        );
    }

    #[test]
    fn test_signal_to_nonexistent_process() {
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

        let fake_id = AgentId::new();
        assert!(handle.pause(fake_id).is_err());
        assert!(handle.resume(fake_id).is_err());
        assert!(handle.interrupt(fake_id).is_err());
    }

    // ---- PipeOps tests -------------------------------------------------------

    #[tokio::test]
    async fn test_pipe_ops_create_pipe() {
        let inner = make_kernel_inner();
        let agent_a = AgentId::new();
        let agent_b = AgentId::new();

        let handle = ScopedKernelHandle {
            agent_id:        agent_a,
            session_id:      SessionId::new("test"),
            principal:       Principal::user("test"),
            manifest:        test_manifest("test"),
            allowed_tools:   vec![],
            tool_registry:   Arc::new(ToolRegistry::new()),
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner:           Arc::clone(&inner),
        };

        let (writer, mut reader) = handle.create_pipe(agent_b).unwrap();

        // Registry should record the pipe
        let entry = inner.pipe_registry.get(writer.pipe_id()).unwrap();
        assert_eq!(entry.owner, agent_a);
        assert_eq!(entry.reader, Some(agent_b));

        // Data flows
        writer.send("from a to b".to_string()).await.unwrap();
        let msg = reader.recv().await.unwrap();
        assert_eq!(
            msg,
            crate::io::pipe::PipeMessage::Data("from a to b".to_string())
        );
    }

    #[tokio::test]
    async fn test_pipe_ops_create_named_pipe_and_connect() {
        let inner = make_kernel_inner();
        let producer = AgentId::new();
        let consumer = AgentId::new();

        let producer_handle = ScopedKernelHandle {
            agent_id:        producer,
            session_id:      SessionId::new("test"),
            principal:       Principal::user("test"),
            manifest:        test_manifest("producer"),
            allowed_tools:   vec![],
            tool_registry:   Arc::new(ToolRegistry::new()),
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner:           Arc::clone(&inner),
        };

        let consumer_handle = ScopedKernelHandle {
            agent_id:        consumer,
            session_id:      SessionId::new("test"),
            principal:       Principal::user("test"),
            manifest:        test_manifest("consumer"),
            allowed_tools:   vec![],
            tool_registry:   Arc::new(ToolRegistry::new()),
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner:           Arc::clone(&inner),
        };

        // Producer creates named pipe and parks the reader
        let (writer, reader) = producer_handle
            .create_named_pipe("data-feed")
            .unwrap();
        inner
            .pipe_registry
            .park_reader(writer.pipe_id().clone(), reader);

        // Consumer connects by name
        let mut reader = consumer_handle.connect_pipe("data-feed").unwrap();

        // Registry records the consumer
        let entry = inner.pipe_registry.get(writer.pipe_id()).unwrap();
        assert_eq!(entry.reader, Some(consumer));

        // Data flows
        writer.send("streamed data".to_string()).await.unwrap();
        let msg = reader.recv().await.unwrap();
        assert_eq!(
            msg,
            crate::io::pipe::PipeMessage::Data("streamed data".to_string())
        );
    }

    #[test]
    fn test_pipe_ops_duplicate_named_pipe_fails() {
        let inner = make_kernel_inner();
        let handle = ScopedKernelHandle {
            agent_id:        AgentId::new(),
            session_id:      SessionId::new("test"),
            principal:       Principal::user("test"),
            manifest:        test_manifest("test"),
            allowed_tools:   vec![],
            tool_registry:   Arc::new(ToolRegistry::new()),
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner,
        };

        // First creation succeeds
        let result = handle.create_named_pipe("unique");
        assert!(result.is_ok());

        // Second creation with same name fails
        let result = handle.create_named_pipe("unique");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("named pipe already exists")
        );
    }

    #[test]
    fn test_pipe_ops_connect_nonexistent_fails() {
        let inner = make_kernel_inner();
        let handle = ScopedKernelHandle {
            agent_id:        AgentId::new(),
            session_id:      SessionId::new("test"),
            principal:       Principal::user("test"),
            manifest:        test_manifest("test"),
            allowed_tools:   vec![],
            tool_registry:   Arc::new(ToolRegistry::new()),
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner,
        };

        let result = handle.connect_pipe("nonexistent");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("named pipe not found")
        );
    }

    #[test]
    fn test_pipe_ops_connect_without_parked_reader_fails() {
        let inner = make_kernel_inner();
        let creator = AgentId::new();
        let connector = AgentId::new();

        let creator_handle = ScopedKernelHandle {
            agent_id:        creator,
            session_id:      SessionId::new("test"),
            principal:       Principal::user("test"),
            manifest:        test_manifest("creator"),
            allowed_tools:   vec![],
            tool_registry:   Arc::new(ToolRegistry::new()),
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner:           Arc::clone(&inner),
        };

        let connector_handle = ScopedKernelHandle {
            agent_id:        connector,
            session_id:      SessionId::new("test"),
            principal:       Principal::user("test"),
            manifest:        test_manifest("connector"),
            allowed_tools:   vec![],
            tool_registry:   Arc::new(ToolRegistry::new()),
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner:           Arc::clone(&inner),
        };

        // Create named pipe but DON'T park the reader
        let (_writer, _reader) = creator_handle
            .create_named_pipe("no-park")
            .unwrap();

        // Connecting fails because no reader was parked
        let result = connector_handle.connect_pipe("no-park");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("no parked reader")
        );
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
            max_children:        None,
            max_context_tokens:  None,
            priority:            crate::process::Priority::default(),
            metadata:            serde_json::Value::Null,
            sandbox:             None,
        }
    }
}
