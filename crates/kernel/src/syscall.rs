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

//! Syscall dispatcher — handles all `ProcessHandle` interactions dispatched
//! by the kernel event loop.
//!
//! Extracted from `event_loop/syscall.rs` to encapsulate the kernel
//! sub-components used exclusively by syscall handling (shared KV, pipe
//! registry, driver registry, tool registry, event bus, config).

use std::sync::Arc;

use jiff::Timestamp;
use snafu::ResultExt;
use tracing::debug_span;

use crate::{
    SessionInfo,
    audit::{AuditEvent, AuditEventType, AuditRef, MemoryOp},
    error::{KernelError, Result},
    event::{Syscall, SyscallEnvelope},
    event_loop::runtime::RuntimeTable,
    io::pipe::{self, PipeEntry, PipeRegistry},
    kernel::KernelConfig,
    kv::{KvScope, SharedKv},
    llm::DriverRegistryRef,
    notification::NotificationBusRef,
    process::{ProcessTable, agent_registry::AgentRegistryRef, principal::Principal},
    security::SecurityRef,
    session::SessionKey,
    tool::ToolRegistryRef,
};

/// Dispatches syscalls from `ProcessHandle` to the appropriate kernel
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
    ) -> Self {
        Self {
            shared_kv,
            pipe_registry,
            driver_registry,
            tool_registry,
            event_bus,
            config,
        }
    }

    // -- Dispatch -----------------------------------------------------------

    /// Handle a syscall from a ProcessHandle.
    ///
    /// All business logic lives here, executed by the kernel event loop.
    /// TODO: implement dispatch by using `syscallEnvelope` to route to more
    /// specific handlers (e.g. `handle_mem_syscall`, `handle_pipe_syscall`,
    /// etc.) for better organization and readability.
    pub async fn dispatch(
        &self,
        syscall: SyscallEnvelope,
        process_table: &ProcessTable,
        runtimes: &RuntimeTable,
        security: &SecurityRef,
        audit: &AuditRef,
        agent_registry: &AgentRegistryRef,
    ) {
        let syscall_sender = syscall.session_key();
        let syscall = syscall.payload;
        let syscall_type: &'static str = (&syscall).into();
        crate::metrics::SYSCALL_TOTAL
            .with_label_values(&[syscall_type])
            .inc();
        let syscall_session_key = syscall.session_key();
        let span = debug_span!(
            "handle_syscall",
            syscall_type,
            session_key = %syscall_session_key,
        );
        let _guard = span.enter();

        match syscall {
            Syscall::QueryStatus { reply_tx } => {
                let result = process_table
                    .get(syscall_sender)
                    .map(|p| SessionInfo::from(&p))
                    .ok_or(KernelError::ProcessNotFound {
                        id: syscall_sender.to_string(),
                    });
                let _ = reply_tx.send(result);
            }

            Syscall::QueryChildren { reply_tx } => {
                let children = process_table.children_of(syscall_sender);
                let _ = reply_tx.send(children);
            }

            Syscall::MemStore {
                session_key,
                session_key: session_id, // WHAT ?
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
                        audit,
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
                let (writer, reader) = pipe::pipe(64);
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
                let (writer, reader) = pipe::pipe(64);
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

            Syscall::RequiresApproval {
                tool_name,
                reply_tx,
            } => {
                let result = security.requires_approval(&tool_name);
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

            Syscall::CheckGuardBatch {
                session_id,
                checks,
                reply_tx,
            } => {
                let (user_id, session_uuid) = process_table
                    .get(syscall_session_key)
                    .map(|proc| {
                        (
                            proc.principal.user_id.0.clone(),
                            proc.session_id.to_string(),
                        )
                    })
                    .unwrap_or_else(|| (String::new(), session_id.to_string()));

                let ctx = crate::guard::GuardContext {
                    session_key: syscall_session_key,
                    user_id:     uuid::Uuid::parse_str(&user_id).unwrap_or(uuid::Uuid::nil()),
                    session_id:  uuid::Uuid::parse_str(&session_uuid).unwrap_or(uuid::Uuid::nil()),
                };

                let security = Arc::clone(security);
                tokio::spawn(async move {
                    let verdicts = security.check_guard_batch(&ctx, &checks).await;
                    let _ = reply_tx.send(verdicts);
                });
            }

            Syscall::GetManifest {
                session_key,
                reply_tx,
            } => {
                let result = process_table
                    .get(session_key)
                    .map(|p| p.manifest.clone())
                    .ok_or(KernelError::ProcessNotFound {
                        id: session_key.to_string(),
                    });
                let _ = reply_tx.send(result);
            }

            Syscall::GetToolRegistry {
                session_key,
                reply_tx,
            } => {
                let mut registry = self.tool_registry.as_ref().clone();
                if let Some(syscall_tool) = runtimes.with(&session_key, |rt| {
                    crate::handle::syscall_tool::SyscallTool::new(
                        Arc::clone(&rt.handle),
                        Arc::clone(agent_registry),
                    )
                }) {
                    registry.register_builtin(Arc::new(syscall_tool));
                }
                let _ = reply_tx.send(Arc::new(registry));
            }

            Syscall::ResolveDriver {
                session_key,
                reply_tx,
            } => {
                let result = match process_table.get(session_key) {
                    Some(process) => self.driver_registry.resolve(
                        &process.manifest.name,
                        process.manifest.provider_hint.as_deref(),
                        process.manifest.model.as_deref(),
                    ),
                    None => Err(KernelError::ProcessNotFound {
                        id: session_key.to_string(),
                    }),
                };
                let _ = reply_tx.send(result);
            }

            Syscall::PublishEvent {
                session_key,
                event_type,
                payload: _,
            } => {
                self.event_bus
                    .publish(crate::notification::KernelNotification::ToolExecuted {
                        session_key: session_key.0,
                        tool_name:   format!("event:{event_type}"),
                        success:     true,
                        timestamp:   Timestamp::now(),
                    })
                    .await;
            }

            Syscall::RecordToolCall {
                session_key,
                tool_name,
                args,
                result,
                success,
                duration_ms,
            } => {
                let agent_name = process_table
                    .get(session_key)
                    .map(|p| p.manifest.name)
                    .unwrap_or_else(|| "unknown".to_string());
                crate::metrics::record_turn_tool_call(&agent_name, &tool_name);
                audit
                    .record_tool_call(
                        session_key,
                        &tool_name,
                        &args,
                        &result,
                        success,
                        duration_ms,
                    )
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
        principal: &Principal,
        key: &str,
        value: serde_json::Value,
        audit: &AuditRef,
    ) -> Result<()> {
        let namespaced = format!("agent:{}:{}", session_key.0, key);

        // Check quota before inserting — only if this is a new key.
        if !self.shared_kv.contains_key(&namespaced).await {
            let max = memory_quota;
            if max > 0 {
                let prefix = format!("agent:{}:", session_key.0);
                let count = self.shared_kv.count_prefix(&prefix).await;
                if count >= max {
                    return Err(KernelError::MemoryQuotaExceeded {
                        session_key: session_key.to_string(),
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

        // Audit: MemoryAccess (Store)
        audit.record(AuditEvent {
            timestamp: Timestamp::now(),
            session_key,
            session_key: session_id.clone(),
            user_id: principal.user_id.clone(),
            event_type: AuditEventType::MemoryAccess {
                operation: MemoryOp::Store,
                key:       key.to_string(),
            },
            details: serde_json::Value::Null,
        });

        Ok(())
    }

    /// Validate scope permissions for shared memory operations.
    fn check_scope_permission(
        session_key: SessionKey,
        principal: &Principal,
        scope: &KvScope,
    ) -> Result<()> {
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
    ) -> Result<()> {
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
    ) -> Result<Option<serde_json::Value>> {
        Self::check_scope_permission(session_key, principal, scope)?;
        let scoped = Self::scoped_key(scope, key);
        Ok(self.shared_kv.get(&scoped).await)
    }
}
