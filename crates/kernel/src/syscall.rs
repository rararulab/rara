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
    notification::{NotificationBusRef, SubscriptionRegistryRef},
    security::SecurityRef,
    session::{SessionKey, SessionTable},
    tool::{DynamicToolProviderRef, ToolRegistryRef, tape::TapeTool},
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
    shared_kv:             SharedKv,
    /// Inter-agent pipe registry for streaming data between agents.
    pipe_registry:         PipeRegistry,
    /// Multi-driver LLM registry with per-agent overrides.
    driver_registry:       DriverRegistryRef,
    /// Global tool registry.
    tool_registry:         ToolRegistryRef,
    /// Event bus for publishing kernel notifications.
    event_bus:             NotificationBusRef,
    /// Kernel configuration.
    config:                KernelConfig,
    /// Tape service for session message persistence (passed to SyscallTool).
    tape_service:          TapeService,
    /// Optional provider of dynamically discovered tools (e.g. MCP servers).
    dynamic_tool_provider: Option<DynamicToolProviderRef>,
    /// Scheduled task wheel for job scheduling.
    job_wheel:             Arc<std::sync::Mutex<crate::schedule::JobWheel>>,
    /// Tag-based subscription registry for task notifications.
    subscription_registry: SubscriptionRegistryRef,
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
        dynamic_tool_provider: Option<DynamicToolProviderRef>,
        subscription_registry: SubscriptionRegistryRef,
    ) -> Self {
        let jobs_path = rara_paths::config_dir().join("scheduler").join("jobs.json");
        let job_wheel = Arc::new(std::sync::Mutex::new(crate::schedule::JobWheel::load(
            jobs_path,
        )));
        Self {
            shared_kv,
            pipe_registry,
            driver_registry,
            tool_registry,
            event_bus,
            config,
            tape_service,
            dynamic_tool_provider,
            job_wheel,
            subscription_registry,
        }
    }

    /// Access the global tool registry.
    pub fn tool_registry(&self) -> &ToolRegistryRef { &self.tool_registry }

    pub fn driver_registry(&self) -> &DriverRegistryRef { &self.driver_registry }

    /// Access the notification bus for publishing kernel events.
    pub fn event_bus(&self) -> &NotificationBusRef { &self.event_bus }

    /// Access the job wheel (for tick-based drain in the event loop).
    pub fn job_wheel(&self) -> &Arc<std::sync::Mutex<crate::schedule::JobWheel>> { &self.job_wheel }

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
                    let syscall_tool = SyscallTool::new(kernel_handle.clone(), syscall_sender);
                    registry.register(Arc::new(syscall_tool));
                    let tape_tool = TapeTool::new(
                        self.tape_service.clone(),
                        tape_name,
                        Arc::clone(kernel_handle.session_index()),
                    );
                    registry.register(Arc::new(tape_tool));
                    // Schedule tools
                    registry.register(Arc::new(crate::tool::schedule::ScheduleOnceTool));
                    registry.register(Arc::new(crate::tool::schedule::ScheduleIntervalTool));
                    registry.register(Arc::new(crate::tool::schedule::ScheduleCronTool));
                    registry.register(Arc::new(crate::tool::schedule::ScheduleRemoveTool));
                    registry.register(Arc::new(crate::tool::schedule::ScheduleListTool));
                    // Background agent tools
                    registry.register(Arc::new(
                        crate::tool::spawn_background::SpawnBackgroundTool::new(
                            kernel_handle.clone(),
                            syscall_sender,
                        ),
                    ));
                    registry.register(Arc::new(
                        crate::tool::cancel_background::CancelBackgroundTool::new(
                            kernel_handle.clone(),
                            syscall_sender,
                        ),
                    ));
                    // Fold-branch tool (synchronous child with result compression)
                    if let Ok((fold_driver, fold_model)) = self.driver_registry.resolve(
                        "fold-branch",
                        None,
                        self.config.context_folding.fold_model.as_deref(),
                    ) {
                        let context_folder = Arc::new(crate::agent::fold::ContextFolder::new(
                            fold_driver,
                            fold_model,
                        ));
                        registry.register(Arc::new(crate::tool::fold_branch::FoldBranchTool::new(
                            kernel_handle.clone(),
                            syscall_sender,
                            context_folder,
                        )));
                    }
                    // Plan tools
                    registry.register(Arc::new(crate::tool::create_plan::CreatePlanTool));
                }
                // Inject dynamic tools (e.g. MCP server tools).
                if let Some(ref provider) = self.dynamic_tool_provider {
                    for tool in provider.tools().await {
                        registry.register(tool);
                    }
                }
                let _ = reply_tx.send(Arc::new(registry));
            }
            Syscall::PublishEvent {
                event_type,
                payload,
            } => {
                let message = payload
                    .get("message")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim())
                    .unwrap_or("");

                if message.is_empty() {
                    tracing::warn!(
                        event_type = %event_type,
                        sender = %syscall_sender,
                        payload_keys = ?payload.as_object().map(|o| o.keys().collect::<Vec<_>>()),
                        "PublishEvent dropped: payload.message is missing or blank"
                    );
                } else {
                    let _ = kernel_handle.event_queue().try_push(
                        crate::event::KernelEventEnvelope::send_notification(message.to_string()),
                    );
                }
            }
            Syscall::RegisterJob {
                trigger,
                message,
                tags,
                reply_tx,
            } => {
                let principal = process_table.with(&syscall_sender, |p| p.principal.clone());
                let result = match principal {
                    Some(principal) => {
                        let entry = crate::schedule::JobEntry {
                            id: crate::schedule::JobId::new(),
                            trigger,
                            message,
                            session_key: syscall_sender,
                            principal,
                            created_at: Timestamp::now(),
                            tags,
                        };
                        let id = entry.id.clone();
                        let wheel_ref = self.job_wheel.clone();
                        tokio::task::spawn_blocking(move || {
                            let mut wheel = wheel_ref.lock().unwrap();
                            wheel.add(entry);
                            wheel.persist();
                        })
                        .await
                        .unwrap_or_else(|e| {
                            warn!(error = %e, "spawn_blocking panicked during RegisterJob");
                        });
                        info!(job_id = %id, session = %syscall_sender, "registered scheduled job");
                        Ok(id)
                    }
                    None => Err(crate::error::KernelError::Other {
                        message: format!("session not found: {syscall_sender}").into(),
                    }),
                };
                let _ = reply_tx.send(result);
            }
            Syscall::RemoveJob { job_id, reply_tx } => {
                let wheel_ref = self.job_wheel.clone();
                let job_id_clone = job_id.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let mut wheel = wheel_ref.lock().unwrap();
                    match wheel.remove(&job_id_clone) {
                        Some(_) => {
                            wheel.persist();
                            true
                        }
                        None => false,
                    }
                })
                .await
                .unwrap_or_else(|e| {
                    warn!(error = %e, "spawn_blocking panicked during RemoveJob");
                    false
                });
                let result = if result {
                    info!(job_id = %job_id, "removed scheduled job");
                    Ok(())
                } else {
                    Err(crate::error::KernelError::Other {
                        message: format!("job not found: {job_id}").into(),
                    })
                };
                let _ = reply_tx.send(result);
            }
            Syscall::ListJobs { reply_tx } => {
                let wheel = self.job_wheel.lock().unwrap();
                let jobs = wheel.list(Some(&syscall_sender));
                let _ = reply_tx.send(Ok(jobs));
            }
            Syscall::Subscribe {
                match_tags,
                on_receive,
                reply_tx,
            } => {
                let owner = process_table.with(&syscall_sender, |p| p.principal.user_id.clone());
                match owner {
                    Some(user_id) => {
                        let sub_id = self
                            .subscription_registry
                            .subscribe(syscall_sender, user_id, match_tags, on_receive)
                            .await;
                        info!(
                            subscription_id = %sub_id,
                            session = %syscall_sender,
                            "registered task notification subscription"
                        );
                        let _ = reply_tx.send(Ok(sub_id));
                    }
                    None => {
                        let _ = reply_tx.send(Err(crate::error::KernelError::Other {
                            message: format!("session not found: {syscall_sender}").into(),
                        }));
                    }
                }
            }
            Syscall::Unsubscribe {
                subscription_id,
                reply_tx,
            } => {
                let removed = self
                    .subscription_registry
                    .unsubscribe(subscription_id)
                    .await;
                info!(
                    subscription_id = %subscription_id,
                    removed,
                    "unsubscribed from task notifications"
                );
                let _ = reply_tx.send(Ok(removed));
            }
            Syscall::PublishTaskReport { report, reply_tx } => {
                let publisher_id =
                    process_table.with(&syscall_sender, |p| p.principal.user_id.clone());
                self.handle_publish_task_report(report, publisher_id, reply_tx, kernel_handle)
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

    /// Access the subscription registry.
    pub fn subscription_registry(&self) -> &SubscriptionRegistryRef { &self.subscription_registry }

    /// Write TaskReport to source tape, match subscriptions, and deliver.
    async fn handle_publish_task_report(
        &self,
        report: crate::task_report::TaskReport,
        publisher_id: Option<crate::identity::UserId>,
        reply_tx: tokio::sync::oneshot::Sender<crate::error::Result<()>>,
        kernel_handle: &KernelHandle,
    ) {
        use crate::{
            memory::TapEntryKind,
            notification::{NotifyAction, TapeEntryRef, TaskNotification},
        };

        let source_session = report.source_session;
        let task_id = report.task_id;
        let tags = report.tags.clone();
        let summary = report.summary.clone();
        let status = report.status;
        let task_type = report.task_type.clone();
        let result = report.result.clone();
        let action_taken = report.action_taken.clone();

        // 1. Write TaskReport to source session's tape.
        let tape_name = source_session.to_string();
        let report_json = serde_json::to_value(&report).unwrap_or_default();
        let entry_id = match self
            .tape_service
            .store()
            .append(
                &tape_name,
                TapEntryKind::TaskReport,
                report_json.clone(),
                None,
            )
            .await
        {
            Ok(entry) => entry.id,
            Err(e) => {
                warn!(error = %e, "failed to append TaskReport to tape");
                let _ = reply_tx.send(Err(crate::error::KernelError::Other {
                    message: format!("tape append failed: {e}").into(),
                }));
                return;
            }
        };

        // 2. Build notification with tape entry ref and full report data.
        let notification = TaskNotification {
            task_id,
            task_type: task_type.clone(),
            tags: tags.clone(),
            status,
            summary: summary.clone(),
            result: result.clone(),
            action_taken: action_taken.clone(),
            report_ref: TapeEntryRef {
                session_key: source_session,
                entry_id,
            },
        };

        // 3. Match subscriptions scoped to publisher's identity.
        let publisher = match publisher_id {
            Some(id) => id,
            None => {
                warn!("publish_task_report: no principal for source session, skipping delivery");
                let _ = reply_tx.send(Ok(()));
                return;
            }
        };
        let matched = self
            .subscription_registry
            .match_tags(&tags, &publisher)
            .await;
        let matched_count = matched.len();
        for sub in matched {
            let notif_json = serde_json::to_value(&notification).unwrap_or_default();
            match sub.on_receive {
                NotifyAction::ProactiveTurn => {
                    // Deliver as a synthetic user message to trigger an LLM turn.
                    let result_str = serde_json::to_string(&result).unwrap_or_default();
                    let action_str = action_taken
                        .as_deref()
                        .map(|a| format!("\naction_taken: {a}"))
                        .unwrap_or_default();
                    let directive = format!(
                        "[TaskNotification] {task_type}: {summary}\nstatus: {status:?}\nresult: \
                         {result_str}{action_str}\nref: {source_session}/entry_{entry_id}"
                    );
                    let msg = crate::io::InboundMessage::synthetic(
                        directive,
                        crate::identity::UserId("system".into()),
                        sub.subscriber,
                    );
                    kernel_handle.deliver_internal(msg).await;
                }
                NotifyAction::SilentAppend => {
                    // Silently append the notification to subscriber's tape.
                    let sub_tape = sub.subscriber.to_string();
                    let _ = self
                        .tape_service
                        .store()
                        .append(&sub_tape, TapEntryKind::TaskReport, notif_json, None)
                        .await;
                }
            }
        }

        info!(
            task_id = %task_id,
            task_type,
            matched_subs = matched_count,
            "task report published and notifications delivered"
        );
        let _ = reply_tx.send(Ok(()));
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
    pub const NAME: &str = crate::tool_names::KERNEL;

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
        let mut rx = agent_handle.result_rx;
        let mut milestones = Vec::new();

        while let Some(event) = rx.recv().await {
            match event {
                crate::io::AgentEvent::Milestone { stage, detail } => {
                    milestones.push(serde_json::json!({
                        "stage": stage,
                        "detail": detail,
                    }));
                }
                crate::io::AgentEvent::Done(result) => {
                    return Ok(serde_json::json!({
                        "milestones": milestones,
                        "output": result.output,
                        "iterations": result.iterations,
                        "tool_calls": result.tool_calls,
                    }));
                }
            }
        }

        Err(anyhow::anyhow!(
            "agent {} was dropped without producing a result",
            child_key
        ))
    }

    async fn exec_spawn_parallel(
        &self,
        tasks: Vec<SpawnRequest>,
        max_concurrency: usize,
    ) -> Result<serde_json::Value, anyhow::Error> {
        info!(
            count = tasks.len(),
            max_concurrency, "kernel: spawning agents in parallel"
        );
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

        // Collect results with bounded concurrency via buffer_unordered.
        use futures::stream::{self, StreamExt};

        let results: Vec<serde_json::Value> = stream::iter(handles)
            .map(|(agent_name, handle)| async move {
                let _agent_id = handle.session_key;
                let mut rx = handle.result_rx;
                let mut milestones = Vec::new();
                let mut final_result = None;

                while let Some(event) = rx.recv().await {
                    match event {
                        crate::io::AgentEvent::Milestone { stage, detail } => {
                            milestones.push(serde_json::json!({
                                "stage": stage,
                                "detail": detail,
                            }));
                        }
                        crate::io::AgentEvent::Done(result) => {
                            final_result = Some(result);
                            break;
                        }
                    }
                }

                match final_result {
                    Some(result) => serde_json::json!({
                        "agent": agent_name,
                        "milestones": milestones,
                        "output": result.output,
                        "iterations": result.iterations,
                        "tool_calls": result.tool_calls,
                    }),
                    None => serde_json::json!({
                        "agent": agent_name,
                        "error": "agent was dropped without producing a result",
                    }),
                }
            })
            .buffer_unordered(max_concurrency)
            .collect()
            .await;

        let total = results.len();
        Ok(serde_json::json!({
            "results": results,
            "total": total,
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
            "interrupt" => crate::session::Signal::Interrupt,
            other => return Err(anyhow::anyhow!("unknown signal: {other}")),
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

    // ========================================================================
    // Task Report & Subscriptions
    // ========================================================================

    async fn exec_subscribe(
        &self,
        match_tags: Vec<String>,
        on_receive: &str,
    ) -> anyhow::Result<serde_json::Value> {
        use crate::notification::NotifyAction;
        let action = match on_receive {
            "proactive_turn" => NotifyAction::ProactiveTurn,
            "silent_append" => NotifyAction::SilentAppend,
            other => {
                return Err(anyhow::anyhow!(
                    "invalid on_receive: '{other}'. Expected 'proactive_turn' or 'silent_append'"
                ));
            }
        };
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.handle
            .syscall_push(crate::event::KernelEventEnvelope::syscall(
                self.session_key,
                crate::event::Syscall::Subscribe {
                    match_tags,
                    on_receive: action,
                    reply_tx: tx,
                },
            ))
            .await
            .map_err(|e| anyhow::anyhow!("subscribe push failed: {e}"))?;
        let sub_id = rx
            .await
            .map_err(|_| anyhow::anyhow!("subscribe: reply channel dropped"))?
            .map_err(|e| anyhow::anyhow!("subscribe failed: {e}"))?;
        Ok(serde_json::json!({ "subscription_id": sub_id.to_string() }))
    }

    async fn exec_unsubscribe(&self, subscription_id: &str) -> anyhow::Result<serde_json::Value> {
        let id: uuid::Uuid = subscription_id
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid subscription_id: {e}"))?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.handle
            .syscall_push(crate::event::KernelEventEnvelope::syscall(
                self.session_key,
                crate::event::Syscall::Unsubscribe {
                    subscription_id: id,
                    reply_tx:        tx,
                },
            ))
            .await
            .map_err(|e| anyhow::anyhow!("unsubscribe push failed: {e}"))?;
        let removed = rx
            .await
            .map_err(|_| anyhow::anyhow!("unsubscribe: reply channel dropped"))?
            .map_err(|e| anyhow::anyhow!("unsubscribe failed: {e}"))?;
        Ok(serde_json::json!({ "removed": removed }))
    }

    async fn exec_publish_report(
        &self,
        report_data: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        use crate::task_report::TaskReport;
        let mut report: TaskReport = serde_json::from_value(report_data)
            .map_err(|e| anyhow::anyhow!("invalid task report: {e}"))?;
        // Ensure source_session is set to caller.
        report.source_session = self.session_key;
        // Ensure task_type is in tags.
        if !report.tags.contains(&report.task_type) {
            report.tags.insert(0, report.task_type.clone());
        }
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.handle
            .syscall_push(crate::event::KernelEventEnvelope::syscall(
                self.session_key,
                crate::event::Syscall::PublishTaskReport {
                    report,
                    reply_tx: tx,
                },
            ))
            .await
            .map_err(|e| anyhow::anyhow!("publish_report push failed: {e}"))?;
        rx.await
            .map_err(|_| anyhow::anyhow!("publish_report: reply channel dropped"))?
            .map_err(|e| anyhow::anyhow!("publish_report failed: {e}"))?;
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
    Interrupt {
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
    // -- Task Report & Subscription --
    Subscribe {
        match_tags: Vec<String>,
        on_receive: String,
    },
    Unsubscribe {
        subscription_id: String,
    },
    PublishReport {
        report: serde_json::Value,
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
    fn name(&self) -> &str { Self::NAME }

    fn description(&self) -> &str {
        "Interact with the kernel: spawn agents, query process status, send signals, manage memory \
         (private & shared), publish events, subscribe to task notifications, and publish task \
         reports. Set the 'action' field to select the operation."
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
                        "status", "children", "kill", "pause", "resume", "interrupt",
                        "mem_store", "mem_recall",
                        "shared_store", "shared_recall",
                        "publish",
                        "subscribe", "unsubscribe", "publish_report"
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
                    "description": "Target agent ID (UUID) for status/kill/pause/resume/interrupt"
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
                },
                "match_tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Tags to match for subscribe"
                },
                "on_receive": {
                    "type": "string",
                    "enum": ["proactive_turn", "silent_append"],
                    "description": "Action when notification matches: proactive_turn or silent_append"
                },
                "subscription_id": {
                    "type": "string",
                    "description": "Subscription UUID for unsubscribe"
                },
                "report": {
                    "type": "object",
                    "description": "TaskReport object for publish_report"
                }
            }
        })
    }

    // FIXME: don't write this like match.
    async fn execute(
        &self,
        params: serde_json::Value,
        _context: &crate::tool::ToolContext,
    ) -> anyhow::Result<crate::tool::ToolOutput> {
        let action: SyscallParams = serde_json::from_value(params)
            .map_err(|e| anyhow::anyhow!("invalid kernel tool params: {e}"))?;

        let result = match action {
            SyscallParams::Spawn { agent, task } => self.exec_spawn(&agent, &task).await,
            SyscallParams::SpawnParallel {
                parallel,
                max_concurrency,
            } => {
                self.exec_spawn_parallel(parallel, max_concurrency.unwrap_or(4))
                    .await
            }
            SyscallParams::Status { target } => self.exec_status(&target).await,
            SyscallParams::Children => self.exec_children().await,
            SyscallParams::Kill { target } => self.exec_signal(&target, "kill").await,
            SyscallParams::Pause { target } => self.exec_signal(&target, "pause").await,
            SyscallParams::Resume { target } => self.exec_signal(&target, "resume").await,
            SyscallParams::Interrupt { target } => self.exec_signal(&target, "interrupt").await,
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
            SyscallParams::Subscribe {
                match_tags,
                on_receive,
            } => self.exec_subscribe(match_tags, &on_receive).await,
            SyscallParams::Unsubscribe { subscription_id } => {
                self.exec_unsubscribe(&subscription_id).await
            }
            SyscallParams::PublishReport { report } => self.exec_publish_report(report).await,
        };
        result.map(Into::into)
    }
}
