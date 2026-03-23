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
//! `spawn` as the primary API for creating sessions.
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
//! Each spawned agent receives a `ProcessHandle` — a thin event pusher that
//! sends `Syscall` variants through the unified event queue.

use std::{sync::Arc, time::Duration};

use futures::FutureExt;
use jiff::Timestamp;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use tracing::{Instrument, error, info, info_span, warn};

use crate::{
    KernelError,
    agent::{
        AgentEnv, AgentManifest, AgentRegistryRef, AgentRole, AgentTurnResult, ExecutionMode,
        Priority, run_agent_loop,
    },
    event::{KernelEvent, KernelEventEnvelope},
    identity::Principal,
    io::{IOSubsystem, InboundMessage, MessageId, OutboundEnvelope, PipeRegistry, StreamId},
    kv::SharedKv,
    llm::DriverRegistryRef,
    memory::TapeService,
    notification::{BroadcastNotificationBus, NotificationBusRef},
    plan::run_plan_loop,
    queue::{EventQueueRef, ShardedEventQueueConfig, ShardedQueueRef},
    security::SecurityRef,
    session::{
        AgentRunLoopResult, Session, SessionIndexRef, SessionKey, SessionState, SessionTable,
        Signal,
    },
    syscall::SyscallDispatcher,
    tool::{DynamicToolProviderRef, ToolRegistryRef},
};

// ---------------------------------------------------------------------------
// KernelConfig
// ---------------------------------------------------------------------------

/// Context folding configuration.
#[derive(Debug, Clone, smart_default::SmartDefault, serde::Serialize, serde::Deserialize)]
pub struct ContextFoldingConfig {
    /// Whether automatic context folding is enabled.
    #[default = true]
    pub enabled:                   bool,
    /// Context pressure ratio at which auto-fold triggers (below the 0.70
    /// warning threshold).
    #[default = 0.60]
    pub fold_threshold:            f64,
    /// Minimum number of new tape entries since the last auto-fold before
    /// another fold is allowed (cooldown).
    #[default = 15]
    pub min_entries_between_folds: usize,
    /// Model to use for fold summarization.  `None` falls back to the
    /// session's current model.
    #[default(_code = "None")]
    pub fold_model:                Option<String>,
}

/// Kernel configuration.
#[derive(Debug, Clone, smart_default::SmartDefault)]
pub struct KernelConfig {
    /// Maximum number of concurrent agent processes globally.
    #[default = 16]
    pub max_concurrency:         usize,
    /// Default maximum number of children per agent.
    #[default = 8]
    pub default_child_limit:     usize,
    /// Default max LLM iterations for spawned agents.
    #[default = 12]
    pub default_max_iterations:  usize,
    /// Hard cap for one tool execution wave inside a turn.
    ///
    /// Must be greater than any individual tool's own timeout (e.g. bash
    /// defaults to 120 s) so per-tool timeouts fire first and only the
    /// offending tool is killed rather than the entire wave.
    #[default(_code = "Duration::from_secs(180)")]
    pub tool_execution_timeout:  Duration,
    /// Default per-tool timeout applied when a tool's
    /// `execution_timeout()` returns `None`.
    ///
    /// Must be strictly less than `tool_execution_timeout` so the per-tool
    /// timeout fires before the global wave timeout.
    #[default(_code = "Duration::from_secs(120)")]
    pub default_tool_timeout:    Duration,
    /// Maximum number of KV entries per agent (0 = unlimited).
    /// Applies to the agent-scoped namespace only.
    #[default = 1000]
    pub memory_quota_per_agent:  usize,
    /// Mita heartbeat interval. `None` disables the heartbeat.
    #[default(_code = "None")]
    pub mita_heartbeat_interval: Option<Duration>,
    // Event queue configuration. Controls whether the kernel uses a single
    // global queue (`num_shards = 0`) or sharded parallel processing.
    pub event_queue:             ShardedEventQueueConfig,
    /// Context folding (auto-anchor) configuration.
    pub context_folding:         ContextFoldingConfig,
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
    config:                KernelConfig,
    // -- Core subsystems (previously in KernelInner) -----------------------
    /// The global process table tracking all running agents.
    process_table:         Arc<SessionTable>,
    /// Global semaphore limiting total concurrent agent processes.
    global_semaphore:      Arc<Semaphore>,
    /// Unified security subsystem (auth + authz + approval).
    security:              SecurityRef,
    /// Agent registry for looking up named agent definitions.
    agent_registry:        AgentRegistryRef,
    /// Tape service for session message persistence.
    tape_service:          TapeService,
    /// Lightweight session metadata index (tape-centric replacement for the
    /// session CRUD subset of `SessionRepository`).
    session_index:         SessionIndexRef,
    /// Flat KV settings provider for runtime configuration.
    settings:              SettingsRef,
    /// Syscall dispatcher (owns shared_kv, pipe_registry, driver_registry,
    /// tool_registry, event_bus).
    syscall:               SyscallDispatcher,
    // -- I/O subsystem -----------------------------------------------------
    /// Bundled I/O subsystem (ingress, stream hub, delivery).
    io:                    Arc<IOSubsystem>,
    /// Unified event queue for all kernel interactions.
    event_queue:           EventQueueRef,
    /// Sharded event queue backing the kernel event loop.
    ///
    /// Always present. When `num_shards == 0` (single-queue mode), all
    /// events are routed to the global queue and processed by a single
    /// `EventProcessor`. When `num_shards > 0`, events are distributed
    /// across N shard queues for parallel processing.
    sharded_queue:         ShardedQueueRef,
    /// When this kernel was created (for uptime calculation).
    started_at:            Timestamp,
    /// Knowledge layer service for long-term memory extraction.
    knowledge:             crate::memory::knowledge::KnowledgeServiceRef,
    /// Security guard pipeline (taint tracking + pattern scanning).
    guard_pipeline:        Arc<crate::guard::pipeline::GuardPipeline>,
    /// Execution trace service for persisting turn-level traces.
    trace_service:         crate::trace::TraceService,
    /// Provider for generating the skills prompt block.
    skill_prompt_provider: crate::handle::SkillPromptProvider,
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
        knowledge: crate::memory::knowledge::KnowledgeServiceRef,
        dynamic_tool_provider: Option<DynamicToolProviderRef>,
        trace_service: crate::trace::TraceService,
        skill_prompt_provider: crate::handle::SkillPromptProvider,
    ) -> Self {
        let event_bus: NotificationBusRef = Arc::new(BroadcastNotificationBus::default());
        // Clamp default_tool_timeout so it never exceeds the global wave timeout.
        let mut config = config;
        if config.default_tool_timeout >= config.tool_execution_timeout {
            warn!(
                default_tool_timeout = ?config.default_tool_timeout,
                tool_execution_timeout = ?config.tool_execution_timeout,
                "default_tool_timeout must be less than tool_execution_timeout — clamping to {}s",
                config.tool_execution_timeout.as_secs().saturating_sub(30),
            );
            // Margin is at most 30s, but shrinks to half the wave timeout
            // when it is very small (e.g. 10s wave → 5s margin → 5s default).
            let margin = Duration::from_secs(30.min(config.tool_execution_timeout.as_secs() / 2));
            config.default_tool_timeout = config
                .tool_execution_timeout
                .checked_sub(margin)
                .unwrap_or(Duration::from_secs(60));
        }
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
        let guard_pipeline = Arc::new(crate::guard::pipeline::GuardPipeline::new(
            rara_paths::workspace_dir().clone(),
            vec![
                rara_paths::config_dir().clone(),
                rara_paths::data_dir().clone(),
                rara_paths::temp_dir().clone(),
                rara_paths::logs_dir().clone(),
                rara_paths::home_dir().join(".claude"),
                std::path::PathBuf::from("/tmp"),
            ],
        ));

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
            tape_service.clone(),
            dynamic_tool_provider,
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
            knowledge,
            guard_pipeline,
            trace_service,
            skill_prompt_provider,
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

    /// Create a `KernelHandle` for external callers.
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
            self.tape_service.clone(),
            self.trace_service.clone(),
            self.syscall.job_wheel().clone(),
            self.skill_prompt_provider.clone(),
        )
    }

    /// Access the security guard pipeline.
    pub fn guard_pipeline(&self) -> &Arc<crate::guard::pipeline::GuardPipeline> {
        &self.guard_pipeline
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
    ///
    /// Processor id=0 also runs the unified scheduler: instead of a fixed
    /// 1-second tick, it computes the next deadline from (a) the Mita
    /// heartbeat interval and (b) the earliest scheduled job, and sleeps
    /// until the soonest one fires.
    async fn run_processor(
        self: &Arc<Self>,
        id: usize,
        queue: Arc<crate::queue::ShardQueue>,
        shutdown: CancellationToken,
    ) {
        info!(processor_id = id, "event processor started");

        // Only processor id=0 runs the unified scheduler.
        let mita_interval = self.config.mita_heartbeat_interval;
        let mut next_mita = mita_interval.map(|d| tokio::time::Instant::now() + d);

        loop {
            // Compute next wake time for the unified scheduler (processor 0 only).
            let scheduler_sleep = if id == 0 {
                let next_job_instant = self
                    .syscall
                    .job_wheel()
                    .lock()
                    .unwrap_or_else(|e| {
                        warn!(error = %e, "job_wheel mutex poisoned, recovering inner data");
                        e.into_inner()
                    })
                    .next_deadline()
                    .map(|ts| {
                        let now_ts = jiff::Timestamp::now();
                        let delta = ts.duration_since(now_ts);
                        let clamped = if delta.is_negative() {
                            std::time::Duration::ZERO
                        } else {
                            delta.unsigned_abs()
                        };
                        tokio::time::Instant::now() + clamped
                    });

                // Find the earliest deadline among: mita heartbeat, next scheduled job.
                let earliest = [next_mita, next_job_instant]
                    .into_iter()
                    .flatten()
                    .min()
                    .unwrap_or(tokio::time::Instant::now() + std::time::Duration::from_secs(3600));

                tokio::time::sleep_until(earliest)
            } else {
                // Non-zero processors never wake for scheduling.
                tokio::time::sleep(std::time::Duration::from_secs(86400))
            };
            tokio::pin!(scheduler_sleep);

            tokio::select! {
                _ = queue.wait() => {
                    loop {
                        let mut events = queue.drain(32).peekable();
                        if events.peek().is_none() { break; }
                        for event in events {
                            let this = Arc::clone(self);
                            let event_type: &'static str = (&event.kind).into();
                            let span = info_span!(
                                "handle_event",
                                processor_id = id,
                                event_type,
                            );
                            tokio::spawn(async move {
                                this.handle_event(event).instrument(span).await;
                            });
                        }
                    }
                }
                _ = &mut scheduler_sleep, if id == 0 => {
                    let now = tokio::time::Instant::now();

                    // Check if Mita heartbeat is due.
                    if let Some(mita_at) = next_mita {
                        if now >= mita_at {
                            if let Err(e) = self.event_queue.try_push(KernelEventEnvelope::mita_heartbeat()) {
                                error!(%e, "failed to push MitaHeartbeat");
                            }
                            next_mita = mita_interval.map(|d| now + d);
                        }
                    }

                    // Evict expired rate-limiter entries.
                    self.io.gc_rate_limiter();

                    // Drain any expired scheduled jobs.
                    self.drain_scheduled_jobs().await;
                }
                _ = shutdown.cancelled() => {
                    info!(processor_id = id, "event processor shutting down");
                    // Persist scheduled jobs on shutdown.
                    if id == 0 {
                        self.syscall
                            .job_wheel()
                            .lock()
                            .unwrap_or_else(|e| {
                                warn!(error = %e, "job_wheel mutex poisoned during shutdown, recovering");
                                e.into_inner()
                            })
                            .persist();
                    }
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

    /// Drain expired scheduled jobs and inject them as `ScheduledTask` events.
    ///
    /// On the first call after startup, any jobs left in the in-flight ledger
    /// from a previous run are also re-fired.
    ///
    /// The mutex lock + file persist are blocking I/O, so they run on the
    /// blocking thread-pool to avoid starving the tokio runtime.
    async fn drain_scheduled_jobs(&self) {
        let wheel_ref = self.syscall.job_wheel().clone();
        let expired = tokio::task::spawn_blocking(move || {
            let now = jiff::Timestamp::now();
            let mut wheel = match wheel_ref.lock() {
                Ok(w) => w,
                Err(e) => {
                    warn!(error = %e, "failed to lock job wheel for drain");
                    return vec![];
                }
            };

            // Re-fire any in-flight jobs from a previous run (only returns
            // entries on the first call; subsequent calls return empty).
            let mut all = wheel.take_in_flight();

            let expired = wheel.drain_expired(now);
            if !expired.is_empty() {
                wheel.persist();
            }
            all.extend(expired);
            all
        })
        .await
        .unwrap_or_default();

        for job in expired {
            info!(
                job_id = %job.id,
                session = %job.session_key,
                "scheduled job fired"
            );
            let _ = self
                .event_queue
                .push(KernelEventEnvelope::scheduled_task(job));
        }
    }

    /// Dispatch a single event to its handler.
    async fn handle_event(&self, event: KernelEventEnvelope) {
        let event_type: &'static str = (&event.kind).into();
        crate::metrics::record_event_processed(event_type);

        let KernelEventEnvelope { base, kind } = event;

        match kind {
            KernelEvent::UserMessage(msg) => {
                self.handle_user_message(msg).await;
            }
            KernelEvent::GroupMessage(msg) => {
                self.handle_group_message(msg).await;
            }
            KernelEvent::CreateSession {
                manifest,
                input,
                principal,
                parent_id,
                desired_session_key,
                reply_tx,
            } => {
                // CreateSession from SessionHandle::create_child() — subagent.
                let result = self
                    .handle_spawn_agent(
                        manifest,
                        input,
                        principal,
                        parent_id,
                        None,
                        desired_session_key,
                        None,
                    )
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
                origin_endpoint,
                interrupted,
            } => {
                self.handle_turn_completed(
                    base.session_key,
                    result,
                    in_reply_to,
                    user,
                    origin_endpoint,
                    interrupted,
                )
                .await;
            }
            KernelEvent::ChildSessionDone {
                child_id,
                result,
                skip_tape_persist,
            } => {
                self.handle_child_completed(base.session_key, child_id, result, skip_tape_persist)
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
            KernelEvent::SendNotification { message } => {
                self.io.send_notification(message);
            }
            KernelEvent::ScheduledTask { job } => {
                self.handle_scheduled_task(job).await;
            }
            KernelEvent::MitaDirective { instruction } => {
                self.handle_mita_directive(base.session_key, instruction)
                    .await;
            }
            KernelEvent::MitaHeartbeat => {
                self.handle_mita_heartbeat().await;
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
        // The originating channel endpoint, stored in the session so that
        // reply routing works even for synthetic re-entry messages.
        origin_endpoint: Option<crate::io::Endpoint>,
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

        // Reject if a process with this key already exists (e.g. Mita still running).
        if self.process_table.contains(&session_key) {
            return Err(KernelError::SpawnFailed {
                message: format!("session {session_key} already exists in process table"),
            });
        }

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
            result_tx: None,
            created_files: vec![],
            metrics,
            turn_traces: vec![],
            execution_mode: None,
            // -- cancellation --
            turn_cancel: CancellationToken::new(),
            process_cancel,
            paused: false,
            origin_endpoint,
            pause_buffer: Vec::new(),
            background_tasks: Vec::new(),
            pending_tool_call_limit: None,
            activated_deferred: std::collections::HashSet::new(),
            child_semaphore: Arc::new(Semaphore::new(child_limit)),
            _parent_child_permit: None,
            _global_permit: global_permit,
        };
        self.process_table.insert(process);

        // Child session inherits parent's taint context.
        if let Some(parent_key) = parent_id {
            self.guard_pipeline
                .taint_tracker()
                .fork_session(&parent_key, &session_key);
        }

        crate::metrics::record_session_created(&manifest.name);
        crate::metrics::inc_session_active(&manifest.name);

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
        skip_tape_persist: bool,
    ) {
        info!(
            parent_id = %parent_id,
            child_id = %child_id,
            output_len = result.output.len(),
            "child result received"
        );

        use crate::agent::CHILD_RESULT_SAFETY_LIMIT_BYTES;
        let output = &result.output;
        let truncated_output = if output.len() > CHILD_RESULT_SAFETY_LIMIT_BYTES {
            let boundary = output.floor_char_boundary(CHILD_RESULT_SAFETY_LIMIT_BYTES);
            format!(
                "{}...(truncated, full result in child tape {child_id})",
                &output[..boundary],
            )
        } else {
            output.clone()
        };

        // Persist child result to parent's conversation history.
        // Fold-branch children already return results as a ToolResult — the
        // `skip_tape_persist` flag is set in cleanup_process based on the
        // child's manifest name prefix, before the child is removed from the
        // process table.
        if !skip_tape_persist {
            let child_result_text = format!(
                "[child_agent_result] child_id={child_id} iterations={} \
                 tool_calls={}\n\n{truncated_output}",
                result.iterations, result.tool_calls,
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
                        "role": "system",
                        "content": &child_result_text,
                    }),
                    None,
                )
                .await
            {
                warn!(%e, "failed to persist child result message to tape");
            }
        }

        // If this child was a background task, trigger a proactive turn on the
        // parent to deliver the result.
        let is_background = self.handle().is_background_task(&parent_id, &child_id);

        if is_background {
            // Capture trigger_message_id before removing from active list.
            let trigger_message_id = self
                .handle()
                .process_table()
                .with(&parent_id, |p| {
                    p.background_tasks
                        .iter()
                        .find(|t| t.child_key == child_id)
                        .map(|t| t.trigger_message_id.clone())
                })
                .flatten();

            // Remove from active list.
            self.handle().remove_background_task(&parent_id, &child_id);

            // TODO: AgentRunLoopResult has no explicit success/error field.
            // This heuristic is fragile — consider adding a status field to
            // AgentRunLoopResult in a follow-up.
            use crate::io::BackgroundTaskStatus;
            let status = if result.output.starts_with("error:")
                || result.output.starts_with("Error:")
                || result.iterations == 0
            {
                BackgroundTaskStatus::Failed
            } else {
                BackgroundTaskStatus::Completed
            };

            // NOTE: The child session may already be removed from the
            // process table by cleanup_process() before this event is
            // handled. In that case turn_traces is unavailable and the
            // debug trace section will be empty. The child's tape file
            // (~/.config/rara/tapes/{child_id}.jsonl) still contains the
            // full history for post-mortem analysis.
            let trace_section = if status == BackgroundTaskStatus::Failed {
                self.process_table
                    .with(&child_id, |p| {
                        p.turn_traces
                            .last()
                            .and_then(|t| serde_json::to_string_pretty(t).ok())
                    })
                    .flatten()
                    .map(|trace| format!("\n\n[Debug Trace]\n{trace}"))
                    .unwrap_or_default()
            } else {
                String::new()
            };

            let status_label = match status {
                BackgroundTaskStatus::Failed => "failed",
                BackgroundTaskStatus::Completed => "completed",
                BackgroundTaskStatus::Cancelled => "cancelled",
            };
            let trigger_info = trigger_message_id
                .as_ref()
                .map(|id| format!("trigger_message_id={id}\n"))
                .unwrap_or_default();
            let directive = format!(
                "[Background Task \
                 {status_label}]\ntask_id={child_id}\n{trigger_info}iterations={}, \
                 tool_calls={}\n\nResult:\n{truncated_output}{trace_section}\n\nProactively \
                 inform the user of the outcome. Be concise. If the task failed, explain what \
                 went wrong.",
                result.iterations, result.tool_calls,
            );

            let system_user = crate::identity::UserId("system".to_string());
            let mut msg = crate::io::InboundMessage::synthetic(directive, system_user, parent_id);
            msg.metadata.insert(
                "background_task_done".to_string(),
                serde_json::json!(child_id.to_string()),
            );
            if let Some(ref mid) = trigger_message_id {
                msg.metadata.insert(
                    "trigger_message_id".to_string(),
                    serde_json::json!(mid.to_string()),
                );
            }

            // Emit BackgroundTaskDone so clients remove the status indicator.
            self.io.stream_hub().emit_to_session(
                &parent_id,
                crate::io::StreamEvent::BackgroundTaskDone {
                    task_id: child_id.to_string(),
                    status,
                },
            );

            info!(
                parent_id = %parent_id,
                child_id = %child_id,
                status = ?status,
                "triggering proactive turn for background task result"
            );

            self.deliver_to_session(parent_id, msg).await;
        }
    }

    /// Clean up a process runtime entry.
    ///
    /// Removing the runtime from the table drops the `process_cancel` token
    /// naturally, so no explicit cancellation-token cleanup is needed.
    async fn cleanup_process(&self, session_key: SessionKey) {
        self.guard_pipeline
            .taint_tracker()
            .clear_session(&session_key);
        self.syscall
            .subscription_registry()
            .remove_session(&session_key)
            .await;
        if let Some(rt) = self.process_table.remove(session_key) {
            let manifest_name = rt.manifest.name.clone();
            let state = rt.state;
            let parent_id = rt.parent_id;

            // Clear in-flight ledger entry for scheduled job agents.
            if manifest_name == "scheduled_job" {
                if let Some(job_id_str) = rt
                    .manifest
                    .metadata
                    .get("scheduled_job_id")
                    .and_then(|v| v.as_str())
                {
                    if let Ok(uuid) = uuid::Uuid::parse_str(job_id_str) {
                        let job_id = crate::schedule::JobId(uuid);
                        self.syscall
                            .job_wheel()
                            .lock()
                            .unwrap_or_else(|e| {
                                warn!(error = %e, "job_wheel mutex poisoned, recovering");
                                e.into_inner()
                            })
                            .complete_in_flight(&job_id);
                    }
                }
            }

            crate::metrics::dec_session_active(&manifest_name);
            crate::metrics::record_session_suspended(&manifest_name, &state.to_string());

            // Notify parent if this is a child process.
            if let Some(parent_id) = parent_id {
                let result = rt.result.clone().unwrap_or(AgentRunLoopResult {
                    output:     "process ended".to_string(),
                    iterations: 0,
                    tool_calls: 0,
                    success:    false,
                });

                // Send final result through mpsc channel if spawn_child is waiting.
                if let Some(tx) = rt.result_tx {
                    let _ = tx.send(crate::io::AgentEvent::Done(result.clone())).await;
                }

                // Fold-branch children return their output inline as a
                // ToolResult, so we tell handle_child_completed to skip the
                // tape append (otherwise the same content appears twice).
                let skip_tape =
                    manifest_name.starts_with(crate::tool::fold_branch::FOLD_BRANCH_NAME_PREFIX);
                let event = KernelEventEnvelope::child_session_done(
                    parent_id,
                    session_key,
                    result,
                    skip_tape,
                );
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
    /// - `None` — first message from this chat. Creates a new `SessionEntry`
    ///   + `ChannelBinding` so future messages are routed automatically, then
    ///   patches `msg.session_key = Some(new_key)`.
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
    /// 3. **Role-based default** (fallback): lookup AgentRegistry by user role,
    ///    spawn a new agent process keyed by `session_id`.
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
        //   2. Writing a ChannelBinding so subsequent messages from the same chat are
        //      routed to this session automatically.
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

        let origin_endpoint = msg.origin_endpoint();

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
                    )
                    .with_origin(origin_endpoint.clone());
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
                    )
                    .with_origin(origin_endpoint.clone());
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
                origin_endpoint.clone(),
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

    /// Handle a scheduled task event.
    ///
    /// Scheduled tasks are system-initiated events — they reuse the same
    /// session/agent pipeline as `UserMessage` but enter through a dedicated
    /// code path so that observability and notifications distinguish them from
    /// user-initiated interactions.
    #[tracing::instrument(
        skip(self, job),
        fields(
            job_id = %job.id,
            session_key = %job.session_key,
        )
    )]
    async fn handle_scheduled_task(&self, job: crate::schedule::JobEntry) {
        let session_key = job.session_key;
        let job_id = job.id;
        let trigger_summary = job.trigger.summary();

        info!(
            job_id = %job_id,
            session_key = %session_key,
            "scheduled task fired"
        );

        // Build a dedicated ScheduledJobAgent manifest with job context
        // baked into the system prompt.
        let manifest = AgentManifest {
            name:                   "scheduled_job".to_string(),
            role:                   AgentRole::Worker,
            description:            "Executes a scheduled task and summarizes the result"
                .to_string(),
            model:                  None,
            system_prompt:          {
                let tags_str = if job.tags.is_empty() {
                    String::new()
                } else {
                    format!("\nRouting tags: {}\n", job.tags.join(", "))
                };
                format!(
                    "You are a scheduled task executor.\n\n## Task\nJob ID: {job_id}\nSchedule: \
                     {trigger_summary}\nTask: {message}\n{tags_str}\n## Instructions\n1. Execute \
                     the task described above using available tools.\n2. After completion, \
                     provide a brief summary of what you did and the outcome.\n\n## After \
                     Completion\nWhen you finish the task, call the `kernel` tool with:\n- \
                     action: \"publish_report\"\n- report: {{ \"task_id\": \"<uuid>\", \
                     \"task_type\": \"<type>\", \"tags\": [<routing tags>], \"status\": \
                     \"completed\", \"summary\": \"<one-line summary>\", \"result\": \
                     {{<structured result>}} }}\n\nAlternatively, use action: \"publish\" with \
                     event_type: \"scheduled_task_done\" and payload: {{ \"message\": \
                     \"<summary>\" }}\n",
                    message = job.message,
                )
            },
            soul_prompt:            None,
            provider_hint:          None,
            max_iterations:         Some(15),
            tools:                  vec![],
            excluded_tools:         vec![],
            max_children:           Some(0),
            max_context_tokens:     None,
            priority:               Priority::default(),
            metadata:               serde_json::json!({
                "scheduled_job_id": job_id.to_string(),
            }),
            sandbox:                None,
            default_execution_mode: None,
            tool_call_limit:        None,
            worker_timeout_secs:    None,
        };

        // 3. Spawn the agent.
        let principal = crate::identity::Principal::lookup(job.principal.user_id.0.clone());
        match self
            .handle_spawn_agent(
                manifest,
                job.message.clone(),
                principal,
                None, // no parent
                None, // no resume
                None, // independent session, don't pollute the original tape
                None, // no origin endpoint
            )
            .await
        {
            Ok(spawned_key) => {
                info!(
                    job_id = %job_id,
                    session_key = %spawned_key,
                    "scheduled job agent spawned"
                );
                // The agent will send a notification via PublishEvent
                // (SendNotification) when it completes.
            }
            Err(e) => {
                error!(
                    job_id = %job_id,
                    error = %e,
                    "failed to spawn scheduled job agent"
                );
            }
        }
    }

    /// Handle a group-chat message where the bot was not directly mentioned.
    ///
    /// 1. Resolve session (reusing the same logic as `handle_user_message`).
    /// 2. Record the message to the session tape (with `[DisplayName]: text`
    ///    format).
    /// 3. Run a lightweight LLM judgment via `proactive::should_reply()`.
    /// 4. If approved, push a `UserMessage` event to go through the normal
    ///    agent turn.
    /// 5. If skipped, end here — the message is already persisted in the tape.
    #[tracing::instrument(
        skip(self, msg),
        fields(
            session_id = ?msg.session_key,
            user_id = %msg.user.0,
            channel = ?msg.source.channel_type,
        )
    )]
    async fn handle_group_message(&self, msg: InboundMessage) {
        // -- Session resolution (same as handle_user_message) ------------------
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
                        error!(error = %e, "group: failed to create session for new binding");
                        return;
                    }
                };
                if let Some(chat_id) = msg.source.platform_chat_id.as_deref() {
                    let binding = crate::session::ChannelBinding {
                        channel_type: msg.source.channel_type.to_string(),
                        chat_id:      chat_id.to_string(),
                        session_key:  session.key.clone(),
                        created_at:   now,
                        updated_at:   now,
                    };
                    if let Err(e) = self.io.session_index().bind_channel(&binding).await {
                        warn!(error = %e, "group: failed to bind channel to new session");
                    }
                }
                session.key
            }
        };

        let mut msg = msg;
        msg.session_key = Some(session_id.clone());

        self.io.register_stateless_endpoint(&msg);

        // -- Record message to tape -------------------------------------------
        let tape_name = session_id.to_string();
        let user_text = msg.content.as_text();
        let display_name = msg
            .metadata
            .get("telegram_display_name")
            .and_then(|v| v.as_str())
            .unwrap_or("User");
        let tape_payload = serde_json::json!({
            "role": "user",
            "content": format!("[{display_name}]: {user_text}"),
        });
        if let Err(e) = &self
            .tape_service
            .append_message(
                &tape_name,
                tape_payload,
                Some(serde_json::json!({"rara_message_id": msg.id.to_string()})),
            )
            .await
        {
            warn!(%e, "group: failed to persist message to tape");
        }

        // -- Proactive reply judgment -----------------------------------------
        let sender_display_name = msg
            .metadata
            .get("telegram_display_name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_owned());

        let judgment_result =
            match self
                .syscall
                .driver_registry()
                .resolve("__proactive_judgment__", None, None)
            {
                Ok((driver, model)) => {
                    crate::proactive::should_reply(
                        &driver,
                        &model,
                        &self.tape_service,
                        &tape_name,
                        &user_text,
                        sender_display_name.as_deref(),
                    )
                    .await
                }
                Err(e) => {
                    warn!(error = %e, "group: failed to resolve driver, skipping");
                    crate::proactive::ProactiveJudgment::ShouldSkip {
                        reason: "driver resolution failed".into(),
                    }
                }
            };

        if matches!(
            judgment_result,
            crate::proactive::ProactiveJudgment::ShouldSkip { .. }
        ) {
            info!(
                session_key = %session_id,
                "group: proactive judgment = skip, message recorded to tape"
            );
            return;
        }

        // -- Judgment approved: promote to UserMessage -------------------------
        info!(
            session_key = %session_id,
            "group: proactive judgment approved reply, promoting to UserMessage"
        );
        if let Err(e) = self
            .event_queue
            .try_push(KernelEventEnvelope::user_message(msg))
        {
            error!(%e, "group: failed to push promoted UserMessage");
        }
    }

    /// Handle a Mita directive — deliver an instruction to a target session.
    /// The directive is persisted as an Event entry in the session tape.
    async fn handle_mita_directive(&self, session_key: SessionKey, instruction: String) {
        // Check target session exists
        if !self.process_table.contains(&session_key) {
            warn!(session_key = %session_key, "MitaDirective target session not found, ignoring");
            return;
        }

        // Construct a synthetic message with a directive prefix so Rara can
        // distinguish it from user messages.
        let directive_text = format!(
            "[Proactive Instruction from Mita]\nThe following is an internally-generated \
             directive based on cross-session analysis. Act on it naturally as if you decided to \
             reach out to the user yourself. Do NOT mention Mita or reveal that this is an \
             automated instruction.\n\nInstruction: {instruction}"
        );

        let system_user = crate::identity::UserId("system".to_string());
        let mut msg = InboundMessage::synthetic(directive_text, system_user, session_key);

        // Set metadata flag so start_llm_turn persists as Event (not Message).
        msg.metadata
            .insert("mita_directive".to_string(), serde_json::json!(true));

        self.deliver_to_session(session_key, msg).await;
    }

    /// Handle a periodic Mita heartbeat.
    ///
    /// Ensures the long-lived Mita session exists (spawning it if necessary)
    /// and delivers a heartbeat message to it.
    async fn handle_mita_heartbeat(&self) {
        let session_key = SessionKey::deterministic("mita");

        if !self.process_table.contains(&session_key) {
            info!("Mita session not found, bootstrapping");

            // Ensure tape exists with a bootstrap anchor.
            let tape_name = session_key.to_string();
            if let Err(e) = self.tape_service.ensure_bootstrap_anchor(&tape_name).await {
                error!(error = %e, "failed to bootstrap Mita tape");
                return;
            }

            let manifest = match self.agent_registry.get("mita") {
                Some(m) => m,
                None => {
                    warn!("Mita agent manifest not found in registry, skipping heartbeat");
                    return;
                }
            };

            let principal = Principal::lookup("system");

            match self
                .handle_spawn_agent(
                    manifest,
                    "Mita session initialized. Awaiting heartbeat instructions.".to_string(),
                    principal,
                    None,
                    None,
                    Some(session_key),
                    None,
                )
                .await
            {
                Ok(key) => info!(session_key = %key, "Mita long-lived session spawned"),
                Err(e) => error!(error = %e, "failed to spawn Mita session"),
            }
            // Session just spawned with initial message, skip this heartbeat
            return;
        }

        // Deliver heartbeat message to the existing Mita session.
        let msg = InboundMessage::synthetic(
            "Heartbeat triggered. Analyze active sessions and determine if any proactive actions \
             are needed. Review your previous tape entries to avoid repeating recent actions."
                .to_string(),
            crate::identity::UserId("system".to_string()),
            session_key,
        );

        self.deliver_to_session(session_key, msg).await;
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

    /// Start an LLM turn for a session, spawning the work as a background
    /// async task.
    ///
    /// Resolve the effective execution mode for a session.
    ///
    /// Priority order:
    /// 1. Session-level override (set via `/msg_version`) — highest priority.
    /// 2. Agent manifest `default_execution_mode` — agent-level default.
    /// 3. Fall back to `ExecutionMode::default()` (plan-execute v2).
    fn resolve_execution_mode(&self, session_key: &SessionKey) -> ExecutionMode {
        // Check session-level override first.
        if let Some(mode) = self
            .process_table
            .with(session_key, |p| p.execution_mode)
            .flatten()
        {
            return mode;
        }

        // Check agent manifest default.
        if let Some(mode) = self
            .process_table
            .with(session_key, |p| p.manifest.default_execution_mode)
            .flatten()
        {
            return mode;
        }

        // Default: reactive (v1), consistent with ExecutionMode::default().
        ExecutionMode::default()
    }

    /// # What this method does
    ///
    /// This is the heart of the agent execution pipeline. It takes an inbound
    /// user message and orchestrates the full lifecycle of one "turn" — from
    /// recording the message, through LLM reasoning, to delivering the reply.
    ///
    /// # Why it exists as a separate method
    ///
    /// The kernel event loop is single-threaded per shard. LLM calls are slow
    /// (seconds to minutes), so the actual work is spawned as a background
    /// tokio task. This method sets up all the context the task needs, spawns
    /// it, and returns immediately — freeing the event loop to process other
    /// events.
    ///
    /// # Lifecycle phases (in order)
    ///
    /// 1. **Validation** — verify the process still exists in the table.
    /// 2. **State transition** — mark the process as `Active` so new messages
    ///    are buffered (not dropped) while the turn runs.
    /// 3. **UX feedback** — send an initial typing indicator so the user knows
    ///    the agent is working.
    /// 4. **Tape persistence** — append the user message to the session tape
    ///    (JSONL on disk) before any LLM call, ensuring no message is lost.
    /// 5. **Context assembly** — build the LLM conversation history from the
    ///    tape, injecting the user's cross-session memory (user tape).
    /// 6. **Stream setup** — open a streaming channel for real-time token
    ///    delivery to the client.
    /// 7. **Spawn background task** — the task runs `run_agent_loop` (which may
    ///    involve multiple LLM calls and tool executions), then pushes a
    ///    `TurnCompleted` event back into the queue.
    ///
    /// # Failure safety
    ///
    /// A `TurnGuard` (RAII) ensures that `TurnCompleted` is always pushed and
    /// the stream is always closed, even if the spawned task panics or is
    /// cancelled. Without this, the process would be stuck in `Active` state
    /// forever — no new messages could be processed.
    ///
    /// Tape forking provides transactional semantics: the agent writes to a
    /// fork during its turn. On success the fork is merged back; on failure
    /// it is discarded, keeping the main tape clean.
    #[tracing::instrument(skip_all, fields(session_key = %session_key, msg_session_key = ?msg.session_key))]
    async fn start_llm_turn(&self, session_key: SessionKey, msg: InboundMessage) {
        // -- TurnGuard: RAII safety net ------------------------------------------
        //
        // Why: The spawned task can panic, be cancelled (user interrupt), or hit
        // an unexpected error at any point. Without cleanup the process stays in
        // `Active` state forever (deadlocked). TurnGuard::drop() ensures:
        //   - the typing indicator task is aborted
        //   - the response stream is closed
        //   - a TurnCompleted(Err) event is pushed so the process returns to Ready
        struct TurnGuard {
            event_queue:     EventQueueRef,
            stream_hub:      Arc<crate::io::StreamHub>,
            stream_id:       StreamId,
            typing_refresh:  Option<tokio::task::JoinHandle<()>>,
            session_key:     SessionKey,
            msg_id:          MessageId,
            user:            crate::identity::UserId,
            origin_endpoint: Option<crate::io::Endpoint>,
            completed:       bool,
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
                        self.origin_endpoint.clone(),
                        false,
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

        // -- Phase 1: Validation -------------------------------------------------
        //
        // Why: Between the time a message is queued and this method runs, the
        // process may have been killed or reaped. Delivering to a missing
        // process would silently lose the message, so we send an error back
        // to the user instead.
        if !self.process_table.contains(&session_key) {
            warn!(session_key = %session_key, "runtime not found for LLM turn");
            // Send error back to the user instead of silently dropping.
            let envelope = OutboundEnvelope::error(
                msg.id.clone(),
                msg.user.clone(),
                session_key.clone(),
                "runtime_not_found",
                format!("agent runtime not found: {session_key}"),
            )
            .with_origin(msg.origin_endpoint());
            if let Err(e) = &self
                .event_queue
                .try_push(KernelEventEnvelope::deliver(envelope))
            {
                error!(%e, "failed to push runtime-not-found error Deliver");
            }
            return;
        }

        // -- Phase 1b: Kernel commands (short-circuit) ----------------------------
        //
        // Kernel-level commands are handled here before the LLM turn starts.
        // They modify session state and return a text response directly,
        // without invoking the agent loop.
        let raw_text = msg.content.as_text();
        if raw_text.starts_with("/msg_version") {
            let arg = raw_text.strip_prefix("/msg_version").unwrap().trim();
            let user = msg.user.clone();
            let msg_id = msg.id.clone();
            let origin_endpoint = msg.origin_endpoint().or_else(|| {
                self.process_table
                    .with(&session_key, |p| p.origin_endpoint.clone())
                    .flatten()
            });

            let response_text = if arg.is_empty() {
                // Query current mode.
                let current = self.resolve_execution_mode(&session_key);
                format!(
                    "Current execution mode: {} (v{})",
                    current,
                    current.version()
                )
            } else if let Some(mode) = ExecutionMode::from_version_str(arg) {
                // Set session execution mode.
                self.process_table.with_mut(&session_key, |p| {
                    p.execution_mode = Some(mode);
                });
                format!("Execution mode set to {} (v{})", mode, mode.version())
            } else {
                format!(
                    "Invalid version: {arg}. Use /msg_version 1 (reactive) or /msg_version 2 \
                     (plan)"
                )
            };

            // Deliver the response directly — no LLM turn needed.
            if origin_endpoint.is_some() {
                let envelope = OutboundEnvelope::reply(
                    msg_id,
                    user,
                    session_key,
                    crate::channel::types::MessageContent::Text(response_text),
                    vec![],
                )
                .with_origin(origin_endpoint);
                if let Err(e) = self
                    .event_queue
                    .try_push(KernelEventEnvelope::deliver(envelope))
                {
                    error!(%e, "failed to push /msg_version reply");
                }
            }
            return;
        }

        // -- Phase 2: State transition -------------------------------------------
        //
        // Why: Moving to `Active` tells the process table that this session is
        // busy. Any new messages arriving for this session while it's Active
        // are buffered in `pause_buffer` (see `deliver_to_session`) rather than
        // starting a concurrent turn — the kernel enforces one turn at a time
        // per session.
        let user = msg.user.clone();
        let msg_id = msg.id.clone();
        // Prefer the message's own origin endpoint (set for real platform
        // messages). Fall back to the session's stored origin endpoint so
        // that synthetic re-entry messages (from handle_spawn_agent or Mita
        // dispatch) still route replies to the correct channel.
        let msg_origin = msg.origin_endpoint();
        let origin_endpoint = msg_origin.clone().or_else(|| {
            self.process_table
                .with(&session_key, |p| p.origin_endpoint.clone())
                .flatten()
        });

        // Update the session's stored origin endpoint when a real platform
        // message provides one, so that subsequent synthetic messages can
        // inherit the correct routing target.
        if msg_origin.is_some() {
            self.process_table.with_mut(&session_key, |p| {
                p.origin_endpoint = msg_origin;
            });
        }

        let _ = self
            .process_table
            .set_state(session_key, SessionState::Active);

        // -- Phase 3: UX feedback ------------------------------------------------
        //
        // Why: LLM calls can take seconds. Sending a typing indicator
        // immediately gives the user visual feedback that their message was
        // received and the agent is working. On Telegram this shows the
        // "typing..." bubble.
        let egress_session_key = session_key;
        // Only send typing indicator when there's an origin endpoint to
        // deliver to. Headless agents (scheduled tasks, subagents) have no
        // user-facing channel.
        if origin_endpoint.is_some() {
            let _ = &self.event_queue.try_push(KernelEventEnvelope::deliver(
                OutboundEnvelope::progress(
                    msg_id.clone(),
                    user.clone(),
                    egress_session_key.clone(),
                    crate::io::stages::THINKING,
                    None,
                )
                .with_origin(origin_endpoint.clone()),
            ));
        }

        if let Some(metrics) = self.process_table.get_metrics(&session_key) {
            metrics.record_message();
        }

        // -- Phase 4: Tape persistence -------------------------------------------
        //
        // Why: The user message is appended to the JSONL tape BEFORE any LLM
        // call. This is a write-ahead pattern — if the process crashes during
        // the LLM turn, the message is already durably stored and won't be
        // lost. The tape is the source of truth for conversation history.
        let user_text = msg.content.as_text();

        // -- Phase 4a: Populate session preview (first message only) --------
        {
            let session_index = self.io.session_index();
            if let Ok(Some(mut entry)) = session_index.get_session(&session_key).await {
                if entry.preview.is_none() && !user_text.is_empty() {
                    let preview: String = user_text.chars().take(50).collect();
                    entry.preview = Some(preview);
                    entry.updated_at = chrono::Utc::now();
                    if let Err(e) = session_index.update_session(&entry).await {
                        tracing::warn!(%e, "failed to write session preview");
                    }
                }
            }
        }

        let turn_data = self
            .process_table
            .with(&session_key, |rt| (rt.session_key, rt.turn_cancel.clone()));

        let Some((rt_session_key, turn_cancel)) = turn_data else {
            warn!(session_key = %session_key, "runtime disappeared during LLM turn setup");
            return;
        };

        let tape_name = session_key.to_string();

        // -- Phase 5: Persist user message to tape --------------------------------
        //
        // The user message is written to tape BEFORE the agent loop starts.
        // `run_agent_loop` rebuilds LLM messages from tape each iteration
        // (tape-driven), so the user message will be included automatically.
        //
        // Persist to tape: Mita directives go as Event entries (recorded but
        // excluded from LLM context by default_tape_context), regular messages
        // go as Message entries.
        let is_mita_directive = msg
            .metadata
            .get("mita_directive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if is_mita_directive {
            if let Err(e) = &self
                .tape_service
                .append_event(
                    &tape_name,
                    "mita-directive",
                    serde_json::json!({
                        "instruction": &user_text,
                    }),
                )
                .await
            {
                warn!(%e, "failed to persist Mita directive to tape");
            }
        } else {
            let tape_payload = serde_json::json!({
                "role": "user",
                "content": &user_text,
            });
            if let Err(e) = &self
                .tape_service
                .append_message(
                    &tape_name,
                    tape_payload,
                    Some(serde_json::json!({"rara_message_id": msg.id.to_string()})),
                )
                .await
            {
                warn!(%e, "failed to persist user message to tape");
            }
        }

        // -- Phase 5b: Persist image metadata to session -------------------------
        //
        // If the inbound message carries image paths (from Telegram adapter),
        // extract the image ID from the filename and merge it into the
        // SessionEntry.metadata.images map so tools can discover uploaded images.
        if let (Some(original), Some(compressed)) = (
            msg.metadata
                .get("image_original_path")
                .and_then(|v| v.as_str()),
            msg.metadata
                .get("image_compressed_path")
                .and_then(|v| v.as_str()),
        ) {
            // Extract image ID from filename: "photo_{uuid}.jpg" → uuid part
            fn extract_image_id(path: &str) -> Option<String> {
                let stem = std::path::Path::new(path).file_stem()?.to_str()?;
                let id = stem.strip_prefix("photo_")?;
                let id = id.strip_suffix("_compressed").unwrap_or(id);
                Some(id.to_owned())
            }

            if let Some(image_id) = extract_image_id(original) {
                match self.session_index.get_session(&session_key).await {
                    Ok(Some(mut entry)) => {
                        let mut meta = entry
                            .metadata
                            .clone()
                            .unwrap_or_else(|| serde_json::json!({}));
                        let images = meta
                            .as_object_mut()
                            .unwrap()
                            .entry("images")
                            .or_insert_with(|| serde_json::json!({}));
                        if let Some(images_map) = images.as_object_mut() {
                            images_map.insert(
                                image_id.clone(),
                                serde_json::json!({
                                    "original_path": original,
                                    "compressed_path": compressed,
                                }),
                            );
                        }
                        entry.metadata = Some(meta);
                        match self.session_index.update_session(&entry).await {
                            Ok(_) => {
                                info!(
                                    session_key = %session_key,
                                    image_id = %image_id,
                                    "persisted image metadata to session"
                                );
                            }
                            Err(e) => {
                                warn!(
                                    session_key = %session_key,
                                    error = %e,
                                    "failed to update session with image metadata"
                                );
                            }
                        }
                    }
                    Ok(None) => {
                        warn!(session_key = %session_key, "session not found for image metadata update");
                    }
                    Err(e) => {
                        warn!(session_key = %session_key, error = %e, "failed to get session for image metadata");
                    }
                }
            }
        }

        // -- Phase 6: Stream setup -----------------------------------------------
        //
        // Why: The stream allows real-time token-by-token delivery to the
        // client (e.g. SSE for web, chunked updates for Telegram). The
        // stream_handle is passed into the agent loop so it can emit tokens
        // as they arrive from the LLM.
        let stream_handle = self.io.stream_hub().open(session_key.clone());

        // Clone dependencies for the spawned task. The task outlives this
        // method call, so it needs owned copies of everything it uses.
        let tape_service = self.tape_service.clone();
        let kernel_handle = self.handle();
        let event_queue = self.event_queue.clone();
        let stream_id = stream_handle.stream_id().clone();
        let typing_session_key = egress_session_key;
        let stream_hub_ref = Arc::clone(self.io.stream_hub());
        let guard_pipeline = self.guard_pipeline.clone();
        let notification_bus = self.syscall.event_bus().clone();

        let milestone_tx = self
            .process_table
            .with(&session_key, |p| p.result_tx.clone())
            .flatten();
        let parent_span = tracing::Span::current();

        // -- Phase 6b: Resolve execution mode ------------------------------------
        //
        // Determine whether this turn uses reactive (v1) or plan-execute (v2).
        // Priority:
        //   1. `/plan <goal>` prefix in user text → v2 (this turn only)
        //   2. Session execution_mode override (set via `/msg_version`) → persistent
        //   3. AgentManifest default_execution_mode → agent-level default
        //   4. Otherwise → v1 (reactive)
        let use_plan_executor = if user_text.starts_with("/plan ") {
            true
        } else {
            self.resolve_execution_mode(&session_key) == ExecutionMode::Plan
        };

        // -- Phase 7: Spawn background task --------------------------------------
        //
        // Why: LLM turns are slow (seconds to minutes) and may involve multiple
        // tool calls. Running them on the event loop would block all other
        // event processing. Instead we spawn a detached tokio task that:
        //
        //   a) Refreshes the typing indicator every 4s (Telegram expires it
        //      after ~5s).
        //   b) Forks the tape for transactional safety — if the turn fails,
        //      the fork is discarded and the main tape stays clean.
        //   c) Runs the full agent loop (LLM calls + tool executions).
        //   d) Merges or discards the tape fork based on outcome.
        //   e) Pushes TurnCompleted back into the event queue so the kernel
        //      can deliver the reply and transition the process back to Ready.
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

            // -- 7a: Typing refresh loop --
            // Why: Telegram's typing indicator expires after ~5s. Re-sending it
            // every 4s keeps the "typing..." bubble visible for the entire
            // duration of the LLM turn.
            // Headless agents (no origin_endpoint) skip the refresh loop
            // entirely — there's no user-facing channel to send indicators to.
            let typing_refresh = {
                let eq = event_queue.clone();
                let sid = typing_session_key.clone();
                let usr = user.clone();
                let mid = msg_id.clone();
                let oe = origin_endpoint.clone();
                let cancel = turn_cancel.clone();
                tokio::spawn(async move {
                    let oe = match oe {
                        Some(ep) => Some(ep),
                        None => return, // no endpoint — nothing to refresh
                    };
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(4));
                    interval.tick().await; // skip the immediate first tick
                    loop {
                        tokio::select! {
                            _ = cancel.cancelled() => break,
                            _ = interval.tick() => {
                                let _ = eq.try_push(KernelEventEnvelope::deliver(
                                    OutboundEnvelope::progress(
                                        mid.clone(),
                                        usr.clone(),
                                        sid.clone(),
                                        crate::io::stages::THINKING,
                                        None,
                                    )
                                    .with_origin(oe.clone()),
                                ));
                            }
                        }
                    }
                })
            };

            // -- 7b: Arm the TurnGuard --
            let mut turn_guard = TurnGuard {
                event_queue:     event_queue.clone(),
                stream_hub:      Arc::clone(&stream_hub_ref),
                stream_id:       stream_id.clone(),
                typing_refresh:  Some(typing_refresh),
                session_key:     session_key.clone(),
                msg_id:          msg_id.clone(),
                user:            user.clone(),
                origin_endpoint: origin_endpoint.clone(),
                completed:       false,
            };

            // Wrap the core turn work in catch_unwind so that if a panic
            // occurs, we capture the actual panic message instead of losing
            // it in TurnGuard::drop's generic error.
            let panic_result = std::panic::AssertUnwindSafe(async {
                // -- 7c: Tape fork (transactional safety) --
                // Why: If the LLM turn fails midway (e.g. tool error, timeout),
                // we don't want partial assistant messages polluting the tape.
                // Forking creates a copy; on success we merge it back, on failure
                // we discard it — the main tape stays clean either way.
                let fork_name = match tape_service.store().fork(&tape_name, None).await {
                    Ok(name) => Some(name),
                    Err(e) => {
                        tracing::warn!(tape = %tape_name, error = %e, "tape fork failed, writing directly to main tape");
                        None
                    }
                };
                let effective_tape = fork_name.as_deref().unwrap_or(&tape_name);

                // -- 7d: Run the agent loop (v1/v2 routing) --
                // Why: This is the core LLM reasoning loop. It may make multiple
                // LLM calls interspersed with tool executions (bash, file I/O,
                // etc.). The ToolContext carries the authenticated user_id so
                // tools can access it without relying on LLM-supplied identity.
                let tool_context = crate::tool::ToolContext {
                    user_id: user.0.clone(),
                    session_key: session_key.clone(),
                    origin_endpoint: origin_endpoint.clone(),
                    event_queue: event_queue.clone(),
                    rara_message_id: msg_id.clone(),
                    context_window_tokens: 0,
                    tool_registry: None, // set later in agent loop with live registry
                    stream_handle: None, // set per-tool-call in agent loop
                    tool_call_id: None,  // set per-tool-call in agent loop
                };

                // Route to v1 (reactive) or v2 (plan-execute) based on the
                // resolved execution mode (set in Phase 6b). For `/plan <goal>`,
                // strip the command prefix and pass the goal text.
                let effective_user_text = if use_plan_executor {
                    user_text
                        .strip_prefix("/plan ")
                        .unwrap_or(&user_text)
                        .to_string()
                } else {
                    user_text
                };

                // Use the inbound message ID as the turn's rara_message_id
                // for end-to-end correlation.
                let rara_message_id = msg_id.clone();

                let turn_result = if use_plan_executor {
                    run_plan_loop(
                        &kernel_handle,
                        rt_session_key,
                        effective_user_text,
                        &stream_handle,
                        &turn_cancel,
                        tape_service.clone(),
                        effective_tape,
                        tool_context,
                        milestone_tx,
                        guard_pipeline,
                        notification_bus,
                        rara_message_id,
                    )
                    .await
                } else {
                    run_agent_loop(
                        &kernel_handle,
                        rt_session_key,
                        effective_user_text,
                        &stream_handle,
                        &turn_cancel,
                        tape_service.clone(),
                        effective_tape,
                        tool_context,
                        milestone_tx,
                        guard_pipeline,
                        notification_bus,
                        rara_message_id,
                    )
                    .await
                };

                // -- 7e: Tape fork resolution --
                if let Some(ref fork) = fork_name {
                    if turn_result.is_ok() {
                        if let Err(e) = tape_service.store().merge(fork, &tape_name).await {
                            tracing::warn!(fork = %fork, tape = %tape_name, error = %e, "tape merge failed, fork entries may be lost");
                        }
                    } else if let Err(e) = tape_service.store().discard(fork).await {
                        tracing::warn!(fork = %fork, error = %e, "tape discard failed, fork file may leak");
                    }
                }

                // -- 7f: Cleanup --
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
                        rara_message_id: result.trace.rara_message_id.to_string(),
                    });
                }

                // Close stream.
                stream_hub_ref.close(&stream_id);

                // -- 7g: Signal completion --
                // Why: The kernel event loop needs to know the turn is done so it
                // can deliver the reply, update metrics, and transition the process
                // back to Ready state. KernelError -> String conversion happens
                // here because KernelEvent requires Clone but KernelError doesn't.
                let interrupted = matches!(
                    turn_result,
                    Err(KernelError::Interrupted)
                );
                let result = turn_result.map_err(|e| e.to_string());
                let event = KernelEventEnvelope::turn_completed(
                    session_key,
                    result,
                    msg_id,
                    user,
                    origin_endpoint,
                    interrupted,
                );
                if let Err(e) = event_queue.try_push(event) {
                    error!(%e, session_key = %session_key, "failed to push TurnCompleted");
                }
            })
            .catch_unwind()
            .await;

            match panic_result {
                Ok(()) => {
                    // Normal completion — TurnCompleted was pushed inside the block.
                    turn_guard.completed = true;
                }
                Err(panic_payload) => {
                    // Extract the panic message from the payload.
                    let panic_msg = if let Some(s) = panic_payload.downcast_ref::<&str>() {
                        (*s).to_string()
                    } else if let Some(s) = panic_payload.downcast_ref::<String>() {
                        s.clone()
                    } else {
                        "unknown panic (non-string payload)".to_string()
                    };
                    error!(
                        session_key = %turn_guard.session_key,
                        panic_message = %panic_msg,
                        "turn task panicked"
                    );

                    // Abort typing refresh if still running.
                    if let Some(handle) = turn_guard.typing_refresh.take() {
                        handle.abort();
                    }
                    // Close stream so the forwarder stops.
                    turn_guard.stream_hub.close(&turn_guard.stream_id);
                    // Push TurnCompleted(Err) with the real panic message.
                    let event = KernelEventEnvelope::turn_completed(
                        turn_guard.session_key.clone(),
                        Err(format!("turn task panicked: {panic_msg}")),
                        turn_guard.msg_id.clone(),
                        turn_guard.user.clone(),
                        turn_guard.origin_endpoint.clone(),
                        false,
                    );
                    if let Err(e) = turn_guard.event_queue.try_push(event) {
                        error!(%e, "failed to push panic TurnCompleted");
                    }
                    // Disarm the guard — we handled cleanup manually.
                    turn_guard.completed = true;
                }
            }
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
        origin_endpoint: Option<crate::io::Endpoint>,
        interrupted: bool,
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
                    success:    turn.trace.success,
                };
                let _ = self.process_table.set_result(session_key, result.clone());

                // Store turn trace for observability — before delivery so the
                // trace is recorded even if the envelope push fails.
                self.process_table
                    .push_turn_trace(session_key, turn.trace.clone());

                // Push Deliver event for the reply — use egress session for routing.
                // When origin_endpoint is None (e.g. scheduled tasks, subagents),
                // skip delivery to avoid broadcasting to all user endpoints.
                // These agents communicate results via other channels
                // (PublishEvent/SendNotification or result_tx).
                if origin_endpoint.is_some() {
                    let envelope = OutboundEnvelope::reply(
                        in_reply_to,
                        user.clone(),
                        egress_session_key.clone(),
                        crate::channel::types::MessageContent::Text(turn.text),
                        vec![],
                    )
                    .with_origin(origin_endpoint.clone());

                    if let Err(e) = &self
                        .event_queue
                        .try_push(KernelEventEnvelope::deliver(envelope))
                    {
                        error!(%e, "failed to push Deliver event");
                    }
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
                _turn_failed = !interrupted;
                if _turn_failed {
                    error!(session_key = %session_key, error = %err_msg, "turn failed");
                } else {
                    info!(session_key = %session_key, "turn interrupted by user");
                }

                // Deliver error — use egress session for routing.
                // Skip for user-initiated interrupts (the /stop handler
                // already sent a confirmation message) and when
                // origin_endpoint is None (same rationale as reply delivery
                // above).
                if _turn_failed && origin_endpoint.is_some() {
                    let envelope = OutboundEnvelope::error(
                        in_reply_to,
                        user.clone(),
                        egress_session_key.clone(),
                        "agent_error",
                        err_msg.clone(),
                    )
                    .with_origin(origin_endpoint.clone());
                    if let Err(e) = &self
                        .event_queue
                        .try_push(KernelEventEnvelope::deliver(envelope))
                    {
                        error!(%e, "failed to push error Deliver event");
                    }
                }
            }
        }

        // Child agents spawned via spawn_child have a result_tx — once their
        // first turn completes we send the result and clean up the process.
        // Regular (long-lived) sessions transition to Ready instead.
        let has_result_tx = self
            .process_table
            .with(&session_key, |p| p.result_tx.is_some())
            .unwrap_or(false);

        if has_result_tx {
            self.cleanup_process(session_key).await;
            return;
        }

        // Session-centric model: sessions are long-lived. After each turn,
        // the session transitions to Ready (idle) instead of a terminal state.
        // The next user message will be routed to the same session via Path 2.

        // -- Knowledge extraction (async, best-effort) -------------------------
        // After each successful turn, spawn an async task to extract long-term
        // memories from the conversation tape. Failures are logged but never
        // block the main event loop.
        if !_turn_failed {
            let tape_service = self.tape_service.clone();
            let knowledge = Arc::clone(&self.knowledge);
            let driver_registry = Arc::clone(self.syscall.driver_registry());
            let user_id = user.0.clone();
            let tape_name = session_key.to_string();
            tokio::spawn(async move {
                let extractor_model = &knowledge.extractor_model;
                let driver = match driver_registry.resolve(
                    "knowledge_extractor",
                    None,
                    Some(extractor_model),
                ) {
                    Ok((d, _model_name)) => d,
                    Err(e) => {
                        tracing::warn!(%e, "knowledge extraction: cannot resolve model");
                        return;
                    }
                };
                let entries = match tape_service.from_last_anchor(&tape_name, None).await {
                    Ok(e) => e,
                    Err(e) => {
                        tracing::warn!(%e, "knowledge extraction: failed to read tape");
                        return;
                    }
                };
                match crate::memory::knowledge::extractor::extract_knowledge(
                    &entries,
                    &user_id,
                    &tape_name,
                    &knowledge.pool,
                    &knowledge.embedding_svc,
                    driver.as_ref(),
                    extractor_model,
                    knowledge.config.similarity_threshold,
                )
                .await
                {
                    Ok(count) if count > 0 => {
                        tracing::info!(user = %user_id, count, "knowledge extraction succeeded");
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(user = %user_id, %e, "knowledge extraction failed");
                    }
                }
            });
        }

        // -- Session title generation (async, best-effort) -------------------
        // After the first successful turn, spawn a task to auto-generate a
        // human-readable title for the session using LLM. Only fires when
        // the session has no title yet.
        if !_turn_failed {
            let session_index = Arc::clone(self.io.session_index());
            let needs_title = match session_index.get_session(&session_key).await {
                Ok(Some(entry)) => entry.title.is_none(),
                Ok(None) => false,
                Err(e) => {
                    tracing::warn!(%e, session_key = %session_key, "title gen: failed to check session");
                    false
                }
            };
            if needs_title {
                let tape_service = self.tape_service.clone();
                let driver_registry = Arc::clone(self.syscall.driver_registry());
                let sk = session_key;
                let tape_name = sk.to_string();
                tokio::spawn(async move {
                    if let Err(e) = generate_session_title(
                        &tape_service,
                        &tape_name,
                        &driver_registry,
                        session_index.as_ref(),
                        &sk,
                    )
                    .await
                    {
                        tracing::warn!(%e, session_key = %sk, "session title generation failed");
                    }
                });
            }
        }

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

// ---------------------------------------------------------------------------
// Session title generation (standalone helper)
// ---------------------------------------------------------------------------

/// Auto-generate a short session title from the first user/assistant exchange.
///
/// Reads the tape for the first user and assistant messages, asks the LLM for a
/// concise title (<=30 chars, matching the user's language), and persists it
/// via the session index. Errors are propagated to the caller for logging.
async fn generate_session_title(
    tape_service: &crate::memory::TapeService,
    tape_name: &str,
    driver_registry: &crate::llm::DriverRegistry,
    session_index: &dyn crate::session::SessionIndex,
    session_key: &SessionKey,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use crate::memory::TapEntryKind;

    let entries = tape_service
        .from_last_anchor(tape_name, Some(&[TapEntryKind::Message]))
        .await?;

    let first_user_msg = entries
        .iter()
        .find_map(|e| {
            let p = &e.payload;
            if p.get("role")?.as_str()? == "user" {
                p.get("content")?.as_str().map(String::from)
            } else {
                None
            }
        })
        .unwrap_or_default();

    if first_user_msg.is_empty() {
        tracing::warn!(tape = %tape_name, "title gen: no user message found in tape");
        return Ok(());
    }

    let first_assistant_msg = entries
        .iter()
        .find_map(|e| {
            let p = &e.payload;
            if p.get("role")?.as_str()? == "assistant" {
                p.get("content")?.as_str().map(String::from)
            } else {
                None
            }
        })
        .unwrap_or_default();

    let (driver, model) = driver_registry.resolve("title_generator", None, None)?;

    let assistant_preview: String = first_assistant_msg.chars().take(500).collect();

    let prompt = format!(
        "Given this conversation opening, generate a concise title (max 30 characters).\nMatch \
         the language of the user's message.\nReturn ONLY the title, nothing else.\n\nUser: \
         {first_user_msg}\nAssistant: {assistant_preview}"
    );

    let request = crate::llm::CompletionRequest {
        model,
        messages: vec![crate::llm::Message::user(prompt)],
        tools: vec![],
        temperature: Some(0.3),
        max_tokens: Some(60),
        thinking: None,
        tool_choice: crate::llm::ToolChoice::None,
        parallel_tool_calls: false,
        frequency_penalty: None,
    };

    let response = driver.complete(request).await?;

    // Prefer non-empty content, fall back to reasoning_content for thinking
    // models that return content = Some("") with actual text in reasoning.
    let raw_title = response
        .content
        .filter(|s| !s.trim().is_empty())
        .or(response.reasoning_content);
    let Some(raw_title) = raw_title else {
        tracing::warn!(session_key = %session_key, "title gen: LLM returned no content");
        return Ok(());
    };

    let title = raw_title.trim().trim_matches('"').to_string();
    if title.is_empty() {
        tracing::warn!(session_key = %session_key, "title gen: LLM returned empty title");
        return Ok(());
    }
    if title.chars().count() > 50 {
        tracing::warn!(
            session_key = %session_key,
            title_len = title.chars().count(),
            title = %title,
            "title gen: title exceeds 50 chars, discarded"
        );
        return Ok(());
    }

    match session_index.get_session(session_key).await {
        Ok(Some(mut entry)) => {
            entry.title = Some(title.clone());
            entry.updated_at = chrono::Utc::now();
            if let Err(e) = session_index.update_session(&entry).await {
                tracing::warn!(%e, session_key = %session_key, "title gen: failed to persist title");
            } else {
                tracing::info!(session_key = %session_key, title = %title, "session title generated");
            }
        }
        Ok(None) => {
            tracing::warn!(session_key = %session_key, "title gen: session not found");
        }
        Err(e) => {
            tracing::warn!(session_key = %session_key, error = %e, "title gen: failed to fetch session");
        }
    }

    Ok(())
}
