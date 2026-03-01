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

//! Agent execution — the "user-space" program that runs inside a kernel
//! process.
//!
//! [`run_agent_turn`] is the agent's main function for processing a single
//! message. It builds an [`AgentRunner`], executes the LLM loop, forwards
//! stream events, and returns the result. It knows nothing about process
//! lifecycle, state management, persistence, or outbound publishing — those
//! are the kernel's responsibilities.

use std::sync::Arc;

use async_openai::types::chat::ChatCompletionRequestMessage;

use crate::{
    handle::scoped::ScopedKernelHandle,
    io::stream::{StreamEvent, StreamHandle},
    runner::{AgentRunner, RunnerEvent, UserContent},
};

/// Result of a single agent turn.
pub(crate) struct AgentTurnResult {
    /// The final text produced by the agent.
    pub text:       String,
    /// Number of LLM iterations consumed.
    pub iterations: usize,
    /// Number of tool calls executed.
    pub tool_calls: usize,
}

/// Execute a single agent turn: build the LLM runner, stream events, and
/// return the result.
///
/// This is the agent's "program" — it only knows how to run the LLM loop
/// with the manifest's configuration. All kernel concerns (state, persistence,
/// streams lifecycle, outbound) are handled by the caller.
pub(crate) async fn run_agent_turn(
    handle: &ScopedKernelHandle,
    user_text: String,
    history: Option<Vec<ChatCompletionRequestMessage>>,
    stream_handle: &StreamHandle,
) -> Result<AgentTurnResult, String> {
    let runner = {
        let b = AgentRunner::builder()
            .llm_provider(Arc::clone(handle.llm_provider()))
            .model_name(handle.manifest().model.clone())
            .system_prompt(handle.manifest().system_prompt.clone())
            .user_content(UserContent::Text(user_text))
            .max_iterations(handle.manifest().max_iterations.unwrap_or(25))
            .maybe_provider_hint(handle.manifest().provider_hint.clone())
            .maybe_history(history);
        b.build()
    };
    let tools = Arc::clone(handle.tool_registry());
    let mut rx = runner.run_streaming(tools);

    let mut final_text = String::new();
    let mut got_done = false;
    let mut iterations = 0usize;
    let mut tool_calls = 0usize;

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
                    id:   id.clone(),
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
                tool_calls = tcs;
                got_done = true;
            }
            RunnerEvent::Error(err_msg) => {
                return Err(err_msg);
            }
            _ => {}
        }
    }

    if got_done || !final_text.is_empty() {
        Ok(AgentTurnResult {
            text: final_text,
            iterations,
            tool_calls,
        })
    } else {
        Ok(AgentTurnResult {
            text:       String::new(),
            iterations: 0,
            tool_calls: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_openai::types::chat::{
        ChatChoiceStream, ChatCompletionResponseStream, ChatCompletionStreamResponseDelta,
        CreateChatCompletionRequest, CreateChatCompletionStreamResponse, FinishReason,
    };
    use async_trait::async_trait;
    use dashmap::DashMap;
    use futures::stream;
    use tokio::sync::Semaphore;

    use super::*;
    use crate::{
        defaults::{
            noop::{NoopEventBus, NoopGuard, NoopMemory, NoopSessionRepository},
            noop_user_store::NoopUserStore,
        },
        io::{memory_bus::InMemoryOutboundBus, stream::StreamHub},
        kernel::KernelInner,
        process::{
            AgentEnv, AgentId, AgentManifest, AgentProcess, ProcessState, ProcessTable, SessionId,
            manifest_loader::ManifestLoader,
            principal::Principal,
        },
        provider::{LlmProvider, LlmProviderLoader, LlmProviderLoaderRef},
        session::SessionRepository,
        tool::ToolRegistry,
    };

    fn test_manifest() -> AgentManifest {
        AgentManifest {
            name:           "test-agent".to_string(),
            description:    "Test agent".to_string(),
            model:          "test-model".to_string(),
            system_prompt:  "You are a test agent.".to_string(),
            provider_hint:  None,
            max_iterations: Some(5),
            tools:          vec![],
            max_children:        None,
            max_context_tokens:  None,
            priority:            crate::process::Priority::default(),
            metadata:            serde_json::Value::Null,
        }
    }

    fn make_test_inner(llm_provider: LlmProviderLoaderRef) -> Arc<KernelInner> {
        Arc::new(KernelInner {
            process_table:          Arc::new(ProcessTable::new()),
            global_semaphore:       Arc::new(Semaphore::new(10)),
            default_child_limit:    5,
            default_max_iterations: 25,
            llm_provider,
            tool_registry:          Arc::new(ToolRegistry::new()),
            memory:                 Arc::new(NoopMemory),
            event_bus:              Arc::new(NoopEventBus),
            guard:                  Arc::new(NoopGuard),
            manifest_loader:        ManifestLoader::new(),
            shared_kv:              DashMap::new(),
            user_store:             Arc::new(NoopUserStore),
            session_repo:           Arc::new(NoopSessionRepository) as Arc<dyn SessionRepository>,
            model_repo:             Arc::new(crate::defaults::noop::NoopModelRepo)
                as Arc<dyn crate::model_repo::ModelRepo>,
            stream_hub:             Arc::new(StreamHub::new(16)),
            outbound_bus:           Arc::new(InMemoryOutboundBus::new(64))
                as Arc<dyn crate::io::bus::OutboundBus>,
        })
    }

    fn setup_handle(inner: &Arc<KernelInner>) -> Arc<ScopedKernelHandle> {
        let agent_id = AgentId::new();
        let session_id = SessionId::new("test-session");
        let manifest = test_manifest();

        let process = AgentProcess {
            agent_id,
            parent_id:   None,
            session_id:  session_id.clone(),
            manifest:    manifest.clone(),
            principal:   Principal::user("test-user"),
            env:         AgentEnv::default(),
            state:       ProcessState::Running,
            created_at:  jiff::Timestamp::now(),
            finished_at: None,
            result:      None,
        };
        inner.process_table.insert(process);

        Arc::new(ScopedKernelHandle {
            agent_id,
            session_id,
            principal:       Principal::user("test-user"),
            manifest,
            allowed_tools:   vec![],
            tool_registry:   Arc::new(ToolRegistry::new()),
            child_semaphore: Arc::new(Semaphore::new(5)),
            inner:           Arc::clone(inner),
        })
    }

    #[derive(Default)]
    struct StubStreamingProvider {
        message_counts: Mutex<Vec<usize>>,
    }

    #[async_trait]
    impl LlmProvider for StubStreamingProvider {
        async fn chat_completion(
            &self,
            _request: CreateChatCompletionRequest,
        ) -> crate::error::Result<async_openai::types::chat::CreateChatCompletionResponse> {
            Err(crate::error::KernelError::Other {
                message: "not supported".into(),
            })
        }

        #[allow(deprecated)]
        async fn chat_completion_stream(
            &self,
            request: CreateChatCompletionRequest,
        ) -> crate::error::Result<ChatCompletionResponseStream> {
            self.message_counts
                .lock()
                .expect("lock")
                .push(request.messages.len());

            let chunk = CreateChatCompletionStreamResponse {
                id:                 "resp_1".to_string(),
                choices:            vec![ChatChoiceStream {
                    index:         0,
                    delta:         ChatCompletionStreamResponseDelta {
                        content:       Some("test reply".to_string()),
                        function_call: None,
                        tool_calls:    None,
                        role:          None,
                        refusal:       None,
                    },
                    finish_reason: Some(FinishReason::Stop),
                    logprobs:      None,
                }],
                created:            0,
                model:              "test-model".to_string(),
                service_tier:       None,
                system_fingerprint: None,
                object:             "chat.completion.chunk".to_string(),
                usage:              None,
            };

            Ok(Box::pin(stream::iter(vec![Ok(chunk)])))
        }
    }

    #[derive(Clone)]
    struct StubProviderLoader {
        provider: Arc<dyn LlmProvider>,
    }

    #[async_trait]
    impl LlmProviderLoader for StubProviderLoader {
        async fn acquire_provider(&self) -> crate::error::Result<Arc<dyn LlmProvider>> {
            Ok(Arc::clone(&self.provider))
        }
    }

    #[tokio::test]
    async fn test_run_agent_turn_basic() {
        let provider = Arc::new(StubStreamingProvider::default());
        let llm_provider = Arc::new(StubProviderLoader {
            provider: provider.clone() as Arc<dyn LlmProvider>,
        }) as LlmProviderLoaderRef;
        let inner = make_test_inner(llm_provider);
        let handle = setup_handle(&inner);

        let stream_handle = inner.stream_hub.open(SessionId::new("test-session"));

        let result = run_agent_turn(&handle, "hello".to_string(), None, &stream_handle).await;

        assert!(result.is_ok());
        let result = result.unwrap();
        assert_eq!(result.text, "test reply");
    }

    #[tokio::test]
    async fn test_run_agent_turn_with_history() {
        use async_openai::types::chat::ChatCompletionRequestUserMessageArgs;

        let provider = Arc::new(StubStreamingProvider::default());
        let llm_provider = Arc::new(StubProviderLoader {
            provider: provider.clone() as Arc<dyn LlmProvider>,
        }) as LlmProviderLoaderRef;
        let inner = make_test_inner(llm_provider);
        let handle = setup_handle(&inner);

        let stream_handle = inner.stream_hub.open(SessionId::new("test-session"));

        let history = vec![
            ChatCompletionRequestUserMessageArgs::default()
                .content("previous question")
                .build()
                .unwrap()
                .into(),
        ];

        let result =
            run_agent_turn(&handle, "new question".to_string(), Some(history), &stream_handle)
                .await;

        assert!(result.is_ok());

        // LLM should receive 3 messages: system + 1 history + 1 current.
        assert_eq!(
            provider
                .message_counts
                .lock()
                .expect("lock")
                .as_slice(),
            [3]
        );
    }
}
