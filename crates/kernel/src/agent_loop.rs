use std::{collections::HashMap, sync::Arc, time::Instant};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, info_span, warn};

use crate::llm;

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

use crate::{
    error::KernelError,
    handle::KernelHandle,
    io::{StreamEvent, StreamHandle},
    llm::ModelCapabilities,
    session::SessionKey,
};

fn parse_tool_call_arguments(arguments: &str) -> Result<serde_json::Value, String> {
    let args = serde_json::from_str::<serde_json::Value>(arguments)
        .map_err(|err| format!("invalid tool arguments: {err}"))?;
    if !args.is_object() {
        return Err(format!(
            "invalid tool arguments: expected JSON object, got {args}"
        ));
    }
    Ok(args)
}

fn sanitize_messages_for_llm(messages: &[llm::Message]) -> Vec<llm::Message> {
    messages
        .iter()
        .cloned()
        .map(|mut message| {
            if !message.tool_calls.is_empty() {
                message
                    .tool_calls
                    .retain(|call| parse_tool_call_arguments(&call.arguments).is_ok());
            }
            message
        })
        .collect()
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
        session_key = %session_key,
    )
)]
pub(crate) async fn run_inline_agent_loop(
    handle: &KernelHandle,
    session_key: SessionKey,
    user_text: String,
    history: Option<Vec<llm::Message>>,
    stream_handle: &StreamHandle,
    turn_cancel: &CancellationToken,
) -> crate::error::Result<AgentTurnResult> {
    // Query context via syscalls.
    let manifest = handle
        .session_manifest(&session_key)
        .await
        .map_err(|e| KernelError::AgentExecution {
            message: format!("failed to get manifest: {e}"),
        })?;
    let full_tools = handle
        .session_tool_registry(session_key)
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
            .session_resolve_driver(session_key)
            .await
            .map_err(|e| KernelError::AgentExecution {
                message: format!("failed to resolve LLM driver: {e}"),
            })?;

    tracing::Span::current().record("model", model.as_str());

    let capabilities = ModelCapabilities::detect(provider_hint, &model);
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
    let mut tool_defs = if tools.is_empty() {
        vec![]
    } else {
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
    let mut llm_error_recovery_used = false;

    for iteration in 0..max_iterations {
        messages = sanitize_messages_for_llm(&messages);
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
            stage: crate::io::stages::THINKING.to_string(),
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
            parallel_tool_calls: !tool_defs.is_empty() && capabilities.supports_parallel_tool_calls,
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
            if !llm_error_recovery_used && crate::error::is_retryable_provider_error(e) {
                warn!(
                    iteration,
                    model = model.as_str(),
                    error = %e,
                    "LLM stream error, attempting recovery without tools"
                );
                llm_error_recovery_used = true;
                messages.push(llm::Message::user(format!(
                    "[系统提示] 上一次请求遇到了服务端错误（{e}），请直接回复用户的问题，\
                     不要使用工具。"
                )));
                tool_defs = vec![];
                continue;
            }

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

        // Terminal response (no tool calls, or recovery iteration must exit)
        if !has_tool_calls || llm_error_recovery_used {
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

        // Parse and validate tool calls
        let mut valid_tool_calls = Vec::new();
        let mut assistant_tool_calls = Vec::new();
        for tool_call in tool_call_list {
            tool_calls_made += 1;
            let args = match parse_tool_call_arguments(&tool_call.arguments_buf) {
                Ok(args) => args,
                Err(error_message) => {
                    messages.push(llm::Message::tool_result(
                        &tool_call.id,
                        serde_json::json!({ "error": error_message }).to_string(),
                    ));
                    continue;
                }
            };

            assistant_tool_calls.push(llm::ToolCallRequest {
                id:        tool_call.id.clone(),
                name:      tool_call.name.clone(),
                arguments: tool_call.arguments_buf.clone(),
            });

            stream_handle.emit(StreamEvent::ToolCallStart {
                name:      tool_call.name.clone(),
                id:        tool_call.id.clone(),
                arguments: args.clone(),
            });
            valid_tool_calls.push((tool_call.id, tool_call.name, args));
        }

        if assistant_tool_calls.is_empty() {
            messages.push(llm::Message::assistant(accumulated_text.clone()));
        } else {
            messages.push(llm::Message::assistant_with_tool_calls(
                accumulated_text.clone(),
                assistant_tool_calls,
            ));
        }

        iter_span.record("tool_count", valid_tool_calls.len());

        // Execute all tool calls concurrently (with timing for traces)
        let tool_futures: Vec<_> = valid_tool_calls
            .iter()
            .map(|((_id, name, args), verdict)| {
                let tool = tools.get(name);
                let args = args.clone();
                let name = name.clone();
                let tool_span = info_span!(
                    "tool_exec",
                    tool_name = name.as_str(),
                    success = tracing::field::Empty,
                );
                async move {
                    let _guard = tool_span.enter();
                    let tool_start = Instant::now();
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
