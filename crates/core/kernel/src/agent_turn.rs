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

//! Inline agent turn — unified LLM streaming + tool execution loop.
//!
//! [`run_inline_agent_loop`] replaces the old `process_loop::run_agent_turn`
//! bridge layer. Instead of spawning a separate tokio task (via
//! `AgentRunner::run_streaming`) and forwarding `RunnerEvent` through an mpsc
//! channel, this function runs the LLM streaming loop **inline** in the
//! caller's task, emitting [`StreamEvent`]s directly and supporting
//! cancellation via `tokio::select!` on a [`CancellationToken`].

use std::collections::HashMap;

use async_openai::types::chat::{
    ChatCompletionMessageToolCall, ChatCompletionMessageToolCalls,
    ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestMessage,
    ChatCompletionRequestSystemMessageArgs, ChatCompletionToolChoiceOption, FinishReason,
    FunctionCall, CreateChatCompletionRequestArgs, ToolChoiceOptions,
};
use futures::StreamExt;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::{
    handle::scoped::ScopedKernelHandle,
    io::stream::{StreamEvent, StreamHandle},
    model::ModelCapabilities,
    runner::{PendingToolCall, UserContent, build_tool_response_message, build_user_message},
};

/// Result of a single agent turn.
#[derive(Debug)]
pub struct AgentTurnResult {
    /// The final text produced by the agent.
    pub text:       String,
    /// Number of LLM iterations consumed.
    pub iterations: usize,
    /// Number of tool calls executed.
    pub tool_calls: usize,
}

/// Execute a single agent turn inline: build messages, stream LLM responses,
/// execute tool calls, and emit [`StreamEvent`]s directly.
///
/// Unlike the old `process_loop::run_agent_turn`, this function:
/// 1. Does **not** spawn a separate tokio task
/// 2. Emits `StreamEvent` directly (no RunnerEvent -> StreamEvent translation)
/// 3. Supports cancellation via `turn_cancel` token in every `tokio::select!`
pub(crate) async fn run_inline_agent_loop(
    handle: &ScopedKernelHandle,
    user_text: String,
    history: Option<Vec<ChatCompletionRequestMessage>>,
    stream_handle: &StreamHandle,
    turn_cancel: &CancellationToken,
) -> Result<AgentTurnResult, String> {
    let manifest = handle.manifest();
    let max_iterations = manifest.max_iterations.unwrap_or(25);
    let model = &manifest.model;
    let system_prompt = &manifest.system_prompt;
    let provider_hint = manifest.provider_hint.as_deref();

    // Build initial messages: system + optional history + user
    let mut messages: Vec<ChatCompletionRequestMessage> = {
        let sys_msg = ChatCompletionRequestSystemMessageArgs::default()
            .content(system_prompt.as_str())
            .build()
            .map_err(|e| format!("failed to build system message: {e}"))?;
        let mut msgs = vec![sys_msg.into()];

        if let Some(hist) = history {
            msgs.extend(hist);
        }

        let user_msg = build_user_message(&UserContent::Text(user_text))
            .map_err(|e| format!("failed to build user message: {e}"))?;
        msgs.push(user_msg);
        msgs
    };

    // Check model tool support
    let tools = handle.tool_registry();
    let request_tools = if tools.is_empty() {
        None
    } else {
        let capabilities = ModelCapabilities::detect(provider_hint, model);
        if !capabilities.supports_tools {
            warn!(
                model_name = model,
                provider_hint = ?provider_hint,
                reason = capabilities.tools_disabled_reason.unwrap_or("unknown"),
                "disabling tool calling for model without tool support"
            );
            None
        } else {
            Some(
                tools
                    .to_chat_completion_tools()
                    .map_err(|e| format!("failed to build tool definitions: {e}"))?,
            )
        }
    };

    let mut tool_calls_made = 0usize;
    let mut last_accumulated_text = String::new();

    for iteration in 0..max_iterations {
        stream_handle.emit(StreamEvent::Progress {
            stage: "thinking".to_string(),
        });
        info!(
            iteration,
            messages_count = messages.len(),
            "calling LLM (inline streaming)"
        );

        // Acquire provider
        let provider = handle
            .llm_provider()
            .acquire_provider()
            .await
            .map_err(|e| format!("failed to acquire LLM provider: {e}"))?;

        // Build streaming request
        let mut request_builder = CreateChatCompletionRequestArgs::default();
        request_builder
            .model(model.as_str())
            .messages(messages.clone())
            .temperature(0.7_f32);

        if let Some(ref tool_defs) = request_tools {
            request_builder.tools(tool_defs.clone());
            request_builder.tool_choice(ChatCompletionToolChoiceOption::Mode(
                ToolChoiceOptions::Auto,
            ));
            request_builder.parallel_tool_calls(true);
        }

        let request = request_builder
            .build()
            .map_err(|e| format!("failed to build streaming request: {e}"))?;

        // Start streaming with cancellation support
        let mut stream = tokio::select! {
            result = provider.chat_completion_stream(request) => {
                result.map_err(|e| format!("LLM streaming request failed: {e}"))?
            }
            _ = turn_cancel.cancelled() => {
                info!("LLM turn cancelled before streaming started");
                return Err("interrupted by user".to_string());
            }
        };

        // Consume streaming chunks
        let mut accumulated_text = String::new();
        let mut pending_tool_calls: HashMap<u32, PendingToolCall> = HashMap::new();
        let mut has_tool_calls = false;

        loop {
            let chunk_result = tokio::select! {
                chunk = stream.next() => chunk,
                _ = turn_cancel.cancelled() => {
                    info!("LLM turn cancelled during streaming");
                    return Err("interrupted by user".to_string());
                }
            };

            let Some(chunk_result) = chunk_result else {
                // Stream ended
                break;
            };

            let response = match chunk_result {
                Ok(r) => r,
                Err(e) => {
                    error!(
                        iteration,
                        model = model.as_str(),
                        error = %e,
                        "streaming chunk error"
                    );
                    return Err(format!(
                        "Model \"{}\" returned an error during streaming: {e}",
                        model
                    ));
                }
            };

            let Some(choice) = response.choices.first() else {
                continue;
            };

            // Text content delta
            if let Some(ref text) = choice.delta.content {
                if !text.is_empty() {
                    accumulated_text.push_str(text);
                    stream_handle.emit(StreamEvent::TextDelta(text.clone()));
                }
            }

            // Tool call delta accumulation
            if let Some(ref tool_calls_delta) = choice.delta.tool_calls {
                for tc in tool_calls_delta {
                    let idx = tc.index;
                    let entry =
                        pending_tool_calls
                            .entry(idx)
                            .or_insert_with(|| PendingToolCall {
                                id:            String::new(),
                                name:          String::new(),
                                arguments_buf: String::new(),
                            });
                    if let Some(ref id) = tc.id {
                        if !id.is_empty() {
                            entry.id = id.clone();
                        }
                    }
                    if let Some(ref func) = tc.function {
                        if let Some(ref name) = func.name {
                            if !name.is_empty() {
                                entry.name = name.clone();
                            }
                        }
                        if let Some(ref args) = func.arguments {
                            entry.arguments_buf.push_str(args);
                        }
                    }
                }
            }

            // Check finish_reason
            if let Some(ref reason) = choice.finish_reason {
                match reason {
                    FinishReason::ToolCalls => {
                        has_tool_calls = true;
                        break;
                    }
                    FinishReason::Stop | FinishReason::Length => {
                        break;
                    }
                    _ => {}
                }
            }
        }

        // Terminal response (no tool calls)
        if !has_tool_calls {
            return Ok(AgentTurnResult {
                text: accumulated_text,
                iterations: iteration + 1,
                tool_calls: tool_calls_made,
            });
        }

        // Stash for partial-result reporting
        last_accumulated_text = accumulated_text.clone();

        // Assemble and execute tool calls
        let mut sorted_indices: Vec<u32> = pending_tool_calls.keys().copied().collect();
        sorted_indices.sort_unstable();

        let tool_call_list: Vec<PendingToolCall> = sorted_indices
            .into_iter()
            .filter_map(|idx| pending_tool_calls.remove(&idx))
            .collect();

        // Reconstruct assistant message with tool_calls for message history
        let openai_tool_calls: Vec<ChatCompletionMessageToolCalls> = tool_call_list
            .iter()
            .map(|tc| {
                ChatCompletionMessageToolCalls::Function(ChatCompletionMessageToolCall {
                    id:       tc.id.clone(),
                    function: FunctionCall {
                        name:      tc.name.clone(),
                        arguments: tc.arguments_buf.clone(),
                    },
                })
            })
            .collect();

        let assistant_msg = ChatCompletionRequestAssistantMessageArgs::default()
            .content(accumulated_text.clone())
            .tool_calls(openai_tool_calls)
            .build()
            .map_err(|e| format!("failed to build assistant message: {e}"))?;
        messages.push(assistant_msg.into());

        // Parse and validate tool calls
        let mut valid_tool_calls = Vec::new();
        for tool_call in tool_call_list {
            tool_calls_made += 1;
            let args =
                match serde_json::from_str::<serde_json::Value>(&tool_call.arguments_buf) {
                    Ok(args) => args,
                    Err(err) => {
                        let error_message = format!("invalid tool arguments: {err}");
                        messages.push(
                            build_tool_response_message(
                                &tool_call.id,
                                &serde_json::json!({ "error": error_message }).to_string(),
                            )
                            .map_err(|e| format!("failed to build tool response: {e}"))?,
                        );
                        continue;
                    }
                };

            stream_handle.emit(StreamEvent::ToolCallStart {
                name: tool_call.name.clone(),
                id:   tool_call.id.clone(),
            });
            valid_tool_calls.push((tool_call.id, tool_call.name, args));
        }

        // Execute all tool calls concurrently
        let tool_futures: Vec<_> = valid_tool_calls
            .iter()
            .map(|(_id, name, args)| {
                let tool = tools.get(name);
                let args = args.clone();
                let name = name.clone();
                async move {
                    if let Some(tool) = tool {
                        match tool.execute(args).await {
                            Ok(result) => (true, result, None::<String>),
                            Err(e) => (
                                false,
                                serde_json::json!({ "error": e.to_string() }),
                                Some(e.to_string()),
                            ),
                        }
                    } else {
                        let err = format!("tool not found: {name}");
                        (false, serde_json::json!({ "error": &err }), Some(err))
                    }
                }
            })
            .collect();

        let results = futures::future::join_all(tool_futures).await;

        // Emit ToolCallEnd events and append tool response messages
        for ((id, _name, _args), (_success, result, _err)) in
            valid_tool_calls.iter().zip(results)
        {
            stream_handle.emit(StreamEvent::ToolCallEnd { id: id.clone() });

            messages.push(
                build_tool_response_message(id, &result.to_string())
                    .map_err(|e| format!("failed to build tool response: {e}"))?,
            );
        }
    }

    // Max iterations exhausted — return partial results
    warn!(
        max_iterations,
        tool_calls_made,
        "inline agent loop hit max iterations limit, returning partial results"
    );
    Ok(AgentTurnResult {
        text:       last_accumulated_text,
        iterations: max_iterations,
        tool_calls: tool_calls_made,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_openai::types::chat::{
        ChatChoiceStream, ChatCompletionRequestUserMessageArgs, ChatCompletionResponseStream,
        ChatCompletionStreamResponseDelta, CreateChatCompletionRequest,
        CreateChatCompletionStreamResponse, FinishReason,
    };
    use async_trait::async_trait;
    use dashmap::DashMap;
    use futures::stream;
    use tokio::sync::Semaphore;

    use super::*;
    use crate::{
        audit::InMemoryAuditLog,
        defaults::{
            noop::{NoopEventBus, NoopGuard, NoopMemory, NoopSessionRepository},
            noop_user_store::NoopUserStore,
        },
        io::stream::StreamHub,
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
            name:                "test-agent".to_string(),
            description:         "Test agent".to_string(),
            model:               "test-model".to_string(),
            system_prompt:       "You are a test agent.".to_string(),
            provider_hint:       None,
            max_iterations:      Some(5),
            tools:               vec![],
            max_children:        None,
            max_context_tokens:  None,
            priority:            crate::process::Priority::default(),
            metadata:            serde_json::Value::Null,
            sandbox:             None,
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
            memory_quota_per_agent: 1000,
            user_store:             Arc::new(NoopUserStore),
            session_repo:           Arc::new(NoopSessionRepository) as Arc<dyn SessionRepository>,
            settings:               Arc::new(crate::defaults::noop::NoopSettingsProvider)
                as Arc<dyn rara_domain_shared::settings::SettingsProvider>,
            stream_hub:             Arc::new(StreamHub::new(16)),
            pipe_registry:          Arc::new(crate::io::pipe::PipeRegistry::new()),
            device_registry:        Arc::new(crate::device_registry::DeviceRegistry::new()),
            audit_log:              Arc::new(InMemoryAuditLog::default())
                as Arc<dyn crate::audit::AuditLog>,
            event_queue:            Arc::new(crate::event_queue::EventQueue::new(4096)),
        })
    }

    fn setup_handle(inner: &Arc<KernelInner>) -> Arc<ScopedKernelHandle> {
        let agent_id = AgentId::new();
        let session_id = SessionId::new("test-session");
        let manifest = test_manifest();

        let process = AgentProcess {
            agent_id,
            parent_id:     None,
            session_id:    session_id.clone(),
            manifest:      manifest.clone(),
            principal:     Principal::user("test-user"),
            env:           AgentEnv::default(),
            state:         ProcessState::Running,
            created_at:    jiff::Timestamp::now(),
            finished_at:   None,
            result:        None,
            created_files: vec![],
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
    async fn test_run_inline_agent_loop_basic() {
        let provider = Arc::new(StubStreamingProvider::default());
        let llm_provider = Arc::new(StubProviderLoader {
            provider: provider.clone() as Arc<dyn LlmProvider>,
        }) as LlmProviderLoaderRef;
        let inner = make_test_inner(llm_provider);
        let handle = setup_handle(&inner);

        let stream_handle = inner.stream_hub.open(SessionId::new("test-session"));
        let cancel = CancellationToken::new();

        let result = run_inline_agent_loop(
            &handle,
            "hello".to_string(),
            None,
            &stream_handle,
            &cancel,
        )
        .await;

        assert!(result.is_ok());
        let result = result.unwrap();
        assert_eq!(result.text, "test reply");
    }

    #[tokio::test]
    async fn test_run_inline_agent_loop_with_history() {
        let provider = Arc::new(StubStreamingProvider::default());
        let llm_provider = Arc::new(StubProviderLoader {
            provider: provider.clone() as Arc<dyn LlmProvider>,
        }) as LlmProviderLoaderRef;
        let inner = make_test_inner(llm_provider);
        let handle = setup_handle(&inner);

        let stream_handle = inner.stream_hub.open(SessionId::new("test-session"));
        let cancel = CancellationToken::new();

        let history = vec![
            ChatCompletionRequestUserMessageArgs::default()
                .content("previous question")
                .build()
                .unwrap()
                .into(),
        ];

        let result = run_inline_agent_loop(
            &handle,
            "new question".to_string(),
            Some(history),
            &stream_handle,
            &cancel,
        )
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

    /// A provider that blocks indefinitely until cancelled.
    struct BlockingStreamingProvider;

    #[async_trait]
    impl LlmProvider for BlockingStreamingProvider {
        async fn chat_completion(
            &self,
            _request: CreateChatCompletionRequest,
        ) -> crate::error::Result<async_openai::types::chat::CreateChatCompletionResponse> {
            Err(crate::error::KernelError::Other {
                message: "not supported".into(),
            })
        }

        async fn chat_completion_stream(
            &self,
            _request: CreateChatCompletionRequest,
        ) -> crate::error::Result<ChatCompletionResponseStream> {
            // Return a stream that never yields — simulates a slow LLM
            let pending_stream = futures::stream::pending();
            Ok(Box::pin(pending_stream))
        }
    }

    #[tokio::test]
    async fn test_run_inline_agent_loop_cancellation() {
        let provider = Arc::new(BlockingStreamingProvider);
        let llm_provider = Arc::new(StubProviderLoader {
            provider: provider as Arc<dyn LlmProvider>,
        }) as LlmProviderLoaderRef;
        let inner = make_test_inner(llm_provider);
        let handle = setup_handle(&inner);

        let stream_handle = inner.stream_hub.open(SessionId::new("test-session"));
        let cancel = CancellationToken::new();

        // Spawn the agent loop and cancel shortly after
        let cancel_clone = cancel.clone();
        let handle_clone = Arc::clone(&handle);
        let join = tokio::spawn(async move {
            run_inline_agent_loop(
                &handle_clone,
                "hello".to_string(),
                None,
                &stream_handle,
                &cancel_clone,
            )
            .await
        });

        // Give a moment for the loop to start, then cancel.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        cancel.cancel();

        let result = join.await.unwrap();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "interrupted by user");
    }
}
