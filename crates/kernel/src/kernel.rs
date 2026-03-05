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

use futures::future::join_all;
use jiff::Timestamp;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use tracing::{Instrument, error, info, info_span, warn};

use crate::{
    KernelError,
    agent::{AgentEnv, AgentManifest, AgentRegistryRef, AgentTurnResult, run_agent_loop},
    event::{KernelEvent, KernelEventEnvelope},
    identity::Principal,
    io::{IOSubsystem, InboundMessage, MessageId, OutboundEnvelope, PipeRegistry, StreamId},
    kv::SharedKv,
    llm::DriverRegistryRef,
    memory::TapeService,
    notification::{BroadcastNotificationBus, NotificationBusRef},
    queue::{EventQueueRef, ShardedEventQueueConfig, ShardedQueueRef},
    security::SecurityRef,
    session::{
        AgentRunLoopResult, Session, SessionIndexRef, SessionKey, SessionState, SessionTable,
        Signal,
    },
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
    /// Tape service for session message persistence.
    tape_service:     TapeService,
    /// Lightweight session metadata index (tape-centric replacement for the
    /// session CRUD subset of `SessionRepository`).
    session_index:    SessionIndexRef,
    /// Flat KV settings provider for runtime configuration.
    settings:         SettingsRef,
    /// Syscall dispatcher (owns shared_kv, pipe_registry, driver_registry,
    /// tool_registry, event_bus).
    syscall:          SyscallDispatcher,
    // -- I/O subsystem -----------------------------------------------------
    /// Bundled I/O subsystem (ingress, stream hub, delivery).
    io:               Arc<IOSubsystem>,
    /// Unified event queue for all kernel interactions.
    event_queue:      EventQueueRef,
    /// Sharded event queue backing the kernel event loop.
    ///
    /// Always present. When `num_shards == 0` (single-queue mode), all
    /// events are routed to the global queue and processed by a single
    /// `EventProcessor`. When `num_shards > 0`, events are distributed
    /// across N shard queues for parallel processing.
    sharded_queue:    ShardedQueueRef,
    /// When this kernel was created (for uptime calculation).
    started_at:       Timestamp,
}

impl Kernel {
    /// Construct a kernel from core infrastructure dependencies.
    ///
    /// Registries are loaded separately via `load_*` methods before `start()`.
    pub fn new(
        config: KernelConfig,
        driver_registry: DriverRegistryRef,
        tool_registry: ToolRegistryRef,
        agent_registry: AgentRegistryRef,
        session_index: SessionIndexRef,
        tape_service: TapeService,
        settings: SettingsRef,
        security: SecurityRef,
        io: IOSubsystem,
    ) -> Self {
        let event_bus: NotificationBusRef = Arc::new(BroadcastNotificationBus::default());

        info!(
            max_concurrency = config.max_concurrency,
            default_child_limit = config.default_child_limit,
            default_max_iterations = config.default_max_iterations,
            "booting kernel"
        );

        let sharded_queue: ShardedQueueRef = Arc::new(crate::queue::ShardedEventQueue::new(
            config.event_queue.clone(),
        ));
        let event_queue: EventQueueRef = sharded_queue.clone();

        let global_semaphore = Arc::new(Semaphore::new(config.max_concurrency));

        let syscall = SyscallDispatcher::new(
            SharedKv::new(
                opendal::Operator::new(opendal::services::Memory::default())
                    .expect("memory operator")
                    .finish(),
            ),
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
            tape_service,
            session_index,
            settings,
            syscall,
            io: Arc::new(io),
            event_queue,
            sharded_queue,
            started_at: Timestamp::now(),
        }
    }

    /// List detailed runtime statistics for all processes.
    pub async fn list_processes(&self) -> Vec<crate::session::SessionStats> {
        self.process_table.all_process_stats()
    }

    /// Get kernel-wide aggregate statistics.
    pub fn system_stats(&self) -> crate::session::SystemStats {
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

        crate::session::SystemStats {
            active_sessions: active,
            total_spawned: pt.total_spawned(),
            total_completed: pt.total_completed(),
            total_failed: pt.total_failed(),
            global_semaphore_available: self.global_semaphore.available_permits(),
            total_tokens_consumed: pt.total_tokens_consumed(),
            uptime_ms,
        }
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
            Arc::clone(&self.io),
            Arc::clone(&self.settings),
            Arc::clone(&self.security),
            self.config.clone(),
            Arc::clone(self.syscall.driver_registry()),
            Arc::clone(self.syscall.tool_registry()),
            Arc::clone(&self.global_semaphore),
            self.started_at,
        )
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
                Kernel::run(k, token).await;
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
    async fn run(kernel: Arc<Kernel>, shutdown: CancellationToken) {
        let sq = kernel.sharded_queue.clone();
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
        queue: Arc<crate::queue::ShardQueue>,
        shutdown: CancellationToken,
    ) {
        info!(processor_id = id, "event processor started");

        loop {
            tokio::select! {
                _ = queue.wait() => {
                    loop {
                        let mut events = queue.drain(32).peekable();
                        if events.peek().is_none() { break; }
                        let futs = events.map(|event| {
                            let event_type: &'static str = (&event.kind).into();
                            let span = info_span!(
                                "handle_event",
                                processor_id = id,
                                event_type,
                            );
                            self.handle_event(event).instrument(span)
                        });
                        join_all(futs).await;
                    }
                }
                _ = shutdown.cancelled() => {
                    info!(processor_id = id, "event processor shutting down");
                    for event in queue.drain(1024) {
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
    async fn handle_event(&self, event: KernelEventEnvelope) {
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
                self.io.deliver(envelope);
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
        // TODO: not yet implemented — intended for restoring a previous
        // session's tape/history so the agent can resume where it left off.
        _resume_session_id: Option<SessionKey>,
        // If provided, the spawned session will use this key instead of
        // generating a fresh one (used when routing a message to an
        // already-known session identity).
        desired_session_key: Option<SessionKey>,
    ) -> crate::Result<SessionKey> {
        // Validate principal.
        let principal = self.security.resolve_principal(&principal).await?;

        // Acquire global semaphore.
        let global_permit = self
            .global_semaphore
            .clone()
            .try_acquire_owned()
            .map_err(|_| KernelError::SpawnLimitReached {
                message: "global concurrency limit reached".to_string(),
            })?;

        let session_key = desired_session_key.unwrap_or_default();
        tracing::Span::current().record("session_key", tracing::field::display(&session_key));

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

        // Register unified session in table. The global permit is stored here
        // so it lives as long as the session — dropping the session
        // automatically releases the semaphore slot.
        let metrics = std::sync::Arc::new(crate::session::RuntimeMetrics::new());
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
            // -- cancellation --
            turn_cancel: CancellationToken::new(),
            process_cancel,
            paused: false,
            pause_buffer: Vec::new(),
            child_semaphore: Arc::new(Semaphore::new(child_limit)),
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
            "process spawned via event loop"
        );

        // Deliver the initial input to the spawned process.
        // The synthetic UserMessage uses session_key so that
        // handle_user_message finds this process via direct table lookup.
        let msg_session = session_key;
        let inbound = InboundMessage::synthetic(input, principal.user_id.clone(), msg_session);
        if let Err(e) = &self
            .event_queue
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
                    crate::identity::UserId("system".to_string()),
                    target.clone(),
                    "interrupted",
                    serde_json::json!({
                        "session_key": target.to_string(),
                        "message": "Agent interrupted by user",
                    }),
                );
                if let Err(e) = &self
                    .event_queue
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
                    if let Err(e) = &self.event_queue.try_push(event) {
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
        let Some(session_id) = self.process_table.with(&parent_id, |p| p.session_key) else {
            error!(parent_id = %parent_id, child_id = %child_id, "cannot persist child result: parent process not found");
            return;
        };

        let tape_name = session_id.to_string();
        if let Err(e) = &self
            .tape_service
            .append_message(
                &tape_name,
                serde_json::json!({
                    "role": "user",
                    "content": &child_result_text,
                }),
                None,
            )
            .await
        {
            warn!(%e, "failed to persist child result message to tape");
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
                if let Err(e) = &self.event_queue.try_push(event) {
                    warn!(%e, "failed to push ChildSessionDone event");
                }
            }
        }
    }

    /// Handle a user message: resolve session, then route.
    ///
    /// ## Session resolution (before routing)
    ///
    /// `msg.session_key` arrives as `Option<SessionKey>` from the I/O layer:
    /// - `Some` — channel binding already exists, reuse the session.
    /// - `None` — first message from this chat. Creates a new
    ///   [`SessionEntry`] + [`ChannelBinding`] so future messages are routed
    ///   automatically, then patches `msg.session_key = Some(new_key)`.
    ///
    /// After resolution, `session_id` is always a valid key and all
    /// downstream code sees `Some`.
    ///
    /// ## 3-path routing
    ///
    /// 1. **ID addressing** (`target_session_key` set): deliver to specific
    ///    session — error if terminal or not found (A2A Protocol pattern).
    /// 2. **Session addressing** (direct process table lookup): deliver to
    ///    existing session — if terminal, respawn transparently.
    /// 3. **Role-based default** (fallback): lookup AgentRegistry by user
    ///    role, spawn a new agent process keyed by `session_id`.
    #[tracing::instrument(
        skip(self, msg),
        fields(
            session_id = ?msg.session_key,
            user_id = %msg.user.0,
            channel = ?msg.source.channel_type,
            routing_path,
        )
    )]
    async fn handle_user_message(&self, msg: InboundMessage) {
        let span = tracing::Span::current();

        // -- Session resolution --------------------------------------------------
        //
        // IOSubsystem::resolve() is read-only: it looks up the channel binding
        // but never creates sessions. When a message arrives from a chat with
        // no binding yet (first message), session_key is None.
        //
        // Here we resolve that None by:
        //   1. Creating a new SessionEntry (UUID key, empty metadata).
        //   2. Writing a ChannelBinding so subsequent messages from the same
        //      chat are routed to this session automatically.
        //
        // After this block, `session_id` is always a valid SessionKey.
        let session_id = match msg.session_key.clone() {
            Some(key) => key,
            None => {
                let now = chrono::Utc::now();
                let entry = crate::session::SessionEntry {
                    key:           SessionKey::new(),
                    title:         None,
                    model:         None,
                    system_prompt: None,
                    message_count: 0,
                    preview:       None,
                    metadata:      None,
                    created_at:    now,
                    updated_at:    now,
                };
                let session = match self.io.session_index().create_session(&entry).await {
                    Ok(s) => s,
                    Err(e) => {
                        error!(error = %e, "failed to create session for new binding");
                        return;
                    }
                };
                // Write a (channel_type, chat_id) → session_key binding so
                // IOSubsystem can resolve future messages from this chat.
                if let Some(chat_id) = msg.source.platform_chat_id.as_deref() {
                    let binding = crate::session::ChannelBinding {
                        channel_type: msg.source.channel_type.to_string(),
                        chat_id:      chat_id.to_string(),
                        session_key:  session.key.clone(),
                        created_at:   now,
                        updated_at:   now,
                    };
                    if let Err(e) = self.io.session_index().bind_channel(&binding).await {
                        warn!(error = %e, "failed to bind channel to new session");
                    }
                }
                session.key
            }
        };

        // Patch msg so downstream code (routing, LLM turn, stream forwarder)
        // always sees Some(session_key). See InboundMessage doc for lifecycle.
        let mut msg = msg;
        msg.session_key = Some(session_id.clone());

        let user = msg.user.clone();

        self.io.register_stateless_endpoint(&msg);

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
                    if let Err(e) = &self
                        .event_queue
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
                    if let Err(e) = &self
                        .event_queue
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

        // ----- Path 3: Role-based default agent (always spawn new) -----
        span.record("routing_path", "role_default");

        let manifest = match self
            .agent_registry
            .agent_for_role(self.security.resolve_user_role(&user).await)
        {
            Some(m) => m,
            None => {
                error!(
                    session_id = %session_id,
                    user = %user,
                    "no default agent registered for user role — check agent registry configuration"
                );
                return;
            }
        };

        let principal = Principal::lookup(user.0.clone());
        let spawn_result = self
            .handle_spawn_agent(
                manifest,
                msg.content.as_text(),
                principal,
                None,
                resume_session_id,
                Some(session_id.clone()),
            )
            .await;
        match spawn_result {
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

    /// Start an LLM turn for the given agent, spawning the work as an async
    /// task that pushes `TurnCompleted` back into the EventQueue when done.
    #[tracing::instrument(skip_all, fields(session_key = %session_key, msg_session_key = ?msg.session_key))]
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
            user:           crate::identity::UserId,
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
                session_key.clone(),
                "runtime_not_found",
                format!("agent runtime not found: {session_key}"),
            );
            if let Err(e) = &self
                .event_queue
                .try_push(KernelEventEnvelope::deliver(envelope))
            {
                error!(%e, "failed to push runtime-not-found error Deliver");
            }
            return;
        }

        // session_key is always resolved by this point (set by handle_user_message or synthetic).
        let session_key = msg.session_key.clone().unwrap_or(session_key);
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
            &self
                .event_queue
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

        // Build tape-backed context and append user message to tape first.
        let user_text = msg.content.as_text();
        let turn_data = self
            .process_table
            .with(&session_key, |rt| (rt.session_key, rt.turn_cancel.clone()));

        let Some((rt_session_key, turn_cancel)) = turn_data else {
            warn!(session_key = %session_key, "runtime disappeared during LLM turn setup");
            return;
        };

        let tape_name = session_key.to_string();
        if let Err(e) = &self
            .tape_service
            .append_message(
                &tape_name,
                // TODO: why ?
                serde_json::json!({
                    "role": "user",
                    "content": &user_text,
                }),
                None,
            )
            .await
        {
            warn!(%e, "failed to persist user message to tape");
        }

        // Build LLM context from tape.
        let history = {
            let msgs = self
                .tape_service
                .clone()
                .build_llm_context(&tape_name)
                .await
                .unwrap_or_default();
            if msgs.is_empty() { None } else { Some(msgs) }
        };

        // Open stream.
        let stream_handle = self.io.stream_hub().open(session_key.clone());

        // Clone what we need for the spawned task.
        let tape_service = self.tape_service.clone();
        let kernel_handle = self.handle();
        let event_queue = self.event_queue.clone();
        let stream_id = stream_handle.stream_id().clone();
        let typing_session_key = egress_session_key;
        let stream_hub_ref = Arc::clone(&self.io.stream_hub());

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
                event_queue:    event_queue.clone(),
                stream_hub:     Arc::clone(&stream_hub_ref),
                stream_id:      stream_id.clone(),
                typing_refresh: Some(typing_refresh),
                session_key:    session_key.clone(),
                msg_id:         msg_id.clone(),
                user:           user.clone(),
                completed:      false,
            };

            // Fork the tape so failed turns don't pollute the main tape.
            // On success the fork is merged back; on failure it is discarded.
            let fork_name = match tape_service.store().fork(&tape_name).await {
                Ok(name) => Some(name),
                Err(e) => {
                    tracing::warn!(tape = %tape_name, error = %e, "tape fork failed, writing directly to main tape");
                    None
                }
            };
            let effective_tape = fork_name.as_deref().unwrap_or(&tape_name);

            let turn_result = run_agent_loop(
                &kernel_handle,
                rt_session_key,
                user_text,
                history,
                &stream_handle,
                &turn_cancel,
                tape_service.clone(),
                effective_tape,
            )
            .await;

            // Merge or discard the fork depending on the turn outcome.
            if let Some(ref fork) = fork_name {
                if turn_result.is_ok() {
                    if let Err(e) = tape_service.store().merge(fork, &tape_name).await {
                        tracing::warn!(fork = %fork, tape = %tape_name, error = %e, "tape merge failed, fork entries may be lost");
                    }
                } else {
                    if let Err(e) = tape_service.store().discard(fork).await {
                        tracing::warn!(fork = %fork, error = %e, "tape discard failed, fork file may leak");
                    }
                }
            }

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
        result: std::result::Result<AgentTurnResult, String>,
        in_reply_to: MessageId,
        user: crate::identity::UserId,
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
        let mut _turn_failed = false;

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
                if let Err(e) = &self
                    .event_queue
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
                _turn_failed = err_msg != "interrupted by user";
                warn!(session_key = %session_key, error = %err_msg, "turn completed (error)");

                // Deliver error — use egress session for routing.
                let envelope = OutboundEnvelope::error(
                    in_reply_to,
                    user.clone(),
                    egress_session_key.clone(),
                    "agent_error",
                    err_msg,
                );
                if let Err(e) = &self
                    .event_queue
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
            if let Err(e) = &self.event_queue.try_push(event) {
                warn!(%e, "failed to re-inject buffered event");
            }
        }
    }
}
