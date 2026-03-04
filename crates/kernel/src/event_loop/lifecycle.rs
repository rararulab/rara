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
        resume_session_id: Option<SessionId>,
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

        let (session_id, initial_messages) = if let Some(session_id) = resume_session_id {
            let messages = self
                .session_repo()
                .read_messages(&session_id, None, None)
                .await
                .whatever_context::<_, KernelError>("failed to load resumed session history")?;
            (session_id, messages)
        } else {
            let session = self
                .session_repo()
                .create()
                .await
                .whatever_context::<_, KernelError>("failed to create session")?;
            (session.key, vec![])
        };

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
                let was_running = self
                    .process_table()
                    .get(target)
                    .map(|p| p.state == ProcessState::Running)
                    .unwrap_or(false);
                let _ = self
                    .process_table()
                    .set_state(target, ProcessState::Cancelled);
                runtimes.cancel_turn(&target);
                // Grace period then force-kill via process_cancel token.
                if let Some(token) = runtimes.clone_process_cancel(&target) {
                    tokio::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        token.cancel();
                    });
                }
                if !was_running {
                    self.cleanup_process(target, runtimes).await;
                }
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
        if let Some((_, rt)) = runtimes.remove(&agent_id) {
            if let Some(process) = self.process_table().get(agent_id) {
                crate::metrics::PROCESS_ACTIVE
                    .with_label_values(&[&process.manifest.name])
                    .dec();
                crate::metrics::PROCESS_COMPLETED
                    .with_label_values(&[&process.manifest.name, &process.state.to_string()])
                    .inc();
            }

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

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use async_trait::async_trait;
    use chrono::Utc;
    use tokio::sync::{Mutex, Semaphore};

    use super::*;
    use crate::{
        agent_turn::{AgentTurnResult, TurnTrace},
        channel::types::ChatMessage,
        device::DeviceRegistry,
        event_loop::runtime::ProcessRuntime,
        io::{pipe::PipeRegistry, stream::StreamHub},
        kernel::{KernelConfig, NoopSettingsProvider},
        llm::DriverRegistryBuilder,
        memory::NoopMemory,
        notification::NoopNotificationBus,
        process::{
            AgentEnv, AgentProcess, RuntimeMetrics, agent_registry::AgentRegistry,
            principal::Principal,
        },
        session::{
            ChannelBinding, SessionEntry, SessionError, SessionKey, SessionRepoRef,
            SessionRepository,
        },
        tool::ToolRegistry,
    };

    #[derive(Default)]
    struct MemorySessionRepository {
        sessions:  Mutex<HashMap<SessionKey, SessionEntry>>,
        messages:  Mutex<HashMap<SessionKey, Vec<ChatMessage>>>,
        bindings:  Mutex<HashMap<(String, String, String), ChannelBinding>>,
    }

    #[async_trait]
    impl SessionRepository for MemorySessionRepository {
        async fn create_session(
            &self,
            entry: &SessionEntry,
        ) -> std::result::Result<SessionEntry, SessionError> {
            self.sessions
                .lock()
                .await
                .insert(entry.key.clone(), entry.clone());
            self.messages
                .lock()
                .await
                .entry(entry.key.clone())
                .or_default();
            Ok(entry.clone())
        }

        async fn get_session(
            &self,
            key: &SessionKey,
        ) -> std::result::Result<Option<SessionEntry>, SessionError> {
            Ok(self.sessions.lock().await.get(key).cloned())
        }

        async fn list_sessions(
            &self,
            _limit: i64,
            _offset: i64,
        ) -> std::result::Result<Vec<SessionEntry>, SessionError> {
            Ok(self.sessions.lock().await.values().cloned().collect())
        }

        async fn update_session(
            &self,
            entry: &SessionEntry,
        ) -> std::result::Result<SessionEntry, SessionError> {
            self.sessions
                .lock()
                .await
                .insert(entry.key.clone(), entry.clone());
            Ok(entry.clone())
        }

        async fn delete_session(
            &self,
            key: &SessionKey,
        ) -> std::result::Result<(), SessionError> {
            self.sessions.lock().await.remove(key);
            self.messages.lock().await.remove(key);
            Ok(())
        }

        async fn append_message(
            &self,
            session_key: &SessionKey,
            message: &ChatMessage,
        ) -> std::result::Result<ChatMessage, SessionError> {
            let mut stored = message.clone();
            let mut messages = self.messages.lock().await;
            let entry = messages.entry(session_key.clone()).or_default();
            stored.seq = entry.len() as i64 + 1;
            entry.push(stored.clone());
            Ok(stored)
        }

        async fn read_messages(
            &self,
            session_key: &SessionKey,
            after_seq: Option<i64>,
            limit: Option<i64>,
        ) -> std::result::Result<Vec<ChatMessage>, SessionError> {
            let messages = self
                .messages
                .lock()
                .await
                .get(session_key)
                .cloned()
                .unwrap_or_default();
            let filtered = messages
                .into_iter()
                .filter(|msg| match after_seq {
                    Some(seq) => msg.seq > seq,
                    None => true,
                })
                .take(limit.unwrap_or(i64::MAX) as usize)
                .collect();
            Ok(filtered)
        }

        async fn clear_messages(
            &self,
            session_key: &SessionKey,
        ) -> std::result::Result<(), SessionError> {
            self.messages
                .lock()
                .await
                .insert(session_key.clone(), Vec::new());
            Ok(())
        }

        async fn fork_session(
            &self,
            source_key: &SessionKey,
            _fork_at_seq: i64,
        ) -> std::result::Result<SessionEntry, SessionError> {
            let now = Utc::now();
            let entry = SessionEntry {
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
            self.create_session(&entry).await?;
            let messages = self.read_messages(source_key, None, None).await?;
            for msg in messages {
                let _ = self.append_message(&entry.key, &msg).await?;
            }
            Ok(entry)
        }

        async fn bind_channel(
            &self,
            binding: &ChannelBinding,
        ) -> std::result::Result<ChannelBinding, SessionError> {
            self.bindings.lock().await.insert(
                (
                    binding.channel_type.clone(),
                    binding.account.clone(),
                    binding.chat_id.clone(),
                ),
                binding.clone(),
            );
            Ok(binding.clone())
        }

        async fn get_channel_binding(
            &self,
            channel_type: &str,
            account: &str,
            chat_id: &str,
        ) -> std::result::Result<Option<ChannelBinding>, SessionError> {
            Ok(self
                .bindings
                .lock()
                .await
                .get(&(channel_type.to_string(), account.to_string(), chat_id.to_string()))
                .cloned())
        }
    }

    fn test_kernel(session_repo: SessionRepoRef) -> Kernel {
        let config = KernelConfig {
            max_concurrency: 4,
            default_child_limit: 2,
            default_max_iterations: 5,
            memory_quota_per_agent: 1000,
            ..Default::default()
        };
        let driver_registry = Arc::new(DriverRegistryBuilder::new("test", "test-model").build());
        Kernel::for_testing(
            config,
            Arc::new(crate::process::ProcessTable::new()),
            Arc::new(Semaphore::new(4)),
            driver_registry,
            Arc::new(ToolRegistry::new()),
            Arc::new(NoopMemory),
            Arc::new(NoopNotificationBus),
            Arc::new(crate::security::SecuritySubsystem::noop()),
            Arc::new(AgentRegistry::new(
                crate::testing::test_manifests(),
                std::env::temp_dir().join("kernel_lifecycle_tests"),
            )),
            Arc::new(crate::audit::AuditSubsystem::noop()),
            session_repo,
            Arc::new(NoopSettingsProvider),
            Arc::new(StreamHub::new(16)),
            PipeRegistry::new(),
            Arc::new(DeviceRegistry::new()),
        )
    }

    async fn insert_runtime(kernel: &Kernel, runtimes: &RuntimeTable, state: ProcessState) -> AgentId {
        let manifest = crate::testing::test_manifests()
            .into_iter()
            .next()
            .expect("test manifest");
        let agent_id = AgentId::new();
        let session_id = SessionId::new();

        kernel.process_table().insert(AgentProcess {
            agent_id,
            parent_id: None,
            session_id: session_id.clone(),
            channel_session_id: Some(SessionId::new()),
            manifest,
            principal: Principal::user("user"),
            env: AgentEnv::default(),
            state,
            created_at: jiff::Timestamp::now(),
            finished_at: None,
            result: None,
            created_files: vec![],
            metrics: Arc::new(RuntimeMetrics::new()),
            turn_traces: vec![],
        });

        let permit = Arc::new(Semaphore::new(1))
            .try_acquire_owned()
            .expect("permit");
        let handle = Arc::new(ProcessHandle::new(
            agent_id,
            session_id,
            Principal::user("user"),
            kernel.event_queue().clone(),
        ));
        runtimes.insert(
            agent_id,
            ProcessRuntime {
                conversation: vec![],
                turn_cancel: CancellationToken::new(),
                process_cancel: CancellationToken::new(),
                paused: false,
                pause_buffer: vec![],
                handle,
                child_semaphore: Arc::new(Semaphore::new(1)),
                max_context_tokens: 1024,
                last_result: None,
                _global_permit: permit,
            },
        );

        agent_id
    }

    #[tokio::test]
    async fn spawn_agent_reuses_existing_session_history() {
        let session_repo = Arc::new(MemorySessionRepository::default());
        let kernel = test_kernel(session_repo.clone());
        let runtimes = RuntimeTable::new();
        let resumed_session = session_repo.create().await.expect("create session").key;
        session_repo
            .append_message(&resumed_session, &ChatMessage::user("hello"))
            .await
            .expect("append user");
        session_repo
            .append_message(&resumed_session, &ChatMessage::assistant("world"))
            .await
            .expect("append assistant");

        let agent_id = kernel
            .handle_spawn_agent(
                crate::testing::test_manifests()
                    .into_iter()
                    .next()
                    .expect("test manifest"),
                "next".to_string(),
                Principal::user("user"),
                Some(SessionId::new()),
                None,
                Some(resumed_session.clone()),
                &runtimes,
            )
            .await
            .expect("spawn");

        let process = kernel.process_table().get(agent_id).expect("process");
        assert_eq!(process.session_id, resumed_session);
        let conversation = runtimes
            .with(&agent_id, |rt| rt.conversation.clone())
            .expect("runtime conversation");
        assert_eq!(conversation.len(), 2);
        assert_eq!(conversation[0].content.as_text(), "hello");
        assert_eq!(conversation[1].content.as_text(), "world");
    }

    #[tokio::test]
    async fn terminate_marks_running_process_terminal_and_defers_cleanup() {
        let kernel = test_kernel(Arc::new(crate::session::NoopSessionRepository));
        let runtimes = RuntimeTable::new();
        let agent_id = insert_runtime(&kernel, &runtimes, ProcessState::Running).await;

        kernel
            .handle_signal(agent_id, Signal::Terminate, &runtimes)
            .await;

        assert_eq!(
            kernel.process_table().get(agent_id).expect("process").state,
            ProcessState::Cancelled
        );
        assert!(runtimes.contains(&agent_id));

        kernel
            .handle_turn_completed(
                agent_id,
                SessionId::new(),
                Ok(AgentTurnResult {
                    text:       "ignored".to_string(),
                    iterations: 1,
                    tool_calls: 0,
                    model:      "test".to_string(),
                    trace:      TurnTrace {
                        duration_ms:      0,
                        model:            "test".to_string(),
                        input_text:       None,
                        iterations:       vec![],
                        final_text_len:   0,
                        total_tool_calls: 0,
                        success:          true,
                        error:            None,
                    },
                }),
                crate::io::types::MessageId::new(),
                crate::process::principal::UserId("user".to_string()),
                &runtimes,
            )
            .await;

        assert_eq!(
            kernel.process_table().get(agent_id).expect("process").state,
            ProcessState::Cancelled
        );
        assert!(!runtimes.contains(&agent_id));
    }
}
