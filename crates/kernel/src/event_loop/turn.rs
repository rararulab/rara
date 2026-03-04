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

//! LLM turn execution and completion handling.

use std::sync::Arc;

use tracing::{error, info, info_span, warn};

use super::runtime::RuntimeTable;
use crate::{
    audit::{AuditEvent, AuditEventType},
    channel::types::ChatMessage,
    event::KernelEvent,
    io::{
        stream::StreamId,
        types::{InboundMessage, MessageId, OutboundEnvelope},
    },
    kernel::Kernel,
    process::{AgentId, AgentResult, SessionState, SessionId},
    queue::EventQueueRef,
};

/// RAII guard ensuring that `TurnCompleted` is always pushed and the stream is
/// always closed, even when the spawned turn task panics or is cancelled.
///
/// On normal completion the caller sets `completed = true` before the guard is
/// dropped; on abnormal exit `Drop` performs the cleanup.
struct TurnGuard {
    event_queue:    EventQueueRef,
    stream_hub:     Arc<crate::io::stream::StreamHub>,
    stream_id:      StreamId,
    typing_refresh: Option<tokio::task::JoinHandle<()>>,
    agent_id:       AgentId,
    session_id:     SessionId,
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
            let event = KernelEvent::turn_completed(
                self.agent_id,
                self.session_id.clone(),
                Err("turn task terminated unexpectedly".to_string()),
                self.msg_id.clone(),
                self.user.clone(),
            );
            if let Err(e) = self.event_queue.try_push(event) {
                error!(
                    %e,
                    agent_id = %self.agent_id,
                    "TurnGuard: failed to push TurnCompleted on abnormal exit"
                );
            } else {
                warn!(
                    agent_id = %self.agent_id,
                    "TurnGuard: turn task exited abnormally, pushed TurnCompleted(Err)"
                );
            }
        }
    }
}

impl Kernel {
    /// Start an LLM turn for the given agent, spawning the work as an async
    /// task that pushes `TurnCompleted` back into the EventQueue when done.
    #[tracing::instrument(skip_all, fields(agent_id = %agent_id, session_id = %msg.session_id))]
    pub(crate) async fn start_llm_turn(
        &self,
        agent_id: AgentId,
        msg: InboundMessage,
        runtimes: &RuntimeTable,
    ) {
        if !runtimes.contains(&agent_id) {
            warn!(agent_id = %agent_id, "runtime not found for LLM turn");
            // Send error back to the user instead of silently dropping.
            let envelope = OutboundEnvelope::error(
                msg.id.clone(),
                msg.user.clone(),
                msg.session_id.clone(),
                "runtime_not_found",
                format!("agent runtime not found: {agent_id}"),
            );
            if let Err(e) = self.event_queue().try_push(KernelEvent::deliver(envelope)) {
                error!(%e, "failed to push runtime-not-found error Deliver");
            }
            return;
        }

        let session_id = msg.session_id.clone();
        let user = msg.user.clone();
        let msg_id = msg.id.clone();

        // Set state to Active.
        let _ = self
            .process_table()
            .set_state(agent_id, SessionState::Active);

        // Send a typing / progress indicator so the user sees feedback
        // while the LLM is thinking (e.g. Telegram "typing..." bubble).
        let egress_session_id = self
            .process_table()
            .get(agent_id)
            .and_then(|p| p.channel_session_id.clone())
            .unwrap_or_else(|| session_id.clone());
        let _ = self
            .event_queue()
            .try_push(KernelEvent::deliver(OutboundEnvelope::progress(
                msg_id.clone(),
                user.clone(),
                egress_session_id.clone(),
                crate::io::types::stages::THINKING,
                None,
            )));

        // Record metrics.
        if let Some(metrics) = self.process_table().get_metrics(&agent_id) {
            metrics.record_message();
        }

        // Apply context compaction + build history + append user message
        // inside a single `with_mut` closure to minimize lock duration.
        let user_text = msg.content.as_text();
        let user_msg = ChatMessage::user(&user_text);

        let turn_data = runtimes.with_mut(&agent_id, |rt| {
            // Swap out the conversation for async compaction, then put it
            // back after compaction completes.
            let conversation = std::mem::take(&mut rt.conversation);
            (
                conversation,
                rt.max_context_tokens,
                rt.handle.clone(),
                rt.turn_cancel.clone(),
            )
        });

        let Some((conversation, max_context_tokens, handle, turn_cancel)) = turn_data else {
            warn!(agent_id = %agent_id, "runtime disappeared during LLM turn setup");
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
            let msgs = crate::agent_turn::build_llm_history(&compacted);
            if msgs.is_empty() { None } else { Some(msgs) }
        };

        // Put compacted conversation back and append user message.
        runtimes.with_mut(&agent_id, |rt| {
            rt.conversation = compacted;
            rt.conversation.push(user_msg.clone());
        });

        // Persist in background to avoid blocking event loop.
        {
            let tape = self.tape_for(&session_id);
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
        let stream_handle = self.stream_hub().open(session_id.clone());

        // Clone what we need for the spawned task.
        let event_queue = self.event_queue().clone();
        let stream_id = stream_handle.stream_id().clone();
        let typing_session_id = egress_session_id;
        let stream_hub_ref = Arc::clone(self.stream_hub());

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
            let _span_guard = turn_span.enter();
            let start = std::time::Instant::now();

            // Spawn a background task that refreshes the typing indicator every
            // 4 seconds.  Telegram's `sendChatAction(typing)` expires after ~5s,
            // so we re-send it periodically to keep the indicator visible while
            // the LLM is reasoning.
            let typing_refresh = {
                let eq = event_queue.clone();
                let sid = typing_session_id.clone();
                let usr = user.clone();
                let mid = msg_id.clone();
                tokio::spawn(async move {
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(4));
                    interval.tick().await; // skip the immediate first tick
                    loop {
                        interval.tick().await;
                        let _ = eq.try_push(KernelEvent::deliver(OutboundEnvelope::progress(
                            mid.clone(),
                            usr.clone(),
                            sid.clone(),
                            crate::io::types::stages::THINKING,
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
                agent_id,
                session_id:     session_id.clone(),
                msg_id:         msg_id.clone(),
                user:           user.clone(),
                completed:      false,
            };

            let turn_result = crate::agent_turn::run_inline_agent_loop(
                &handle,
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
                stream_handle.emit(crate::io::stream::StreamEvent::TurnMetrics {
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
            let event = KernelEvent::turn_completed(agent_id, session_id, result, msg_id, user);
            if let Err(e) = event_queue.try_push(event) {
                error!(%e, agent_id = %agent_id, "failed to push TurnCompleted");
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
    #[tracing::instrument(skip_all, fields(agent_id = %agent_id, session_id = %session_id, success, iterations, tool_calls, reply_len))]
    pub(crate) async fn handle_turn_completed(
        &self,
        agent_id: AgentId,
        session_id: SessionId,
        result: std::result::Result<crate::agent_turn::AgentTurnResult, String>,
        in_reply_to: MessageId,
        user: crate::process::principal::UserId,
        runtimes: &RuntimeTable,
    ) {
        let span = tracing::Span::current();

        if self
            .process_table()
            .get(agent_id)
            .map(|process| process.state.is_terminal())
            .unwrap_or(false)
        {
            info!(
                agent_id = %agent_id,
                "ignoring turn completion for terminal process"
            );
            self.cleanup_process(agent_id, runtimes).await;
            return;
        }

        // Determine the egress session: use the channel_session_id if this
        // process has one (root process), otherwise fall back to the
        // process's own session. Subagents without a channel binding won't
        // have egress delivery — their results flow back to the parent via
        // ChildSessionDone.
        let egress_session_id = self
            .process_table()
            .get(agent_id)
            .and_then(|p| p.channel_session_id.clone())
            .unwrap_or_else(|| session_id.clone());

        // Update metrics.
        if let Some(metrics) = self.process_table().get_metrics(&agent_id) {
            metrics.touch().await;
        }

        // Track whether the turn errored so we can choose the right terminal
        // state below (Completed vs Failed).
        let mut turn_failed = false;

        let agent_name = self
            .process_table()
            .get(agent_id)
            .map(|p| p.manifest.name.clone())
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
                self.process_table()
                    .push_turn_trace(agent_id, turn.trace.clone());

                // Record metrics.
                if let Some(metrics) = self.process_table().get_metrics(&agent_id) {
                    metrics.record_llm_call();
                    metrics.record_tool_calls(turn.tool_calls as u64);
                    let estimated_tokens = (turn.text.len() as u64).saturating_div(4).max(1);
                    metrics.record_tokens(estimated_tokens);
                }

                // Persist assistant reply to the process's own session.
                let assistant_msg = ChatMessage::assistant(&turn.text);
                runtimes.with_mut(&agent_id, |rt| {
                    rt.conversation.push(assistant_msg.clone());
                });
                {
                    let tape = self.tape_for(&session_id);
                    if let Err(e) = tape
                        .append_message(
                            serde_json::to_value(&assistant_msg).unwrap_or_default(),
                        )
                        .await
                    {
                        warn!(%e, "failed to persist assistant message to tape");
                    }
                }

                let result = AgentResult {
                    output:     turn.text.clone(),
                    iterations: turn.iterations,
                    tool_calls: turn.tool_calls,
                };
                let _ = self.process_table().set_result(agent_id, result.clone());

                // Push Deliver event for the reply — use egress session for routing.
                let envelope = OutboundEnvelope::reply(
                    in_reply_to,
                    user.clone(),
                    egress_session_id.clone(),
                    crate::channel::types::MessageContent::Text(turn.text),
                    vec![],
                );
                if let Err(e) = self.event_queue().try_push(KernelEvent::deliver(envelope)) {
                    error!(%e, "failed to push Deliver event");
                }

                // Audit: ProcessCompleted
                self.audit().record(AuditEvent {
                    timestamp: jiff::Timestamp::now(),
                    agent_id,
                    session_id: session_id.clone(),
                    user_id: user.clone(),
                    event_type: AuditEventType::ProcessCompleted {
                        result: result.output.clone(),
                    },
                    details: serde_json::json!({
                        "iterations": result.iterations,
                        "tool_calls": result.tool_calls,
                    }),
                });

                info!(
                    agent_id = %agent_id,
                    iterations = result.iterations,
                    tool_calls = result.tool_calls,
                    reply_len = result.output.len(),
                    "turn completed"
                );

                runtimes.with_mut(&agent_id, |rt| {
                    rt.last_result = Some(result);
                });
            }
            Ok(turn) => {
                span.record("success", true);
                span.record("iterations", turn.iterations);
                span.record("tool_calls", turn.tool_calls);
                span.record("reply_len", 0u64);
                info!(agent_id = %agent_id, "turn completed (empty result)");

                // Store turn trace for observability.
                self.process_table()
                    .push_turn_trace(agent_id, turn.trace.clone());

                // Empty result — LLM call was made but produced no text.
                if let Some(metrics) = self.process_table().get_metrics(&agent_id) {
                    metrics.record_llm_call();
                    metrics.record_tool_calls(turn.tool_calls as u64);
                }
            }
            Err(err_msg) => {
                span.record("success", false);
                turn_failed = err_msg != "interrupted by user";
                warn!(agent_id = %agent_id, error = %err_msg, "turn completed (error)");

                if err_msg != "interrupted by user" {
                    self.audit().record(AuditEvent {
                        timestamp: jiff::Timestamp::now(),
                        agent_id,
                        session_id: session_id.clone(),
                        user_id: user.clone(),
                        event_type: AuditEventType::ProcessFailed {
                            error: err_msg.clone(),
                        },
                        details: serde_json::Value::Null,
                    });
                }

                // Deliver error — use egress session for routing.
                let envelope = OutboundEnvelope::error(
                    in_reply_to,
                    user.clone(),
                    egress_session_id.clone(),
                    "agent_error",
                    err_msg,
                );
                if let Err(e) = self.event_queue().try_push(KernelEvent::deliver(envelope)) {
                    error!(%e, "failed to push error Deliver event");
                }
            }
        }

        // Session-centric model: sessions are long-lived. After each turn,
        // the session transitions to Ready (idle) instead of a terminal state.
        // The next user message will be routed to the same session via Path 2.

        // Drain pause buffer — if the user sent messages while the turn was
        // running, re-inject them so they start a new turn on this session.
        let buffered = runtimes.drain_pause_buffer(&agent_id);

        // Transition to Ready (idle, awaiting next message).
        let _ = self
            .process_table()
            .set_state(agent_id, SessionState::Ready);

        // Re-inject buffered events so they trigger a new turn on this session.
        for event in buffered {
            if let Err(e) = self.event_queue().try_push(event) {
                warn!(%e, "failed to re-inject buffered event");
            }
        }
    }
}
