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

//! Syscall dispatcher — handles all session-scoped kernel operations dispatched
//! by the kernel event loop.
//!
//! Extracted from `event_loop/syscall.rs` to encapsulate the kernel
//! sub-components used exclusively by syscall handling (shared KV, pipe
//! registry, driver registry, tool registry, event bus, config).

use std::sync::Arc;

use async_trait::async_trait;
use jiff::Timestamp;
use serde::Deserialize;
use snafu::ResultExt;
use tracing::{debug_span, info, warn};

use crate::{
    agent::{AgentManifest, AgentRegistryRef},
    error::KernelError,
    event::{Syscall, SyscallEnvelope},
    handle::KernelHandle,
    identity::Principal,
    io::{AgentHandle, PipeEntry, PipeRegistry, pipe},
    kernel::KernelConfig,
    kv::{KvScope, SharedKv},
    llm::DriverRegistryRef,
    memory::TapeService,
    notification::NotificationBusRef,
    security::SecurityRef,
    session::{SessionKey, SessionTable},
    tape_tool::TapeTool,
    tool::ToolRegistryRef,
};

/// Dispatches syscalls from session-scoped operations to the appropriate kernel
/// sub-component.
///
/// Owns the kernel fields used exclusively by syscall handling: shared KV,
/// pipe registry, driver registry, tool registry, event bus, and config.
/// Other shared state (process table, security, audit, etc.) is passed as
/// parameters to `dispatch()`.
pub(crate) struct SyscallDispatcher {
    /// Cross-agent shared key-value store (OpenDAL-backed).
    shared_kv:       SharedKv,
    /// Inter-agent pipe registry for streaming data between agents.
    pipe_registry:   PipeRegistry,
    /// Multi-driver LLM registry with per-agent overrides.
    driver_registry: DriverRegistryRef,
    /// Global tool registry.
    tool_registry:   ToolRegistryRef,
    /// Event bus for publishing kernel notifications.
    event_bus:       NotificationBusRef,
    /// Kernel configuration.
    config:          KernelConfig,
    /// Tape service for session message persistence (passed to SyscallTool).
    tape_service:    TapeService,
}

impl SyscallDispatcher {
    /// Create a new syscall dispatcher.
    pub fn new(
        shared_kv: SharedKv,
        pipe_registry: PipeRegistry,
        driver_registry: DriverRegistryRef,
        tool_registry: ToolRegistryRef,
        event_bus: NotificationBusRef,
        config: KernelConfig,
        tape_service: TapeService,
    ) -> Self {
        Self {
            shared_kv,
            pipe_registry,
            driver_registry,
            tool_registry,
            event_bus,
            config,
            tape_service,
        }
    }

    /// Access the global tool registry.
    pub fn tool_registry(&self) -> &ToolRegistryRef { &self.tool_registry }

    pub fn driver_registry(&self) -> &DriverRegistryRef { &self.driver_registry }

    // -- Dispatch -----------------------------------------------------------

    /// Handle a syscall from a session.
    ///
    /// All business logic lives here, executed by the kernel event loop.
    /// TODO: implement dispatch by using `syscallEnvelope` to route to more
    /// specific handlers (e.g. `handle_mem_syscall`, `handle_pipe_syscall`,
    /// etc.) for better organization and readability.
    pub async fn dispatch(
        &self,
        syscall: SyscallEnvelope,
        process_table: &SessionTable,
        security: &SecurityRef,
        _agent_registry: &AgentRegistryRef,
        kernel_handle: &KernelHandle,
    ) {
        let syscall_sender = syscall.session_key;
        let syscall = syscall.payload;
        let syscall_type: &'static str = (&syscall).into();
        crate::metrics::SYSCALL_TOTAL
            .with_label_values(&[syscall_type])
            .inc();
        let span = debug_span!(
            "handle_syscall",
            syscall_type,
            session_key = %syscall_sender,
        );
        let _guard = span.enter();

        match syscall {
            Syscall::MemStore {
                session_key,
                principal,
                key,
                value,
                reply_tx,
            } => {
                let result = self
                    .do_mem_store(
                        self.config.memory_quota_per_agent,
                        session_key,
                        &principal,
                        &key,
                        value,
                    )
                    .await;
                let _ = reply_tx.send(result);
            }
            Syscall::MemRecall { key, reply_tx } => {
                let namespaced = format!("session:{}:{}", syscall_sender, key);
                let result = Ok(self.shared_kv.get(&namespaced).await);
                let _ = reply_tx.send(result);
            }
            Syscall::SharedStore {
                principal,
                scope,
                key,
                value,
                reply_tx,
            } => {
                let result = self
                    .do_shared_store(syscall_sender, &principal, &scope, &key, value)
                    .await;
                let _ = reply_tx.send(result);
            }
            Syscall::SharedRecall {
                principal,
                scope,
                key,
                reply_tx,
            } => {
                let result = self
                    .do_shared_recall(syscall_sender, &principal, &scope, &key)
                    .await;
                let _ = reply_tx.send(result);
            }
            Syscall::CreatePipe { target, reply_tx } => {
                let (writer, reader) = pipe(64);
                self.pipe_registry.register(
                    writer.pipe_id().clone(),
                    PipeEntry {
                        owner:      syscall_sender,
                        reader:     Some(target),
                        created_at: Timestamp::now(),
                    },
                );
                let _ = reply_tx.send(Ok((writer, reader)));
            }
            Syscall::CreateNamedPipe { name, reply_tx } => {
                if self.pipe_registry.resolve_name(&name).is_some() {
                    let _ = reply_tx.send(Err(KernelError::Other {
                        message: format!("named pipe already exists: {name}").into(),
                    }));
                    return;
                }
                let (writer, reader) = pipe(64);
                let pipe_id = writer.pipe_id().clone();
                self.pipe_registry.register_named(
                    name,
                    pipe_id,
                    PipeEntry {
                        owner:      syscall_sender,
                        reader:     None,
                        created_at: Timestamp::now(),
                    },
                );
                let _ = reply_tx.send(Ok((writer, reader)));
            }
            Syscall::ConnectPipe { name, reply_tx } => {
                let result = match self.pipe_registry.resolve_name(&name) {
                    Some(pipe_id) => match self.pipe_registry.take_parked_reader(&pipe_id) {
                        Some(reader) => {
                            self.pipe_registry.set_reader(&pipe_id, syscall_sender);
                            Ok(reader)
                        }
                        None => Err(KernelError::Other {
                            message: format!(
                                "named pipe '{name}' has no parked reader (already taken or not \
                                 parked)"
                            )
                            .into(),
                        }),
                    },
                    None => Err(KernelError::Other {
                        message: format!("named pipe not found: {name}").into(),
                    }),
                };
                let _ = reply_tx.send(result);
            }
            Syscall::RequestApproval {
                principal: _,
                tool_name,
                summary,
                reply_tx,
            } => {
                let approval = Arc::clone(security.approval());
                let policy = approval.policy();
                let req = crate::security::ApprovalRequest {
                    id: uuid::Uuid::new_v4(),
                    session_key: syscall_sender,
                    tool_name: tool_name.clone(),
                    tool_args: serde_json::json!({"summary": &summary}),
                    summary,
                    risk_level: crate::security::ApprovalManager::classify_risk(&tool_name),
                    requested_at: Timestamp::now(),
                    timeout_secs: policy.timeout_secs,
                };

                // Spawn a task so the event loop is not blocked while waiting
                // for human approval.
                tokio::spawn(async move {
                    let decision = approval.request_approval(req).await;
                    let approved = matches!(decision, crate::security::ApprovalDecision::Approved);
                    let _ = reply_tx.send(Ok(approved));
                });
            }
            Syscall::GetToolRegistry { reply_tx } => {
                let mut registry = self.tool_registry.as_ref().clone();
                if process_table.contains(&syscall_sender) {
                    let tape_name = syscall_sender.to_string();
                    let syscall_tool = SyscallTool::new(
                        kernel_handle.clone(),
                        syscall_sender,
                    );
                    registry.register(Arc::new(syscall_tool));
                    let tape_tool = TapeTool::new(
                        self.tape_service.clone(),
                        tape_name,
                    );
                    registry.register(Arc::new(tape_tool));
                }
                let _ = reply_tx.send(Arc::new(registry));
            }
            Syscall::PublishEvent {
                event_type,
                payload: _,
            } => {
                self.event_bus
                    .publish(crate::notification::KernelNotification::ToolExecuted {
                        session_key: syscall_sender,
                        tool_name:   format!("event:{event_type}"),
                        success:     true,
                        timestamp:   Timestamp::now(),
                    })
                    .await;
            }
        }
    }

    // -----------------------------------------------------------------------
    // Syscall helper methods
    // -----------------------------------------------------------------------

    /// Store a value in an agent's private memory namespace.
    async fn do_mem_store(
        &self,
        memory_quota: usize,
        session_key: SessionKey,
        _principal: &Principal,
        key: &str,
        value: serde_json::Value,
    ) -> crate::error::Result<()> {
        let namespaced = format!("agent:{}:{}", session_key.0, key);

        // Check quota before inserting — only if this is a new key.
        if !self.shared_kv.contains_key(&namespaced).await {
            let max = memory_quota;
            if max > 0 {
                let prefix = format!("agent:{}:", session_key.0);
                let count = self.shared_kv.count_prefix(&prefix).await;
                if count >= max {
                    return Err(KernelError::MemoryQuotaExceeded {
                        session_key,
                        current: count,
                        max,
                    });
                }
            }
        }

        self.shared_kv
            .set(&namespaced, value)
            .await
            .whatever_context::<_, KernelError>("KV store error")?;

        Ok(())
    }

    /// Validate scope permissions for shared memory operations.
    fn check_scope_permission(
        session_key: SessionKey,
        principal: &Principal,
        scope: &KvScope,
    ) -> crate::error::Result<()> {
        match scope {
            KvScope::Global | KvScope::Team(_) => {
                if !principal.is_admin() {
                    return Err(KernelError::MemoryScopeDenied {
                        reason: format!(
                            "agent {} (role {:?}) cannot access {:?} scope — requires Root or \
                             Admin",
                            session_key, principal.role, scope,
                        ),
                    });
                }
            }
            KvScope::Agent(target_id) => {
                if *target_id != session_key.0 && !principal.is_admin() {
                    return Err(KernelError::MemoryScopeDenied {
                        reason: format!(
                            "agent {} cannot access agent {}'s scope — not admin",
                            session_key, target_id,
                        ),
                    });
                }
            }
        }
        Ok(())
    }

    /// Build a scoped key from a KvScope.
    fn scoped_key(scope: &KvScope, key: &str) -> String {
        match scope {
            KvScope::Global => key.to_string(),
            KvScope::Team(name) => format!("team:{name}:{key}"),
            KvScope::Agent(id) => format!("agent:{id}:{key}"),
        }
    }

    /// Store a value in a shared (scoped) memory namespace.
    async fn do_shared_store(
        &self,
        session_key: SessionKey,
        principal: &Principal,
        scope: &KvScope,
        key: &str,
        value: serde_json::Value,
    ) -> crate::error::Result<()> {
        Self::check_scope_permission(session_key, principal, scope)?;
        let scoped = Self::scoped_key(scope, key);
        self.shared_kv
            .set(&scoped, value)
            .await
            .whatever_context::<_, KernelError>("KV store error")?;
        Ok(())
    }

    /// Recall a value from a shared (scoped) memory namespace.
    async fn do_shared_recall(
        &self,
        session_key: SessionKey,
        principal: &Principal,
        scope: &KvScope,
        key: &str,
    ) -> crate::error::Result<Option<serde_json::Value>> {
        Self::check_scope_permission(session_key, principal, scope)?;
        let scoped = Self::scoped_key(scope, key);
        Ok(self.shared_kv.get(&scoped).await)
    }
}

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

/// Unified LLM-callable tool wrapping all session-scoped kernel syscall
/// operations (process management, memory, events).
pub struct SyscallTool {
    handle:      KernelHandle,
    session_key: SessionKey,
}

impl SyscallTool {
    pub fn new(handle: KernelHandle, session_key: SessionKey) -> Self {
        Self {
            handle,
            session_key,
        }
    }

    fn available_agents(&self) -> Vec<String> {
        self.handle
            .agent_registry()
            .list()
            .iter()
            .map(|m| m.name.clone())
            .collect()
    }

    fn resolve_manifest(&self, name: &str) -> Result<AgentManifest, anyhow::Error> {
        self.handle.agent_registry().get(name).ok_or_else(|| {
            anyhow::anyhow!(
                "unknown agent: '{}'. Available agents: {:?}",
                name,
                self.available_agents()
            )
        })
    }

    // ========================================================================
    // Spawn
    // ========================================================================

    /// Look up the principal for the current session from the process table.
    fn principal(&self) -> Result<Principal, anyhow::Error> {
        self.handle
            .process_table()
            .with(&self.session_key, |p| p.principal.clone())
            .ok_or_else(|| anyhow::anyhow!("session not found: {}", self.session_key))
    }

    async fn exec_spawn(
        &self,
        agent_name: &str,
        task: &str,
    ) -> Result<serde_json::Value, anyhow::Error> {
        let manifest = self.resolve_manifest(agent_name)?;
        let principal = self.principal()?;

        info!(
            agent = agent_name,
            task = task,
            "kernel: spawning single agent"
        );

        let agent_handle = self
            .handle
            .spawn_child(&self.session_key, &principal, manifest, task.to_string())
            .await
            .map_err(|e| anyhow::anyhow!("spawn failed: {e}"))?;

        let child_key = agent_handle.session_key;

        let result = agent_handle.result_rx.await.map_err(|_| {
            anyhow::anyhow!("agent {} was dropped without producing a result", child_key)
        })?;

        Ok(serde_json::json!({
            "agent_id": child_key.to_string(),
            "output": result.output,
            "iterations": result.iterations,
            "tool_calls": result.tool_calls,
        }))
    }

    async fn exec_spawn_parallel(
        &self,
        tasks: Vec<SpawnRequest>,
    ) -> Result<serde_json::Value, anyhow::Error> {
        info!(count = tasks.len(), "kernel: spawning agents in parallel");
        let principal = self.principal()?;

        let mut handles: Vec<(String, AgentHandle)> = Vec::new();
        for task_req in &tasks {
            let manifest = self.resolve_manifest(&task_req.agent)?;
            match self
                .handle
                .spawn_child(
                    &self.session_key,
                    &principal,
                    manifest,
                    task_req.task.clone(),
                )
                .await
            {
                Ok(h) => handles.push((task_req.agent.clone(), h)),
                Err(e) => {
                    warn!(
                        agent = %task_req.agent,
                        error = %e,
                        "failed to spawn parallel agent"
                    );
                }
            }
        }

        let mut results = Vec::new();
        for (agent_name, handle) in handles {
            let agent_id = handle.session_key;
            match handle.result_rx.await {
                Ok(result) => {
                    results.push(serde_json::json!({
                        "agent": agent_name,
                        "agent_id": agent_id.to_string(),
                        "output": result.output,
                        "iterations": result.iterations,
                        "tool_calls": result.tool_calls,
                    }));
                }
                Err(_) => {
                    results.push(serde_json::json!({
                        "agent": agent_name,
                        "agent_id": agent_id.to_string(),
                        "error": "agent was dropped without producing a result",
                    }));
                }
            }
        }

        Ok(serde_json::json!({
            "results": results,
            "total": results.len(),
        }))
    }

    // ========================================================================
    // Process queries & signals
    // ========================================================================

    async fn exec_status(&self, target: &str) -> anyhow::Result<serde_json::Value> {
        let target_key = parse_session_key(target)?;
        let info = self
            .handle
            .session_status(target_key)
            .await
            .map_err(|e| anyhow::anyhow!("status failed: {e}"))?;
        Ok(serde_json::json!({
            "agent_id": info.session_key.to_string(),
            "name": info.manifest_name,
            "state": info.state.to_string(),
            "parent_id": info.parent_id.map(|id| id.to_string()),
        }))
    }

    async fn exec_children(&self) -> anyhow::Result<serde_json::Value> {
        let children = self.handle.session_children(self.session_key).await;
        let list: Vec<serde_json::Value> = children
            .iter()
            .map(|c| {
                serde_json::json!({
                    "agent_id": c.session_key.to_string(),
                    "name": c.manifest_name,
                    "state": c.state.to_string(),
                })
            })
            .collect();
        Ok(serde_json::json!({ "children": list, "count": list.len() }))
    }

    async fn exec_signal(&self, target: &str, signal: &str) -> anyhow::Result<serde_json::Value> {
        let target_key = parse_session_key(target)?;
        let sig = match signal {
            "kill" => crate::session::Signal::Kill,
            "pause" => crate::session::Signal::Pause,
            "resume" => crate::session::Signal::Resume,
            _ => unreachable!(),
        };
        self.handle
            .send_signal(target_key, sig)
            .map_err(|e| anyhow::anyhow!("{signal} failed: {e}"))?;
        Ok(serde_json::json!({ "ok": true, "signal": signal, "target": target }))
    }

    // ========================================================================
    // Memory
    // ========================================================================

    async fn exec_mem_store(
        &self,
        key: &str,
        value: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let principal = self.principal()?;
        self.handle
            .mem_store(&self.session_key, &principal, key, value)
            .await
            .map_err(|e| anyhow::anyhow!("mem_store failed: {e}"))?;
        Ok(serde_json::json!({ "ok": true }))
    }

    async fn exec_mem_recall(&self, key: &str) -> anyhow::Result<serde_json::Value> {
        let value = self
            .handle
            .mem_recall(self.session_key, key)
            .await
            .map_err(|e| anyhow::anyhow!("mem_recall failed: {e}"))?;
        Ok(serde_json::json!({ "key": key, "value": value }))
    }

    async fn exec_shared_store(
        &self,
        scope: &str,
        key: &str,
        value: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let principal = self.principal()?;
        let scope = parse_scope(scope)?;
        self.handle
            .shared_store(self.session_key, &principal, scope, key, value)
            .await
            .map_err(|e| anyhow::anyhow!("shared_store failed: {e}"))?;
        Ok(serde_json::json!({ "ok": true }))
    }

    async fn exec_shared_recall(
        &self,
        scope: &str,
        key: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let principal = self.principal()?;
        let scope = parse_scope(scope)?;
        let value = self
            .handle
            .shared_recall(self.session_key, &principal, scope, key)
            .await
            .map_err(|e| anyhow::anyhow!("shared_recall failed: {e}"))?;
        Ok(serde_json::json!({ "key": key, "value": value }))
    }

    // ========================================================================
    // Events
    // ========================================================================

    async fn exec_publish(
        &self,
        event_type: &str,
        payload: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        self.handle
            .publish_event(self.session_key, event_type, payload)
            .await
            .map_err(|e| anyhow::anyhow!("publish failed: {e}"))?;
        Ok(serde_json::json!({ "ok": true }))
    }

}

// ============================================================================
// Parameter types
// ============================================================================

/// Top-level parameters: `action` selects the kernel operation.
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum SyscallParams {
    // -- Process --
    Spawn {
        agent: String,
        task:  String,
    },
    SpawnParallel {
        parallel:        Vec<SpawnRequest>,
        #[serde(default)]
        max_concurrency: Option<usize>,
    },
    Status {
        target: String,
    },
    Children,
    Kill {
        target: String,
    },
    Pause {
        target: String,
    },
    Resume {
        target: String,
    },
    // -- Memory --
    MemStore {
        key:   String,
        value: serde_json::Value,
    },
    MemRecall {
        key: String,
    },
    SharedStore {
        scope: String,
        key:   String,
        value: serde_json::Value,
    },
    SharedRecall {
        scope: String,
        key:   String,
    },
    // -- Events --
    Publish {
        event_type: String,
        payload:    serde_json::Value,
    },
}

#[derive(Debug, Deserialize)]
struct SpawnRequest {
    agent: String,
    task:  String,
}

// ============================================================================
// Helpers
// ============================================================================

fn parse_session_key(s: &str) -> anyhow::Result<SessionKey> {
    let uuid =
        uuid::Uuid::parse_str(s).map_err(|e| anyhow::anyhow!("invalid session key '{s}': {e}"))?;
    Ok(SessionKey(uuid))
}

fn parse_scope(scope: &str) -> anyhow::Result<KvScope> {
    match scope {
        "global" => Ok(KvScope::Global),
        s if s.starts_with("team:") => {
            Ok(KvScope::Team(s.strip_prefix("team:").unwrap().to_string()))
        }
        s if s.starts_with("agent:") => {
            let uuid_str = s.strip_prefix("agent:").unwrap();
            let uuid = uuid::Uuid::parse_str(uuid_str)
                .map_err(|e| anyhow::anyhow!("invalid agent UUID in scope: {e}"))?;
            Ok(KvScope::Agent(uuid))
        }
        _ => Err(anyhow::anyhow!(
            "invalid scope '{scope}'. Expected 'global', 'team:<name>', or 'agent:<uuid>'"
        )),
    }
}

// ============================================================================
// AgentTool impl
// ============================================================================

#[async_trait]
impl crate::tool::AgentTool for SyscallTool {
    fn name(&self) -> &str { "kernel" }

    fn description(&self) -> &str {
        "Interact with the kernel: spawn agents, query process status, send signals, manage \
         memory (private & shared), and publish events. Set the 'action' field to select the \
         operation."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        // FIXME: we should not expose all internal agent for syscall !.
        let agents = self.available_agents();
        serde_json::json!({
            "type": "object",
            "required": ["action"],
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "spawn", "spawn_parallel",
                        "status", "children", "kill", "pause", "resume",
                        "mem_store", "mem_recall",
                        "shared_store", "shared_recall",
                        "publish"
                    ],
                    "description": "The kernel operation to perform."
                },
                "agent": {
                    "type": "string",
                    "description": format!("Agent name for spawn. Available: {:?}", agents),
                    "enum": agents,
                },
                "task": {
                    "type": "string",
                    "description": "Task description for spawn"
                },
                "parallel": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "agent": { "type": "string" },
                            "task":  { "type": "string" }
                        },
                        "required": ["agent", "task"]
                    },
                    "description": "Array of {agent, task} for spawn_parallel"
                },
                "max_concurrency": {
                    "type": "integer",
                    "description": "Max concurrent agents for spawn_parallel"
                },
                "target": {
                    "type": "string",
                    "description": "Target agent ID (UUID) for status/kill/pause/resume"
                },
                "key": {
                    "type": "string",
                    "description": "Memory key for mem_store/mem_recall/shared_store/shared_recall"
                },
                "value": {
                    "description": "Value to store (any JSON) for mem_store/shared_store"
                },
                "scope": {
                    "type": "string",
                    "description": "Scope for shared memory: 'global', 'team:<name>', or 'agent:<uuid>'"
                },
                "event_type": {
                    "type": "string",
                    "description": "Event type string for publish"
                },
                "payload": {
                    "description": "Event payload (any JSON) for publish"
                }
            }
        })
    }

    // FIXME: don't write this like match.
    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let action: SyscallParams = serde_json::from_value(params)
            .map_err(|e| anyhow::anyhow!("invalid kernel tool params: {e}"))?;

        match action {
            SyscallParams::Spawn { agent, task } => self.exec_spawn(&agent, &task).await,
            SyscallParams::SpawnParallel {
                parallel,
                max_concurrency: _,
            } => self.exec_spawn_parallel(parallel).await,
            SyscallParams::Status { target } => self.exec_status(&target).await,
            SyscallParams::Children => self.exec_children().await,
            SyscallParams::Kill { target } => self.exec_signal(&target, "kill").await,
            SyscallParams::Pause { target } => self.exec_signal(&target, "pause").await,
            SyscallParams::Resume { target } => self.exec_signal(&target, "resume").await,
            SyscallParams::MemStore { key, value } => self.exec_mem_store(&key, value).await,
            SyscallParams::MemRecall { key } => self.exec_mem_recall(&key).await,
            SyscallParams::SharedStore { scope, key, value } => {
                self.exec_shared_store(&scope, &key, value).await
            }
            SyscallParams::SharedRecall { scope, key } => {
                self.exec_shared_recall(&scope, &key).await
            }
            SyscallParams::Publish {
                event_type,
                payload,
            } => self.exec_publish(&event_type, payload).await,
        }
    }
}
