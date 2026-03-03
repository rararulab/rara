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

//! Inline agent turn — unified LLM streaming + tool execution loop.
//!
//! [`run_inline_agent_loop`] runs the LLM streaming loop **inline** in the
//! caller's task, emitting [`StreamEvent`]s directly and supporting
//! cancellation via `tokio::select!` on a [`CancellationToken`].
//!
//! This module uses the new [`LlmDriver`](crate::llm::LlmDriver) abstraction
//! which provides first-class `reasoning_content` support for models like
//! DeepSeek-R1.

use std::{collections::HashMap, sync::Arc, time::Instant};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, info_span, warn};

use crate::{
    error::KernelError,
    handle::process_handle::ProcessHandle,
    io::stream::{StreamEvent, StreamHandle},
    llm,
    model::ModelCapabilities,
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

/// A tool call being incrementally assembled from streaming deltas.
struct PendingToolCall {
    id:            String,
    name:          String,
    arguments_buf: String,
}

/// Trace of a single tool call within an iteration.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolCallTrace {
    pub name:           String,
    pub id:             String,
    pub duration_ms:    u64,
    pub success:        bool,
    pub arguments:      serde_json::Value,
    pub result_preview: String,
    pub error:          Option<String>,
}

/// Trace of a single LLM iteration within a turn.
#[derive(Debug, Clone, serde::Serialize)]
pub struct IterationTrace {
    pub index:          usize,
    pub first_token_ms: Option<u64>,
    pub stream_ms:      u64,
    /// First 200 chars of accumulated text.
    pub text_preview:   String,
    /// Full accumulated reasoning text for this iteration.
    pub reasoning_text: Option<String>,
    pub tool_calls:     Vec<ToolCallTrace>,
}

/// Complete trace of a single agent turn.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TurnTrace {
    pub duration_ms:      u64,
    pub model:            String,
    /// The user message that triggered this turn.
    pub input_text:       Option<String>,
    pub iterations:       Vec<IterationTrace>,
    pub final_text_len:   usize,
    pub total_tool_calls: usize,
    pub success:          bool,
    pub error:            Option<String>,
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
/// Uses the new [`LlmDriver`] abstraction with first-class `reasoning_content`
/// (thinking tokens) support. The driver sends [`StreamDelta`] events through
/// an `mpsc` channel, which this function consumes.
///
/// # Cancellation
///
/// Respects `turn_cancel` at every `tokio::select!` point — both before the
/// stream starts and during delta consumption.
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
    history: Option<Vec<llm::Message>>,
    stream_handle: &StreamHandle,
    turn_cancel: &CancellationToken,
) -> crate::error::Result<AgentTurnResult> {
    // Query context via syscalls.
    let manifest = handle.manifest().await.map_err(|e| KernelError::AgentExecution {
        message: format!("failed to get manifest: {e}"),
    })?;
    let full_tools = handle
        .tool_registry()
        .await
        .map_err(|e| KernelError::AgentExecution {
            message: format!("failed to get tool registry: {e}"),
        })?;

    // Filter tools by manifest.tools whitelist.
    let tools = Arc::new(full_tools.filtered(&manifest.tools));

    let max_iterations = manifest.max_iterations.unwrap_or(25);
    let effective_prompt = match &manifest.soul_prompt {
        Some(soul) => format!("{soul}\n\n---\n\n{}", manifest.system_prompt),
        None => manifest.system_prompt.clone(),
    };
    let provider_hint = manifest.provider_hint.as_deref();

    // Resolve driver + model via the DriverRegistry syscall.
    let (driver, model) =
        handle
            .resolve_driver()
            .await
            .map_err(|e| KernelError::AgentExecution {
                message: format!("failed to resolve LLM driver: {e}"),
            })?;

    tracing::Span::current().record("model", model.as_str());

    let input_text = user_text.clone();

    // Build initial messages: system + optional history + user
    let mut messages: Vec<llm::Message> = {
        let mut msgs = vec![llm::Message::system(&effective_prompt)];
        if let Some(hist) = history {
            msgs.extend(hist);
        }
        msgs.push(llm::Message::user(user_text));
        msgs
    };

    // Check model tool support
    let tool_defs = if tools.is_empty() {
        vec![]
    } else {
        let capabilities = ModelCapabilities::detect(provider_hint, &model);
        if !capabilities.supports_tools {
            warn!(
                model_name = %model,
                provider_hint = ?provider_hint,
                reason = capabilities.tools_disabled_reason.unwrap_or("unknown"),
                "disabling tool calling for model without tool support"
            );
            vec![]
        } else {
            tools.to_llm_tool_definitions()
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
            "calling LLM (inline streaming via LlmDriver)"
        );

        // Build completion request
        let request = llm::CompletionRequest {
            model:               model.clone(),
            messages:            messages.clone(),
            tools:               tool_defs.clone(),
            temperature:         Some(0.7),
            max_tokens:          None,
            thinking:            None,
            tool_choice:         if tool_defs.is_empty() {
                llm::ToolChoice::None
            } else {
                llm::ToolChoice::Auto
            },
            parallel_tool_calls: true,
        };

        // Start streaming via LlmDriver
        let (tx, mut rx) = mpsc::channel::<llm::StreamDelta>(128);
        let driver_clone = Arc::clone(&driver);
        let request_clone = request;

        // Spawn driver.stream() — it sends deltas to tx and returns when done.
        let stream_task = tokio::spawn(async move { driver_clone.stream(request_clone, tx).await });

        // Consume streaming deltas
        let stream_start = Instant::now();
        let mut first_token_at: Option<Instant> = None;
        let mut accumulated_text = String::new();
        let mut accumulated_reasoning = String::new();
        let mut pending_tool_calls: HashMap<u32, PendingToolCall> = HashMap::new();
        let mut has_tool_calls = false;

        loop {
            let delta = tokio::select! {
                delta = rx.recv() => delta,
                _ = turn_cancel.cancelled() => {
                    stream_task.abort();
                    info!("LLM turn cancelled during streaming");
                    return Err(KernelError::AgentExecution {
                        message: "interrupted by user".into(),
                    });
                }
            };

            let Some(delta) = delta else {
                // Channel closed — driver finished (or errored).
                break;
            };

            match delta {
                llm::StreamDelta::TextDelta { text } => {
                    if !text.is_empty() {
                        if first_token_at.is_none() {
                            first_token_at = Some(Instant::now());
                            iter_span.record(
                                "first_token_ms",
                                first_token_at
                                    .unwrap()
                                    .duration_since(stream_start)
                                    .as_millis() as u64,
                            );
                        }
                        accumulated_text.push_str(&text);
                        stream_handle.emit(StreamEvent::TextDelta { text });
                    }
                }
                llm::StreamDelta::ReasoningDelta { text } => {
                    if !text.is_empty() {
                        if first_token_at.is_none() {
                            first_token_at = Some(Instant::now());
                        }
                        accumulated_reasoning.push_str(&text);
                        // KEY: emit ReasoningDelta to the stream!
                        stream_handle.emit(StreamEvent::ReasoningDelta { text });
                    }
                }
                llm::StreamDelta::ToolCallStart { index, id, name } => {
                    pending_tool_calls
                        .entry(index)
                        .or_insert_with(|| PendingToolCall {
                            id,
                            name,
                            arguments_buf: String::new(),
                        });
                }
                llm::StreamDelta::ToolCallArgumentsDelta { index, arguments } => {
                    if let Some(tc) = pending_tool_calls.get_mut(&index) {
                        tc.arguments_buf.push_str(&arguments);
                    }
                }
                llm::StreamDelta::Done { stop_reason, .. } => {
                    has_tool_calls = stop_reason == llm::StopReason::ToolCalls;
                    break;
                }
            }
        }

        // Wait for the stream task to complete (the driver accumulates the
        // full response internally).
        let driver_result = match stream_task.await {
            Ok(result) => result,
            Err(join_err) if join_err.is_cancelled() => {
                return Err(KernelError::AgentExecution {
                    message: "interrupted by user".into(),
                });
            }
            Err(join_err) => {
                return Err(KernelError::AgentExecution {
                    message: format!("driver stream task panicked: {join_err}"),
                });
            }
        };

        if let Err(ref e) = driver_result {
            error!(
                iteration,
                model = model.as_str(),
                error = %e,
                "LLM driver stream error"
            );
            return Err(KernelError::AgentExecution {
                message: format!("Model \"{model}\" returned an error during streaming: {e}"),
            });
        }

        iter_span.record("stream_ms", stream_start.elapsed().as_millis() as u64);
        iter_span.record("has_tools", has_tool_calls);

        // Terminal response (no tool calls)
        if !has_tool_calls {
            let first_token_ms =
                first_token_at.map(|t| t.duration_since(stream_start).as_millis() as u64);
            let stream_ms = stream_start.elapsed().as_millis() as u64;
            let text_preview: String = accumulated_text.chars().take(200).collect();
            iteration_traces.push(IterationTrace {
                index: iteration,
                first_token_ms,
                stream_ms,
                text_preview,
                reasoning_text: if accumulated_reasoning.is_empty() {
                    None
                } else {
                    Some(accumulated_reasoning)
                },
                tool_calls: vec![],
            });
            let trace = TurnTrace {
                duration_ms:      turn_start.elapsed().as_millis() as u64,
                model:            model.clone(),
                input_text:       Some(input_text.clone()),
                iterations:       iteration_traces,
                final_text_len:   accumulated_text.len(),
                total_tool_calls: tool_calls_made,
                success:          true,
                error:            None,
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
        let assistant_tool_calls: Vec<llm::ToolCallRequest> = tool_call_list
            .iter()
            .map(|tc| llm::ToolCallRequest {
                id:        tc.id.clone(),
                name:      tc.name.clone(),
                arguments: tc.arguments_buf.clone(),
            })
            .collect();

        messages.push(llm::Message::assistant_with_tool_calls(
            accumulated_text.clone(),
            assistant_tool_calls,
        ));

        // Parse and validate tool calls
        let mut valid_tool_calls = Vec::new();
        for tool_call in tool_call_list {
            tool_calls_made += 1;
            let args = match serde_json::from_str::<serde_json::Value>(&tool_call.arguments_buf) {
                Ok(args) => args,
                Err(err) => {
                    let error_message = format!("invalid tool arguments: {err}");
                    messages.push(llm::Message::tool_result(
                        &tool_call.id,
                        serde_json::json!({ "error": error_message }).to_string(),
                    ));
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
            handle
                .check_guard_batch(guard_checks)
                .await
                .unwrap_or_else(|_| vec![crate::guard::Verdict::Allow; valid_tool_calls.len()])
        } else {
            vec![]
        };

        // Execute all tool calls concurrently (with timing for traces)
        let tool_futures: Vec<_> = valid_tool_calls
            .iter()
            .zip(
                verdicts
                    .iter()
                    .chain(std::iter::repeat(&crate::guard::Verdict::Allow)),
            )
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
                id: id.clone(),
                result_preview: result_preview.clone(),
                success,
                error: err.clone(),
            });

            // Fire-and-forget tool call audit recording.
            let _ = handle
                .record_tool_call(
                    name.clone(),
                    args.clone(),
                    result.clone(),
                    success,
                    duration_ms,
                )
                .await;
            tool_call_traces.push(ToolCallTrace {
                name: name.clone(),
                id: id.clone(),
                duration_ms,
                success,
                arguments: args.clone(),
                result_preview,
                error: err,
            });

            messages.push(llm::Message::tool_result(id, result_str));
        }

        // Collect iteration trace (with tool calls)
        {
            let first_token_ms =
                first_token_at.map(|t| t.duration_since(stream_start).as_millis() as u64);
            let stream_ms = stream_start.elapsed().as_millis() as u64;
            let text_preview: String = accumulated_text.chars().take(200).collect();
            iteration_traces.push(IterationTrace {
                index: iteration,
                first_token_ms,
                stream_ms,
                text_preview,
                reasoning_text: if accumulated_reasoning.is_empty() {
                    None
                } else {
                    Some(accumulated_reasoning.clone())
                },
                tool_calls: tool_call_traces,
            });
        }
    }

    // Max iterations exhausted — return partial results
    warn!(
        max_iterations,
        tool_calls_made, "inline agent loop hit max iterations limit, returning partial results"
    );
    let trace = TurnTrace {
        duration_ms:      turn_start.elapsed().as_millis() as u64,
        model:            model.clone(),
        input_text:       Some(input_text.clone()),
        iterations:       iteration_traces,
        final_text_len:   last_accumulated_text.len(),
        total_tool_calls: tool_calls_made,
        success:          true,
        error:            None,
    };
    Ok(AgentTurnResult {
        text: last_accumulated_text,
        iterations: max_iterations,
        tool_calls: tool_calls_made,
        model: model.clone(),
        trace,
    })
}

/// Convert persisted chat history into [`llm::Message`] format.
///
/// This is the `LlmDriver`-native equivalent of the legacy
/// `runner::build_history_messages` which returns async-openai types.
pub(crate) fn build_llm_history(
    history: &[crate::channel::types::ChatMessage],
) -> Vec<llm::Message> {
    history
        .iter()
        .filter_map(|msg| {
            use crate::channel::types::MessageRole;
            match msg.role {
                MessageRole::System => Some(llm::Message::system(msg.content.as_text())),
                MessageRole::User => Some(llm::Message::user(msg.content.as_text())),
                MessageRole::Assistant => {
                    if msg.tool_calls.is_empty() {
                        Some(llm::Message::assistant(msg.content.as_text()))
                    } else {
                        let tool_calls: Vec<llm::ToolCallRequest> = msg
                            .tool_calls
                            .iter()
                            .map(|tc| llm::ToolCallRequest {
                                id:        tc.id.to_string(),
                                name:      tc.name.to_string(),
                                arguments: tc.arguments.to_string(),
                            })
                            .collect();
                        Some(llm::Message::assistant_with_tool_calls(
                            msg.content.as_text(),
                            tool_calls,
                        ))
                    }
                }
                MessageRole::Tool | MessageRole::ToolResult => {
                    let tool_call_id = msg.tool_call_id.as_deref().unwrap_or("");
                    Some(llm::Message::tool_result(
                        tool_call_id,
                        msg.content.as_text(),
                    ))
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::{
        handle::process_handle::ProcessHandle,
        kernel::Kernel,
        llm::{
            self, DriverRegistryBuilder, LlmDriver, LlmDriverRef,
            stream::StreamDelta,
            types::{CompletionRequest, CompletionResponse, StopReason, Usage},
        },
        process::{AgentId, AgentManifest, SessionId, principal::Principal},
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

    /// A stub LlmDriver that returns a simple text response.
    struct StubDriver;

    #[async_trait]
    impl LlmDriver for StubDriver {
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> crate::error::Result<CompletionResponse> {
            Ok(CompletionResponse {
                content:           Some("test reply".to_string()),
                reasoning_content: None,
                tool_calls:        vec![],
                stop_reason:       StopReason::Stop,
                usage:             Some(Usage {
                    prompt_tokens:     10,
                    completion_tokens: 5,
                    total_tokens:      15,
                }),
                model:             "test-model".to_string(),
            })
        }

        async fn stream(
            &self,
            _request: CompletionRequest,
            tx: mpsc::Sender<StreamDelta>,
        ) -> crate::error::Result<CompletionResponse> {
            let _ = tx
                .send(StreamDelta::TextDelta {
                    text: "test reply".to_string(),
                })
                .await;
            let _ = tx
                .send(StreamDelta::Done {
                    stop_reason: StopReason::Stop,
                    usage:       Some(Usage {
                        prompt_tokens:     10,
                        completion_tokens: 5,
                        total_tokens:      15,
                    }),
                })
                .await;
            Ok(CompletionResponse {
                content:           Some("test reply".to_string()),
                reasoning_content: None,
                tool_calls:        vec![],
                stop_reason:       StopReason::Stop,
                usage:             Some(Usage {
                    prompt_tokens:     10,
                    completion_tokens: 5,
                    total_tokens:      15,
                }),
                model:             "test-model".to_string(),
            })
        }
    }

    /// A stub driver that emits reasoning deltas (simulating DeepSeek-R1).
    struct ReasoningDriver;

    #[async_trait]
    impl LlmDriver for ReasoningDriver {
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> crate::error::Result<CompletionResponse> {
            Ok(CompletionResponse {
                content:           Some("answer".to_string()),
                reasoning_content: Some("thinking...".to_string()),
                tool_calls:        vec![],
                stop_reason:       StopReason::Stop,
                usage:             None,
                model:             "test-model".to_string(),
            })
        }

        async fn stream(
            &self,
            _request: CompletionRequest,
            tx: mpsc::Sender<StreamDelta>,
        ) -> crate::error::Result<CompletionResponse> {
            let _ = tx
                .send(StreamDelta::ReasoningDelta {
                    text: "thinking...".to_string(),
                })
                .await;
            let _ = tx
                .send(StreamDelta::TextDelta {
                    text: "answer".to_string(),
                })
                .await;
            let _ = tx
                .send(StreamDelta::Done {
                    stop_reason: StopReason::Stop,
                    usage:       None,
                })
                .await;
            Ok(CompletionResponse {
                content:           Some("answer".to_string()),
                reasoning_content: Some("thinking...".to_string()),
                tool_calls:        vec![],
                stop_reason:       StopReason::Stop,
                usage:             None,
                model:             "test-model".to_string(),
            })
        }
    }

    /// A stub driver that blocks indefinitely (for cancellation tests).
    struct BlockingDriver;

    #[async_trait]
    impl LlmDriver for BlockingDriver {
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> crate::error::Result<CompletionResponse> {
            futures::future::pending().await
        }

        async fn stream(
            &self,
            _request: CompletionRequest,
            _tx: mpsc::Sender<StreamDelta>,
        ) -> crate::error::Result<CompletionResponse> {
            // Never sends anything — blocks forever.
            futures::future::pending().await
        }
    }

    /// Set up a test kernel with a DriverRegistry. Returns (kernel_arc,
    /// agent_id, cancel).
    async fn setup_test_kernel_with_driver(
        driver: LlmDriverRef,
    ) -> (Arc<Kernel>, AgentId, CancellationToken) {
        let driver_registry = Arc::new(
            DriverRegistryBuilder::new("test", "test-model")
                .driver("test", driver)
                .build(),
        );

        let kernel = TestKernelBuilder::new()
            .driver_registry(driver_registry)
            .build();

        let cancel = CancellationToken::new();
        let (kernel_arc, handle) = kernel.start(cancel.clone());

        let manifest = test_manifest();
        let agent_id = handle
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

        (kernel_arc, agent_id, cancel)
    }

    #[tokio::test]
    async fn test_run_inline_agent_loop_basic() {
        let driver: LlmDriverRef = Arc::new(StubDriver);
        let (kernel, agent_id, cancel) = setup_test_kernel_with_driver(driver).await;

        let handle = ProcessHandle::new(
            agent_id,
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

        assert!(result.is_ok(), "expected Ok, got: {:?}", result);
        let result = result.unwrap();
        assert_eq!(result.text, "test reply");

        cancel.cancel();
    }

    #[tokio::test]
    async fn test_run_inline_agent_loop_with_history() {
        let driver: LlmDriverRef = Arc::new(StubDriver);
        let (kernel, agent_id, cancel) = setup_test_kernel_with_driver(driver).await;

        let handle = ProcessHandle::new(
            agent_id,
            SessionId::new("test-session"),
            Principal::user("test-user"),
            kernel.event_queue().clone(),
        );

        let stream_handle = kernel.stream_hub().open(SessionId::new("test-session"));
        let turn_cancel = CancellationToken::new();

        let history = vec![llm::Message::user("previous question")];

        let result = run_inline_agent_loop(
            &handle,
            "new question".to_string(),
            Some(history),
            &stream_handle,
            &turn_cancel,
        )
        .await;

        assert!(result.is_ok());
        cancel.cancel();
    }

    #[tokio::test]
    async fn test_run_inline_agent_loop_reasoning_delta() {
        let driver: LlmDriverRef = Arc::new(ReasoningDriver);
        let (kernel, agent_id, cancel) = setup_test_kernel_with_driver(driver).await;

        let handle = ProcessHandle::new(
            agent_id,
            SessionId::new("test-session"),
            Principal::user("test-user"),
            kernel.event_queue().clone(),
        );

        let session_id = SessionId::new("test-session");
        let stream_handle = kernel.stream_hub().open(session_id.clone());

        // Subscribe to stream events BEFORE running the loop.
        let subs = kernel.stream_hub().subscribe_session(&session_id);
        let (_, mut rx) = subs.into_iter().next().unwrap();

        let turn_cancel = CancellationToken::new();

        let result = run_inline_agent_loop(
            &handle,
            "think about this".to_string(),
            None,
            &stream_handle,
            &turn_cancel,
        )
        .await;

        assert!(result.is_ok());
        let result = result.unwrap();
        assert_eq!(result.text, "answer");

        // Verify that ReasoningDelta was emitted to the stream.
        let mut saw_reasoning = false;
        while let Ok(event) = rx.try_recv() {
            if matches!(event, StreamEvent::ReasoningDelta { .. }) {
                saw_reasoning = true;
                break;
            }
        }
        assert!(saw_reasoning, "expected ReasoningDelta stream event");

        cancel.cancel();
    }

    #[tokio::test]
    async fn test_run_inline_agent_loop_cancellation() {
        let driver: LlmDriverRef = Arc::new(BlockingDriver);
        let (kernel, agent_id, cancel_kernel) = setup_test_kernel_with_driver(driver).await;

        let handle = Arc::new(ProcessHandle::new(
            agent_id,
            SessionId::new("test-session"),
            Principal::user("test-user"),
            kernel.event_queue().clone(),
        ));

        let stream_handle = kernel.stream_hub().open(SessionId::new("test-session"));
        let turn_cancel = CancellationToken::new();

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
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("interrupted by user"),
            "expected 'interrupted by user', got: {err}"
        );

        cancel_kernel.cancel();
    }

    #[test]
    fn test_build_llm_history() {
        use crate::channel::types::ChatMessage;

        let messages = vec![
            ChatMessage::user("hello"),
            ChatMessage::assistant("hi there"),
            ChatMessage::user("how are you?"),
        ];

        let llm_msgs = build_llm_history(&messages);
        assert_eq!(llm_msgs.len(), 3);
        assert_eq!(llm_msgs[0].role, llm::Role::User);
        assert_eq!(llm_msgs[0].content.as_text(), "hello");
        assert_eq!(llm_msgs[1].role, llm::Role::Assistant);
        assert_eq!(llm_msgs[1].content.as_text(), "hi there");
        assert_eq!(llm_msgs[2].role, llm::Role::User);
        assert_eq!(llm_msgs[2].content.as_text(), "how are you?");
    }
}
