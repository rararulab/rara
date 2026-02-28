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

//! process_loop — the long-lived event loop for a single agent process.
//!
//! Each agent process runs a `process_loop` that receives messages from
//! its mailbox (`mpsc::Receiver<ProcessMessage>`), processes them through
//! the LLM pipeline, and publishes results to the outbound bus.
//!
//! The loop continues until:
//! - A `Signal::Kill` is received
//! - The mailbox sender is dropped (all handles gone)
//!
//! This replaces the short-lived per-message `AgentExecutor` model with
//! a long-lived per-session process model.

use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::channel::types::MessageContent;
use crate::io::bus::OutboundBus;
use crate::io::stream::{StreamEvent, StreamHub};
use crate::io::types::{MessageId, OutboundEnvelope, OutboundPayload, OutboundRouting};
use crate::process::{
    AgentId, AgentManifest, AgentResult, ProcessMessage, ProcessState, ProcessTable, SessionId,
    Signal,
};
use crate::provider::LlmProviderLoaderRef;
use crate::runner::{AgentRunner, RunnerEvent, UserContent};
use crate::session_manager::SessionManager;
use crate::tool::ToolRegistry;

/// Run the long-lived process loop for an agent.
///
/// This function receives messages from the mailbox and processes each one
/// through the full LLM pipeline (history loading, streaming execution,
/// result publishing). It runs until a Kill signal is received or the
/// mailbox sender is dropped.
#[allow(clippy::too_many_arguments)]
pub async fn process_loop(
    agent_id: AgentId,
    session_id: SessionId,
    manifest: AgentManifest,
    mut mailbox: mpsc::Receiver<ProcessMessage>,
    process_table: Arc<ProcessTable>,
    session_manager: Arc<SessionManager>,
    stream_hub: Arc<StreamHub>,
    outbound_bus: Arc<dyn OutboundBus>,
    llm_provider: LlmProviderLoaderRef,
    tool_registry: Arc<ToolRegistry>,
) {
    info!(agent_id = %agent_id, session_id = %session_id, "process loop started");

    while let Some(msg) = mailbox.recv().await {
        match msg {
            ProcessMessage::UserMessage(inbound) => {
                // Mark as Running
                let _ = process_table.set_state(agent_id, ProcessState::Running);

                // 1. Persist user message
                if let Err(e) = session_manager
                    .append_message(&session_id, &inbound)
                    .await
                {
                    warn!(error = %e, "failed to persist inbound message");
                }

                // 2. Load history
                // TODO: ChatMessage -> ChatCompletionRequestMessage conversion
                // is not yet implemented. History is skipped for now.
                let history = None;

                // 3. Open stream
                let stream_handle = stream_hub.open(session_id.clone());

                // 4. Build Runner
                let user_text = inbound.content.as_text();
                let runner = {
                    let b = AgentRunner::builder()
                        .llm_provider(Arc::clone(&llm_provider))
                        .model_name(manifest.model.clone())
                        .system_prompt(manifest.system_prompt.clone())
                        .user_content(UserContent::Text(user_text))
                        .max_iterations(manifest.max_iterations.unwrap_or(25))
                        .maybe_provider_hint(manifest.provider_hint.clone())
                        .maybe_history(history);
                    b.build()
                };
                let tools = Arc::clone(&tool_registry);
                let mut rx = runner.run_streaming(tools);

                // 5. Consume RunnerEvent stream
                let mut final_text = String::new();
                let mut got_done = false;
                let mut iterations = 0usize;
                let mut tool_calls_made = 0usize;
                let mut had_error = false;

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
                            iterations: iters,
                            tool_calls_made: tcs,
                        } => {
                            final_text = text;
                            iterations = iters;
                            tool_calls_made = tcs;
                            got_done = true;
                        }
                        RunnerEvent::Error(err_msg) => {
                            stream_hub.close(stream_handle.stream_id());
                            let _ =
                                process_table.set_state(agent_id, ProcessState::Failed);
                            // Publish error to outbound
                            let envelope = OutboundEnvelope {
                                id: MessageId::new(),
                                in_reply_to: inbound.id.clone(),
                                user: inbound.user.clone(),
                                session_id: session_id.clone(),
                                routing: OutboundRouting::BroadcastAll,
                                payload: OutboundPayload::Error {
                                    code: "agent_error".to_string(),
                                    message: err_msg,
                                },
                                timestamp: jiff::Timestamp::now(),
                            };
                            if let Err(e) = outbound_bus.publish(envelope).await {
                                error!(%e, "failed to publish error");
                            }
                            had_error = true;
                            break;
                        }
                        _ => {}
                    }
                }

                if had_error {
                    // Reset state to Waiting so the process stays alive for
                    // the next message.
                    let _ = process_table.set_state(agent_id, ProcessState::Waiting);
                    continue;
                }

                // 6. Close stream
                stream_hub.close(stream_handle.stream_id());

                // 7. Handle result
                if got_done || !final_text.is_empty() {
                    // Persist reply
                    if let Err(e) = session_manager
                        .append_assistant_message(&session_id, &final_text)
                        .await
                    {
                        warn!(error = %e, "failed to persist assistant reply");
                    }

                    // Update process result
                    let result = AgentResult {
                        output: final_text.clone(),
                        iterations,
                        tool_calls: tool_calls_made,
                    };
                    let _ = process_table.set_result(agent_id, result);

                    // Publish reply
                    let envelope = OutboundEnvelope {
                        id: MessageId::new(),
                        in_reply_to: inbound.id.clone(),
                        user: inbound.user.clone(),
                        session_id: session_id.clone(),
                        routing: OutboundRouting::BroadcastAll,
                        payload: OutboundPayload::Reply {
                            content: MessageContent::Text(final_text),
                            attachments: vec![],
                        },
                        timestamp: jiff::Timestamp::now(),
                    };
                    if let Err(e) = outbound_bus.publish(envelope).await {
                        error!(%e, "failed to publish reply");
                    }

                    info!(
                        agent_id = %agent_id,
                        iterations,
                        tool_calls = tool_calls_made,
                        "message processed"
                    );
                }

                // Return to Waiting state for next message
                let _ = process_table.set_state(agent_id, ProcessState::Waiting);
            }
            ProcessMessage::ChildResult { child_id, result } => {
                info!(
                    agent_id = %agent_id,
                    child_id = %child_id,
                    output_len = result.output.len(),
                    "child result received"
                );
                // TODO: integrate child result into current context
            }
            ProcessMessage::Signal(Signal::Interrupt) => {
                warn!(agent_id = %agent_id, "interrupt received");
                // TODO: cancel current LLM call
            }
            ProcessMessage::Signal(Signal::Kill) => {
                info!(agent_id = %agent_id, "kill signal received");
                break;
            }
        }
    }

    // Process ended — set terminal state
    let _ = process_table.set_state(agent_id, ProcessState::Completed);
    info!(agent_id = %agent_id, session_id = %session_id, "process loop ended");
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use tokio::sync::mpsc;

    use super::*;
    use crate::channel::types::{ChannelType, MessageContent};
    use crate::defaults::noop::{NoopSessionRepository};
    use crate::io::memory_bus::InMemoryOutboundBus;
    use crate::io::stream::StreamHub;
    use crate::io::types::{ChannelSource, InboundMessage, MessageId};
    use crate::process::principal::UserId;
    use crate::process::{
        AgentEnv, AgentManifest, AgentProcess, ProcessState, ProcessTable, SessionId,
    };
    use crate::process::principal::Principal;
    use crate::session_manager::SessionManager;

    fn test_manifest() -> AgentManifest {
        AgentManifest {
            name: "test-agent".to_string(),
            description: "Test agent".to_string(),
            model: "test-model".to_string(),
            system_prompt: "You are a test agent.".to_string(),
            provider_hint: None,
            max_iterations: Some(5),
            tools: vec![],
            max_children: None,
            metadata: serde_json::Value::Null,
        }
    }

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

    #[tokio::test]
    async fn test_process_loop_kill_signal() {
        let agent_id = AgentId::new();
        let session_id = SessionId::new("test-session");
        let manifest = test_manifest();

        let process_table = Arc::new(ProcessTable::new());
        let process = AgentProcess {
            agent_id,
            parent_id: None,
            session_id: session_id.clone(),
            manifest: manifest.clone(),
            principal: Principal::user("test-user"),
            env: AgentEnv::default(),
            state: ProcessState::Running,
            created_at: jiff::Timestamp::now(),
            finished_at: None,
            result: None,
        };
        process_table.insert(process);

        let session_manager = Arc::new(SessionManager::new(
            Arc::new(NoopSessionRepository),
        ));
        let stream_hub = Arc::new(StreamHub::new(16));
        let outbound_bus = Arc::new(InMemoryOutboundBus::new(64));
        let llm_provider = Arc::new(crate::provider::EnvLlmProviderLoader::default())
            as crate::provider::LlmProviderLoaderRef;
        let tool_registry = Arc::new(ToolRegistry::new());

        let (tx, rx) = mpsc::channel(16);

        let pt = process_table.clone();
        let handle = tokio::spawn(process_loop(
            agent_id,
            session_id.clone(),
            manifest,
            rx,
            pt,
            session_manager,
            stream_hub,
            outbound_bus as Arc<dyn OutboundBus>,
            llm_provider,
            tool_registry,
        ));

        // Send a Kill signal
        tx.send(ProcessMessage::Signal(Signal::Kill))
            .await
            .unwrap();

        // The loop should exit
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            handle,
        )
        .await;

        assert!(result.is_ok(), "process loop should exit on Kill signal");
        assert!(result.unwrap().is_ok());

        // Process should be Completed
        let p = process_table.get(agent_id).unwrap();
        assert_eq!(p.state, ProcessState::Completed);
    }

    #[tokio::test]
    async fn test_process_loop_mailbox_dropped() {
        let agent_id = AgentId::new();
        let session_id = SessionId::new("test-session");
        let manifest = test_manifest();

        let process_table = Arc::new(ProcessTable::new());
        let process = AgentProcess {
            agent_id,
            parent_id: None,
            session_id: session_id.clone(),
            manifest: manifest.clone(),
            principal: Principal::user("test-user"),
            env: AgentEnv::default(),
            state: ProcessState::Running,
            created_at: jiff::Timestamp::now(),
            finished_at: None,
            result: None,
        };
        process_table.insert(process);

        let session_manager = Arc::new(SessionManager::new(
            Arc::new(NoopSessionRepository),
        ));
        let stream_hub = Arc::new(StreamHub::new(16));
        let outbound_bus = Arc::new(InMemoryOutboundBus::new(64));
        let llm_provider = Arc::new(crate::provider::EnvLlmProviderLoader::default())
            as crate::provider::LlmProviderLoaderRef;
        let tool_registry = Arc::new(ToolRegistry::new());

        let (tx, rx) = mpsc::channel(16);

        let pt = process_table.clone();
        let handle = tokio::spawn(process_loop(
            agent_id,
            session_id.clone(),
            manifest,
            rx,
            pt,
            session_manager,
            stream_hub,
            outbound_bus as Arc<dyn OutboundBus>,
            llm_provider,
            tool_registry,
        ));

        // Drop the sender — mailbox closes
        drop(tx);

        // The loop should exit
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            handle,
        )
        .await;

        assert!(result.is_ok(), "process loop should exit when mailbox is dropped");
        assert!(result.unwrap().is_ok());
    }

    #[tokio::test]
    async fn test_process_loop_child_result() {
        let agent_id = AgentId::new();
        let session_id = SessionId::new("test-session");
        let manifest = test_manifest();

        let process_table = Arc::new(ProcessTable::new());
        let process = AgentProcess {
            agent_id,
            parent_id: None,
            session_id: session_id.clone(),
            manifest: manifest.clone(),
            principal: Principal::user("test-user"),
            env: AgentEnv::default(),
            state: ProcessState::Running,
            created_at: jiff::Timestamp::now(),
            finished_at: None,
            result: None,
        };
        process_table.insert(process);

        let session_manager = Arc::new(SessionManager::new(
            Arc::new(NoopSessionRepository),
        ));
        let stream_hub = Arc::new(StreamHub::new(16));
        let outbound_bus = Arc::new(InMemoryOutboundBus::new(64));
        let llm_provider = Arc::new(crate::provider::EnvLlmProviderLoader::default())
            as crate::provider::LlmProviderLoaderRef;
        let tool_registry = Arc::new(ToolRegistry::new());

        let (tx, rx) = mpsc::channel(16);

        let pt = process_table.clone();
        let handle = tokio::spawn(process_loop(
            agent_id,
            session_id.clone(),
            manifest,
            rx,
            pt,
            session_manager,
            stream_hub,
            outbound_bus as Arc<dyn OutboundBus>,
            llm_provider,
            tool_registry,
        ));

        // Send a child result followed by kill
        let child_id = AgentId::new();
        tx.send(ProcessMessage::ChildResult {
            child_id,
            result: AgentResult {
                output: "child done".to_string(),
                iterations: 1,
                tool_calls: 0,
            },
        })
        .await
        .unwrap();

        tx.send(ProcessMessage::Signal(Signal::Kill))
            .await
            .unwrap();

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            handle,
        )
        .await;

        assert!(result.is_ok());
        assert!(result.unwrap().is_ok());
    }
}
