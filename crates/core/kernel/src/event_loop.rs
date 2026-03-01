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
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::{
    audit::{AuditEvent, AuditEventType},
    channel::types::ChatMessage,
    error::{KernelError, Result},
    handle::scoped::ScopedKernelHandle,
    io::types::{InboundMessage, MessageId, OutboundEnvelope, OutboundPayload, OutboundRouting},
    kernel::Kernel,
    process::{
        AgentEnv, AgentId, AgentManifest, AgentProcess, AgentResult, ProcessState, SessionId,
        Signal, principal::Principal,
    },
    unified_event::KernelEvent,
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
    /// Whether this process is paused. When true, incoming messages are
    /// buffered in `pause_buffer` instead of being processed.
    pub paused: bool,
    /// Buffered events received while the process was paused or busy.
    pub pause_buffer: Vec<KernelEvent>,
    /// The ScopedKernelHandle for this process (needed to run LLM turns).
    pub handle: Arc<ScopedKernelHandle>,
    /// Maximum context tokens for compaction.
    pub max_context_tokens: usize,
    /// Last successful result (for final output when process ends).
    pub last_result: Option<AgentResult>,
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
    /// This is the single driver for all kernel activity. It drains the
    /// `EventQueue` in priority order and dispatches each event to the
    /// appropriate handler.
    ///
    /// Replaces: TickLoop + process_loop + Egress subscribe loop.
    pub async fn run_event_loop(&self, shutdown: CancellationToken) {
        let runtimes: Arc<RuntimeTable> = Arc::new(DashMap::new());

        info!("kernel event loop started");
        loop {
            tokio::select! {
                _ = self.event_queue().wait() => {
                    let events = self.event_queue().drain(32).await;
                    for event in events {
                        self.handle_event(event, &runtimes).await;
                    }
                }
                _ = shutdown.cancelled() => {
                    info!("kernel event loop shutting down");
                    // Drain any remaining critical events.
                    let remaining = self.event_queue().drain(1024).await;
                    for event in remaining {
                        if matches!(event, KernelEvent::SendSignal { .. } | KernelEvent::Shutdown) {
                            self.handle_event(event, &runtimes).await;
                        }
                    }
                    break;
                }
            }
        }
        info!("kernel event loop stopped");
    }

    /// Dispatch a single event to its handler.
    async fn handle_event(&self, event: KernelEvent, runtimes: &RuntimeTable) {
        match event {
            KernelEvent::UserMessage(msg) => {
                self.handle_user_message(msg, runtimes).await;
            }
            KernelEvent::SpawnAgent {
                manifest,
                input,
                principal,
                session_id,
                parent_id,
                reply_tx,
            } => {
                let result = self
                    .handle_spawn_agent(manifest, input, principal, session_id, parent_id, runtimes)
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
    // handle_user_message
    // -----------------------------------------------------------------------

    /// Handle a user message: find or spawn a process, then start an LLM turn.
    async fn handle_user_message(&self, msg: InboundMessage, runtimes: &RuntimeTable) {
        let session_id = msg.session_id.clone();
        let user = msg.user.clone();
        // 1. Find existing process for this session.
        let agent_id = if let Some(process) = self.process_table().find_by_session(&session_id) {
            let aid = process.agent_id;

            // Check if process is paused or already running — buffer the event.
            if let Some(mut rt) = runtimes.get_mut(&aid) {
                if rt.paused {
                    rt.pause_buffer.push(KernelEvent::UserMessage(msg));
                    return;
                }
                // Check if process state is Running (LLM turn in progress).
                if let Some(p) = self.process_table().get(aid) {
                    if p.state == ProcessState::Running {
                        // Buffer — we'll drain after the current turn completes.
                        rt.pause_buffer.push(KernelEvent::UserMessage(msg));
                        return;
                    }
                }
            }
            aid
        } else {
            // No existing process — auto-spawn one.
            let manifest = match self.resolve_manifest_for_auto_spawn().await {
                Some(m) => m,
                None => {
                    error!(
                        session_id = %session_id,
                        "no model configured — cannot spawn agent"
                    );
                    return;
                }
            };
            let principal = Principal::user(user.0.clone());
            match self
                .handle_spawn_agent(
                    manifest,
                    msg.content.as_text(),
                    principal,
                    session_id.clone(),
                    None,
                    runtimes,
                )
                .await
            {
                Ok(aid) => aid,
                Err(e) => {
                    error!(session_id = %session_id, error = %e, "failed to spawn agent");
                    return;
                }
            }
        };

        // 2. Start LLM turn for this process.
        self.start_llm_turn(agent_id, msg, runtimes).await;
    }

    /// Start an LLM turn for the given agent, spawning the work as an async
    /// task that pushes `TurnCompleted` back into the EventQueue when done.
    async fn start_llm_turn(
        &self,
        agent_id: AgentId,
        msg: InboundMessage,
        runtimes: &RuntimeTable,
    ) {
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

        // Record metrics.
        if let Some(metrics) = self.inner().process_table.get_metrics(&agent_id) {
            metrics.record_message();
            // Deliberately NOT calling `.touch().await` here since we hold
            // the DashMap guard — instead we'll touch after the turn.
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

        // Spawn async task for the LLM turn.
        tokio::spawn(async move {
            let turn_result = crate::agent_turn::run_inline_agent_loop(
                &handle,
                user_text,
                history,
                &stream_handle,
                &turn_cancel,
            )
            .await;

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
        let inner = self.inner();

        // Update metrics.
        if let Some(metrics) = inner.process_table.get_metrics(&agent_id) {
            metrics.touch().await;
        }

        match result {
            Ok(turn) if !turn.text.is_empty() => {
                // Record metrics.
                if let Some(metrics) = inner.process_table.get_metrics(&agent_id) {
                    metrics.record_llm_call();
                    metrics.record_tool_calls(turn.tool_calls as u64);
                    let estimated_tokens = (turn.text.len() as u64).saturating_div(4).max(1);
                    metrics.record_tokens(estimated_tokens);
                }

                // Persist assistant reply.
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

                // Push Deliver event for the reply.
                let envelope = OutboundEnvelope {
                    id:          MessageId::new(),
                    in_reply_to,
                    user:        user.clone(),
                    session_id:  session_id.clone(),
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
                    "message processed"
                );

                if let Some(mut rt) = runtimes.get_mut(&agent_id) {
                    rt.last_result = Some(result);
                }
            }
            Ok(turn) => {
                // Empty result — LLM call was made but produced no text.
                if let Some(metrics) = inner.process_table.get_metrics(&agent_id) {
                    metrics.record_llm_call();
                    metrics.record_tool_calls(turn.tool_calls as u64);
                }
            }
            Err(err_msg) => {
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

                // Deliver error.
                let envelope = OutboundEnvelope {
                    id:          MessageId::new(),
                    in_reply_to,
                    user:        user.clone(),
                    session_id:  session_id.clone(),
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
    async fn handle_spawn_agent(
        &self,
        manifest: AgentManifest,
        input: String,
        principal: Principal,
        session_id: SessionId,
        parent_id: Option<AgentId>,
        runtimes: &RuntimeTable,
    ) -> Result<AgentId> {
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

        // Ensure session exists + load initial history.
        inner.ensure_session(&session_id).await;
        let initial_messages = inner.load_session_messages(&session_id).await;

        let agent_id = AgentId::new();

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
        let process = AgentProcess {
            agent_id,
            parent_id,
            session_id: session_id.clone(),
            manifest: manifest.clone(),
            principal: principal.clone(),
            env: AgentEnv::default(),
            state: ProcessState::Waiting,
            created_at: jiff::Timestamp::now(),
            finished_at: None,
            result: None,
            created_files: vec![],
        };
        inner.process_table.insert(process);

        // Create cancellation token.
        let token = if let Some(pid) = parent_id {
            inner
                .process_table
                .get_cancellation_token(&pid)
                .map(|parent_token| parent_token.child_token())
                .unwrap_or_default()
        } else {
            CancellationToken::new()
        };
        inner
            .process_table
            .set_cancellation_token(agent_id, token);

        // Build ScopedKernelHandle.
        let child_limit = manifest
            .max_children
            .unwrap_or(inner.default_child_limit);
        let effective_registry = Arc::clone(&inner.tool_registry);

        let handle = Arc::new(ScopedKernelHandle {
            agent_id,
            session_id: session_id.clone(),
            principal,
            manifest: manifest.clone(),
            allowed_tools: vec![],
            tool_registry: effective_registry,
            child_semaphore: Arc::new(tokio::sync::Semaphore::new(child_limit)),
            inner: Arc::clone(inner),
        });

        let max_context_tokens = manifest
            .max_context_tokens
            .unwrap_or(crate::memory::compaction::DEFAULT_MAX_CONTEXT_TOKENS);

        // Create runtime entry.
        let runtime = ProcessRuntime {
            conversation: initial_messages,
            turn_cancel: CancellationToken::new(),
            paused: false,
            pause_buffer: Vec::new(),
            handle,
            max_context_tokens,
            last_result: None,
        };
        runtimes.insert(agent_id, runtime);

        // Store the global permit so it lives as long as the process.
        // We leak it into the runtime — it will be dropped when the runtime
        // entry is removed.
        // TODO: track permits properly in ProcessRuntime
        std::mem::forget(global_permit);

        info!(
            agent_id = %agent_id,
            manifest = %manifest.name,
            session_id = %session_id,
            "process spawned via event loop"
        );

        // Now push a UserMessage event for the initial input so it gets
        // processed by handle_user_message.
        let inbound = InboundMessage::synthetic(
            input,
            crate::process::principal::UserId("system".to_string()),
            session_id,
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
                // Notify via Deliver event.
                let session_id = inner
                    .process_table
                    .get(target)
                    .map(|p| p.session_id.clone())
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
                // Grace period then force-kill.
                let pt = inner.process_table.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    if let Some(token) = pt.get_cancellation_token(&target) {
                        token.cancel();
                    }
                });
                // Clean up runtime.
                self.cleanup_process(target, runtimes).await;
            }
            Signal::Kill => {
                info!(agent_id = %target, "kill signal");
                if let Some(token) = inner.process_table.get_cancellation_token(&target) {
                    token.cancel();
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
        self.inner().process_table.clear_cancellation_token(&agent_id);
    }

    /// Resolve a manifest for auto-spawning (when a user message arrives
    /// with no existing process).
    async fn resolve_manifest_for_auto_spawn(&self) -> Option<AgentManifest> {
        let model = self
            .model_repo()
            .get(crate::model_repo::model_keys::CHAT)
            .await?;
        Some(AgentManifest {
            name:               "io-agent".to_string(),
            description:        "I/O bus agent".to_string(),
            model,
            system_prompt:      "You are a helpful assistant.".to_string(),
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
