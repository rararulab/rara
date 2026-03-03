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

//! Syscall handling — all `ProcessHandle` interactions dispatched by the
//! kernel event loop.

use std::sync::Arc;

use jiff::Timestamp;
use snafu::ResultExt;
use tracing::debug_span;

use super::runtime::RuntimeTable;
use crate::{
    audit::{AuditEvent, AuditEventType, MemoryOp},
    error::{KernelError, Result},
    event::Syscall,
    io::pipe::{self, PipeEntry},
    kernel::Kernel,
    memory::KvScope,
    process::{AgentId, ProcessInfo, principal::Principal},
};

impl Kernel {
    /// Handle a syscall from a ProcessHandle.
    ///
    /// All business logic lives here, executed by the kernel event loop.
    pub(crate) async fn handle_syscall(&self, syscall: Syscall, runtimes: &RuntimeTable) {
        let syscall_type: &'static str = (&syscall).into();
        crate::metrics::SYSCALL_TOTAL
            .with_label_values(&[syscall_type])
            .inc();
        let syscall_agent_id = syscall.agent_id();
        let span = debug_span!(
            "handle_syscall",
            syscall_type,
            agent_id = %syscall_agent_id,
        );
        let _guard = span.enter();

        match syscall {
            Syscall::QueryStatus { target, reply_tx } => {
                let result = self
                    .process_table()
                    .get(target)
                    .map(|p| ProcessInfo::from(&p))
                    .ok_or(KernelError::ProcessNotFound {
                        id: target.to_string(),
                    });
                let _ = reply_tx.send(result);
            }

            Syscall::QueryChildren { parent, reply_tx } => {
                let children = self.process_table().children_of(parent);
                let _ = reply_tx.send(children);
            }

            Syscall::MemStore {
                agent_id,
                session_id,
                principal,
                key,
                value,
                reply_tx,
            } => {
                let result = self
                    .do_mem_store(
                        self.config().memory_quota_per_agent,
                        agent_id,
                        &session_id,
                        &principal,
                        &key,
                        value,
                    )
                    .await;
                let _ = reply_tx.send(result);
            }

            Syscall::MemRecall {
                agent_id,
                key,
                reply_tx,
            } => {
                let namespaced = format!("agent:{}:{}", agent_id.0, key);
                let result = Ok(self.shared_kv().get(&namespaced).await);
                let _ = reply_tx.send(result);
            }

            Syscall::SharedStore {
                agent_id,
                principal,
                scope,
                key,
                value,
                reply_tx,
            } => {
                let result = self
                    .do_shared_store(agent_id, &principal, &scope, &key, value)
                    .await;
                let _ = reply_tx.send(result);
            }

            Syscall::SharedRecall {
                agent_id,
                principal,
                scope,
                key,
                reply_tx,
            } => {
                let result = self
                    .do_shared_recall(agent_id, &principal, &scope, &key)
                    .await;
                let _ = reply_tx.send(result);
            }

            Syscall::CreatePipe {
                owner,
                target,
                reply_tx,
            } => {
                let (writer, reader) = pipe::pipe(64);
                self.pipe_registry().register(
                    writer.pipe_id().clone(),
                    PipeEntry {
                        owner,
                        reader: Some(target),
                        created_at: Timestamp::now(),
                    },
                );
                let _ = reply_tx.send(Ok((writer, reader)));
            }

            Syscall::CreateNamedPipe {
                owner,
                name,
                reply_tx,
            } => {
                if self.pipe_registry().resolve_name(&name).is_some() {
                    let _ = reply_tx.send(Err(KernelError::Other {
                        message: format!("named pipe already exists: {name}").into(),
                    }));
                    return;
                }
                let (writer, reader) = pipe::pipe(64);
                let pipe_id = writer.pipe_id().clone();
                self.pipe_registry().register_named(
                    name,
                    pipe_id,
                    PipeEntry {
                        owner,
                        reader: None,
                        created_at: Timestamp::now(),
                    },
                );
                let _ = reply_tx.send(Ok((writer, reader)));
            }

            Syscall::ConnectPipe {
                connector,
                name,
                reply_tx,
            } => {
                let result = match self.pipe_registry().resolve_name(&name) {
                    Some(pipe_id) => match self.pipe_registry().take_parked_reader(&pipe_id) {
                        Some(reader) => {
                            self.pipe_registry().set_reader(&pipe_id, connector);
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
                let result = self.security().requires_approval(&tool_name);
                let _ = reply_tx.send(result);
            }

            Syscall::RequestApproval {
                agent_id,
                principal: _,
                tool_name,
                summary,
                reply_tx,
            } => {
                let approval = Arc::clone(self.security().approval());
                let policy = approval.policy();
                let req = crate::security::approval::ApprovalRequest {
                    id: uuid::Uuid::new_v4(),
                    agent_id,
                    tool_name: tool_name.clone(),
                    tool_args: serde_json::json!({"summary": &summary}),
                    summary,
                    risk_level: crate::security::approval::ApprovalManager::classify_risk(&tool_name),
                    requested_at: Timestamp::now(),
                    timeout_secs: policy.timeout_secs,
                };

                // Spawn a task so the event loop is not blocked while waiting
                // for human approval.
                tokio::spawn(async move {
                    let decision = approval.request_approval(req).await;
                    let approved = matches!(decision, crate::security::approval::ApprovalDecision::Approved);
                    let _ = reply_tx.send(Ok(approved));
                });
            }

            Syscall::CheckGuardBatch {
                agent_id,
                session_id,
                checks,
                reply_tx,
            } => {
                let (user_id, session_uuid) = self
                    .process_table()
                    .get(agent_id)
                    .map(|proc| {
                        (
                            proc.principal.user_id.0.clone(),
                            proc.session_id.to_string(),
                        )
                    })
                    .unwrap_or_else(|| (String::new(), session_id.to_string()));

                let ctx = crate::guard::GuardContext {
                    agent_id:   agent_id.0,
                    user_id:    uuid::Uuid::parse_str(&user_id).unwrap_or(uuid::Uuid::nil()),
                    session_id: uuid::Uuid::parse_str(&session_uuid).unwrap_or(uuid::Uuid::nil()),
                };

                let security = Arc::clone(&self.security());
                tokio::spawn(async move {
                    let verdicts = security.check_guard_batch(&ctx, &checks).await;
                    let _ = reply_tx.send(verdicts);
                });
            }

            Syscall::GetManifest { agent_id, reply_tx } => {
                let result = self
                    .process_table()
                    .get(agent_id)
                    .map(|p| p.manifest.clone())
                    .ok_or(KernelError::ProcessNotFound {
                        id: agent_id.to_string(),
                    });
                let _ = reply_tx.send(result);
            }

            Syscall::GetToolRegistry { agent_id, reply_tx } => {
                let mut registry = self.tool_registry().as_ref().clone();
                if let Some(rt) = runtimes.get(&agent_id) {
                    let syscall_tool = crate::handle::syscall_tool::SyscallTool::new(
                        Arc::clone(&rt.handle),
                        Arc::clone(self.agent_registry()),
                    );
                    registry.register_builtin(Arc::new(syscall_tool));
                }
                let _ = reply_tx.send(Arc::new(registry));
            }

            Syscall::ResolveDriver { agent_id, reply_tx } => {
                let result = match self.process_table().get(agent_id) {
                    Some(process) => self.driver_registry().resolve(
                        &process.manifest.name,
                        process.manifest.provider_hint.as_deref(),
                        process.manifest.model.as_deref(),
                    ),
                    None => Err(KernelError::ProcessNotFound {
                        id: agent_id.to_string(),
                    }),
                };
                let _ = reply_tx.send(result);
            }

            Syscall::PublishEvent {
                agent_id,
                event_type,
                payload: _,
            } => {
                self.event_bus()
                    .publish(crate::notification::KernelNotification::ToolExecuted {
                        agent_id:  agent_id.0,
                        tool_name: format!("event:{event_type}"),
                        success:   true,
                        timestamp: Timestamp::now(),
                    })
                    .await;
            }

            Syscall::RecordToolCall {
                agent_id,
                tool_name,
                args,
                result,
                success,
                duration_ms,
            } => {
                let agent_name = self
                    .process_table()
                    .get(agent_id)
                    .map(|p| p.manifest.name)
                    .unwrap_or_else(|| "unknown".to_string());
                crate::metrics::record_turn_tool_call(&agent_name, &tool_name);
                self.audit()
                    .record_tool_call(agent_id, &tool_name, &args, &result, success, duration_ms)
                    .await;
            }
        }
    }

    // -----------------------------------------------------------------------
    // Syscall helper methods
    // -----------------------------------------------------------------------

    /// Store a value in an agent's private memory namespace.
    pub(crate) async fn do_mem_store(
        &self,
        memory_quota: usize,
        agent_id: AgentId,
        session_id: &crate::process::SessionId,
        principal: &Principal,
        key: &str,
        value: serde_json::Value,
    ) -> Result<()> {
        let namespaced = format!("agent:{}:{}", agent_id.0, key);

        // Check quota before inserting — only if this is a new key.
        if !self.shared_kv().contains_key(&namespaced).await {
            let max = memory_quota;
            if max > 0 {
                let prefix = format!("agent:{}:", agent_id.0);
                let count = self.shared_kv().count_prefix(&prefix).await;
                if count >= max {
                    return Err(KernelError::MemoryQuotaExceeded {
                        agent_id: agent_id.to_string(),
                        current: count,
                        max,
                    });
                }
            }
        }

        self.shared_kv()
            .set(&namespaced, value)
            .await
            .whatever_context::<_, KernelError>("KV store error")?;

        // Audit: MemoryAccess (Store)
        self.audit().record(AuditEvent {
            timestamp: Timestamp::now(),
            agent_id,
            session_id: session_id.clone(),
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
    pub(crate) fn check_scope_permission(
        agent_id: AgentId,
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
                            agent_id, principal.role, scope,
                        ),
                    });
                }
            }
            KvScope::Agent(target_id) => {
                if *target_id != agent_id.0 && !principal.is_admin() {
                    return Err(KernelError::MemoryScopeDenied {
                        reason: format!(
                            "agent {} cannot access agent {}'s scope — not admin",
                            agent_id, target_id,
                        ),
                    });
                }
            }
        }
        Ok(())
    }

    /// Build a scoped key from a KvScope.
    pub(crate) fn scoped_key(scope: &KvScope, key: &str) -> String {
        match scope {
            KvScope::Global => key.to_string(),
            KvScope::Team(name) => format!("team:{name}:{key}"),
            KvScope::Agent(id) => format!("agent:{id}:{key}"),
        }
    }

    /// Store a value in a shared (scoped) memory namespace.
    pub(crate) async fn do_shared_store(
        &self,
        agent_id: AgentId,
        principal: &Principal,
        scope: &KvScope,
        key: &str,
        value: serde_json::Value,
    ) -> Result<()> {
        Self::check_scope_permission(agent_id, principal, scope)?;
        let scoped = Self::scoped_key(scope, key);
        self.shared_kv()
            .set(&scoped, value)
            .await
            .whatever_context::<_, KernelError>("KV store error")?;
        Ok(())
    }

    /// Recall a value from a shared (scoped) memory namespace.
    pub(crate) async fn do_shared_recall(
        &self,
        agent_id: AgentId,
        principal: &Principal,
        scope: &KvScope,
        key: &str,
    ) -> Result<Option<serde_json::Value>> {
        Self::check_scope_permission(agent_id, principal, scope)?;
        let scoped = Self::scoped_key(scope, key);
        Ok(self.shared_kv().get(&scoped).await)
    }
}
