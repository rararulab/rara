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

//! Kernel — the unified OS-inspired orchestrator for agent lifecycle.
//!
//! The [`Kernel`] is the single entry point for all agent operations.
//! It manages a [`SessionTable`] of running sessions, enforces concurrency
//! limits via dual semaphores (global + per-session), and provides
//! [`spawn`](Kernel::spawn) as the primary API for creating sessions.
//!
//! # Architecture
//!
//! ```text
//! Kernel (top-level, behind Arc after start())
//!   ├── SessionTable  (all running sessions)
//!   ├── global_semaphore (max total concurrent agents)
//!   ├── AgentRegistry   (named agent definitions)
//!   ├── DriverRegistry  (multi-driver LLM)
//!   ├── ToolRegistry
//!   ├── FileTapeStore (tape-based memory)
//!   ├── NotificationBus
//!   ├── SecuritySubsystem (auth + authz + approval + guard)
//!   ├── shared_kv (cross-agent KV)
//!   ├── StreamHub + IngressPipeline + EndpointRegistry
//!   └── ShardedEventQueue (single-queue or multi-shard mode)
//! ```
//!
//! Each spawned agent receives a [`ProcessHandle`] — a thin event pusher that
//! sends [`Syscall`] variants through the unified event queue.

use std::sync::Arc;

use jiff::Timestamp;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use tracing::{Instrument, error, info, info_span, warn};

use crate::{
    KernelError,
    channel::types::{ChannelType, ChatMessage},
    compaction::DEFAULT_MAX_CONTEXT_TOKENS,
    event::{KernelEvent, KernelEventEnvelope},
    io::{
        EgressAdapterRef, EndpointRegistry, IdentityResolverRef,
        InboundMessage, IngressPipeline, IngressPipelineRef, MessageId, OutboundEnvelope,
        PipeRegistry, SessionResolverRef, StreamHubRef, StreamId,
    },
    kv::SharedKv,
    llm::DriverRegistryRef,
    notification::NotificationBusRef,
    process::{
        AgentEnv, AgentManifest, AgentRole, AgentRunLoopResult, Session, SessionState, SessionTable,
        Signal,
        agent_registry::AgentRegistryRef, principal::Principal,
    },
    queue::{EventQueueRef, ObservableEventQueue, ShardedEventQueueConfig, ShardedQueueRef},
    security::SecurityRef,
    session::{SessionIndexRef, SessionKey},
    syscall::SyscallDispatcher,
    tool::ToolRegistryRef,
};

// ---------------------------------------------------------------------------
// KernelConfig
// ---------------------------------------------------------------------------

/// Kernel configuration.
#[derive(Debug, Clone, smart_default::SmartDefault)]
pub struct KernelConfig {
    /// Maximum number of concurrent agent processes globally.
    #[default = 16]
    pub max_concurrency:        usize,
    /// Default maximum number of children per agent.
    #[default = 8]
    pub default_child_limit:    usize,
    /// Default max LLM iterations for spawned agents.
    #[default = 25]
    pub default_max_iterations: usize,
    /// Maximum number of KV entries per agent (0 = unlimited).
    /// Applies to the agent-scoped namespace only.
    #[default = 1000]
    pub memory_quota_per_agent: usize,
    // Event queue configuration. Controls whether the kernel uses a single
    // global queue (`num_shards = 0`) or sharded parallel processing.
    pub event_queue:            ShardedEventQueueConfig,
}

/// Shared reference to a
/// [`SettingsProvider`](rara_domain_shared::settings::SettingsProvider).
pub type SettingsRef = Arc<dyn rara_domain_shared::settings::SettingsProvider>;

/// The unified agent orchestrator.
///
/// Acts as an OS kernel for agents: manages the process table, enforces
/// concurrency limits, and provides the event loop as the single driver
/// for all kernel activity.
///
/// The Kernel owns its I/O subsystem: stream hub, endpoint registry, and
/// ingress pipeline. Call [`start()`](Self::start) to spawn the unified
/// event loop and egress delivery as background tasks.
///
/// After [`start()`](Self::start), the Kernel lives behind `Arc<Kernel>`.
/// Fields that were previously in a separate `KernelInner` are now
/// flattened directly into this struct.
pub struct Kernel {
    /// Kernel configuration.
    config:           KernelConfig,
    // -- Core subsystems (previously in KernelInner) -----------------------
    /// The global process table tracking all running agents.
    process_table:    Arc<SessionTable>,
    /// Global semaphore limiting total concurrent agent processes.
    global_semaphore: Arc<Semaphore>,
    /// Unified security subsystem (auth + authz + approval).
    security:         SecurityRef,
    /// Agent registry for looking up named agent definitions.
    agent_registry:   AgentRegistryRef,
    /// File-backed tape store for session message persistence.
    tape_store:       Arc<rara_memory::tape::FileTapeStore>,
    /// Lightweight session metadata index (tape-centric replacement for the
    /// session CRUD subset of `SessionRepository`).
    session_index:    SessionIndexRef,
    /// Flat KV settings provider for runtime configuration.
    settings:         SettingsRef,
    /// Syscall dispatcher (owns shared_kv, pipe_registry, driver_registry,
    /// tool_registry, event_bus).
    syscall:          SyscallDispatcher,
    // -- I/O subsystem -----------------------------------------------------
    /// Ephemeral stream hub for real-time token deltas.
    stream_hub:       StreamHubRef,
    /// Ingress pipeline for adapters to push inbound messages.
    ingress_pipeline: IngressPipelineRef,
    /// Unified event queue for all kernel interactions.
    event_queue:      EventQueueRef,
    /// Sharded event queue backing the kernel event loop.
    ///
    /// Always present. When `num_shards == 0` (single-queue mode), all
    /// events are routed to the global queue and processed by a single
    /// `EventProcessor`. When `num_shards > 0`, events are distributed
    /// across N shard queues for parallel processing.
    sharded_queue:    ShardedQueueRef,
    /// Egress delivery subsystem (adapters + endpoint registry).
    delivery:         crate::io::DeliverySubsystem,
    /// When this kernel was created (for uptime calculation).
    started_at:       Timestamp,
}

impl Kernel {
    /// Default agent name for admin/root users.
    const ADMIN_AGENT_NAME: &'static str = "nana";
    /// Default agent name for regular users.
    const USER_AGENT_NAME: &'static str = "nana";

    /// Create a new Kernel with the given configuration, components, and I/O
    /// subsystem.
    ///
    /// The I/O subsystem is fully assembled at construction time. Call
    /// [`start()`](Self::start) to spawn the unified event loop.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: KernelConfig,
        driver_registry: DriverRegistryRef,
        tool_registry: ToolRegistryRef,
        tape_store: Arc<rara_memory::tape::FileTapeStore>,
        event_bus: NotificationBusRef,
        security: SecurityRef,
        agent_registry: AgentRegistryRef,
        session_index: SessionIndexRef,
        settings: SettingsRef,
        stream_hub: StreamHubRef,
        identity_resolver: IdentityResolverRef,
        session_resolver: SessionResolverRef,
        kv_operator: opendal::Operator,
    ) -> Self {
        info!(
            max_concurrency = config.max_concurrency,
            default_child_limit = config.default_child_limit,
            default_max_iterations = config.default_max_iterations,
            "booting kernel"
        );

        let endpoint_registry = Arc::new(EndpointRegistry::new());
        let delivery = crate::io::DeliverySubsystem::new(endpoint_registry);

        let sharded_queue: ShardedQueueRef = Arc::new(crate::queue::ShardedEventQueue::new(
            config.event_queue.clone(),
        ));
        let event_queue: EventQueueRef =
            Arc::new(ObservableEventQueue::new(sharded_queue.clone(), 512));

        let ingress_pipeline = Arc::new(IngressPipeline::new(identity_resolver, session_resolver));

        let global_semaphore = Arc::new(Semaphore::new(config.max_concurrency));

        let syscall = SyscallDispatcher::new(
            SharedKv::new(kv_operator),
            PipeRegistry::new(),
            driver_registry,
            tool_registry,
            event_bus,
            config.clone(),
        );

        Self {
            config,
            process_table: Arc::new(SessionTable::new()),
            global_semaphore,
            security,
            agent_registry,
            tape_store,
            session_index,
            settings,
            syscall,
            stream_hub,
            ingress_pipeline,
            event_queue,
            sharded_queue,
            delivery,
            started_at: Timestamp::now(),
        }
    }

    /// Create a [`TapeService`](rara_memory::tape::TapeService) bound to the
    /// given session.
    // FIXME: why it is create a tapeService? we should replace the original memory
    // service as tapeMemory !
    fn tape_for(&self, session_id: &SessionKey) -> rara_memory::tape::TapeService {
        rara_memory::tape::TapeService::new(
            session_id.to_string(),
            self.tape_store.as_ref().clone(),
        )
    }

    /// List detailed runtime statistics for all processes.
    pub async fn list_processes(&self) -> Vec<crate::process::SessionStats> {
        self.process_table.all_process_stats()
    }

    /// Get kernel-wide aggregate statistics.
    pub fn system_stats(&self) -> crate::process::SystemStats {
        let pt = &self.process_table;
        let active = pt
            .list()
            .iter()
            .filter(|p| matches!(p.state, SessionState::Active | SessionState::Ready))
            .count();

        let uptime_ms = Timestamp::now()
            .since(self.started_at)
            .ok()
            .map(|span| span.get_milliseconds().unsigned_abs())
            .unwrap_or(0);

        crate::process::SystemStats {
            active_sessions: active,
            total_spawned: pt.total_spawned(),
            total_completed: pt.total_completed(),
            total_failed: pt.total_failed(),
            global_semaphore_available: self.global_semaphore.available_permits(),
            total_tokens_consumed: pt.total_tokens_consumed(),
            uptime_ms,
        }
    }

    /// Get the detailed turn traces for a specific agent process.
    pub fn get_process_turns(&self, session_key: SessionKey) -> Vec<crate::agent_loop::TurnTrace> {
        self.process_table.get_turn_traces(session_key)
    }

    /// Create a [`KernelHandle`] for external callers.
    ///
    /// The handle is cheap to clone (all `Arc`s) and routes all mutations
    /// through the event queue, while exposing read-only accessors for
    /// kernel subsystems.
    pub fn handle(&self) -> crate::handle::KernelHandle {
        crate::handle::KernelHandle::new(
            self.event_queue.clone(),
            Arc::clone(&self.agent_registry),
            Arc::clone(&self.process_table),
            Arc::clone(&self.ingress_pipeline),
            Arc::clone(&self.stream_hub),
            Arc::clone(self.delivery.endpoint_registry()),
            Arc::clone(&self.settings),
            Arc::clone(&self.security),
            self.config.clone(),
            Arc::clone(self.syscall.tool_registry()),
            Arc::clone(&self.global_semaphore),
            self.started_at,
        )
    }

    /// Access the unified event queue.
    pub fn event_queue(&self) -> &EventQueueRef { &self.event_queue }

    /// Access the sharded event queue.
    fn sharded_queue(&self) -> &ShardedQueueRef { &self.sharded_queue }

    /// Register an egress adapter for a channel type.
    ///
    /// Must be called **before** [`start()`](Self::start).
    pub fn register_adapter(&mut self, channel_type: ChannelType, adapter: EgressAdapterRef) {
        self.delivery.register_adapter(channel_type, adapter);
    }

    /// Start the unified event loop as a background task.
    ///
    /// Consumes `self` by value, wraps it in `Arc`, spawns the event loop,
    /// and returns `(Arc<Kernel>, KernelHandle)`.
    ///
    /// The `KernelHandle` is the preferred external API — it provides
    /// read-only accessors and mutation methods that flow through the event
    /// queue. The `Arc<Kernel>` is retained for internal use and backwards
    /// compatibility during the migration period.
    pub fn start(
        self,
        cancel_token: CancellationToken,
    ) -> (Arc<Self>, crate::handle::KernelHandle) {
        let kernel = Arc::new(self);
        let handle = kernel.handle();

        // Unified event loop — spawns 1 global + N shard EventProcessors.
        // When num_shards == 0 (single-queue mode), only the global
        // processor is created.
        tokio::spawn({
            let k = kernel.clone();
            let token = cancel_token;
            async move {
                Kernel::run_event_loop_arc(k, token).await;
            }
        });

        info!("kernel event loop started");
        (kernel, handle)
    }

    /// Run the unified event loop, spawning 1 global + N shard processor
    /// tasks.
    ///
    /// When `num_shards == 0` (single-queue mode), only the global processor
    /// is spawned — functionally identical to the former single-queue path
    /// but using the same code path for both modes.
    ///
    /// Called from [`start()`](Kernel::start) which already wraps Kernel in
    /// Arc.
    async fn run_event_loop_arc(kernel: Arc<Kernel>, shutdown: CancellationToken) {
        let sq = kernel.sharded_queue().clone();
        let num_shards = sq.num_shards();

        info!(
            num_shards = num_shards,
            total_processors = num_shards + 1,
            "kernel event loop started"
        );

        let mut handles = Vec::with_capacity(num_shards + 1);

        // Global processor (id=0) — always present.
        {
            let k = Arc::clone(&kernel);
            let q = Arc::clone(sq.global());
            let sd = shutdown.clone();
            handles.push(tokio::spawn(async move {
                k.run_processor(0, q, sd).await;
            }));
        }

        // Shard processors (id=1..=N) — only when sharding is enabled.
        for i in 0..num_shards {
            let k = Arc::clone(&kernel);
            let q = Arc::clone(sq.shard(i));
            let sd = shutdown.clone();
            handles.push(tokio::spawn(async move {
                k.run_processor(i + 1, q, sd).await;
            }));
        }

        for handle in handles {
            if let Err(e) = handle.await {
                error!("event processor panicked: {e}");
            }
        }

        info!("kernel event loop stopped");
    }

    /// Run a single event processor that drains events from one shard queue.
    ///
    /// Each processor runs independently, allowing parallel event handling
    /// across different agent shards. Drains in batches of up to 32.
    async fn run_processor(
        &self,
        id: usize,
        queue: Arc<crate::queue::shard::ShardQueue>,
        shutdown: CancellationToken,
    ) {
        info!(processor_id = id, "event processor started");

        loop {
            tokio::select! {
                _ = queue.wait() => {
                    loop {
                        let events = queue.drain(32);
                        if events.is_empty() { break; }
                        for event in events {
                            let event_type: &'static str = (&event.kind).into();
                            let span = info_span!(
                                "handle_event",
                                processor_id = id,
                                event_type,
                            );
                            self.handle_event(event)
                                .instrument(span)
                                .await;
                        }
                    }
                }
                _ = shutdown.cancelled() => {
                    info!(processor_id = id, "event processor shutting down");
                    let remaining = queue.drain(1024);
                    for event in remaining {
                        if matches!(event.kind, KernelEvent::SendSignal { .. } | KernelEvent::Shutdown) {
                            self.handle_event(event).await;
                        } else {
                            warn!(
                                processor_id = id,
                                event = ?event,
                                "dropping non-critical event during shutdown"
                            );
                        }
                    }
                    break;
                }
            }
        }

        info!(processor_id = id, "event processor stopped");
    }

    /// Dispatch a single event to its handler.
    pub(crate) async fn handle_event(&self, event: KernelEventEnvelope) {
        let event_type: &'static str = (&event.kind).into();
        crate::metrics::EVENT_PROCESSED
            .with_label_values(&[event_type])
            .inc();

        let KernelEventEnvelope { base, kind } = event;

        match kind {
            KernelEvent::UserMessage(msg) => {
                self.handle_user_message(msg).await;
            }
            KernelEvent::CreateSession {
                manifest,
                input,
                principal,
                parent_id,
                reply_tx,
            } => {
                // CreateSession from SessionHandle::create_child() — subagent.
                let result = self
                    .handle_spawn_agent(manifest, input, principal, parent_id, None, None)
                    .await;
                let _ = reply_tx.send(result);
            }
            KernelEvent::SendSignal { signal } => {
                self.handle_signal(base.session_key, signal).await;
            }
            KernelEvent::TurnCompleted {
                result,
                in_reply_to,
                user,
            } => {
                self.handle_turn_completed(base.session_key, result, in_reply_to, user)
                    .await;
            }
            KernelEvent::ChildSessionDone { child_id, result } => {
                self.handle_child_completed(base.session_key, child_id, result)
                    .await;
            }
            KernelEvent::Deliver(envelope) => {
                self.delivery.deliver(envelope);
            }
            KernelEvent::SessionCommand(syscall) => {
                let kernel_handle = self.handle();
                self.syscall
                    .dispatch(
                        syscall,
                        &self.process_table,
                        &self.security,
                        &self.agent_registry,
                        &kernel_handle,
                    )
                    .await;
            }
            KernelEvent::IdleCheck => {
                // Periodic idle check — handled by session table reaping.
                self.process_table
                    .reap_terminal(std::time::Duration::from_secs(300));
            }
            KernelEvent::Shutdown => {
                info!("shutdown event received");
            }
        }
    }

    /// Handle a SpawnAgent event — create a new process and its runtime.
    ///
    /// If `desired_session_key` is provided, the process is keyed by that
    /// session (used by auto-spawn in Path 3 so future messages find the
    /// process). Otherwise a fresh random key is generated.
    #[tracing::instrument(skip_all, fields(manifest_name = %manifest.name, parent_id = ?parent_id, session_key))]
    async fn handle_spawn_agent(
        &self,
        manifest: AgentManifest,
        input: String,
        principal: Principal,
        parent_id: Option<SessionKey>,
        resume_session_id: Option<SessionKey>,
        desired_session_key: Option<SessionKey>,
    ) -> crate::Result<SessionKey> {
        // Validate principal.
        self.security.validate_principal(&principal).await?;

        // Acquire global semaphore.
        let global_permit = self
            .global_semaphore
            .clone()
            .try_acquire_owned()
            .map_err(|_| KernelError::SpawnLimitReached {
                message: "global concurrency limit reached".to_string(),
            })?;

        let session_key = desired_session_key.unwrap_or_else(SessionKey::new);
        tracing::Span::current().record("session_key", tracing::field::display(&session_key));

        // TODO: fix me
        let (session_id, initial_messages) = if let Some(session_key) = resume_session_id {
            // Load previous conversation from the tape store.
            let tape = self.tape_for(&session_key);
            let entries = tape.entries().await.unwrap_or_default();
            let messages: Vec<crate::channel::types::ChatMessage> = entries
                .into_iter()
                .filter(|e| e.kind == rara_memory::tape::TapEntryKind::Message)
                .filter_map(|e| serde_json::from_value(e.payload).ok())
                .collect();
            (session_key, messages)
        } else {
            let session_id = SessionKey::new();
            (session_id, vec![])
        };

        // Create process-level cancellation token.
        // Child processes derive their token from the parent's, so cancelling
        // a parent cascades to all children automatically.
        let process_cancel = if let Some(pid) = parent_id {
            self.process_table
                .with(&pid, |parent_rt| parent_rt.process_cancel.child_token())
                .unwrap_or_default()
        } else {
            CancellationToken::new()
        };

        let child_limit = manifest
            .max_children
            .unwrap_or(self.config.default_child_limit);

        let max_context_tokens = manifest
            .max_context_tokens
            .unwrap_or(DEFAULT_MAX_CONTEXT_TOKENS);

        // Register unified session in table. The global permit is stored here
        // so it lives as long as the session — dropping the session
        // automatically releases the semaphore slot.
        let metrics = std::sync::Arc::new(crate::process::RuntimeMetrics::new());
        let process = Session {
            // -- identity & metadata --
            session_key,
            parent_id,
            manifest: manifest.clone(),
            principal: principal.clone(),
            env: AgentEnv::default(),
            state: SessionState::Ready,
            created_at: jiff::Timestamp::now(),
            finished_at: None,
            result: None,
            created_files: vec![],
            metrics,
            turn_traces: vec![],
            // -- conversation & cancellation --
            conversation: initial_messages,
            turn_cancel: CancellationToken::new(),
            process_cancel,
            paused: false,
            pause_buffer: Vec::new(),
            child_semaphore: Arc::new(Semaphore::new(child_limit)),
            max_context_tokens,
            _global_permit: global_permit,
        };
        self.process_table.insert(process);

        crate::metrics::SESSION_CREATED
            .with_label_values(&[&manifest.name])
            .inc();
        crate::metrics::SESSION_ACTIVE
            .with_label_values(&[&manifest.name])
            .inc();

        info!(
            session_key = %session_key,
            manifest = %manifest.name,
            session_id = %session_id,
            "process spawned via event loop"
        );

        // Deliver the initial input to the spawned process.
        // The synthetic UserMessage uses session_key so that
        // handle_user_message finds this process via direct table lookup.
        let msg_session = session_key;
        let inbound = InboundMessage::synthetic(
            input,
            principal.user_id.clone(),
            msg_session,
        );
        if let Err(e) = self
            .event_queue()
            .try_push(KernelEventEnvelope::user_message(inbound))
        {
            error!(%e, "failed to push initial UserMessage for spawned agent");
        }

        Ok(session_key)
    }

    // -----------------------------------------------------------------------
    // handle_signal
    // -----------------------------------------------------------------------

    /// Handle a control signal sent to a session runtime.
    #[tracing::instrument(skip_all, fields(session_key = %target, signal = ?signal))]
    async fn handle_signal(&self, target: SessionKey, signal: Signal) {
        match signal {
            Signal::Interrupt => {
                info!(session_key = %target, "interrupt signal");
                self.process_table.cancel_and_refresh_turn(&target);
                // Notify via Deliver event — use session key directly for egress.
                if self.process_table.with(&target, |_| ()).is_none() {
                    error!(session_key = %target, "cannot send interrupt notification: process not found");
                    return;
                }
                let envelope = OutboundEnvelope::state_change(
                    MessageId::new(),
                    crate::process::principal::UserId("system".to_string()),
                    target.clone(),
                    "interrupted",
                    serde_json::json!({
                        "session_key": target.to_string(),
                        "message": "Agent interrupted by user",
                    }),
                );
                if let Err(e) = self
                    .event_queue()
                    .try_push(KernelEventEnvelope::deliver(envelope))
                {
                    error!(%e, "failed to push interrupt notification");
                }
            }
            Signal::Pause => {
                info!(session_key = %target, "pause signal");
                self.process_table.set_paused(&target, true);
                let _ = self.process_table.set_state(target, SessionState::Paused);
            }
            Signal::Resume => {
                info!(session_key = %target, "resume signal");
                self.process_table.set_paused(&target, false);
                let buffered = self.process_table.drain_pause_buffer(&target);
                let _ = self.process_table.set_state(target, SessionState::Ready);
                for event in buffered {
                    if let Err(e) = self.event_queue().try_push(event) {
                        warn!(%e, "failed to re-inject buffered event on resume");
                    }
                }
            }
            Signal::Terminate => {
                info!(session_key = %target, "terminate signal — graceful shutdown");
                let was_active = self
                    .process_table
                    .with(&target, |p| p.state == SessionState::Active)
                    .unwrap_or(false);
                let _ = self
                    .process_table
                    .set_state(target, SessionState::Suspended);
                self.process_table.cancel_turn(&target);
                // Grace period then force-kill via process_cancel token.
                if let Some(token) = self.process_table.clone_process_cancel(&target) {
                    tokio::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        token.cancel();
                    });
                }
                if !was_active {
                    self.cleanup_process(target).await;
                }
            }
            Signal::Kill => {
                info!(session_key = %target, "kill signal");
                self.process_table.cancel_process(&target);
                let _ = self
                    .process_table
                    .set_state(target, SessionState::Suspended);
                self.cleanup_process(target).await;
            }
        }
    }

    // -----------------------------------------------------------------------
    // handle_child_completed
    // -----------------------------------------------------------------------

    /// Handle a child agent completion — persist result to parent's
    /// conversation.
    #[tracing::instrument(skip_all, fields(parent_id = %parent_id, child_id = %child_id, output_len = result.output.len()))]
    async fn handle_child_completed(
        &self,
        parent_id: SessionKey,
        child_id: SessionKey,
        result: AgentRunLoopResult,
    ) {
        info!(
            parent_id = %parent_id,
            child_id = %child_id,
            output_len = result.output.len(),
            "child result received"
        );

        // Persist child result to parent's conversation history.
        let child_result_text = format!(
            "[child_agent_result] child_id={child_id} iterations={} tool_calls={}\n\n{}",
            result.iterations, result.tool_calls, result.output,
        );
        let child_msg = crate::channel::types::ChatMessage::system(&child_result_text);

        self.process_table.with_mut(&parent_id, |rt| {
            rt.conversation.push(child_msg.clone());
        });

        let Some(session_id) = self.process_table.with(&parent_id, |p| p.session_key) else {
            error!(parent_id = %parent_id, child_id = %child_id, "cannot persist child result: parent process not found");
            return;
        };

        {
            let tape = self.tape_for(&session_id);
            if let Err(e) = tape
                .append_message(serde_json::to_value(&child_msg).unwrap_or_default())
                .await
            {
                warn!(%e, "failed to persist child result message to tape");
            }
        }
    }

    /// Clean up a process runtime entry.
    ///
    /// Removing the runtime from the table drops the `process_cancel` token
    /// naturally, so no explicit cancellation-token cleanup is needed.
    async fn cleanup_process(&self, session_key: SessionKey) {
        if let Some(rt) = self.process_table.remove(session_key) {
            let manifest_name = rt.manifest.name.clone();
            let state = rt.state;
            let parent_id = rt.parent_id;

            crate::metrics::SESSION_ACTIVE
                .with_label_values(&[&manifest_name])
                .dec();
            crate::metrics::SESSION_SUSPENDED
                .with_label_values(&[&manifest_name, &state.to_string()])
                .inc();

            // Notify parent if this is a child process.
            if let Some(parent_id) = parent_id {
                let result = rt.result.unwrap_or(AgentRunLoopResult {
                    output:     "process ended".to_string(),
                    iterations: 0,
                    tool_calls: 0,
                });
                let event = KernelEventEnvelope::child_session_done(parent_id, session_key, result);
                if let Err(e) = self.event_queue().try_push(event) {
                    warn!(%e, "failed to push ChildSessionDone event");
                }
            }
        }
    }

    /// Handle a user message with 3-path routing:
    ///
    /// 1. **ID addressing** (`target_session_key` set): deliver to specific
    ///    session — error if terminal or not found (A2A Protocol pattern).
    /// 2. **Session addressing** (direct process table lookup): deliver to
    ///    existing session — if terminal, respawn transparently.
    /// 3. **Name addressing** (fallback): lookup AgentRegistry by name, spawn
    ///    a new session keyed by the incoming session_key.
    async fn handle_user_message(&self, msg: InboundMessage) {
        let span = info_span!(
            "handle_user_message",
            session_id = %msg.session_key,
            user_id = %msg.user.0,
            channel = ?msg.source.channel_type,
            routing_path = tracing::field::Empty,
        );
        let _guard = span.enter();

        let session_id = msg.session_key.clone();
        let user = msg.user.clone();

        self.delivery.register_stateless_endpoint(&msg);

        // ----- Path 1: ID addressing (agent-to-agent) -----
        if let Some(target_id) = msg.target_session_key {
            span.record("routing_path", "id_addressing");
            let target_state = self.process_table.with(&target_id, |p| p.state);
            match target_state {
                Some(state) if state.is_terminal() => {
                    let envelope = OutboundEnvelope::error(
                        msg.id.clone(),
                        user.clone(),
                        session_id.clone(),
                        "process_terminal",
                        format!("process {} is {}", target_id, state),
                    );
                    if let Err(e) = self
                        .event_queue()
                        .try_push(KernelEventEnvelope::deliver(envelope))
                    {
                        error!(%e, "failed to push process-terminal error Deliver");
                    }
                    return;
                }
                Some(_) => {
                    self.deliver_to_session(target_id, msg).await;
                    return;
                }
                None => {
                    let envelope = OutboundEnvelope::error(
                        msg.id.clone(),
                        user.clone(),
                        session_id.clone(),
                        "process_not_found",
                        format!("process not found: {target_id}"),
                    );
                    if let Err(e) = self
                        .event_queue()
                        .try_push(KernelEventEnvelope::deliver(envelope))
                    {
                        error!(%e, "failed to push process-not-found error Deliver");
                    }
                    return;
                }
            }
        }

        // ----- Path 2: Session addressing (external user) -----
        let mut resume_session_id = None;
        let path2_info = self
            .process_table
            .with(&session_id, |p| (p.session_key, p.state));
        if let Some((session_key_found, state)) = path2_info {
            span.record("routing_path", "session_addressing");

            if state.is_terminal() {
                info!(
                    session_key = %session_key_found,
                    session_id = %session_id,
                    state = %state,
                    "process terminal — will respawn"
                );
                resume_session_id = Some(session_key_found);
                // Fall through to Path 3 below.
            } else {
                self.deliver_to_session(session_key_found, msg).await;
                return;
            }
        }

        // ----- Path 3: Name addressing (always spawn new) -----
        span.record("routing_path", "name_addressing");
        let target_name = self.default_agent_for_user(&msg.user).await;

        let manifest = if let Some(m) = self.agent_registry.get(&target_name) {
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
            let envelope = OutboundEnvelope::error(
                msg.id.clone(),
                user.clone(),
                session_id.clone(),
                "unknown_agent",
                format!("unknown target agent: {target_name}"),
            );
            if let Err(e) = self
                .event_queue()
                .try_push(KernelEventEnvelope::deliver(envelope))
            {
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
                None,
                resume_session_id,
                Some(session_id.clone()),
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

    /// Deliver a message to a live process: buffer if the process is paused
    /// or busy (Running state), otherwise start a new LLM turn.
    async fn deliver_to_session(&self, session_key: SessionKey, msg: InboundMessage) {
        // Check process state outside the runtime closure to avoid nested locks.
        let is_active = self
            .process_table
            .with(&session_key, |p| p.state == SessionState::Active)
            .unwrap_or(false);

        let should_buffer = self.process_table.with_mut(&session_key, |rt| {
            if rt.paused {
                rt.pause_buffer
                    .push(KernelEventEnvelope::user_message(msg.clone()));
                return true;
            }
            if is_active {
                rt.pause_buffer
                    .push(KernelEventEnvelope::user_message(msg.clone()));
                return true;
            }
            false
        });
        if should_buffer == Some(true) {
            return;
        }
        self.start_llm_turn(session_key, msg).await;
    }

    /// Determine the default agent name for a user based on their role.
    ///
    /// - Root / Admin users -> "rara" (full-capability agent)
    /// - Regular users -> "nana" (chat-only companion)
    /// - Unknown users -> "nana" (safe default)
    async fn default_agent_for_user(&self, user: &crate::process::principal::UserId) -> String {
        use crate::process::principal::Role;

        match self.security.resolve_user_role(user).await {
            Role::Root | Role::Admin => Self::ADMIN_AGENT_NAME.to_string(),
            Role::User => Self::USER_AGENT_NAME.to_string(),
        }
    }

    /// Resolve a manifest for auto-spawning (when a user message arrives
    /// with no existing process).
    /// TODO: what?
    async fn resolve_manifest_for_auto_spawn(&self) -> Option<crate::process::AgentManifest> {
        let model = rara_domain_shared::settings::get_default_model(self.settings.as_ref()).await;
        Some(crate::process::AgentManifest {
            name: "io-agent".to_string(),
            role: AgentRole::default(),
            description: "I/O bus agent".to_string(),
            model,
            system_prompt: "You are a helpful assistant.".to_string(),
            soul_prompt: None,
            provider_hint: None,
            max_iterations: Some(25),
            tools: vec![],
            max_children: None,
            max_context_tokens: None,
            priority: crate::process::Priority::default(),
            metadata: serde_json::Value::Null,
            sandbox: None,
        })
    }

    /// Start an LLM turn for the given agent, spawning the work as an async
    /// task that pushes `TurnCompleted` back into the EventQueue when done.
    #[tracing::instrument(skip_all, fields(session_key = %session_key, session_key = %msg.session_key))]
    async fn start_llm_turn(&self, session_key: SessionKey, msg: InboundMessage) {
        /// RAII guard ensuring that `TurnCompleted` is always pushed and the
        /// stream is always closed, even when the spawned turn task
        /// panics or is cancelled.
        ///
        /// On normal completion the caller sets `completed = true` before the
        /// guard is dropped; on abnormal exit `Drop` performs the
        /// cleanup.
        struct TurnGuard {
            event_queue:    EventQueueRef,
            stream_hub:     Arc<crate::io::StreamHub>,
            stream_id:      StreamId,
            typing_refresh: Option<tokio::task::JoinHandle<()>>,
            session_key:    SessionKey,
            msg_id:         MessageId,
            user:           crate::process::principal::UserId,
            completed:      bool,
        }

        impl Drop for TurnGuard {
            fn drop(&mut self) {
                if !self.completed {
                    // Abort typing refresh if still running.
                    if let Some(handle) = self.typing_refresh.take() {
                        handle.abort();
                    }

                    // Close stream so the forwarder stops.
                    self.stream_hub.close(&self.stream_id);

                    // Push a failed TurnCompleted so the process exits Running state.
                    let event = KernelEventEnvelope::turn_completed(
                        self.session_key,
                        Err("turn task terminated unexpectedly".to_string()),
                        self.msg_id.clone(),
                        self.user.clone(),
                    );
                    if let Err(e) = self.event_queue.try_push(event) {
                        error!(
                            %e,
                            session_key = %self.session_key,
                            "TurnGuard: failed to push TurnCompleted on abnormal exit"
                        );
                    } else {
                        warn!(
                            session_key = %self.session_key,
                            "TurnGuard: turn task exited abnormally, pushed TurnCompleted(Err)"
                        );
                    }
                }
            }
        }

        if !self.process_table.contains(&session_key) {
            warn!(session_key = %session_key, "runtime not found for LLM turn");
            // Send error back to the user instead of silently dropping.
            let envelope = OutboundEnvelope::error(
                msg.id.clone(),
                msg.user.clone(),
                msg.session_key.clone(),
                "runtime_not_found",
                format!("agent runtime not found: {session_key}"),
            );
            if let Err(e) = self
                .event_queue()
                .try_push(KernelEventEnvelope::deliver(envelope))
            {
                error!(%e, "failed to push runtime-not-found error Deliver");
            }
            return;
        }

        let session_key = msg.session_key.clone();
        let user = msg.user.clone();
        let msg_id = msg.id.clone();

        // Set state to Active.
        let _ = self
            .process_table
            .set_state(session_key, SessionState::Active);

        // Send a typing / progress indicator so the user sees feedback
        // while the LLM is thinking (e.g. Telegram "typing..." bubble).
        let egress_session_key = session_key;
        let _ =
            self.event_queue()
                .try_push(KernelEventEnvelope::deliver(OutboundEnvelope::progress(
                    msg_id.clone(),
                    user.clone(),
                    egress_session_key.clone(),
                    crate::io::stages::THINKING,
                    None,
                )));

        // Record metrics.
        if let Some(metrics) = self.process_table.get_metrics(&session_key) {
            metrics.record_message();
        }

        // Apply context compaction + build history + append user message
        // inside a single `with_mut` closure to minimize lock duration.
        let user_text = msg.content.as_text();
        let user_msg = ChatMessage::user(&user_text);

        let turn_data = self.process_table.with_mut(&session_key, |rt| {
            // Swap out the conversation for async compaction, then put it
            // back after compaction completes.
            let conversation = std::mem::take(&mut rt.conversation);
            (
                conversation,
                rt.max_context_tokens,
                rt.session_key,
                rt.principal.clone(),
                rt.turn_cancel.clone(),
            )
        });

        let Some((conversation, max_context_tokens, rt_session_key, rt_principal, turn_cancel)) =
            turn_data
        else {
            warn!(session_key = %session_key, "runtime disappeared during LLM turn setup");
            return;
        };

        // Apply context compaction (async).
        let compaction_strategy = crate::compaction::SlidingWindowCompaction;
        let compacted = crate::compaction::maybe_compact(
            conversation,
            max_context_tokens,
            &compaction_strategy,
        )
        .await;

        // Convert history to LLM format.
        let history = {
            let msgs = crate::agent_loop::build_llm_history(&compacted);
            if msgs.is_empty() { None } else { Some(msgs) }
        };

        // Put compacted conversation back and append user message.
        self.process_table.with_mut(&session_key, |rt| {
            rt.conversation = compacted;
            rt.conversation.push(user_msg.clone());
        });

        // Persist in background to avoid blocking event loop.
        {
            let tape = self.tape_for(&session_key);
            let user_msg = user_msg.clone();
            tokio::spawn(async move {
                if let Err(e) = tape
                    .append_message(serde_json::to_value(&user_msg).unwrap_or_default())
                    .await
                {
                    warn!(%e, "failed to persist user message to tape");
                }
            });
        }

        // Open stream.
        let stream_handle = self.stream_hub.open(session_key.clone());

        // Clone what we need for the spawned task.
        let kernel_handle = self.handle();
        let event_queue = self.event_queue().clone();
        let stream_id = stream_handle.stream_id().clone();
        let typing_session_key = egress_session_key;
        let stream_hub_ref = Arc::clone(&self.stream_hub);

        // Capture parent span for the spawned task.
        let parent_span = tracing::Span::current();

        // Spawn async task for the LLM turn.
        tokio::spawn(async move {
            let turn_span = info_span!(
                parent: &parent_span,
                "agent_turn",
                session_key = %session_key,
                session_key = %session_key,
                total_ms = tracing::field::Empty,
                iterations = tracing::field::Empty,
                tool_calls = tracing::field::Empty,
            );
            let _span_guard = turn_span.enter();
            let start = std::time::Instant::now();

            // Spawn a background task that refreshes the typing indicator every
            // 4 seconds.  Telegram's `sendChatAction(typing)` expires after ~5s,
            // so we re-send it periodically to keep the indicator visible while
            // the LLM is reasoning.
            let typing_refresh = {
                let eq = event_queue.clone();
                let sid = typing_session_key.clone();
                let usr = user.clone();
                let mid = msg_id.clone();
                tokio::spawn(async move {
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(4));
                    interval.tick().await; // skip the immediate first tick
                    loop {
                        interval.tick().await;
                        let _ =
                            eq.try_push(KernelEventEnvelope::deliver(OutboundEnvelope::progress(
                                mid.clone(),
                                usr.clone(),
                                sid.clone(),
                                crate::io::stages::THINKING,
                                None,
                            )));
                    }
                })
            };

            // TurnGuard ensures cleanup on panic or cancellation.
            let mut turn_guard = TurnGuard {
                event_queue: event_queue.clone(),
                stream_hub: Arc::clone(&stream_hub_ref),
                stream_id: stream_id.clone(),
                typing_refresh: Some(typing_refresh),
                session_key: session_key.clone(),
                msg_id: msg_id.clone(),
                user: user.clone(),
                completed: false,
            };

            let turn_result = crate::agent_loop::run_inline_agent_loop(
                &kernel_handle,
                rt_session_key,
                user_text,
                history,
                &stream_handle,
                &turn_cancel,
            )
            .await;

            // Stop the typing refresh loop now that the turn is done.
            if let Some(handle) = turn_guard.typing_refresh.take() {
                handle.abort();
            }

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
                stream_handle.emit(crate::io::StreamEvent::TurnMetrics {
                    duration_ms: elapsed_ms,
                    iterations:  result.iterations,
                    tool_calls:  result.tool_calls,
                    model:       result.model.clone(),
                });
            }

            // Close stream.
            stream_hub_ref.close(&stream_id);

            // Push TurnCompleted back into the event queue.
            // Convert KernelError -> String at the event boundary because
            // KernelEvent requires Clone but KernelError does not implement it.
            let result = turn_result.map_err(|e| e.to_string());
            let event = KernelEventEnvelope::turn_completed(session_key, result, msg_id, user);
            if let Err(e) = event_queue.try_push(event) {
                error!(%e, session_key = %session_key, "failed to push TurnCompleted");
            }

            // Normal completion — disarm the guard.
            turn_guard.completed = true;
        });
    }

    // -----------------------------------------------------------------------
    // handle_turn_completed
    // -----------------------------------------------------------------------

    /// Handle an LLM turn completion — persist result, deliver reply, drain
    /// pause buffer.
    #[tracing::instrument(skip_all, fields(session_key = %session_key,  success, iterations, tool_calls, reply_len))]
    async fn handle_turn_completed(
        &self,
        session_key: SessionKey,
        result: std::result::Result<crate::agent_loop::AgentTurnResult, String>,
        in_reply_to: MessageId,
        user: crate::process::principal::UserId,
    ) {
        let span = tracing::Span::current();

        if self
            .process_table
            .with(&session_key, |p| p.state.is_terminal())
            .unwrap_or(false)
        {
            info!(
                session_key = %session_key,
                "ignoring turn completion for terminal process"
            );
            self.cleanup_process(session_key).await;
            return;
        }

        // Egress uses session_key directly. Subagents without external
        // delivery have their results flow back to the parent via
        // ChildSessionDone.
        let egress_session_key = session_key;

        // Update metrics.
        if let Some(metrics) = self.process_table.get_metrics(&session_key) {
            metrics.touch().await;
        }

        // Track whether the turn errored so we can choose the right terminal
        // state below (Completed vs Failed).
        let mut turn_failed = false;

        let agent_name = self
            .process_table
            .with(&session_key, |p| p.manifest.name.clone())
            .unwrap_or_else(|| "unknown".to_string());

        match result {
            Ok(turn) if !turn.text.is_empty() => {
                span.record("success", true);
                span.record("iterations", turn.iterations);
                span.record("tool_calls", turn.tool_calls);
                span.record("reply_len", turn.text.len());

                let estimated_input_tokens = turn
                    .trace
                    .input_text
                    .as_deref()
                    .map(|text| (text.len() as u64).saturating_div(4).max(1))
                    .unwrap_or(0);
                let estimated_output_tokens = (turn.text.len() as u64).saturating_div(4).max(1);
                crate::metrics::record_turn_metrics(
                    &agent_name,
                    &turn.model,
                    turn.trace.duration_ms,
                    estimated_input_tokens,
                    estimated_output_tokens,
                );

                // Store turn trace for observability.
                self.process_table
                    .push_turn_trace(session_key, turn.trace.clone());

                // Record metrics.
                if let Some(metrics) = self.process_table.get_metrics(&session_key) {
                    metrics.record_llm_call();
                    metrics.record_tool_calls(turn.tool_calls as u64);
                    let estimated_tokens = (turn.text.len() as u64).saturating_div(4).max(1);
                    metrics.record_tokens(estimated_tokens);
                }

                // Persist assistant reply to the process's own session.
                let assistant_msg = ChatMessage::assistant(&turn.text);
                self.process_table.with_mut(&session_key, |rt| {
                    rt.conversation.push(assistant_msg.clone());
                });
                {
                    let tape = self.tape_for(&session_key);
                    if let Err(e) = tape
                        .append_message(serde_json::to_value(&assistant_msg).unwrap_or_default())
                        .await
                    {
                        warn!(%e, "failed to persist assistant message to tape");
                    }
                }

                let result = AgentRunLoopResult {
                    output:     turn.text.clone(),
                    iterations: turn.iterations,
                    tool_calls: turn.tool_calls,
                };
                let _ = self.process_table.set_result(session_key, result.clone());

                // Push Deliver event for the reply — use egress session for routing.
                let envelope = OutboundEnvelope::reply(
                    in_reply_to,
                    user.clone(),
                    egress_session_key.clone(),
                    crate::channel::types::MessageContent::Text(turn.text),
                    vec![],
                );
                if let Err(e) = self
                    .event_queue()
                    .try_push(KernelEventEnvelope::deliver(envelope))
                {
                    error!(%e, "failed to push Deliver event");
                }

                info!(
                    session_key = %session_key,
                    iterations = result.iterations,
                    tool_calls = result.tool_calls,
                    reply_len = result.output.len(),
                    "turn completed"
                );

                self.process_table.with_mut(&session_key, |rt| {
                    rt.result = Some(result);
                });
            }
            Ok(turn) => {
                span.record("success", true);
                span.record("iterations", turn.iterations);
                span.record("tool_calls", turn.tool_calls);
                span.record("reply_len", 0u64);
                info!(session_key = %session_key, "turn completed (empty result)");

                // Store turn trace for observability.
                self.process_table
                    .push_turn_trace(session_key, turn.trace.clone());

                // Empty result — LLM call was made but produced no text.
                if let Some(metrics) = self.process_table.get_metrics(&session_key) {
                    metrics.record_llm_call();
                    metrics.record_tool_calls(turn.tool_calls as u64);
                }
            }
            Err(err_msg) => {
                span.record("success", false);
                turn_failed = err_msg != "interrupted by user";
                warn!(session_key = %session_key, error = %err_msg, "turn completed (error)");

                // Deliver error — use egress session for routing.
                let envelope = OutboundEnvelope::error(
                    in_reply_to,
                    user.clone(),
                    egress_session_key.clone(),
                    "agent_error",
                    err_msg,
                );
                if let Err(e) = self
                    .event_queue()
                    .try_push(KernelEventEnvelope::deliver(envelope))
                {
                    error!(%e, "failed to push error Deliver event");
                }
            }
        }

        // Session-centric model: sessions are long-lived. After each turn,
        // the session transitions to Ready (idle) instead of a terminal state.
        // The next user message will be routed to the same session via Path 2.

        // Drain pause buffer — if the user sent messages while the turn was
        // running, re-inject them so they start a new turn on this session.
        let buffered = self.process_table.drain_pause_buffer(&session_key);

        // Transition to Ready (idle, awaiting next message).
        let _ = self
            .process_table
            .set_state(session_key, SessionState::Ready);

        // Re-inject buffered events so they trigger a new turn on this session.
        for event in buffered {
            if let Err(e) = self.event_queue().try_push(event) {
                warn!(%e, "failed to re-inject buffered event");
            }
        }
    }
}
