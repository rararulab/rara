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

//! Agent process lifecycle — spawning, signal handling, child completion,
//! and cleanup.

use std::sync::Arc;

use snafu::ResultExt;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use super::runtime::{ProcessRuntime, RuntimeTable};
use crate::{
    audit::{AuditEvent, AuditEventType},
    error::{KernelError, Result},
    event::KernelEvent,
    handle::process_handle::ProcessHandle,
    io::types::{InboundMessage, MessageId, OutboundEnvelope},
    kernel::Kernel,
    process::{
        AgentEnv, AgentId, AgentManifest, AgentProcess, AgentResult, ProcessState, SessionId,
        Signal, principal::Principal,
    },
};

impl Kernel {
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
    #[tracing::instrument(skip_all, fields(manifest_name = %manifest.name, parent_id = ?parent_id, agent_id))]
    pub(crate) async fn handle_spawn_agent(
        &self,
        manifest: AgentManifest,
        input: String,
        principal: Principal,
        channel_session_id: Option<SessionId>,
        parent_id: Option<AgentId>,
        runtimes: &RuntimeTable,
    ) -> Result<AgentId> {
        // Validate principal.
        self.security().validate_principal(&principal).await?;

        // Acquire global semaphore.
        let global_permit = self
            .global_semaphore()
            .clone()
            .try_acquire_owned()
            .map_err(|_| KernelError::SpawnLimitReached {
                message: "global concurrency limit reached".to_string(),
            })?;

        let agent_id = AgentId::new();
        tracing::Span::current().record("agent_id", tracing::field::display(&agent_id));

        // Each process gets its own session — context isolation.
        let session = self
            .session_repo()
            .create()
            .await
            .whatever_context::<_, KernelError>("failed to create session")?;
        let session_id = session.key;
        // Clean start: no loaded history. Task input arrives as synthetic
        // message (below) or is injected directly into the conversation.
        let initial_messages = vec![];

        // Audit: ProcessSpawned
        self.audit().record(AuditEvent {
            timestamp: jiff::Timestamp::now(),
            agent_id,
            session_id: session_id.clone(),
            user_id: principal.user_id.clone(),
            event_type: AuditEventType::ProcessSpawned {
                manifest_name: manifest.name.clone(),
                parent_id,
            },
            details: serde_json::json!({
                "model": manifest.model,
                "max_iterations": manifest.max_iterations,
            }),
        });

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
            state: ProcessState::Idle,
            created_at: jiff::Timestamp::now(),
            finished_at: None,
            result: None,
            created_files: vec![],
            metrics,
            turn_traces: vec![],
        };
        self.process_table().insert(process);

        crate::metrics::PROCESS_SPAWNED
            .with_label_values(&[&manifest.name])
            .inc();
        crate::metrics::PROCESS_ACTIVE
            .with_label_values(&[&manifest.name])
            .inc();

        // Create process-level cancellation token.
        // Child processes derive their token from the parent's, so cancelling
        // a parent cascades to all children automatically.
        let process_cancel = if let Some(pid) = parent_id {
            runtimes
                .with(&pid, |parent_rt| parent_rt.process_cancel.child_token())
                .unwrap_or_default()
        } else {
            CancellationToken::new()
        };

        // Build ProcessHandle — uses the process's own session.
        let child_limit = manifest
            .max_children
            .unwrap_or(self.config().default_child_limit);

        let handle = Arc::new(ProcessHandle::new(
            agent_id,
            session_id.clone(),
            principal.clone(),
            self.event_queue().clone(),
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
            principal.user_id.clone(),
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
    #[tracing::instrument(skip_all, fields(agent_id = %target, signal = ?signal))]
    pub(crate) async fn handle_signal(
        &self,
        target: AgentId,
        signal: Signal,
        runtimes: &RuntimeTable,
    ) {
        match signal {
            Signal::Interrupt => {
                info!(agent_id = %target, "interrupt signal");
                runtimes.cancel_and_refresh_turn(&target);
                // Notify via Deliver event — use channel session for egress.
                let session_id = self
                    .process_table()
                    .get(target)
                    .and_then(|p| p.channel_session_id.clone());
                let Some(session_id) = session_id else {
                    error!(agent_id = %target, "cannot send interrupt notification: process not found or has no channel session");
                    return;
                };
                let envelope = OutboundEnvelope::state_change(
                    MessageId::new(),
                    crate::process::principal::UserId("system".to_string()),
                    session_id,
                    "interrupted",
                    serde_json::json!({
                        "agent_id": target.to_string(),
                        "message": "Agent interrupted by user",
                    }),
                );
                if let Err(e) = self.event_queue().try_push(KernelEvent::Deliver(envelope)) {
                    error!(%e, "failed to push interrupt notification");
                }
            }
            Signal::Pause => {
                info!(agent_id = %target, "pause signal");
                runtimes.set_paused(&target, true);
                let _ = self.process_table().set_state(target, ProcessState::Paused);
            }
            Signal::Resume => {
                info!(agent_id = %target, "resume signal");
                runtimes.set_paused(&target, false);
                let buffered = runtimes.drain_pause_buffer(&target);
                let _ = self.process_table().set_state(target, ProcessState::Idle);
                for event in buffered {
                    if let Err(e) = self.event_queue().try_push(event) {
                        warn!(%e, "failed to re-inject buffered event on resume");
                    }
                }
            }
            Signal::Terminate => {
                info!(agent_id = %target, "terminate signal — graceful shutdown");
                runtimes.cancel_turn(&target);
                // Grace period then force-kill via process_cancel token.
                if let Some(token) = runtimes.clone_process_cancel(&target) {
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
                runtimes.cancel_process(&target);
                let _ = self
                    .process_table()
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
    #[tracing::instrument(skip_all, fields(parent_id = %parent_id, child_id = %child_id, output_len = result.output.len()))]
    pub(crate) async fn handle_child_completed(
        &self,
        parent_id: AgentId,
        child_id: AgentId,
        result: AgentResult,
        runtimes: &RuntimeTable,
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

        runtimes.with_mut(&parent_id, |rt| {
            rt.conversation.push(child_msg.clone());
        });

        let Some(session_id) = self
            .process_table()
            .get(parent_id)
            .map(|p| p.session_id.clone())
        else {
            error!(parent_id = %parent_id, child_id = %child_id, "cannot persist child result: parent process not found");
            return;
        };

        if let Err(e) = self
            .session_repo()
            .append_message(&session_id, &child_msg)
            .await
        {
            warn!(%e, "failed to persist child result message");
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Clean up a process runtime entry.
    ///
    /// Removing the runtime from the table drops the `process_cancel` token
    /// naturally, so no explicit cancellation-token cleanup is needed.
    pub(crate) async fn cleanup_process(&self, agent_id: AgentId, runtimes: &RuntimeTable) {
        if let Some(process) = self.process_table().get(agent_id) {
            crate::metrics::PROCESS_ACTIVE
                .with_label_values(&[&process.manifest.name])
                .dec();
            crate::metrics::PROCESS_COMPLETED
                .with_label_values(&[&process.manifest.name, &process.state.to_string()])
                .inc();
        }

        let rt = runtimes.remove(&agent_id);
        if let Some((_, rt)) = rt {
            // Notify parent if this is a child process.
            if let Some(process) = self.process_table().get(agent_id) {
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
}
