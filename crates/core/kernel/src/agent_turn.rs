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
use std::sync::Arc;
use std::time::Instant;

use async_openai::types::chat::{
    ChatCompletionMessageToolCall, ChatCompletionMessageToolCalls,
    ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestMessage,
    ChatCompletionRequestSystemMessageArgs, ChatCompletionToolChoiceOption, FinishReason,
    FunctionCall, CreateChatCompletionRequestArgs, ToolChoiceOptions,
};
use futures::StreamExt;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, info_span, warn};

use crate::{
    handle::process_handle::ProcessHandle,
    io::stream::{StreamEvent, StreamHandle},
    model::ModelCapabilities,
    runner::{PendingToolCall, UserContent, build_tool_response_message, build_user_message},
};

/// Maximum byte length for result preview strings.
const RESULT_PREVIEW_MAX_BYTES: usize = 2048;

/// Truncate a string to at most `max_bytes` bytes on a valid char boundary.
fn truncate_preview(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let boundary = s.floor_char_boundary(max_bytes);
    format!("{}... (truncated)", &s[..boundary])
}

/// Trace of a single tool call within an iteration.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolCallTrace {
    pub name: String,
    pub id: String,
    pub duration_ms: u64,
    pub success: bool,
    pub arguments: serde_json::Value,
    pub result_preview: String,
    pub error: Option<String>,
}

/// Trace of a single LLM iteration within a turn.
#[derive(Debug, Clone, serde::Serialize)]
pub struct IterationTrace {
    pub index: usize,
    pub first_token_ms: Option<u64>,
    pub stream_ms: u64,
    /// First 200 chars of accumulated text.
    pub text_preview: String,
    /// Full accumulated text for this iteration (the agent's "thinking").
    pub reasoning_text: Option<String>,
    pub tool_calls: Vec<ToolCallTrace>,
}

/// Complete trace of a single agent turn.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TurnTrace {
    pub duration_ms: u64,
    pub model: String,
    /// The user message that triggered this turn.
    pub input_text: Option<String>,
    pub iterations: Vec<IterationTrace>,
    pub final_text_len: usize,
    pub total_tool_calls: usize,
    pub success: bool,
    pub error: Option<String>,
}

/// Result of a single agent turn.
#[derive(Debug)]
pub struct AgentTurnResult {
    /// The final text produced by the agent.
    pub text:       String,
    /// Number of LLM iterations consumed.
    pub iterations: usize,
    /// Number of tool calls executed.
    pub tool_calls: usize,
    /// Model used for this turn.
    pub model:      String,
    /// Detailed trace of the turn for observability.
    pub trace:      TurnTrace,
}

/// Execute a single agent turn inline: build messages, stream LLM responses,
/// execute tool calls, and emit [`StreamEvent`]s directly.
///
/// Unlike the old `process_loop::run_agent_turn`, this function:
/// 1. Does **not** spawn a separate tokio task
/// 2. Emits `StreamEvent` directly (no RunnerEvent -> StreamEvent translation)
/// 3. Supports cancellation via `turn_cancel` token in every `tokio::select!`
///
/// Context (manifest, tools, provider) is queried via syscalls through the
/// ProcessHandle.
#[tracing::instrument(
    skip(handle, history, stream_handle, turn_cancel),
    fields(
        agent_id = %handle.agent_id(),
        session_id = %handle.session_id(),
    )
)]
pub(crate) async fn run_inline_agent_loop(
    handle: &ProcessHandle,
    user_text: String,
    history: Option<Vec<ChatCompletionRequestMessage>>,
    stream_handle: &StreamHandle,
    turn_cancel: &CancellationToken,
) -> Result<AgentTurnResult, String> {
    // Query context via syscalls.
    let manifest = handle
        .manifest()
        .await
        .map_err(|e| format!("failed to get manifest: {e}"))?;
    let full_tools = handle
        .tool_registry()
        .await
        .map_err(|e| format!("failed to get tool registry: {e}"))?;

    // Filter tools by manifest.tools whitelist. If the list is empty (e.g.
    // rara), all tools are included. If specific tools are listed (e.g.
    // scout: ["read_file", "grep"]), only those are available.
    let tools = Arc::new(full_tools.filtered(&manifest.tools));

    let max_iterations = manifest.max_iterations.unwrap_or(25);
    // Build effective system prompt (prepend soul_prompt if present)
    let effective_prompt = match &manifest.soul_prompt {
        Some(soul) => format!("{soul}\n\n---\n\n{}", manifest.system_prompt),
        None => manifest.system_prompt.clone(),
    };
    let provider_hint = manifest.provider_hint.as_deref();

    // Resolve provider + model via the ProviderRegistry syscall.
    // This uses the resolution priority chain:
    //   agent_overrides > manifest > global default
    let (provider, model) = handle
        .resolve_provider()
        .await
        .map_err(|e| format!("failed to resolve LLM provider: {e}"))?;

    // Record model on the parent agent_turn span.
    tracing::Span::current().record("model", model.as_str());

    // Clone user_text before it's consumed by build_user_message.
    let input_text = user_text.clone();

    // Build initial messages: system + optional history + user
    let mut messages: Vec<ChatCompletionRequestMessage> = {
        let sys_msg = ChatCompletionRequestSystemMessageArgs::default()
            .content(effective_prompt.as_str())
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
    let request_tools = if tools.is_empty() {
        None
    } else {
        let capabilities = ModelCapabilities::detect(provider_hint, &model);
        if !capabilities.supports_tools {
            warn!(
                model_name = %model,
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
    let turn_start = Instant::now();
    let mut iteration_traces: Vec<IterationTrace> = Vec::new();

    for iteration in 0..max_iterations {
        let iter_span = info_span!(
            "llm_iteration",
            iter = iteration,
            model = model.as_str(),
            first_token_ms = tracing::field::Empty,
            stream_ms = tracing::field::Empty,
            has_tools = tracing::field::Empty,
            tool_count = tracing::field::Empty,
        );
        let _iter_guard = iter_span.enter();

        stream_handle.emit(StreamEvent::Progress {
            stage: "thinking".to_string(),
        });
        info!(
            iteration,
            messages_count = messages.len(),
            "calling LLM (inline streaming)"
        );

        // Build streaming request (provider already resolved above)
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
        let stream_start = Instant::now();
        let mut first_token_at: Option<Instant> = None;
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
                    if first_token_at.is_none() {
                        first_token_at = Some(Instant::now());
                        iter_span.record(
                            "first_token_ms",
                            first_token_at.unwrap().duration_since(stream_start).as_millis() as u64,
                        );
                    }
                    accumulated_text.push_str(text);
                    stream_handle.emit(StreamEvent::TextDelta { text: text.clone() });
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

        // Record stream duration on the iteration span.
        iter_span.record("stream_ms", stream_start.elapsed().as_millis() as u64);
        iter_span.record("has_tools", has_tool_calls);

        // Terminal response (no tool calls)
        if !has_tool_calls {
            let first_token_ms = first_token_at
                .map(|t| t.duration_since(stream_start).as_millis() as u64);
            let stream_ms = stream_start.elapsed().as_millis() as u64;
            let text_preview: String = accumulated_text.chars().take(200).collect();
            iteration_traces.push(IterationTrace {
                index: iteration,
                first_token_ms,
                stream_ms,
                text_preview,
                reasoning_text: Some(accumulated_text.clone()),
                tool_calls: vec![],
            });
            let trace = TurnTrace {
                duration_ms: turn_start.elapsed().as_millis() as u64,
                model: model.clone(),
                input_text: Some(input_text.clone()),
                iterations: iteration_traces,
                final_text_len: accumulated_text.len(),
                total_tool_calls: tool_calls_made,
                success: true,
                error: None,
            };
            return Ok(AgentTurnResult {
                text: accumulated_text,
                iterations: iteration + 1,
                tool_calls: tool_calls_made,
                model: model.clone(),
                trace,
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
                name:      tool_call.name.clone(),
                id:        tool_call.id.clone(),
                arguments: args.clone(),
            });
            valid_tool_calls.push((tool_call.id, tool_call.name, args));
        }

        iter_span.record("tool_count", valid_tool_calls.len());

        // Guard check: batch-verify all tool calls before execution
        let guard_checks: Vec<(String, serde_json::Value)> = valid_tool_calls
            .iter()
            .map(|(_, name, args)| (name.clone(), args.clone()))
            .collect();

        let verdicts = if !guard_checks.is_empty() {
            handle.check_guard_batch(guard_checks).await
                .unwrap_or_else(|_| vec![crate::guard::Verdict::Allow; valid_tool_calls.len()])
        } else {
            vec![]
        };

        // Execute all tool calls concurrently (with timing for traces)
        let tool_futures: Vec<_> = valid_tool_calls
            .iter()
            .zip(verdicts.iter().chain(std::iter::repeat(&crate::guard::Verdict::Allow)))
            .map(|((_id, name, args), verdict)| {
                let tool = tools.get(name);
                let args = args.clone();
                let name = name.clone();
                let is_denied = matches!(verdict, crate::guard::Verdict::Deny { .. });
                let deny_reason = match verdict {
                    crate::guard::Verdict::Deny { reason } => Some(reason.clone()),
                    _ => None,
                };
                let tool_span = info_span!(
                    "tool_exec",
                    tool_name = name.as_str(),
                    success = tracing::field::Empty,
                );
                async move {
                    let _guard = tool_span.enter();
                    let tool_start = Instant::now();
                    // Check guard verdict first
                    if is_denied {
                        tool_span.record("success", false);
                        let reason = deny_reason.unwrap_or_default();
                        let err = format!("sandbox denied: {reason}");
                        let dur = tool_start.elapsed().as_millis() as u64;
                        return (false, serde_json::json!({ "error": &err }), Some(err), dur);
                    }
                    if let Some(tool) = tool {
                        match tool.execute(args).await {
                            Ok(result) => {
                                tool_span.record("success", true);
                                let dur = tool_start.elapsed().as_millis() as u64;
                                (true, result, None::<String>, dur)
                            }
                            Err(e) => {
                                tool_span.record("success", false);
                                let dur = tool_start.elapsed().as_millis() as u64;
                                (
                                    false,
                                    serde_json::json!({ "error": e.to_string() }),
                                    Some(e.to_string()),
                                    dur,
                                )
                            }
                        }
                    } else {
                        tool_span.record("success", false);
                        let err = format!("tool not found: {name}");
                        let dur = tool_start.elapsed().as_millis() as u64;
                        (false, serde_json::json!({ "error": &err }), Some(err), dur)
                    }
                }
            })
            .collect();

        let results = futures::future::join_all(tool_futures).await;

        // Build tool call traces from results
        let mut tool_call_traces: Vec<ToolCallTrace> = Vec::with_capacity(results.len());

        // Emit ToolCallEnd events and append tool response messages
        for ((id, name, args), (success, result, err, duration_ms)) in
            valid_tool_calls.iter().zip(results)
        {
            let result_str = result.to_string();
            let result_preview = truncate_preview(&result_str, RESULT_PREVIEW_MAX_BYTES);

            stream_handle.emit(StreamEvent::ToolCallEnd {
                id:             id.clone(),
                result_preview: result_preview.clone(),
                success,
                error:          err.clone(),
            });

            tool_call_traces.push(ToolCallTrace {
                name: name.clone(),
                id: id.clone(),
                duration_ms,
                success,
                arguments: args.clone(),
                result_preview,
                error: err,
            });

            messages.push(
                build_tool_response_message(id, &result_str)
                    .map_err(|e| format!("failed to build tool response: {e}"))?,
            );
        }

        // Collect iteration trace (with tool calls)
        {
            let first_token_ms = first_token_at
                .map(|t| t.duration_since(stream_start).as_millis() as u64);
            let stream_ms = stream_start.elapsed().as_millis() as u64;
            let text_preview: String = accumulated_text.chars().take(200).collect();
            iteration_traces.push(IterationTrace {
                index: iteration,
                first_token_ms,
                stream_ms,
                text_preview,
                reasoning_text: if accumulated_text.is_empty() { None } else { Some(accumulated_text.clone()) },
                tool_calls: tool_call_traces,
            });
        }
    }

    // Max iterations exhausted — return partial results
    warn!(
        max_iterations,
        tool_calls_made,
        "inline agent loop hit max iterations limit, returning partial results"
    );
    let trace = TurnTrace {
        duration_ms: turn_start.elapsed().as_millis() as u64,
        model: model.clone(),
        input_text: Some(input_text.clone()),
        iterations: iteration_traces,
        final_text_len: last_accumulated_text.len(),
        total_tool_calls: tool_calls_made,
        success: true,
        error: None,
    };
    Ok(AgentTurnResult {
        text:       last_accumulated_text,
        iterations: max_iterations,
        tool_calls: tool_calls_made,
        model:      model.clone(),
        trace,
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
    use futures::stream;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::{
        handle::process_handle::ProcessHandle,
        kernel::Kernel,
        process::{
            AgentId, AgentManifest, SessionId,
            principal::Principal,
        },
        provider::{LlmProvider, ProviderRegistryBuilder},
        testing::TestKernelBuilder,
    };

    fn test_manifest() -> AgentManifest {
        AgentManifest {
            name:               "test-agent".to_string(),
            role:               None,
            description:        "Test agent".to_string(),
            model:              Some("test-model".to_string()),
            system_prompt:      "You are a test agent.".to_string(),
            soul_prompt:        None,
            provider_hint:      None,
            max_iterations:     Some(5),
            tools:              vec![],
            max_children:       None,
            max_context_tokens: None,
            priority:           crate::process::Priority::default(),
            metadata:           serde_json::Value::Null,
            sandbox:            None,
        }
    }

    /// Set up a test kernel with the event loop running, spawn a process,
    /// and return the kernel + agent_id + cancel token.
    async fn setup_test_kernel_with_process(
        provider: Arc<dyn LlmProvider>,
    ) -> (Arc<Kernel>, AgentId, CancellationToken) {
        let registry = Arc::new(
            ProviderRegistryBuilder::new("test", "test-model")
                .provider("test", provider)
                .build(),
        );
        let kernel = TestKernelBuilder::new()
            .provider_registry(registry)
            .build();
        let cancel = CancellationToken::new();
        let kernel = kernel.start(cancel.clone());

        let manifest = test_manifest();
        let agent_id = kernel
            .spawn_with_input(
                manifest,
                "init".to_string(),
                Principal::user("test-user"),
                None,
            )
            .await
            .unwrap();

        // Wait briefly for spawn to complete.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        (kernel, agent_id, cancel)
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

    #[tokio::test]
    async fn test_run_inline_agent_loop_basic() {
        let provider = Arc::new(StubStreamingProvider::default());

        let (kernel, _agent_id, cancel) =
            setup_test_kernel_with_process(provider.clone() as Arc<dyn LlmProvider>).await;

        // Create a handle that talks to this kernel's event queue.
        let handle = ProcessHandle::new(
            _agent_id,
            SessionId::new("test-session"),
            Principal::user("test-user"),
            kernel.event_queue().clone(),
        );

        let stream_handle = kernel.stream_hub().open(SessionId::new("test-session"));
        let turn_cancel = CancellationToken::new();

        let result = run_inline_agent_loop(
            &handle,
            "hello".to_string(),
            None,
            &stream_handle,
            &turn_cancel,
        )
        .await;

        assert!(result.is_ok());
        let result = result.unwrap();
        assert_eq!(result.text, "test reply");

        cancel.cancel();
    }

    #[tokio::test]
    async fn test_run_inline_agent_loop_with_history() {
        let provider = Arc::new(StubStreamingProvider::default());

        let (kernel, _agent_id, cancel) =
            setup_test_kernel_with_process(provider.clone() as Arc<dyn LlmProvider>).await;

        let handle = ProcessHandle::new(
            _agent_id,
            SessionId::new("test-session"),
            Principal::user("test-user"),
            kernel.event_queue().clone(),
        );

        let stream_handle = kernel.stream_hub().open(SessionId::new("test-session"));
        let turn_cancel = CancellationToken::new();

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
            &turn_cancel,
        )
        .await;

        assert!(result.is_ok());

        // message_counts includes the initial spawn_with_input("init") call
        // (system + "init" = 2) plus this call (system + 1 history + 1 current = 3).
        let counts = provider.message_counts.lock().expect("lock");
        assert_eq!(*counts.last().unwrap(), 3);

        cancel.cancel();
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
        let provider = Arc::new(BlockingStreamingProvider) as Arc<dyn LlmProvider>;

        let (kernel, _agent_id, cancel_kernel) =
            setup_test_kernel_with_process(provider).await;

        let handle = Arc::new(ProcessHandle::new(
            _agent_id,
            SessionId::new("test-session"),
            Principal::user("test-user"),
            kernel.event_queue().clone(),
        ));

        let stream_handle = kernel.stream_hub().open(SessionId::new("test-session"));
        let turn_cancel = CancellationToken::new();

        // Spawn the agent loop and cancel shortly after
        let turn_cancel_clone = turn_cancel.clone();
        let handle_clone = Arc::clone(&handle);
        let join = tokio::spawn(async move {
            run_inline_agent_loop(
                &handle_clone,
                "hello".to_string(),
                None,
                &stream_handle,
                &turn_cancel_clone,
            )
            .await
        });

        // Give a moment for the loop to start, then cancel.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        turn_cancel.cancel();

        let result = join.await.unwrap();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "interrupted by user");

        cancel_kernel.cancel();
    }
}
