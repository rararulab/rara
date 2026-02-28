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

//! AgentExecutor — processes a single [`InboundMessage`] through the full
//! agent pipeline: session management, process registration, LLM execution,
//! streaming, and outbound publishing.
//!
//! This is the core execution unit spawned by the [`TickLoop`](super::tick::TickLoop)
//! for each scheduled message.

use std::sync::Arc;

use tokio::sync::Semaphore;
use tracing::{error, info, warn};

use crate::io::bus::{InboundBus, OutboundBus, OutboxStore};
use crate::io::scheduler::SessionScheduler;
use crate::io::session_manager::SessionManager;
use crate::io::stream::{StreamEvent, StreamHub};
use crate::io::types::{
    InboundMessage, MessageId, OutboundEnvelope, OutboundPayload, OutboundRouting,
};
use crate::process::principal::Principal;
use crate::process::{
    AgentEnv, AgentId, AgentManifest, AgentProcess, AgentResult, ProcessState, ProcessTable,
    SessionId,
};
use crate::provider::LlmProviderLoaderRef;
use crate::runner::{AgentRunner, RunnerEvent, UserContent};
use crate::tool::ToolRegistry;

use crate::channel::types::MessageContent;

// ---------------------------------------------------------------------------
// AgentExecutor
// ---------------------------------------------------------------------------

/// Processes a single [`InboundMessage`] through the full agent pipeline.
///
/// Pipeline steps:
/// 1. Acquire global semaphore
/// 2. Register in ProcessTable
/// 3. Load history + persist current message
/// 4. Open StreamHandle for ephemeral streaming
/// 5. Build and run AgentRunner (streaming, bridging events)
/// 6. Close stream
/// 7. Handle result (persist reply, publish to OutboundBus)
/// 8. Release semaphore
/// 9. Release session via SessionScheduler, re-publish next if any
pub struct AgentExecutor {
    /// Process table for tracking agent lifecycle.
    process_table: ProcessTable,
    /// Global concurrency limit semaphore.
    global_semaphore: Arc<Semaphore>,
    /// Per-session serial execution scheduler.
    session_scheduler: Arc<SessionScheduler>,
    /// Inbound bus for re-publishing next messages.
    inbound_bus: Arc<dyn InboundBus>,
    /// Outbound bus for publishing final responses.
    outbound_bus: Arc<dyn OutboundBus>,
    /// Durable outbox for fallback on bus failures.
    outbox_store: Arc<dyn OutboxStore>,
    /// Ephemeral stream hub for real-time events.
    stream_hub: Arc<StreamHub>,
    /// Session management (history, persistence).
    session_manager: Arc<SessionManager>,
    /// LLM provider loader.
    llm_provider: LlmProviderLoaderRef,
    /// Tool registry for agent tools.
    tool_registry: Arc<ToolRegistry>,
}

impl AgentExecutor {
    /// Create a new AgentExecutor.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        process_table: ProcessTable,
        global_semaphore: Arc<Semaphore>,
        session_scheduler: Arc<SessionScheduler>,
        inbound_bus: Arc<dyn InboundBus>,
        outbound_bus: Arc<dyn OutboundBus>,
        outbox_store: Arc<dyn OutboxStore>,
        stream_hub: Arc<StreamHub>,
        session_manager: Arc<SessionManager>,
        llm_provider: LlmProviderLoaderRef,
        tool_registry: Arc<ToolRegistry>,
    ) -> Self {
        Self {
            process_table,
            global_semaphore,
            session_scheduler,
            inbound_bus,
            outbound_bus,
            outbox_store,
            stream_hub,
            session_manager,
            llm_provider,
            tool_registry,
        }
    }

    /// Run the full execution pipeline for a single inbound message.
    pub async fn run(&self, msg: InboundMessage) {
        let session_id = msg.session_id.clone();
        let msg_id = msg.id.clone();

        // 1. Acquire global semaphore (graceful on shutdown)
        let permit = match self.global_semaphore.acquire().await {
            Ok(p) => p,
            Err(_) => {
                self.reliable_publish_error(
                    &msg,
                    "system_shutdown",
                    "System is shutting down",
                )
                .await;
                self.release_session(&session_id);
                return;
            }
        };

        // 2. Register in ProcessTable
        let agent_id = AgentId::new();
        let manifest = AgentManifest {
            name: "io-executor".to_string(),
            description: "I/O bus executor agent".to_string(),
            model: "default".to_string(),
            system_prompt: "You are a helpful assistant.".to_string(),
            provider_hint: None,
            max_iterations: Some(25),
            tools: vec![],
            max_children: None,
            metadata: serde_json::Value::Null,
        };

        let process = AgentProcess {
            agent_id,
            parent_id: None,
            session_id: session_id.clone(),
            manifest: manifest.clone(),
            principal: Principal::user(msg.user.0.clone()),
            env: AgentEnv::default(),
            state: ProcessState::Running,
            created_at: jiff::Timestamp::now(),
            finished_at: None,
            result: None,
        };
        self.process_table.insert(process);

        // 3. Ensure session exists + persist current message
        if let Err(e) = self
            .session_manager
            .ensure_session(&session_id, &msg.user)
            .await
        {
            warn!(
                session_id = %session_id,
                error = %e,
                "failed to ensure session, continuing without persistence"
            );
        }

        if let Err(e) = self
            .session_manager
            .append_message(&session_id, &msg)
            .await
        {
            warn!(
                session_id = %session_id,
                error = %e,
                "failed to persist inbound message"
            );
        }

        // 4. Open StreamHandle
        let stream_handle = self.stream_hub.open(session_id.clone());

        // 5. Build and run AgentRunner with streaming
        let user_text = msg.content.as_text();
        let runner = AgentRunner::builder()
            .llm_provider(Arc::clone(&self.llm_provider))
            .model_name("default")
            .system_prompt("You are a helpful assistant.")
            .user_content(UserContent::Text(user_text))
            .max_iterations(25_usize)
            .build();

        let tools = Arc::clone(&self.tool_registry);
        let mut rx = runner.run_streaming(tools);

        // Collect final response text by consuming RunnerEvents
        let mut final_text = String::new();
        let mut got_done = false;

        while let Some(event) = rx.recv().await {
            match event {
                RunnerEvent::TextDelta(delta) => {
                    stream_handle.emit(StreamEvent::TextDelta(delta.clone()));
                    final_text.push_str(&delta);
                }
                RunnerEvent::ReasoningDelta(delta) => {
                    stream_handle.emit(StreamEvent::ReasoningDelta(delta));
                }
                RunnerEvent::ToolCallStart { id, name, .. } => {
                    stream_handle.emit(StreamEvent::ToolCallStart {
                        name: name.clone(),
                        id: id.clone(),
                    });
                }
                RunnerEvent::ToolCallEnd { id, .. } => {
                    stream_handle.emit(StreamEvent::ToolCallEnd { id });
                }
                RunnerEvent::Thinking => {
                    stream_handle.emit(StreamEvent::Progress {
                        stage: "thinking".to_string(),
                    });
                }
                RunnerEvent::Done {
                    text,
                    iterations: _,
                    tool_calls_made: _,
                } => {
                    final_text = text;
                    got_done = true;
                }
                RunnerEvent::Error(err_msg) => {
                    // 6. Close stream
                    self.stream_hub.close(stream_handle.stream_id());

                    // Mark process as failed
                    let _ = self
                        .process_table
                        .set_state(agent_id, ProcessState::Failed);

                    // Publish error
                    self.reliable_publish_error(&msg, "agent_error", &err_msg)
                        .await;

                    // 8. Release semaphore
                    drop(permit);

                    // 9. Release session
                    self.release_session(&session_id);
                    return;
                }
                _ => {
                    // Iteration, ThinkingDone — no bridging needed
                }
            }
        }

        // 6. Close stream
        self.stream_hub.close(stream_handle.stream_id());

        // 7. Handle success result
        if got_done || !final_text.is_empty() {
            // Persist the assistant reply
            if let Err(e) = self
                .session_manager
                .append_assistant_message(&session_id, &final_text)
                .await
            {
                warn!(
                    session_id = %session_id,
                    error = %e,
                    "failed to persist assistant reply"
                );
            }

            // Set process result
            let agent_result = AgentResult {
                output: final_text.clone(),
                iterations: 0,
                tool_calls: 0,
            };
            let _ = self
                .process_table
                .set_state(agent_id, ProcessState::Completed);
            let _ = self.process_table.set_result(agent_id, agent_result);

            // Publish reply
            self.reliable_publish_reply(&msg, final_text).await;

            info!(
                agent_id = %agent_id,
                session_id = %session_id,
                msg_id = %msg_id,
                "agent executor completed"
            );
        } else {
            // No response received — unexpected
            warn!(
                agent_id = %agent_id,
                session_id = %session_id,
                "agent executor produced no response"
            );
            let _ = self
                .process_table
                .set_state(agent_id, ProcessState::Failed);
            self.reliable_publish_error(
                &msg,
                "no_response",
                "Agent produced no response",
            )
            .await;
        }

        // 8. Release semaphore
        drop(permit);

        // 9. Release session, re-publish next to InboundBus
        self.release_session(&session_id);
    }

    /// Reliable publish a reply — fallback to OutboxStore on bus failure.
    async fn reliable_publish_reply(&self, msg: &InboundMessage, content: String) {
        let envelope = OutboundEnvelope {
            id: MessageId::new(),
            in_reply_to: msg.id.clone(),
            user: msg.user.clone(),
            session_id: msg.session_id.clone(),
            routing: OutboundRouting::BroadcastAll,
            payload: OutboundPayload::Reply {
                content: MessageContent::Text(content),
                attachments: vec![],
            },
            timestamp: jiff::Timestamp::now(),
        };

        if let Err(e) = self.outbound_bus.publish(envelope.clone()).await {
            error!(%e, "outbound_bus failed, writing to outbox");
            if let Err(e2) = self.outbox_store.append(envelope).await {
                error!(%e2, "CRITICAL: outbox also failed");
            }
        }
    }

    /// Reliable publish an error — fallback to OutboxStore on bus failure.
    async fn reliable_publish_error(
        &self,
        msg: &InboundMessage,
        code: &str,
        message: &str,
    ) {
        let envelope = OutboundEnvelope {
            id: MessageId::new(),
            in_reply_to: msg.id.clone(),
            user: msg.user.clone(),
            session_id: msg.session_id.clone(),
            routing: OutboundRouting::BroadcastAll,
            payload: OutboundPayload::Error {
                code: code.to_string(),
                message: message.to_string(),
            },
            timestamp: jiff::Timestamp::now(),
        };

        if let Err(e) = self.outbound_bus.publish(envelope.clone()).await {
            error!(%e, "outbound_bus failed on error, writing to outbox");
            if let Err(e2) = self.outbox_store.append(envelope).await {
                error!(%e2, "CRITICAL: outbox also failed on error");
            }
        }
    }

    /// Release the session and re-publish the next queued message.
    fn release_session(&self, session_id: &SessionId) {
        if let Some(next_msg) = self.session_scheduler.release_and_next(session_id) {
            let bus = self.inbound_bus.clone();
            tokio::spawn(async move {
                if let Err(e) = bus.publish(next_msg).await {
                    error!(%e, "failed to re-publish next message to inbound bus");
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::channel::types::ChannelType;
    use crate::io::memory_bus::{InMemoryInboundBus, InMemoryOutboundBus};
    use crate::defaults::noop::{NoopOutboxStore, NoopSessionRepository};
    use crate::io::session_manager::SessionManager;
    use crate::io::types::ChannelSource;
    use crate::process::principal::UserId;
    use crate::provider::EnvLlmProviderLoader;

    /// Helper: build a test InboundMessage.
    fn test_inbound(session: &str, text: &str) -> InboundMessage {
        InboundMessage {
            id: MessageId::new(),
            source: ChannelSource {
                channel_type: ChannelType::Telegram,
                platform_message_id: None,
                platform_user_id: "tg-user".to_string(),
                platform_chat_id: None,
            },
            user: UserId("u1".to_string()),
            session_id: SessionId::new(session),
            content: MessageContent::Text(text.to_string()),
            reply_context: None,
            timestamp: jiff::Timestamp::now(),
            metadata: HashMap::new(),
        }
    }

    /// Helper: create a test executor with stub components.
    fn make_test_executor(
        inbound_bus: Arc<dyn InboundBus>,
        session_scheduler: Arc<SessionScheduler>,
    ) -> AgentExecutor {
        AgentExecutor::new(
            ProcessTable::new(),
            Arc::new(Semaphore::new(16)),
            session_scheduler,
            inbound_bus,
            Arc::new(InMemoryOutboundBus::new(64)),
            Arc::new(NoopOutboxStore),
            Arc::new(StreamHub::new(64)),
            Arc::new(SessionManager::new(
                Arc::new(NoopSessionRepository),
            )),
            Arc::new(EnvLlmProviderLoader::default()) as LlmProviderLoaderRef,
            Arc::new(ToolRegistry::new()),
        )
    }

    #[tokio::test]
    async fn test_executor_release_session_with_next() {
        let inbound_bus = Arc::new(InMemoryInboundBus::new(100));
        let scheduler = Arc::new(SessionScheduler::new(5));

        let executor = make_test_executor(
            inbound_bus.clone() as Arc<dyn InboundBus>,
            scheduler.clone(),
        );

        let session_id = SessionId::new("s1");

        // Schedule first message (becomes Ready, marks slot as running)
        let msg1 = test_inbound("s1", "first");
        let result = scheduler.schedule(msg1);
        assert!(matches!(result, crate::io::scheduler::ScheduleResult::Ready(_)));

        // Schedule second message (becomes Queued)
        let msg2 = test_inbound("s1", "second");
        let result = scheduler.schedule(msg2);
        assert!(matches!(result, crate::io::scheduler::ScheduleResult::Queued));

        // Release session — should re-publish the next message
        executor.release_session(&session_id);

        // Give the spawned task time to publish
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // The next message should be in the inbound bus
        let drained = inbound_bus.drain(10).await;
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].content.as_text(), "second");
    }

    #[tokio::test]
    async fn test_executor_release_session_empty() {
        let inbound_bus = Arc::new(InMemoryInboundBus::new(100));
        let scheduler = Arc::new(SessionScheduler::new(5));

        let executor = make_test_executor(
            inbound_bus.clone() as Arc<dyn InboundBus>,
            scheduler.clone(),
        );

        let session_id = SessionId::new("s1");

        // Schedule one message (Ready)
        let msg1 = test_inbound("s1", "only");
        let _ = scheduler.schedule(msg1);

        // Release with no pending — should NOT publish anything
        executor.release_session(&session_id);

        // Give time
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Bus should be empty
        let drained = inbound_bus.drain(10).await;
        assert!(drained.is_empty());
    }
}
