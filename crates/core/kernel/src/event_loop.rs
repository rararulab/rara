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

//! Unified event loop — single `Kernel::run()` that processes all
//! [`KernelEvent`](crate::unified_event::KernelEvent) variants.
//!
//! This replaces the separate TickLoop + process_loop + Egress subscribe loop
//! with a single event-driven loop. The kernel directly manages process state
//! (conversation, turn cancellation, pause buffer) instead of delegating to
//! per-process tokio tasks.

use std::sync::Arc;

use dashmap::DashMap;
use jiff::Timestamp;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;
use tracing::{debug_span, error, info, info_span, warn, Instrument};

use crate::{
    audit::{AuditEvent, AuditEventType, MemoryOp},
    channel::types::ChatMessage,
    error::{KernelError, Result},
    handle::process_handle::ProcessHandle,
    io::{
        pipe::{self, PipeEntry},
        types::{InboundMessage, MessageId, OutboundEnvelope, OutboundPayload, OutboundRouting},
    },
    kernel::Kernel,
    memory::KvScope,
    process::{
        AgentEnv, AgentId, AgentManifest, AgentProcess, AgentResult, ProcessInfo, ProcessState,
        SessionId, Signal,
        principal::Principal,
    },
    unified_event::{KernelEvent, Syscall},
};

// ---------------------------------------------------------------------------
// ProcessRuntime — per-process mutable state managed by the kernel
// ---------------------------------------------------------------------------

/// Mutable runtime state for each agent process, managed by the kernel's
/// event loop rather than by individual per-process tokio tasks.
///
/// This is stored separately from `AgentProcess` (which lives in ProcessTable
/// and must be Clone) because it contains non-Clone types like
/// `CancellationToken` and `Vec<KernelEvent>`.
pub(crate) struct ProcessRuntime {
    /// In-memory conversation history (ChatMessage list).
    pub conversation: Vec<ChatMessage>,
    /// Per-turn cancellation token — cancelled by Signal::Interrupt to abort
    /// the current LLM call without killing the process.
    pub turn_cancel: CancellationToken,
    /// Process-level cancellation token — cancelled by Signal::Kill or
    /// Signal::Terminate to shut down the entire process. Child processes
    /// use `parent_token.child_token()` so cancelling a parent cascades.
    pub process_cancel: CancellationToken,
    /// Whether this process is paused. When true, incoming messages are
    /// buffered in `pause_buffer` instead of being processed.
    pub paused: bool,
    /// Buffered events received while the process was paused or busy.
    pub pause_buffer: Vec<KernelEvent>,
    /// The ProcessHandle for this process (needed to run LLM turns).
    pub handle: Arc<ProcessHandle>,
    /// Per-agent semaphore limiting concurrent child processes.
    pub child_semaphore: Arc<Semaphore>,
    /// Maximum context tokens for compaction.
    pub max_context_tokens: usize,
    /// Last successful result (for final output when process ends).
    pub last_result: Option<AgentResult>,
    /// Global semaphore permit — dropped when this runtime is removed,
    /// automatically releasing one slot for new process spawns.
    pub _global_permit: OwnedSemaphorePermit,
}

/// Table of per-process runtime state, managed by the kernel event loop.
///
/// Keyed by `AgentId`. Created when a process is spawned, removed when it
/// terminates.
pub(crate) type RuntimeTable = DashMap<AgentId, ProcessRuntime>;

// ---------------------------------------------------------------------------
// Kernel::run() — unified event loop
// ---------------------------------------------------------------------------

impl Kernel {
    /// Run the unified event loop until shutdown.
    ///
    /// If the kernel was created with a [`ShardedEventQueue`], spawns N+1
    /// [`EventProcessor`] tasks (1 global + N shard) for parallel processing.
    /// Otherwise, falls back to the single-loop behavior for backward
    /// compatibility.
    ///
    /// Replaces: TickLoop + process_loop + Egress subscribe loop.
    pub async fn run_event_loop(&self, shutdown: CancellationToken) {
        let runtimes: Arc<RuntimeTable> = Arc::new(DashMap::new());
        self.run_single_event_loop(&runtimes, shutdown).await;
    }

    /// Run the event loop with an `Arc<Kernel>`, enabling true parallel
    /// processing when a sharded queue is configured.
    ///
    /// Called from [`start()`](Kernel::start) which already wraps Kernel in Arc.
    pub(crate) async fn run_event_loop_arc(
        kernel: Arc<Kernel>,
        shutdown: CancellationToken,
    ) {
        let runtimes: Arc<RuntimeTable> = Arc::new(DashMap::new());

        // Clone the sharded queue Arc before moving kernel into the parallel path.
        let sharded = kernel.sharded_queue().cloned();
        if let Some(sq) = sharded {
            Self::run_parallel_event_loop(kernel, sq, runtimes, shutdown)
                .await;
        } else {
            kernel.run_single_event_loop(&runtimes, shutdown).await;
        }
    }

    /// Single-processor event loop (backward compatible).
    async fn run_single_event_loop(
        &self,
        runtimes: &RuntimeTable,
        shutdown: CancellationToken,
    ) {
        info!("kernel event loop started (single-processor mode)");
        loop {
            tokio::select! {
                _ = self.event_queue().wait() => {
                    let events = self.event_queue().drain(32).await;
                    for (event, wal_id) in events {
                        let event_type = event.variant_name();
                        let span = info_span!(
                            "handle_event",
                            processor_id = 0u64,
                            event_type,
                        );
                        self.handle_event(event, runtimes)
                            .instrument(span)
                            .await;
                        if let Some(id) = wal_id {
                            self.event_queue().mark_completed(id);
                        }
                    }
                }
                _ = shutdown.cancelled() => {
                    info!("kernel event loop shutting down");
                    // Drain any remaining critical events.
                    let remaining = self.event_queue().drain(1024).await;
                    for (event, wal_id) in remaining {
                        if matches!(event, KernelEvent::SendSignal { .. } | KernelEvent::Shutdown) {
                            self.handle_event(event, runtimes).await;
                        }
                        if let Some(id) = wal_id {
                            self.event_queue().mark_completed(id);
                        }
                    }
                    break;
                }
            }
        }
        info!("kernel event loop stopped");
    }

    /// Multi-processor event loop — spawns N+1 independent tokio tasks.
    ///
    /// Each task runs an [`EventProcessor`] draining its own shard queue,
    /// achieving true parallel event processing across different agents.
    ///
    /// - **Processor 0** (global): UserMessage, SpawnAgent, Timer, Shutdown, Deliver
    /// - **Processors 1..=N** (shards): Syscall, TurnCompleted, ChildCompleted, SendSignal
    async fn run_parallel_event_loop(
        kernel: Arc<Kernel>,
        sharded_queue: Arc<crate::sharded_event_queue::ShardedEventQueue>,
        runtimes: Arc<RuntimeTable>,
        shutdown: CancellationToken,
    ) {
        use crate::event_processor::EventProcessor;

        let num_shards = sharded_queue.num_shards();
        info!(
            num_shards = num_shards,
            total_processors = num_shards + 1,
            "kernel event loop started (parallel mode)"
        );

        let mut handles = Vec::with_capacity(num_shards + 1);

        // Global processor (id=0)
        {
            let proc = EventProcessor { id: 0, queue: Arc::clone(sharded_queue.global()) };
            let k = Arc::clone(&kernel);
            let rt = Arc::clone(&runtimes);
            let sd = shutdown.clone();
            handles.push(tokio::spawn(async move {
                proc.run(&k, &rt, sd).await;
            }));
        }

        // Shard processors (id=1..=N)
        for i in 0..num_shards {
            let proc = EventProcessor { id: i + 1, queue: Arc::clone(sharded_queue.shard(i)) };
            let k = Arc::clone(&kernel);
            let rt = Arc::clone(&runtimes);
            let sd = shutdown.clone();
            handles.push(tokio::spawn(async move {
                proc.run(&k, &rt, sd).await;
            }));
        }

        // Wait for all processors to finish.
        for handle in handles {
            if let Err(e) = handle.await {
                error!("event processor panicked: {e}");
            }
        }

        info!("kernel parallel event loop stopped");
    }

    /// Dispatch a single event to its handler.
    pub(crate) async fn handle_event(&self, event: KernelEvent, runtimes: &RuntimeTable) {
        match event {
            KernelEvent::UserMessage(msg) => {
                self.handle_user_message(msg, runtimes).await;
            }
            KernelEvent::SpawnAgent {
                manifest,
                input,
                principal,
                parent_id,
                reply_tx,
            } => {
                // SpawnAgent from ProcessHandle::spawn() — subagent, no
                // channel binding.
                let result = self
                    .handle_spawn_agent(manifest, input, principal, None, parent_id, runtimes)
                    .await;
                let _ = reply_tx.send(result);
            }
            KernelEvent::SendSignal { target, signal } => {
                self.handle_signal(target, signal, runtimes).await;
            }
            KernelEvent::TurnCompleted {
                agent_id,
                session_id,
                result,
                in_reply_to,
                user,
            } => {
                self.handle_turn_completed(
                    agent_id, session_id, result, in_reply_to, user, runtimes,
                )
                .await;
            }
            KernelEvent::ChildCompleted {
                parent_id,
                child_id,
                result,
            } => {
                self.handle_child_completed(parent_id, child_id, result, runtimes)
                    .await;
            }
            KernelEvent::Deliver(envelope) => {
                self.handle_deliver(envelope).await;
            }
            KernelEvent::Syscall(syscall) => {
                self.handle_syscall(syscall).await;
            }
            KernelEvent::Timer { name, payload } => {
                info!(name = %name, "timer event received (not yet implemented)");
                let _ = payload;
            }
            KernelEvent::Shutdown => {
                info!("shutdown event received");
            }
        }
    }

    // -----------------------------------------------------------------------
    // handle_syscall — all ProcessHandle interactions
    // -----------------------------------------------------------------------

    /// Handle a syscall from a ProcessHandle.
    ///
    /// All business logic lives here, executed by the kernel event loop.
    async fn handle_syscall(&self, syscall: Syscall) {
        let syscall_type = syscall.variant_name();
        let syscall_agent_id = syscall.agent_id();
        let span = debug_span!(
            "handle_syscall",
            syscall_type,
            agent_id = %syscall_agent_id,
        );
        let _guard = span.enter();

        let inner = self.inner();

        match syscall {
            Syscall::QueryStatus { target, reply_tx } => {
                let result = inner
                    .process_table
                    .get(target)
                    .map(|p| ProcessInfo::from(&p))
                    .ok_or(KernelError::ProcessNotFound {
                        id: target.to_string(),
                    });
                let _ = reply_tx.send(result);
            }

            Syscall::QueryChildren { parent, reply_tx } => {
                let children = inner.process_table.children_of(parent);
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
                let result = Self::do_mem_store(inner, agent_id, &session_id, &principal, &key, value);
                let _ = reply_tx.send(result);
            }

            Syscall::MemRecall {
                agent_id,
                key,
                reply_tx,
            } => {
                let namespaced = format!("agent:{}:{}", agent_id.0, key);
                let result = Ok(inner
                    .shared_kv
                    .get(&namespaced)
                    .map(|v| v.value().clone()));
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
                let result =
                    Self::do_shared_store(inner, agent_id, &principal, &scope, &key, value);
                let _ = reply_tx.send(result);
            }

            Syscall::SharedRecall {
                agent_id,
                principal,
                scope,
                key,
                reply_tx,
            } => {
                let result =
                    Self::do_shared_recall(inner, agent_id, &principal, &scope, &key);
                let _ = reply_tx.send(result);
            }

            Syscall::CreatePipe {
                owner,
                target,
                reply_tx,
            } => {
                let (writer, reader) = pipe::pipe(64);
                inner.pipe_registry.register(
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
                if inner.pipe_registry.resolve_name(&name).is_some() {
                    let _ = reply_tx.send(Err(KernelError::Other {
                        message: format!("named pipe already exists: {name}").into(),
                    }));
                    return;
                }
                let (writer, reader) = pipe::pipe(64);
                let pipe_id = writer.pipe_id().clone();
                inner.pipe_registry.register_named(
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
                let result = match inner.pipe_registry.resolve_name(&name) {
                    Some(pipe_id) => {
                        match inner.pipe_registry.take_parked_reader(&pipe_id) {
                            Some(reader) => {
                                inner.pipe_registry.set_reader(&pipe_id, connector);
                                Ok(reader)
                            }
                            None => Err(KernelError::Other {
                                message: format!(
                                    "named pipe '{name}' has no parked reader \
                                     (already taken or not parked)"
                                )
                                .into(),
                            }),
                        }
                    }
                    None => Err(KernelError::Other {
                        message: format!("named pipe not found: {name}").into(),
                    }),
                };
                let _ = reply_tx.send(result);
            }

            Syscall::RequiresApproval { tool_name, reply_tx } => {
                let result = inner.approval.requires_approval(&tool_name);
                let _ = reply_tx.send(result);
            }

            Syscall::RequestApproval {
                agent_id,
                principal: _,
                tool_name,
                summary,
                reply_tx,
            } => {
                let approval = Arc::clone(&inner.approval);
                let policy = approval.policy();
                let req = crate::approval::ApprovalRequest {
                    id:           uuid::Uuid::new_v4(),
                    agent_id,
                    tool_name:    tool_name.clone(),
                    tool_args:    serde_json::json!({"summary": &summary}),
                    summary,
                    risk_level:   crate::approval::ApprovalManager::classify_risk(&tool_name),
                    requested_at: Timestamp::now(),
                    timeout_secs: policy.timeout_secs,
                };

                // Spawn a task so the event loop is not blocked while waiting
                // for human approval.
                tokio::spawn(async move {
                    let decision = approval.request_approval(req).await;
                    let approved = matches!(decision, crate::approval::ApprovalDecision::Approved);
                    let _ = reply_tx.send(Ok(approved));
                });
            }

            Syscall::GetManifest { agent_id, reply_tx } => {
                let result = inner
                    .process_table
                    .get(agent_id)
                    .map(|p| p.manifest.clone())
                    .ok_or(KernelError::ProcessNotFound {
                        id: agent_id.to_string(),
                    });
                let _ = reply_tx.send(result);
            }

            Syscall::GetToolRegistry { reply_tx } => {
                let _ = reply_tx.send(Arc::clone(&inner.tool_registry));
            }

            Syscall::ResolveProvider { agent_id, reply_tx } => {
                let result = match inner.process_table.get(agent_id) {
                    Some(process) => inner.provider_registry.resolve(
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
                inner
                    .event_bus
                    .publish(crate::event::KernelEvent::ToolExecuted {
                        agent_id: agent_id.0,
                        tool_name: format!("event:{event_type}"),
                        success: true,
                        timestamp: Timestamp::now(),
                    })
                    .await;
            }
        }
    }

    // -----------------------------------------------------------------------
    // Syscall helper methods
    // -----------------------------------------------------------------------

    /// Store a value in an agent's private memory namespace.
    fn do_mem_store(
        inner: &Arc<crate::kernel::KernelInner>,
        agent_id: AgentId,
        session_id: &SessionId,
        principal: &Principal,
        key: &str,
        value: serde_json::Value,
    ) -> Result<()> {
        let namespaced = format!("agent:{}:{}", agent_id.0, key);

        // Check quota before inserting — only if this is a new key.
        if !inner.shared_kv.contains_key(&namespaced) {
            let max = inner.memory_quota_per_agent;
            if max > 0 {
                let prefix = format!("agent:{}:", agent_id.0);
                let count = inner
                    .shared_kv
                    .iter()
                    .filter(|entry| entry.key().starts_with(&prefix))
                    .count();
                if count >= max {
                    return Err(KernelError::MemoryQuotaExceeded {
                        agent_id: agent_id.to_string(),
                        current: count,
                        max,
                    });
                }
            }
        }

        inner.shared_kv.insert(namespaced, value);

        // Audit: MemoryAccess (Store)
        crate::audit::record_async(
            &inner.audit_log,
            AuditEvent {
                timestamp: Timestamp::now(),
                agent_id,
                session_id: session_id.clone(),
                user_id: principal.user_id.clone(),
                event_type: AuditEventType::MemoryAccess {
                    operation: MemoryOp::Store,
                    key: key.to_string(),
                },
                details: serde_json::Value::Null,
            },
        );

        Ok(())
    }

    /// Validate scope permissions for shared memory operations.
    fn check_scope_permission(
        agent_id: AgentId,
        principal: &Principal,
        scope: &KvScope,
    ) -> Result<()> {
        match scope {
            KvScope::Global | KvScope::Team(_) => {
                if !principal.is_admin() {
                    return Err(KernelError::MemoryScopeDenied {
                        reason: format!(
                            "agent {} (role {:?}) cannot access {:?} scope — requires Root or Admin",
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
    fn scoped_key(scope: &KvScope, key: &str) -> String {
        match scope {
            KvScope::Global => key.to_string(),
            KvScope::Team(name) => format!("team:{name}:{key}"),
            KvScope::Agent(id) => format!("agent:{id}:{key}"),
        }
    }

    /// Store a value in a shared (scoped) memory namespace.
    fn do_shared_store(
        inner: &Arc<crate::kernel::KernelInner>,
        agent_id: AgentId,
        principal: &Principal,
        scope: &KvScope,
        key: &str,
        value: serde_json::Value,
    ) -> Result<()> {
        Self::check_scope_permission(agent_id, principal, scope)?;
        let scoped = Self::scoped_key(scope, key);
        inner.shared_kv.insert(scoped, value);
        Ok(())
    }

    /// Recall a value from a shared (scoped) memory namespace.
    fn do_shared_recall(
        inner: &Arc<crate::kernel::KernelInner>,
        agent_id: AgentId,
        principal: &Principal,
        scope: &KvScope,
        key: &str,
    ) -> Result<Option<serde_json::Value>> {
        Self::check_scope_permission(agent_id, principal, scope)?;
        let scoped = Self::scoped_key(scope, key);
        Ok(inner.shared_kv.get(&scoped).map(|v| v.value().clone()))
    }

    // -----------------------------------------------------------------------
    // handle_user_message
    // -----------------------------------------------------------------------

    /// Agent name for admin/root users.
    const ADMIN_AGENT_NAME: &'static str = "rara";
    /// Agent name for regular users.
    const USER_AGENT_NAME: &'static str = "nana";

    /// Handle a user message with 3-path routing:
    ///
    /// 1. **ID addressing** (`target_agent_id` set): deliver to specific
    ///    process — error if terminal or not found (A2A Protocol pattern).
    /// 2. **Session addressing** (session_index match): deliver to bound
    ///    process — if terminal, clear binding and respawn transparently
    ///    (AutoGen lazy instantiation pattern).
    /// 3. **Name addressing** (fallback): lookup AgentRegistry by name,
    ///    always spawn a new process (Anthropic spawn-new pattern).
    async fn handle_user_message(&self, msg: InboundMessage, runtimes: &RuntimeTable) {
        let span = info_span!(
            "handle_user_message",
            session_id = %msg.session_id,
            user_id = %msg.user.0,
            channel = ?msg.source.channel_type,
            routing_path = tracing::field::Empty,
        );
        let _guard = span.enter();

        let session_id = msg.session_id.clone();
        let user = msg.user.clone();

        // Register egress endpoint for non-connection-oriented channels (e.g.
        // Telegram) so that the Egress layer can route replies back.  Web
        // endpoints are registered by WebAdapter on WS/SSE connect; Telegram
        // has no persistent connection, so we register on every inbound
        // message (idempotent — EndpointRegistry uses a HashSet).
        if msg.source.channel_type == crate::channel::types::ChannelType::Telegram {
            if let Some(ref chat_id_str) = msg.source.platform_chat_id {
                if let Ok(chat_id) = chat_id_str.parse::<i64>() {
                    self.endpoint_registry().register(
                        &user,
                        crate::io::egress::Endpoint {
                            channel_type: crate::channel::types::ChannelType::Telegram,
                            address:      crate::io::egress::EndpointAddress::Telegram {
                                chat_id,
                                thread_id: None,
                            },
                        },
                    );
                }
            }
        }

        // ----- Path 1: ID addressing (agent-to-agent) -----
        if let Some(target_id) = msg.target_agent_id {
            span.record("routing_path", "id_addressing");
            match self.process_table().get(target_id) {
                Some(process) if process.state.is_terminal() => {
                    // Terminal process — return error (A2A pattern).
                    let envelope = OutboundEnvelope {
                        id:          MessageId::new(),
                        in_reply_to: msg.id.clone(),
                        user:        user.clone(),
                        session_id:  session_id.clone(),
                        routing:     OutboundRouting::BroadcastAll,
                        payload:     OutboundPayload::Error {
                            code:    "process_terminal".to_string(),
                            message: format!(
                                "process {} is {}", target_id, process.state
                            ),
                        },
                        timestamp:   jiff::Timestamp::now(),
                    };
                    if let Err(e) = self.event_queue().try_push(KernelEvent::Deliver(envelope)) {
                        error!(%e, "failed to push process-terminal error Deliver");
                    }
                    return;
                }
                Some(_) => {
                    // Process alive — buffer if busy/paused, else deliver.
                    if let Some(mut rt) = runtimes.get_mut(&target_id) {
                        if rt.paused {
                            rt.pause_buffer.push(KernelEvent::UserMessage(msg));
                            return;
                        }
                        if let Some(p) = self.process_table().get(target_id) {
                            if p.state == ProcessState::Running {
                                rt.pause_buffer.push(KernelEvent::UserMessage(msg));
                                return;
                            }
                        }
                    }
                    self.start_llm_turn(target_id, msg, runtimes).await;
                    return;
                }
                None => {
                    // Process not found — return error.
                    let envelope = OutboundEnvelope {
                        id:          MessageId::new(),
                        in_reply_to: msg.id.clone(),
                        user:        user.clone(),
                        session_id:  session_id.clone(),
                        routing:     OutboundRouting::BroadcastAll,
                        payload:     OutboundPayload::Error {
                            code:    "process_not_found".to_string(),
                            message: format!("process not found: {target_id}"),
                        },
                        timestamp:   jiff::Timestamp::now(),
                    };
                    if let Err(e) = self.event_queue().try_push(KernelEvent::Deliver(envelope)) {
                        error!(%e, "failed to push process-not-found error Deliver");
                    }
                    return;
                }
            }
        }

        // ----- Path 2: Session addressing (external user) -----
        if let Some(process) = self.process_table().find_by_session(&session_id) {
            span.record("routing_path", "session_addressing");
            let aid = process.agent_id;

            if process.state.is_terminal() {
                // Terminal process — clear session binding, fall through to
                // Path 3 (Name addressing) to spawn a replacement.
                info!(
                    agent_id = %aid,
                    session_id = %session_id,
                    state = %process.state,
                    "session-bound process terminal — clearing binding, will respawn"
                );
                self.process_table().remove(aid);
                // Fall through to Path 3 below.
            } else {
                // Process alive — buffer if busy/paused, else deliver.
                if let Some(mut rt) = runtimes.get_mut(&aid) {
                    if rt.paused {
                        rt.pause_buffer.push(KernelEvent::UserMessage(msg));
                        return;
                    }
                    if let Some(p) = self.process_table().get(aid) {
                        if p.state == ProcessState::Running {
                            rt.pause_buffer.push(KernelEvent::UserMessage(msg));
                            return;
                        }
                    }
                }
                self.start_llm_turn(aid, msg, runtimes).await;
                return;
            }
        }

        // ----- Path 3: Name addressing (always spawn new) -----
        span.record("routing_path", "name_addressing");
        let target_name = if let Some(name) = msg.target_agent.as_deref() {
            name.to_string()
        } else {
            self.default_agent_for_user(&msg.user).await
        };

        let manifest = if let Some(m) = self.inner().agent_registry.get(&target_name) {
            m
        } else if target_name == Self::ADMIN_AGENT_NAME {
            match self.resolve_manifest_for_auto_spawn().await {
                Some(m) => m,
                None => {
                    error!(
                        session_id = %session_id,
                        "no model configured — cannot spawn root agent"
                    );
                    return;
                }
            }
        } else {
            warn!(
                target_name = %target_name,
                session_id = %session_id,
                "unknown target agent"
            );
            let envelope = OutboundEnvelope {
                id:          MessageId::new(),
                in_reply_to: msg.id.clone(),
                user:        user.clone(),
                session_id:  session_id.clone(),
                routing:     OutboundRouting::BroadcastAll,
                payload:     OutboundPayload::Error {
                    code:    "unknown_agent".to_string(),
                    message: format!("unknown target agent: {target_name}"),
                },
                timestamp:   jiff::Timestamp::now(),
            };
            if let Err(e) = self.event_queue().try_push(KernelEvent::Deliver(envelope)) {
                error!(%e, "failed to push unknown-agent error Deliver");
            }
            return;
        };

        let principal = Principal::user(user.0.clone());
        match self
            .handle_spawn_agent(
                manifest,
                msg.content.as_text(),
                principal,
                Some(session_id.clone()),
                None,
                runtimes,
            )
            .await
        {
            Ok(_aid) => {
                // handle_spawn_agent pushes a synthetic UserMessage that will
                // re-enter handle_user_message and be routed via Path 2.
            }
            Err(e) => {
                error!(session_id = %session_id, error = %e, "failed to spawn agent");
            }
        }
    }

    /// Start an LLM turn for the given agent, spawning the work as an async
    /// task that pushes `TurnCompleted` back into the EventQueue when done.
    async fn start_llm_turn(
        &self,
        agent_id: AgentId,
        msg: InboundMessage,
        runtimes: &RuntimeTable,
    ) {
        let span = info_span!(
            "start_llm_turn",
            agent_id = %agent_id,
            session_id = %msg.session_id,
        );
        let _guard = span.enter();

        let Some(mut rt) = runtimes.get_mut(&agent_id) else {
            warn!(agent_id = %agent_id, "runtime not found for LLM turn");
            return;
        };

        let session_id = msg.session_id.clone();
        let user = msg.user.clone();
        let msg_id = msg.id.clone();

        // Set state to Running.
        let _ = self
            .inner()
            .process_table
            .set_state(agent_id, ProcessState::Running);

        // Send a typing / progress indicator so the user sees feedback
        // while the LLM is thinking (e.g. Telegram "typing..." bubble).
        let egress_session_id = self
            .inner()
            .process_table
            .get(agent_id)
            .and_then(|p| p.channel_session_id.clone())
            .unwrap_or_else(|| session_id.clone());
        let _ = self.event_queue().try_push(KernelEvent::Deliver(OutboundEnvelope {
            id:          MessageId::new(),
            in_reply_to: msg_id.clone(),
            user:        user.clone(),
            session_id:  egress_session_id,
            routing:     OutboundRouting::BroadcastAll,
            payload:     OutboundPayload::Progress {
                stage:  "thinking".to_string(),
                detail: Some(String::new()),
            },
            timestamp:   jiff::Timestamp::now(),
        }));

        // Record metrics.
        if let Some(metrics) = self.inner().process_table.get_metrics(&agent_id) {
            metrics.record_message();
        }

        // Apply context compaction.
        let compaction_strategy = crate::memory::compaction::SlidingWindowCompaction;
        rt.conversation = crate::memory::compaction::maybe_compact(
            std::mem::take(&mut rt.conversation),
            rt.max_context_tokens,
            &compaction_strategy,
        )
        .await;

        // Convert history to LLM format.
        let history = match crate::runner::build_history_messages(&rt.conversation) {
            Ok(msgs) if !msgs.is_empty() => Some(msgs),
            Ok(_) => None,
            Err(e) => {
                warn!(%e, "failed to convert history");
                None
            }
        };

        // Append user message to conversation + persist.
        let user_text = msg.content.as_text();
        let user_msg = ChatMessage::user(&user_text);
        rt.conversation.push(user_msg.clone());
        let inner = Arc::clone(self.inner());
        let session_id_persist = session_id.clone();
        // Persist in background to avoid blocking event loop.
        tokio::spawn({
            let inner = inner.clone();
            let session_id = session_id_persist.clone();
            let user_msg = user_msg.clone();
            async move {
                if let Err(e) = inner.session_repo.append_message(&session_id, &user_msg).await {
                    warn!(%e, "failed to persist user message");
                }
            }
        });

        // Open stream.
        let stream_handle = inner.stream_hub.open(session_id.clone());

        // Clone what we need for the spawned task.
        let handle = Arc::clone(&rt.handle);
        let turn_cancel = rt.turn_cancel.clone();
        let event_queue = self.event_queue().clone();
        let stream_id = stream_handle.stream_id().clone();

        // Drop the DashMap guard before spawning.
        drop(rt);

        // Capture parent span for the spawned task.
        let parent_span = tracing::Span::current();

        // Spawn async task for the LLM turn.
        tokio::spawn(async move {
            let turn_span = info_span!(
                parent: &parent_span,
                "agent_turn",
                agent_id = %agent_id,
                session_id = %session_id,
                total_ms = tracing::field::Empty,
                iterations = tracing::field::Empty,
                tool_calls = tracing::field::Empty,
            );
            let _guard = turn_span.enter();
            let start = std::time::Instant::now();

            let turn_result = crate::agent_turn::run_inline_agent_loop(
                &handle,
                user_text,
                history,
                &stream_handle,
                &turn_cancel,
            )
            .await;

            // Record timing and result metrics on the span.
            let elapsed = start.elapsed();
            let elapsed_ms = elapsed.as_millis() as u64;
            turn_span.record("total_ms", elapsed_ms);
            if let Ok(ref result) = turn_result {
                turn_span.record("iterations", result.iterations);
                turn_span.record("tool_calls", result.tool_calls);
            }

            // Emit turn metrics before closing stream.
            if let Ok(ref result) = turn_result {
                stream_handle.emit(crate::io::stream::StreamEvent::TurnMetrics {
                    duration_ms: elapsed_ms,
                    iterations: result.iterations,
                    tool_calls: result.tool_calls,
                    model: result.model.clone(),
                });
            }

            // Close stream.
            inner.stream_hub.close(&stream_id);

            // Push TurnCompleted back into the event queue.
            let result = match turn_result {
                Ok(turn) => Ok(turn),
                Err(msg) => Err(msg),
            };
            let event = KernelEvent::TurnCompleted {
                agent_id,
                session_id,
                result,
                in_reply_to: msg_id,
                user,
            };
            if let Err(e) = event_queue.try_push(event) {
                error!(%e, agent_id = %agent_id, "failed to push TurnCompleted");
            }
        });
    }

    // -----------------------------------------------------------------------
    // handle_turn_completed
    // -----------------------------------------------------------------------

    /// Handle an LLM turn completion — persist result, deliver reply, drain
    /// pause buffer.
    async fn handle_turn_completed(
        &self,
        agent_id: AgentId,
        session_id: SessionId,
        result: std::result::Result<crate::agent_turn::AgentTurnResult, String>,
        in_reply_to: MessageId,
        user: crate::process::principal::UserId,
        runtimes: &RuntimeTable,
    ) {
        let span = info_span!(
            "handle_turn_completed",
            agent_id = %agent_id,
            session_id = %session_id,
            success = tracing::field::Empty,
            iterations = tracing::field::Empty,
            tool_calls = tracing::field::Empty,
            reply_len = tracing::field::Empty,
        );
        let _guard = span.enter();

        let inner = self.inner();

        // Determine the egress session: use the channel_session_id if this
        // process has one (root process), otherwise fall back to the
        // process's own session. Subagents without a channel binding won't
        // have egress delivery — their results flow back to the parent via
        // ChildCompleted.
        let egress_session_id = inner
            .process_table
            .get(agent_id)
            .and_then(|p| p.channel_session_id.clone())
            .unwrap_or_else(|| session_id.clone());

        // Update metrics.
        if let Some(metrics) = inner.process_table.get_metrics(&agent_id) {
            metrics.touch().await;
        }

        match result {
            Ok(turn) if !turn.text.is_empty() => {
                span.record("success", true);
                span.record("iterations", turn.iterations);
                span.record("tool_calls", turn.tool_calls);
                span.record("reply_len", turn.text.len());

                // Record metrics.
                if let Some(metrics) = inner.process_table.get_metrics(&agent_id) {
                    metrics.record_llm_call();
                    metrics.record_tool_calls(turn.tool_calls as u64);
                    let estimated_tokens = (turn.text.len() as u64).saturating_div(4).max(1);
                    metrics.record_tokens(estimated_tokens);
                }

                // Persist assistant reply to the process's own session.
                let assistant_msg = ChatMessage::assistant(&turn.text);
                if let Some(mut rt) = runtimes.get_mut(&agent_id) {
                    rt.conversation.push(assistant_msg.clone());
                }
                if let Err(e) = inner
                    .session_repo
                    .append_message(&session_id, &assistant_msg)
                    .await
                {
                    warn!(%e, "failed to persist assistant message");
                }

                let result = AgentResult {
                    output:     turn.text.clone(),
                    iterations: turn.iterations,
                    tool_calls: turn.tool_calls,
                };
                let _ = inner.process_table.set_result(agent_id, result.clone());

                // Push Deliver event for the reply — use egress session for routing.
                let envelope = OutboundEnvelope {
                    id:          MessageId::new(),
                    in_reply_to,
                    user:        user.clone(),
                    session_id:  egress_session_id.clone(),
                    routing:     OutboundRouting::BroadcastAll,
                    payload:     OutboundPayload::Reply {
                        content:     crate::channel::types::MessageContent::Text(turn.text),
                        attachments: vec![],
                    },
                    timestamp:   jiff::Timestamp::now(),
                };
                if let Err(e) = self.event_queue().try_push(KernelEvent::Deliver(envelope)) {
                    error!(%e, "failed to push Deliver event");
                }

                // Audit: ProcessCompleted
                crate::audit::record_async(
                    &inner.audit_log,
                    AuditEvent {
                        timestamp:  jiff::Timestamp::now(),
                        agent_id,
                        session_id: session_id.clone(),
                        user_id:    user.clone(),
                        event_type: AuditEventType::ProcessCompleted {
                            result: result.output.clone(),
                        },
                        details:    serde_json::json!({
                            "iterations": result.iterations,
                            "tool_calls": result.tool_calls,
                        }),
                    },
                );

                info!(
                    agent_id = %agent_id,
                    iterations = result.iterations,
                    tool_calls = result.tool_calls,
                    reply_len = result.output.len(),
                    "turn completed"
                );

                if let Some(mut rt) = runtimes.get_mut(&agent_id) {
                    rt.last_result = Some(result);
                }
            }
            Ok(turn) => {
                span.record("success", true);
                span.record("iterations", turn.iterations);
                span.record("tool_calls", turn.tool_calls);
                span.record("reply_len", 0u64);
                info!(agent_id = %agent_id, "turn completed (empty result)");

                // Empty result — LLM call was made but produced no text.
                if let Some(metrics) = inner.process_table.get_metrics(&agent_id) {
                    metrics.record_llm_call();
                    metrics.record_tool_calls(turn.tool_calls as u64);
                }
            }
            Err(err_msg) => {
                span.record("success", false);
                info!(agent_id = %agent_id, error = %err_msg, "turn completed (error)");

                if err_msg != "interrupted by user" {
                    let _ = inner
                        .process_table
                        .set_state(agent_id, ProcessState::Failed);

                    crate::audit::record_async(
                        &inner.audit_log,
                        AuditEvent {
                            timestamp:  jiff::Timestamp::now(),
                            agent_id,
                            session_id: session_id.clone(),
                            user_id:    user.clone(),
                            event_type: AuditEventType::ProcessFailed {
                                error: err_msg.clone(),
                            },
                            details:    serde_json::Value::Null,
                        },
                    );
                }

                // Deliver error — use egress session for routing.
                let envelope = OutboundEnvelope {
                    id:          MessageId::new(),
                    in_reply_to,
                    user:        user.clone(),
                    session_id:  egress_session_id.clone(),
                    routing:     OutboundRouting::BroadcastAll,
                    payload:     OutboundPayload::Error {
                        code:    "agent_error".to_string(),
                        message: err_msg,
                    },
                    timestamp:   jiff::Timestamp::now(),
                };
                if let Err(e) = self.event_queue().try_push(KernelEvent::Deliver(envelope)) {
                    error!(%e, "failed to push error Deliver event");
                }
            }
        }

        // Set state to Waiting.
        let _ = inner
            .process_table
            .set_state(agent_id, ProcessState::Waiting);

        // Drain pause buffer: re-inject buffered events into the queue.
        if let Some(mut rt) = runtimes.get_mut(&agent_id) {
            let buffered = std::mem::take(&mut rt.pause_buffer);
            drop(rt); // Release the lock before pushing to queue.
            for event in buffered {
                if let Err(e) = self.event_queue().try_push(event) {
                    warn!(%e, "failed to re-inject buffered event");
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // handle_spawn_agent
    // -----------------------------------------------------------------------

    /// Handle a SpawnAgent event — create a new process and its runtime.
    ///
    /// `channel_session_id` is the external channel binding (e.g.,
    /// `web:chat123`). Set for root processes that entered via a channel
    /// adapter; `None` for subagents spawned by other agents.
    ///
    /// Every process gets its own `agent:{id}` session for conversation
    /// isolation. Only processes with a `channel_session_id` are inserted
    /// into the `session_index` for inbound message routing.
    async fn handle_spawn_agent(
        &self,
        manifest: AgentManifest,
        input: String,
        principal: Principal,
        channel_session_id: Option<SessionId>,
        parent_id: Option<AgentId>,
        runtimes: &RuntimeTable,
    ) -> Result<AgentId> {
        let span = info_span!(
            "handle_spawn_agent",
            manifest_name = %manifest.name,
            parent_id = ?parent_id,
            agent_id = tracing::field::Empty,
        );
        let _guard = span.enter();

        let inner = self.inner();

        // Validate principal.
        inner.validate_principal(&principal).await?;

        // Acquire global semaphore.
        let global_permit = inner
            .global_semaphore
            .clone()
            .try_acquire_owned()
            .map_err(|_| KernelError::SpawnLimitReached {
                message: "global concurrency limit reached".to_string(),
            })?;

        let agent_id = AgentId::new();
        span.record("agent_id", tracing::field::display(&agent_id));

        // Each process gets its own session — context isolation.
        let session_id = SessionId::new(format!("agent:{}", agent_id));
        inner.ensure_session(&session_id).await;
        // Clean start: no loaded history. Task input arrives as synthetic
        // message (below) or is injected directly into the conversation.
        let initial_messages = vec![];

        // Audit: ProcessSpawned
        crate::audit::record_async(
            &inner.audit_log,
            AuditEvent {
                timestamp:  jiff::Timestamp::now(),
                agent_id,
                session_id: session_id.clone(),
                user_id:    principal.user_id.clone(),
                event_type: AuditEventType::ProcessSpawned {
                    manifest_name: manifest.name.clone(),
                    parent_id,
                },
                details:    serde_json::json!({
                    "model": manifest.model,
                    "max_iterations": manifest.max_iterations,
                }),
            },
        );

        // Register process in table.
        let metrics = std::sync::Arc::new(crate::process::RuntimeMetrics::new());
        let process = AgentProcess {
            agent_id,
            parent_id,
            session_id: session_id.clone(),
            channel_session_id: channel_session_id.clone(),
            manifest: manifest.clone(),
            principal: principal.clone(),
            env: AgentEnv::default(),
            state: ProcessState::Waiting,
            created_at: jiff::Timestamp::now(),
            finished_at: None,
            result: None,
            created_files: vec![],
            metrics,
        };
        inner.process_table.insert(process);

        // Create process-level cancellation token.
        // Child processes derive their token from the parent's, so cancelling
        // a parent cascades to all children automatically.
        let process_cancel = if let Some(pid) = parent_id {
            runtimes
                .get(&pid)
                .map(|parent_rt| parent_rt.process_cancel.child_token())
                .unwrap_or_default()
        } else {
            CancellationToken::new()
        };

        // Build ProcessHandle — uses the process's own session.
        let child_limit = manifest
            .max_children
            .unwrap_or(inner.default_child_limit);

        let handle = Arc::new(ProcessHandle::new(
            agent_id,
            session_id.clone(),
            principal,
            inner.event_queue.clone(),
        ));

        let max_context_tokens = manifest
            .max_context_tokens
            .unwrap_or(crate::memory::compaction::DEFAULT_MAX_CONTEXT_TOKENS);

        // Create runtime entry. The global permit is stored here so it lives
        // as long as the process — dropping the runtime entry automatically
        // releases the semaphore slot.
        let runtime = ProcessRuntime {
            conversation: initial_messages,
            turn_cancel: CancellationToken::new(),
            process_cancel,
            paused: false,
            pause_buffer: Vec::new(),
            handle,
            child_semaphore: Arc::new(Semaphore::new(child_limit)),
            max_context_tokens,
            last_result: None,
            _global_permit: global_permit,
        };
        runtimes.insert(agent_id, runtime);

        info!(
            agent_id = %agent_id,
            manifest = %manifest.name,
            session_id = %session_id,
            channel_session_id = ?channel_session_id,
            "process spawned via event loop"
        );

        // Deliver the initial input to the spawned process.
        //
        // For root processes (channel_session_id.is_some()), push a synthetic
        // UserMessage — the session-first router finds the process via
        // session_index (bound to the channel session above).
        //
        // For subagents (channel_session_id.is_none()), also push a synthetic
        // UserMessage using the process's own agent-scoped session and target
        // the agent by name. handle_user_message will fall through to the
        // name-based lookup path and find this process.
        let msg_session = channel_session_id.unwrap_or(session_id);
        let inbound = InboundMessage::synthetic_to(
            input,
            crate::process::principal::UserId("system".to_string()),
            msg_session,
            manifest.name.clone(),
        );
        if let Err(e) = self
            .event_queue()
            .try_push(KernelEvent::UserMessage(inbound))
        {
            error!(%e, "failed to push initial UserMessage for spawned agent");
        }

        Ok(agent_id)
    }

    // -----------------------------------------------------------------------
    // handle_signal
    // -----------------------------------------------------------------------

    /// Handle a control signal sent to an agent process.
    async fn handle_signal(
        &self,
        target: AgentId,
        signal: Signal,
        runtimes: &RuntimeTable,
    ) {
        let span = info_span!(
            "handle_signal",
            agent_id = %target,
            signal = ?signal,
        );
        let _guard = span.enter();

        let inner = self.inner();

        match signal {
            Signal::Interrupt => {
                info!(agent_id = %target, "interrupt signal");
                if let Some(mut rt) = runtimes.get_mut(&target) {
                    // Cancel the current LLM turn token.
                    rt.turn_cancel.cancel();
                    // Replace with a fresh token for the next turn.
                    rt.turn_cancel = CancellationToken::new();
                }
                // Notify via Deliver event — use channel session for egress.
                let session_id = inner
                    .process_table
                    .get(target)
                    .and_then(|p| p.channel_session_id.clone())
                    .unwrap_or_else(|| SessionId::new("unknown"));
                let envelope = OutboundEnvelope {
                    id:          MessageId::new(),
                    in_reply_to: MessageId::new(),
                    user:        crate::process::principal::UserId("system".to_string()),
                    session_id,
                    routing:     OutboundRouting::BroadcastAll,
                    payload:     OutboundPayload::StateChange {
                        event_type: "interrupted".to_string(),
                        data:       serde_json::json!({
                            "agent_id": target.to_string(),
                            "message": "Agent interrupted by user",
                        }),
                    },
                    timestamp:   jiff::Timestamp::now(),
                };
                if let Err(e) = self.event_queue().try_push(KernelEvent::Deliver(envelope)) {
                    error!(%e, "failed to push interrupt notification");
                }
            }
            Signal::Pause => {
                info!(agent_id = %target, "pause signal");
                if let Some(mut rt) = runtimes.get_mut(&target) {
                    rt.paused = true;
                }
                let _ = inner.process_table.set_state(target, ProcessState::Paused);
            }
            Signal::Resume => {
                info!(agent_id = %target, "resume signal");
                let buffered = if let Some(mut rt) = runtimes.get_mut(&target) {
                    rt.paused = false;
                    std::mem::take(&mut rt.pause_buffer)
                } else {
                    vec![]
                };
                let _ = inner.process_table.set_state(target, ProcessState::Waiting);
                if !buffered.is_empty() {
                    for event in buffered {
                        if let Err(e) = self.event_queue().try_push(event) {
                            warn!(%e, "failed to re-inject buffered event on resume");
                        }
                    }
                }
            }
            Signal::Terminate => {
                info!(agent_id = %target, "terminate signal — graceful shutdown");
                if let Some(rt) = runtimes.get(&target) {
                    rt.turn_cancel.cancel();
                }
                // Grace period then force-kill via process_cancel token.
                let process_cancel = runtimes
                    .get(&target)
                    .map(|rt| rt.process_cancel.clone());
                if let Some(token) = process_cancel {
                    tokio::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        token.cancel();
                    });
                }
                // Clean up runtime.
                self.cleanup_process(target, runtimes).await;
            }
            Signal::Kill => {
                info!(agent_id = %target, "kill signal");
                if let Some(rt) = runtimes.get(&target) {
                    rt.process_cancel.cancel();
                }
                let _ = inner
                    .process_table
                    .set_state(target, ProcessState::Cancelled);
                self.cleanup_process(target, runtimes).await;
            }
        }
    }

    // -----------------------------------------------------------------------
    // handle_child_completed
    // -----------------------------------------------------------------------

    /// Handle a child agent completion — persist result to parent's
    /// conversation.
    async fn handle_child_completed(
        &self,
        parent_id: AgentId,
        child_id: AgentId,
        result: AgentResult,
        runtimes: &RuntimeTable,
    ) {
        let span = info_span!(
            "handle_child_completed",
            parent_id = %parent_id,
            child_id = %child_id,
            output_len = result.output.len(),
        );
        let _guard = span.enter();

        let inner = self.inner();

        info!(
            parent_id = %parent_id,
            child_id = %child_id,
            output_len = result.output.len(),
            "child result received"
        );

        // Persist child result to parent's conversation history.
        let child_result_text = format!(
            "[child_agent_result] child_id={child_id} \
             iterations={} tool_calls={}\n\n{}",
            result.iterations, result.tool_calls, result.output,
        );
        let child_msg = ChatMessage::system(&child_result_text);

        if let Some(mut rt) = runtimes.get_mut(&parent_id) {
            rt.conversation.push(child_msg.clone());
        }

        let session_id = inner
            .process_table
            .get(parent_id)
            .map(|p| p.session_id.clone())
            .unwrap_or_else(|| SessionId::new("unknown"));

        if let Err(e) = inner
            .session_repo
            .append_message(&session_id, &child_msg)
            .await
        {
            warn!(%e, "failed to persist child result message");
        }
    }

    // -----------------------------------------------------------------------
    // handle_deliver
    // -----------------------------------------------------------------------

    /// Handle a Deliver event — call Egress::deliver directly.
    async fn handle_deliver(&self, envelope: OutboundEnvelope) {
        let payload_type = match &envelope.payload {
            OutboundPayload::Reply { .. } => "reply",
            OutboundPayload::Progress { .. } => "progress",
            OutboundPayload::StateChange { .. } => "state_change",
            OutboundPayload::Error { .. } => "error",
        };
        let span = info_span!(
            "handle_deliver",
            session_id = %envelope.session_id,
            payload_type,
        );
        let _guard = span.enter();

        crate::io::egress::Egress::deliver(
            &self.egress_adapters,
            self.endpoint_registry(),
            envelope,
        )
        .await;
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Clean up a process runtime entry.
    ///
    /// Removing the runtime from the table drops the `process_cancel` token
    /// naturally, so no explicit cancellation-token cleanup is needed.
    async fn cleanup_process(&self, agent_id: AgentId, runtimes: &RuntimeTable) {
        let rt = runtimes.remove(&agent_id);
        if let Some((_, rt)) = rt {
            // Notify parent if this is a child process.
            if let Some(process) = self.inner().process_table.get(agent_id) {
                if let Some(parent_id) = process.parent_id {
                    let result = rt.last_result.unwrap_or(AgentResult {
                        output:     "process ended".to_string(),
                        iterations: 0,
                        tool_calls: 0,
                    });
                    let event = KernelEvent::ChildCompleted {
                        parent_id,
                        child_id: agent_id,
                        result,
                    };
                    if let Err(e) = self.event_queue().try_push(event) {
                        warn!(%e, "failed to push ChildCompleted event");
                    }
                }
            }
        }
    }

    /// Determine the default agent name for a user based on their role.
    ///
    /// - Root / Admin users → "rara" (full-capability agent)
    /// - Regular users → "nana" (chat-only companion)
    /// - Unknown users → "nana" (safe default)
    async fn default_agent_for_user(
        &self,
        user: &crate::process::principal::UserId,
    ) -> String {
        use crate::process::principal::Role;

        let inner = self.inner();

        // Try looking up the user by their raw UserId string first.
        let user_id_str = &user.0;

        let kernel_user = match inner.user_store.get_by_name(user_id_str).await {
            Ok(Some(u)) => Some(u),
            _ => {
                // UserId might be in "channel:name" format (e.g. "web:web-user").
                // Try extracting the part after ':'.
                if let Some((_prefix, name)) = user_id_str.split_once(':') {
                    match inner.user_store.get_by_name(name).await {
                        Ok(found) => found,
                        Err(_) => None,
                    }
                } else {
                    None
                }
            }
        };

        match kernel_user {
            Some(u) => match u.role {
                Role::Root | Role::Admin => Self::ADMIN_AGENT_NAME.to_string(),
                Role::User => Self::USER_AGENT_NAME.to_string(),
            },
            None => Self::USER_AGENT_NAME.to_string(),
        }
    }

    /// Resolve a manifest for auto-spawning (when a user message arrives
    /// with no existing process).
    async fn resolve_manifest_for_auto_spawn(&self) -> Option<AgentManifest> {
        let model = rara_domain_shared::settings::get_model(
            self.settings().as_ref(),
            "chat",
        )
        .await;
        Some(AgentManifest {
            name:               "io-agent".to_string(),
        role:           None,
            description:        "I/O bus agent".to_string(),
            model,
            system_prompt:      "You are a helpful assistant.".to_string(),
            soul_prompt:    None,
            provider_hint:      None,
            max_iterations:     Some(25),
            tools:              vec![],
            max_children:       None,
            max_context_tokens: None,
            priority:           crate::process::Priority::default(),
            metadata:           serde_json::Value::Null,
            sandbox:            None,
        })
    }
}
