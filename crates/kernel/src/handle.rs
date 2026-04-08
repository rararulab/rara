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

//! Kernel handles — the public API for interacting with the kernel.
//!
//! `KernelHandle` is the unified entry point for both external callers
//! (channels, boot) and internal agent code (syscalls). Session-scoped
//! syscall methods accept an explicit `session_key` parameter.

use std::sync::Arc;

use jiff::Timestamp;
use tokio::sync::Semaphore;

use crate::{
    agent::{AgentManifest, AgentRegistryRef, TurnTrace},
    error::{KernelError, Result},
    event::{KernelEventEnvelope, Syscall},
    identity::Principal,
    io::{
        AgentHandle, EndpointRegistryRef, IOError, IOSubsystem, InboundMessage, PipeReader,
        PipeWriter, RawPlatformMessage, StreamHubRef, Unresolved,
    },
    kernel::{KernelConfig, SettingsRef},
    kv::KvScope,
    queue::ShardedQueueRef,
    security::SecurityRef,
    session::{
        SessionIndex, SessionKey, SessionState, SessionStats, SessionTable, Signal, SystemStats,
    },
    tool::{ToolRegistry, ToolRegistryRef},
};

/// Provider that generates a skills prompt block for injection into the agent
/// system prompt. Called on each agent turn to reflect the latest registry
/// state.
///
/// Stored as a boxed closure rather than a trait object alias so the concrete
/// erased type is visible at the field site.
pub type SkillPromptProvider = Arc<dyn Fn() -> String + Send + Sync>;
/// Public entry point for interacting with the kernel.
///
/// Provides both mutation methods (spawn, signal, shutdown) that flow through
/// the event queue, and read-only accessors for kernel subsystems.
///
/// Cheap to clone (all fields are `Arc`s). External callers should prefer
/// `KernelHandle` over `Arc<Kernel>`.
///
/// # Usage
///
/// Obtain a `KernelHandle` via
/// [`Kernel::handle()`](crate::kernel::Kernel::handle):
///
/// ```ignore
/// let handle = kernel.handle();
/// let session_key = handle.spawn_with_input(manifest, "hello".into(), principal, None, None).await?;
/// handle.send_signal(session_key, Signal::Pause)?;
/// handle.shutdown()?;
/// ```
#[derive(Clone)]
pub struct KernelHandle {
    /// Core: the unified event queue sender.
    event_queue:           ShardedQueueRef,
    /// Agent registry for resolving named agents to manifests.
    agent_registry:        AgentRegistryRef,
    /// The session table tracking all running sessions.
    process_table:         Arc<SessionTable>,
    /// Bundled I/O subsystem (ingress resolution, streaming, delivery).
    io:                    Arc<IOSubsystem>,
    /// Flat KV settings provider for runtime configuration.
    settings:              SettingsRef,
    /// Unified security subsystem (auth + authz + approval + guard).
    security:              SecurityRef,
    /// Kernel configuration.
    config:                KernelConfig,
    /// Multi-driver LLM registry for resolving drivers per-agent.
    driver_registry:       crate::llm::DriverRegistryRef,
    /// Global tool registry.
    tool_registry:         ToolRegistryRef,
    /// Global semaphore limiting total concurrent agent processes.
    global_semaphore:      Arc<Semaphore>,
    /// When the kernel was created (for uptime calculation).
    started_at:            Timestamp,
    /// Tape service for persistent session traces.
    tape:                  crate::memory::TapeService,
    /// Execution trace service for persisting turn-level traces.
    trace_service:         crate::trace::TraceService,
    /// Shared job wheel for querying scheduled tasks.
    job_wheel:             Arc<parking_lot::Mutex<crate::schedule::JobWheel>>,
    /// Provider for generating the skills prompt block.
    skill_prompt_provider: SkillPromptProvider,
}

impl KernelHandle {
    /// Create a new `KernelHandle`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        event_queue: ShardedQueueRef,
        agent_registry: AgentRegistryRef,
        process_table: Arc<SessionTable>,
        io: Arc<IOSubsystem>,
        settings: SettingsRef,
        security: SecurityRef,
        config: KernelConfig,
        driver_registry: crate::llm::DriverRegistryRef,
        tool_registry: ToolRegistryRef,
        global_semaphore: Arc<Semaphore>,
        started_at: Timestamp,
        tape: crate::memory::TapeService,
        trace_service: crate::trace::TraceService,
        job_wheel: Arc<parking_lot::Mutex<crate::schedule::JobWheel>>,
        skill_prompt_provider: SkillPromptProvider,
    ) -> Self {
        Self {
            event_queue,
            agent_registry,
            process_table,
            io,
            settings,
            security,
            config,
            driver_registry,
            tool_registry,
            global_semaphore,
            started_at,
            tape,
            trace_service,
            job_wheel,
            skill_prompt_provider,
        }
    }

    // -- Mutation methods (flow through event queue) -------------------------

    /// Spawn a new agent process via the unified event queue.
    ///
    /// Pushes a `KernelEvent::SpawnAgent` into the event queue and waits
    /// for the reply. The kernel generates a fresh isolated session for
    /// the new process.
    #[tracing::instrument(skip_all, fields(manifest_name = %manifest.name))]
    pub async fn spawn_with_input(
        &self,
        manifest: AgentManifest,
        input: String,
        principal: Principal<crate::identity::Lookup>,
        parent_id: Option<SessionKey>,
        desired_session_key: Option<SessionKey>,
    ) -> Result<SessionKey> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let event = KernelEventEnvelope::spawn_agent(
            manifest,
            input,
            principal,
            parent_id,
            desired_session_key,
            reply_tx,
        );
        self.event_queue
            .push(event)
            .map_err(|_| KernelError::SpawnFailed {
                message: "event queue full".to_string(),
            })?;

        reply_rx.await.map_err(|_| KernelError::SpawnFailed {
            message: "spawn reply channel closed".to_string(),
        })?
    }

    /// Spawn a named agent by looking up its manifest in the agent registry.
    #[tracing::instrument(skip(self, input, principal, parent_id))]
    pub async fn spawn_named(
        &self,
        agent_name: &str,
        input: String,
        principal: Principal<crate::identity::Lookup>,
        parent_id: Option<SessionKey>,
    ) -> Result<SessionKey> {
        let manifest =
            self.agent_registry
                .get(agent_name)
                .ok_or(KernelError::ManifestNotFound {
                    name: agent_name.to_string(),
                })?;

        self.spawn_with_input(manifest, input, principal, parent_id, None)
            .await
    }

    /// Send a control signal to an agent process (fire-and-forget).
    ///
    /// Uses `try_push` (non-async) so this can be called from synchronous
    /// contexts.
    pub fn send_signal(&self, target: SessionKey, signal: Signal) -> Result<()> {
        self.event_queue
            .try_push(KernelEventEnvelope::send_signal(target, signal))
            .map_err(|_| KernelError::Other {
                message: "event queue full for signal".into(),
            })
    }

    /// Ingest a raw platform message: resolve identity + session, then push
    /// the resulting [`InboundMessage`] into the event queue.
    ///
    /// This is the primary entry point for channel adapters.
    pub async fn ingest(&self, raw: RawPlatformMessage) -> std::result::Result<(), IOError> {
        let msg = self.io.resolve(raw).await?;
        let channel_label = format!("{:?}", msg.source.channel_type);

        self.submit_message(msg).map_err(|_| IOError::SystemBusy)?;

        crate::metrics::record_message_inbound(&channel_label);

        Ok(())
    }

    /// Submit an inbound user message (fire-and-forget).
    ///
    /// Uses `try_push` (non-async) so this can be called from synchronous
    /// contexts.
    pub fn submit_message(&self, msg: InboundMessage<Unresolved>) -> Result<()> {
        self.event_queue
            .try_push(KernelEventEnvelope::user_message(msg))
            .map_err(|_| KernelError::Other {
                message: "event queue full for user message".into(),
            })
    }

    /// Submit a group-chat message for proactive judgment (fire-and-forget).
    ///
    /// The kernel will record the message to tape, run a lightweight LLM
    /// judgment, and only promote to a full agent turn if approved.
    pub fn submit_group_message(&self, msg: InboundMessage<Unresolved>) -> Result<()> {
        self.event_queue
            .try_push(KernelEventEnvelope::group_message(msg))
            .map_err(|_| KernelError::Other {
                message: "event queue full for group message".into(),
            })
    }

    /// Dispatch a Mita directive to a target session.
    ///
    /// The directive is delivered as an ephemeral instruction — it triggers
    /// an LLM turn but is NOT persisted to the session's tape.
    pub fn dispatch_directive(&self, target: SessionKey, instruction: String) -> Result<()> {
        self.event_queue
            .push(KernelEventEnvelope::mita_directive(target, instruction))
            .map_err(|_| KernelError::SpawnFailed {
                message: "event queue full".to_string(),
            })
    }

    /// Request a graceful kernel shutdown (fire-and-forget).
    ///
    /// Uses `try_push` (non-async) so this can be called from synchronous
    /// contexts.
    pub fn shutdown(&self) -> Result<()> {
        self.event_queue
            .try_push(KernelEventEnvelope::shutdown())
            .map_err(|_| KernelError::Other {
                message: "event queue full for shutdown".into(),
            })
    }

    // -- Read-only accessors ------------------------------------------------

    /// Access the process table for querying.
    pub fn process_table(&self) -> &Arc<SessionTable> { &self.process_table }

    /// Resolve identity and (possibly) session for a raw platform message.
    ///
    /// Returns an [`InboundMessage<Unresolved>`] — the caller must still
    /// promote it to `Resolved` before routing.
    ///
    /// Delegates to [`IOSubsystem::resolve`].
    pub async fn resolve(
        &self,
        raw: RawPlatformMessage,
    ) -> std::result::Result<InboundMessage<crate::io::Unresolved>, IOError> {
        self.io.resolve(raw).await
    }

    /// Access the ephemeral stream hub (WebAdapter needs this for token
    /// deltas).
    pub fn stream_hub(&self) -> &StreamHubRef { self.io.stream_hub() }

    /// Access the endpoint registry (WebAdapter needs this for connection
    /// tracking).
    pub fn endpoint_registry(&self) -> &EndpointRegistryRef { self.io.endpoint_registry() }

    /// Access the session index for session and channel binding lookups.
    pub fn session_index(&self) -> &Arc<dyn SessionIndex> { self.io.session_index() }

    /// Access the agent registry for looking up named manifests.
    pub fn agent_registry(&self) -> &AgentRegistryRef { &self.agent_registry }

    /// Access the LLM driver registry.
    pub fn driver_registry(&self) -> &crate::llm::DriverRegistryRef { &self.driver_registry }

    /// Access the tool registry.
    pub fn tool_registry(&self) -> &ToolRegistryRef { &self.tool_registry }

    /// Access the flat KV settings provider.
    pub fn settings(&self) -> &SettingsRef { &self.settings }

    /// Access the unified security subsystem.
    pub fn security(&self) -> &SecurityRef { &self.security }

    /// Access the kernel config.
    pub fn config(&self) -> &KernelConfig { &self.config }

    /// Access the unified event queue.
    pub fn event_queue(&self) -> &ShardedQueueRef { &self.event_queue }

    /// Access the tape service for persistent read/write.
    pub fn tape(&self) -> &crate::memory::TapeService { &self.tape }

    /// Access the execution trace service.
    pub fn trace_service(&self) -> &crate::trace::TraceService { &self.trace_service }

    /// List scheduled jobs, optionally filtered by session key.
    pub fn list_jobs(&self, session_key: Option<SessionKey>) -> Vec<crate::schedule::JobEntry> {
        self.job_wheel.lock().list(session_key.as_ref())
    }

    /// Generate the skills prompt block for injection into the agent system
    /// prompt.
    pub fn skills_prompt(&self) -> String { (self.skill_prompt_provider)() }

    // -- Query methods ------------------------------------------------------

    /// Get detailed runtime statistics for a single session.
    ///
    /// Returns `None` if the session does not exist.
    pub fn session_stats(&self, session_key: SessionKey) -> Option<crate::session::SessionStats> {
        self.process_table.stats(session_key)
    }

    /// List detailed runtime statistics for all sessions.
    pub fn list_processes(&self) -> Vec<crate::session::SessionStats> {
        self.process_table.all_process_stats()
    }

    /// Get kernel-wide aggregate statistics.
    pub fn system_stats(&self) -> SystemStats {
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

        SystemStats {
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
    pub fn get_process_turns(&self, session_key: SessionKey) -> Vec<TurnTrace> {
        self.process_table.get_turn_traces(session_key)
    }

    // ========================================================================
    // Session-scoped syscall methods (formerly on ProcessHandle)
    // ========================================================================

    /// Push a syscall event into the event queue.
    pub(crate) async fn syscall_push(&self, event: KernelEventEnvelope) -> Result<()> {
        self.event_queue
            .push(event)
            .map_err(|_| KernelError::Other {
                message: "event queue full for syscall".into(),
            })
    }

    /// Await a oneshot reply, converting channel-closed to KernelError.
    async fn await_reply<T>(rx: tokio::sync::oneshot::Receiver<T>) -> Result<T> {
        rx.await.map_err(|_| KernelError::Other {
            message: "syscall reply channel closed".into(),
        })
    }

    // -- Process operations --

    /// Spawn a child agent via the unified event queue.
    ///
    /// Acquires a permit from the parent session's `child_semaphore` before
    /// spawning. The permit is stored in the child session and released
    /// automatically when the child is removed from the process table.
    pub async fn spawn_child(
        &self,
        session_key: SessionKey,
        principal: &Principal,
        manifest: AgentManifest,
        input: String,
    ) -> Result<AgentHandle> {
        // Acquire a permit from the parent's child_semaphore to enforce the
        // per-session child limit.
        let child_sem = self
            .process_table
            .with(&session_key, |p| p.child_semaphore.clone())
            .ok_or_else(|| KernelError::SessionNotFound { key: session_key })?;

        let child_permit =
            child_sem
                .acquire_owned()
                .await
                .map_err(|_| KernelError::SpawnFailed {
                    message: format!("parent session {} child semaphore closed", session_key),
                })?;

        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let event = KernelEventEnvelope::spawn_agent(
            manifest,
            input,
            principal.clone().into_lookup(),
            Some(session_key),
            None,
            reply_tx,
        );
        self.syscall_push(event).await?;

        let child_key = reply_rx.await.map_err(|_| KernelError::SpawnFailed {
            message: "spawn reply channel closed".to_string(),
        })??;

        let (result_tx, result_rx) = tokio::sync::mpsc::channel(64);

        // Store result_tx and child_permit in the child session so
        // cleanup_process can send the result. The permit is released
        // automatically when the child session is dropped.
        self.process_table.with_mut(&child_key, |session| {
            session.result_tx = Some(result_tx);
            session._parent_child_permit = Some(child_permit);
        });

        Ok(AgentHandle {
            session_key: child_key,
            result_rx,
        })
    }

    /// Query session state (direct access — no event queue roundtrip).
    pub fn session_status(&self, session_key: SessionKey) -> Result<SessionStats> {
        self.process_table
            .stats(session_key)
            .ok_or(KernelError::ProcessNotFound {
                id: session_key.to_string(),
            })
    }

    /// List child sessions (direct access — no event queue roundtrip).
    pub fn session_children(&self, session_key: SessionKey) -> Vec<SessionStats> {
        self.process_table.children_of(session_key)
    }

    /// Register a background task entry on the parent session.
    pub fn register_background_task(
        &self,
        session_key: SessionKey,
        entry: crate::session::BackgroundTaskEntry,
    ) {
        self.process_table.with_mut(&session_key, |session| {
            session.background_tasks.push(entry);
        });
    }

    /// Remove a background task entry from the parent session.
    /// Returns `true` if the task was found and removed.
    pub fn remove_background_task(&self, session_key: SessionKey, child_key: SessionKey) -> bool {
        self.process_table
            .with_mut(&session_key, |session| {
                let before = session.background_tasks.len();
                session
                    .background_tasks
                    .retain(|t| t.child_key != child_key);
                session.background_tasks.len() < before
            })
            .unwrap_or(false)
    }

    /// Check if a child session is a background task of the given parent.
    pub fn is_background_task(&self, parent_key: SessionKey, child_key: SessionKey) -> bool {
        self.process_table
            .with(&parent_key, |session| {
                session
                    .background_tasks
                    .iter()
                    .any(|t| t.child_key == child_key)
            })
            .unwrap_or(false)
    }

    /// List active background tasks for a session.
    pub fn background_tasks(
        &self,
        session_key: SessionKey,
    ) -> Vec<crate::session::BackgroundTaskEntry> {
        self.process_table
            .with(&session_key, |session| session.background_tasks.clone())
            .unwrap_or_default()
    }

    /// Register a tool call limit oneshot sender on the session.
    ///
    /// Called by the agent loop (inline or plan) when cumulative tool calls
    /// reach the `tool_call_limit`. The `tx` end is stored alongside
    /// `limit_id` so that
    /// [`resolve_tool_call_limit`](Self::resolve_tool_call_limit)
    /// can validate the ID before delivering the decision.
    ///
    /// Only one limit can be pending per session at a time — registering a
    /// new one implicitly drops the previous sender (if any), which causes
    /// the old `rx` to receive a channel-closed error (treated as Stop).
    pub fn register_tool_call_limit(
        &self,
        session_key: SessionKey,
        limit_id: u64,
        tx: tokio::sync::oneshot::Sender<crate::io::ToolCallLimitDecision>,
    ) {
        self.process_table.with_mut(&session_key, |session| {
            session.pending_tool_call_limit = Some((limit_id, tx));
        });
    }

    /// Resolve a pending tool call limit decision.
    ///
    /// Called by channel adapters (e.g. Telegram callback handler) when the
    /// user clicks continue/stop on the inline keyboard.
    ///
    /// **Stale button protection:** only resolves if `limit_id` matches the
    /// currently pending one. This prevents a button from an earlier limit
    /// (which the user didn't click in time) from accidentally resolving a
    /// newer limit instance. Mismatched IDs are silently ignored.
    ///
    /// Returns `true` if the decision was successfully delivered to the
    /// waiting agent loop.
    pub fn resolve_tool_call_limit(
        &self,
        session_key: SessionKey,
        limit_id: u64,
        decision: crate::io::ToolCallLimitDecision,
    ) -> bool {
        self.process_table
            .with_mut(&session_key, |session| {
                if let Some((pending_id, _)) = &session.pending_tool_call_limit {
                    if *pending_id != limit_id {
                        return false; // stale callback — ignore
                    }
                }
                if let Some((_, tx)) = session.pending_tool_call_limit.take() {
                    tx.send(decision).is_ok()
                } else {
                    false
                }
            })
            .unwrap_or(false)
    }

    // -- Memory operations --

    /// Store a value in a session's private namespace.
    pub async fn mem_store(
        &self,
        session_key: SessionKey,
        principal: &Principal,
        key: &str,
        value: serde_json::Value,
    ) -> Result<()> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.syscall_push(KernelEventEnvelope::syscall(
            session_key,
            Syscall::MemStore {
                session_key,
                principal: principal.clone(),
                key: key.to_string(),
                value,
                reply_tx,
            },
        ))
        .await?;
        Self::await_reply(reply_rx).await?
    }

    /// Recall a value from a session's private namespace.
    pub async fn mem_recall(
        &self,
        session_key: SessionKey,
        key: &str,
    ) -> Result<Option<serde_json::Value>> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.syscall_push(KernelEventEnvelope::syscall(
            session_key,
            Syscall::MemRecall {
                key: key.to_string(),
                reply_tx,
            },
        ))
        .await?;
        Self::await_reply(reply_rx).await?
    }

    /// Store a value in an explicit shared scope.
    pub async fn shared_store(
        &self,
        session_key: SessionKey,
        principal: &Principal,
        scope: KvScope,
        key: &str,
        value: serde_json::Value,
    ) -> Result<()> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.syscall_push(KernelEventEnvelope::syscall(
            session_key,
            Syscall::SharedStore {
                principal: principal.clone(),
                scope,
                key: key.to_string(),
                value,
                reply_tx,
            },
        ))
        .await?;
        Self::await_reply(reply_rx).await?
    }

    /// Recall a value from an explicit shared scope.
    pub async fn shared_recall(
        &self,
        session_key: SessionKey,
        principal: &Principal,
        scope: KvScope,
        key: &str,
    ) -> Result<Option<serde_json::Value>> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.syscall_push(KernelEventEnvelope::syscall(
            session_key,
            Syscall::SharedRecall {
                principal: principal.clone(),
                scope,
                key: key.to_string(),
                reply_tx,
            },
        ))
        .await?;
        Self::await_reply(reply_rx).await?
    }

    // -- Pipe operations --

    /// Create an anonymous pipe targeting a specific session.
    pub async fn create_pipe(
        &self,
        session_key: SessionKey,
        target: SessionKey,
    ) -> Result<(PipeWriter, PipeReader)> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.syscall_push(KernelEventEnvelope::syscall(
            session_key,
            Syscall::CreatePipe { target, reply_tx },
        ))
        .await?;
        Self::await_reply(reply_rx).await?
    }

    /// Create a named pipe that any session can connect to.
    pub async fn create_named_pipe(
        &self,
        session_key: SessionKey,
        name: &str,
    ) -> Result<(PipeWriter, PipeReader)> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.syscall_push(KernelEventEnvelope::syscall(
            session_key,
            Syscall::CreateNamedPipe {
                name: name.to_string(),
                reply_tx,
            },
        ))
        .await?;
        Self::await_reply(reply_rx).await?
    }

    /// Connect to a named pipe as a reader.
    pub async fn connect_pipe(&self, session_key: SessionKey, name: &str) -> Result<PipeReader> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.syscall_push(KernelEventEnvelope::syscall(
            session_key,
            Syscall::ConnectPipe {
                name: name.to_string(),
                reply_tx,
            },
        ))
        .await?;
        Self::await_reply(reply_rx).await?
    }

    // -- Guard operations --

    /// Check whether a tool requires approval before execution
    /// (direct access — no event queue roundtrip).
    pub fn requires_approval(&self, tool_name: &str) -> bool {
        self.security.requires_approval(tool_name)
    }

    /// Request approval for a tool execution.
    pub async fn request_approval(
        &self,
        session_key: SessionKey,
        principal: &Principal,
        tool_name: &str,
        summary: &str,
    ) -> Result<bool> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.syscall_push(KernelEventEnvelope::syscall(
            session_key,
            Syscall::RequestApproval {
                principal: principal.clone(),
                tool_name: tool_name.to_string(),
                summary: summary.to_string(),
                reply_tx,
            },
        ))
        .await?;
        Self::await_reply(reply_rx).await?
    }

    // -- Context queries (used by agent_loop) --

    /// Get the manifest for a session (direct access — no event queue
    /// roundtrip).
    pub fn session_manifest(&self, session_key: SessionKey) -> Result<AgentManifest> {
        self.process_table
            .with(&session_key, |p| p.manifest.clone())
            .ok_or(KernelError::ProcessNotFound {
                id: session_key.to_string(),
            })
    }

    /// Get the tool registry, enriched with per-session tools (e.g.
    /// SyscallTool).
    pub async fn session_tool_registry(
        &self,
        session_key: SessionKey,
    ) -> Result<Arc<ToolRegistry>> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.syscall_push(KernelEventEnvelope::syscall(
            session_key,
            Syscall::GetToolRegistry { reply_tx },
        ))
        .await?;
        Self::await_reply(reply_rx).await
    }

    /// Resolve an [`LlmDriver`](crate::llm::LlmDriver) + model for this
    /// session via the kernel's `DriverRegistry`
    /// (direct access — no event queue roundtrip).
    pub fn session_resolve_driver(
        &self,
        session_key: SessionKey,
    ) -> Result<(crate::llm::LlmDriverRef, String)> {
        let driver_info = self.process_table.with(&session_key, |p| {
            (
                p.manifest.name.clone(),
                p.manifest.provider_hint.clone(),
                p.manifest.model.clone(),
            )
        });
        match driver_info {
            Some((name, hint, model)) => {
                self.driver_registry
                    .resolve(&name, hint.as_deref(), model.as_deref())
            }
            None => Err(KernelError::ProcessNotFound {
                id: session_key.to_string(),
            }),
        }
    }

    // -- Event publishing --

    /// Publish an event to the kernel event bus.
    pub async fn publish_event(
        &self,
        session_key: SessionKey,
        event_type: &str,
        payload: serde_json::Value,
    ) -> Result<()> {
        self.syscall_push(KernelEventEnvelope::syscall(
            session_key,
            Syscall::PublishEvent {
                event_type: event_type.to_string(),
                payload,
            },
        ))
        .await
    }

    /// Deliver a system-generated message to a session, triggering an LLM turn.
    ///
    /// Used by the notification bus to deliver proactive-turn notifications.
    pub async fn deliver_internal(&self, msg: crate::io::InboundMessage) {
        let _ = self
            .event_queue
            .push(KernelEventEnvelope::user_message(msg.into_unresolved()));
    }
}
