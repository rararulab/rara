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

use std::collections::HashMap;
use std::sync::Arc;

use backon::{ExponentialBuilder, Retryable};
use base::shared_string::SharedString;
use bon::Builder;
use futures::StreamExt;
use openrouter_rs::{
    api::chat::{Content, Message},
    types::{Choice, Role, completion::CompletionsResponse},
};
use snafu::ResultExt;
use tokio::sync::mpsc;
use tracing::{error, info, trace, warn};

use crate::{err::prelude::*, model::OpenRouterLoaderRef, tool_registry::ToolRegistry};

/// Maximum number of tool-call loop iterations before giving up.
pub const MAX_ITERATIONS: usize = 25;

/// Result of running the agent loop.
#[derive(Debug, Clone)]
pub struct AgentRunResponse {
    /// Raw provider response for the terminal assistant turn.
    pub provider_response: CompletionsResponse,
    /// Number of loop iterations consumed before termination.
    pub iterations:        usize,
    /// Total number of tool calls executed across all iterations.
    pub tool_calls_made:   usize,
}

/// Events emitted during the agent run.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunnerEvent {
    /// LLM is processing (show a "thinking" indicator).
    Thinking,
    /// LLM finished thinking (hide the indicator).
    ThinkingDone,
    Iteration(usize),
    ToolCallStart {
        id:        String,
        name:      String,
        arguments: serde_json::Value,
    },
    ToolCallEnd {
        id:      String,
        name:    String,
        success: bool,
        error:   Option<String>,
        result:  Option<serde_json::Value>,
    },
    /// Incremental text content from a streaming LLM response.
    TextDelta(String),
    /// Incremental reasoning content from a streaming LLM response.
    ReasoningDelta(String),
    /// The agent loop completed successfully.
    Done {
        text:             String,
        iterations:       usize,
        tool_calls_made:  usize,
    },
    /// The agent loop failed with an error.
    Error(String),
}

/// Callback for streaming events out of the runner.
pub type OnEvent = Box<dyn Fn(RunnerEvent) + Send + Sync>;

#[derive(Builder)]
#[builder(on(SharedString, into))]
pub struct AgentRunner {
    llm_provider:    OpenRouterLoaderRef,
    model_name:      SharedString,
    system_prompt:   SharedString,
    user_content:    Content,
    history:         Option<Vec<Message>>,
    #[builder(default = MAX_ITERATIONS)]
    max_iterations:  usize,
    /// Fallback models to try (in order) when the primary model fails with a
    /// fallback-eligible error. Empty means no fallback.
    #[builder(default)]
    fallback_models: Vec<SharedString>,
}

impl AgentRunner {
    /// Run the agent loop: send messages to the LLM, execute tool calls,
    /// repeat.
    ///
    /// If `history` is provided, those messages are inserted between the system
    /// prompt and the current user message, giving the LLM conversational
    /// context.
    ///
    /// When the primary model fails with a fallback-eligible error and
    /// `fallback_models` is non-empty, the runner will try each fallback
    /// model in order before giving up.
    pub async fn run(
        self,
        tools: &ToolRegistry,
        on_event: Option<&OnEvent>,
    ) -> Result<AgentRunResponse> {
        // Try the primary model first.
        match self
            .run_with_model(self.model_name.as_ref(), tools, on_event)
            .await
        {
            Ok(response) => Ok(response),
            Err(err) if !self.fallback_models.is_empty() && is_fallback_eligible(&err) => {
                let mut last_err = err;
                for fallback in &self.fallback_models {
                    warn!(
                        from = %self.model_name,
                        to = %fallback,
                        error = %last_err,
                        "switching to fallback model"
                    );
                    match self
                        .run_with_model(fallback.as_ref(), tools, on_event)
                        .await
                    {
                        Ok(response) => return Ok(response),
                        Err(err) if is_fallback_eligible(&err) => {
                            last_err = err;
                            continue;
                        }
                        Err(err) => return Err(err),
                    }
                }
                // All fallback models exhausted — return the last error.
                Err(last_err)
            }
            Err(err) => Err(err),
        }
    }

    /// Core agent loop for a single model. Extracted so that [`run`] can
    /// wrap it with fallback logic.
    async fn run_with_model(
        &self,
        model: &str,
        tools: &ToolRegistry,
        on_event: Option<&OnEvent>,
    ) -> Result<AgentRunResponse> {
        let is_multimodal = matches!(self.user_content, Content::Parts(_));
        info!(model_name = model, is_multimodal, "starting agent loop");

        // prepare messages with system prompt, optional history, and current user
        // message
        let mut messages = {
            let mut messages = vec![Message::new(
                openrouter_rs::types::Role::System,
                self.system_prompt.as_ref(),
            )];
            // Insert conversation history before the current user message.
            if let Some(hist) = &self.history {
                messages.extend(hist.clone());
            }
            messages.push(Message::new(
                openrouter_rs::types::Role::User,
                self.user_content.clone(),
            ));
            messages
        };

        let request_tools = if tools.is_empty() {
            None
        } else {
            Some(tools.to_openrouter_tools()?)
        };
        let mut tool_calls_made = 0_usize;

        for iteration in 0..self.max_iterations {
            // ---- Phase 1: begin one loop tick ----
            // Emit iteration and "thinking" events so upper layers (UI/logs)
            // can show the model is actively processing this turn.
            if let Some(cb) = on_event {
                cb(RunnerEvent::Iteration(iteration));
            }
            info!(iteration, messages_count = messages.len(), "calling LLM");
            trace!(iteration = iteration, messages = ?messages, "LLM request messages");
            if let Some(cb) = on_event {
                cb(RunnerEvent::Thinking);
            }

            let openrouter_client: openrouter_rs::OpenRouterClient =
                self.llm_provider.acquire_client().await?;
            // ---- Phase 2: build/send request with retry ----
            // We retry only for transient provider failures classified as
            // `RetryableServer` (e.g. overload / 5xx-like errors).
            //
            // `with_max_times(2)` means: one initial attempt + one retry.
            // All non-retryable errors fail fast and exit this iteration.
            let response = (|| async {
                let mut request_builder =
                    openrouter_rs::api::chat::ChatCompletionRequest::builder();
                request_builder
                    .model(model)
                    .messages(messages.clone())
                    .temperature(0.7);

                if let Some(tool_defs) = &request_tools {
                    // Advertise all registered tools to the model and let it
                    // decide whether to call them.
                    request_builder.tools(tool_defs.clone());
                    request_builder.tool_choice(openrouter_rs::types::ToolChoice::auto());
                    request_builder.parallel_tool_calls(true);
                }

                let request = request_builder.build().context(OpenRouterSnafu)?;
                openrouter_client
                    .send_chat_completion(&request)
                    .await
                    .context(OpenRouterSnafu)
            })
            .retry(ExponentialBuilder::default().with_max_times(2))
            .sleep(tokio::time::sleep)
            .when(is_retryable_provider_error)
            .notify(|err: &Error, dur| {
                error!(
                    iteration,
                    error = %err,
                    retry_in_ms = dur.as_millis(),
                    "LLM call failed, retrying"
                );
            })
            .await;
            if let Some(cb) = on_event {
                cb(RunnerEvent::ThinkingDone);
            }

            // If retries are exhausted, convert to domain error and abort run.
            let response = response.inspect_err(|err| {
                error!(iteration, error = %err, "LLM call failed");
            })?;

            // ---- Phase 3: validate and inspect response ----
            // We currently read only the first choice (OpenAI/OpenRouter style).
            trace!(
                iteration = iteration,
                has_content = !response.choices.is_empty(),
                prompt_tokens = response.usage.as_ref().map_or(0, |it| it.prompt_tokens),
                completion_tokens = response.usage.as_ref().map_or(0, |it| it.completion_tokens),
                "LLM response received, contents: {:?}",
                response.choices.iter().map(|c| c.content())
            );

            // We intentionally consume only the first choice.
            //
            // This runner is a single-track agent loop: one response drives one
            // next action (either final answer or tool calls). Handling multiple
            // choices would require branching message histories and tool
            // executions per branch, which is out of scope for this loop.
            let Some(choice) = response.choices.first() else {
                return Err(Error::Other {
                    message: "LLM returned no choices".into(),
                });
            };

            if let Some(error_detail) = choice.error() {
                return Err(Error::from((error_detail.message.as_str(), None)));
            }

            let assistant_text = choice.content().unwrap_or_default().to_owned();
            let tool_calls = choice.tool_calls().unwrap_or(&[]);
            if tool_calls.is_empty() {
                // Terminal path: model produced final assistant output and
                // requested no more tools, so the loop can end successfully.
                return Ok(AgentRunResponse {
                    provider_response: response,
                    iterations: iteration + 1,
                    tool_calls_made,
                });
            }

            // ---- Phase 4: execute model-requested tools ----
            // Important ordering:
            // 1) Append assistant message that contains `tool_calls`
            // 2) Append each tool response message
            // This preserves the protocol expected by tool-capable models.
            info!(
                iteration,
                tool_calls = tool_calls.len(),
                "assistant requested tool calls"
            );
            messages.push(build_assistant_tool_call_message(choice, &assistant_text));

            for tool_call in tool_calls {
                tool_calls_made = tool_calls_made.saturating_add(1);
                let tool_name = tool_call.name();
                let tool_id = tool_call.id();

                // Provider sends function args as JSON string. Parse to Value
                // so we can pass typed-ish payload into our tool trait.
                let tool_arguments =
                    match serde_json::from_str::<serde_json::Value>(tool_call.arguments_json()) {
                        Ok(value) => value,
                        Err(err) => {
                            let error_message = format!("invalid tool arguments: {err}");
                            if let Some(cb) = on_event {
                                cb(RunnerEvent::ToolCallEnd {
                                    id:      tool_id.to_owned(),
                                    name:    tool_name.to_owned(),
                                    success: false,
                                    error:   Some(error_message.clone()),
                                    result:  None,
                                });
                            }
                            messages.push(Message::tool_response_named(
                                tool_id,
                                tool_name,
                                serde_json::json!({ "error": error_message }).to_string(),
                            ));
                            continue;
                        }
                    };

                // Emit start event before invoking tool execution.
                if let Some(cb) = on_event {
                    cb(RunnerEvent::ToolCallStart {
                        id:        tool_id.to_owned(),
                        name:      tool_name.to_owned(),
                        arguments: tool_arguments.clone(),
                    });
                }

                // Execute local tool if found; otherwise synthesize an error
                // payload so the model can recover and choose another action.
                let tool_response_payload = if let Some(tool) = tools.get(tool_name) {
                    match tool.execute(tool_arguments.clone()).await {
                        Ok(result) => {
                            if let Some(cb) = on_event {
                                cb(RunnerEvent::ToolCallEnd {
                                    id:      tool_id.to_owned(),
                                    name:    tool_name.to_owned(),
                                    success: true,
                                    error:   None,
                                    result:  Some(result.clone()),
                                });
                            }
                            result
                        }
                        Err(err) => {
                            let error_message = err.to_string();
                            if let Some(cb) = on_event {
                                cb(RunnerEvent::ToolCallEnd {
                                    id:      tool_id.to_owned(),
                                    name:    tool_name.to_owned(),
                                    success: false,
                                    error:   Some(error_message.clone()),
                                    result:  None,
                                });
                            }
                            serde_json::json!({ "error": error_message })
                        }
                    }
                } else {
                    let error_message = format!("tool not found: {tool_name}");
                    if let Some(cb) = on_event {
                        cb(RunnerEvent::ToolCallEnd {
                            id:      tool_id.to_owned(),
                            name:    tool_name.to_owned(),
                            success: false,
                            error:   Some(error_message.clone()),
                            result:  None,
                        });
                    }
                    serde_json::json!({ "error": error_message })
                };

                messages.push(Message::tool_response_named(
                    tool_id,
                    tool_name,
                    tool_response_payload.to_string(),
                ));
            }
            // Loop continues with expanded `messages`, giving model the new
            // tool outputs so it can either call more tools or answer finally.
        }

        Err(Error::Other {
            message: "agent loop exceeded max iterations".into(),
        })?
    }

    /// Streaming variant of [`run`]. Spawns the agent loop in a background
    /// task and returns a channel receiver yielding [`RunnerEvent`]s in
    /// real-time, including incremental text/reasoning deltas.
    ///
    /// Unlike [`run`], which blocks until the full response is available,
    /// this method returns immediately with an `mpsc::Receiver`. The caller
    /// can consume events as they arrive (e.g. to build an SSE stream or
    /// progressively update a Telegram message).
    ///
    /// The channel buffer size is 128 — large enough to absorb bursts of
    /// small `TextDelta` events without back-pressuring the LLM stream.
    ///
    /// # Error handling
    ///
    /// If the agent loop encounters a fatal error, a terminal
    /// [`RunnerEvent::Error`] is sent before the background task exits.
    /// The caller should treat both `Done` and `Error` as terminal events.
    ///
    /// # Fallback models
    ///
    /// Not yet implemented for streaming — only the primary model is used.
    /// (The non-streaming [`run`] supports fallback models.)
    pub fn run_streaming(self, tools: Arc<ToolRegistry>) -> mpsc::Receiver<RunnerEvent> {
        let (tx, rx) = mpsc::channel(128);
        tokio::spawn(async move {
            let model = self.model_name.as_ref().to_owned();
            if let Err(e) = self.run_streaming_inner(&model, &tools, &tx).await {
                let _ = tx.send(RunnerEvent::Error(e.to_string())).await;
            }
        });
        rx
    }

    /// Core streaming agent loop for a single model.
    ///
    /// This is the streaming counterpart to [`run_with_model`]. The loop
    /// structure is identical — prepare messages, call LLM, handle tool
    /// calls, repeat — but differs in two key ways:
    ///
    /// 1. Uses `stream_chat_completion()` instead of `send_chat_completion()`
    ///    so that partial text/reasoning/tool-call fragments arrive as SSE
    ///    chunks. Each chunk is forwarded to the caller via the `tx` channel
    ///    as a [`RunnerEvent`].
    ///
    /// 2. Tool calls are executed **concurrently** via `join_all`, not
    ///    sequentially. This reduces total latency when the model requests
    ///    multiple independent tool calls in a single turn.
    async fn run_streaming_inner(
        &self,
        model: &str,
        tools: &ToolRegistry,
        tx: &mpsc::Sender<RunnerEvent>,
    ) -> Result<()> {
        info!(model_name = model, "starting streaming agent loop");

        // ---- Prepare messages ----
        // Identical to run_with_model: system prompt + optional history + user message.
        let mut messages = {
            let mut msgs = vec![Message::new(Role::System, self.system_prompt.as_ref())];
            if let Some(hist) = &self.history {
                msgs.extend(hist.clone());
            }
            msgs.push(Message::new(Role::User, self.user_content.clone()));
            msgs
        };

        // Convert registered tools to the OpenRouter tool definition format.
        // `None` means "no tools" — the model won't attempt function calls.
        let request_tools = if tools.is_empty() {
            None
        } else {
            Some(tools.to_openrouter_tools()?)
        };
        let mut tool_calls_made = 0_usize;

        // ---- Main agent loop ----
        // Each iteration: call LLM → consume streaming chunks → either finish
        // (Done) or execute tool calls and loop again.
        for iteration in 0..self.max_iterations {
            // Notify caller that a new iteration is starting.
            let _ = tx.send(RunnerEvent::Iteration(iteration)).await;
            let _ = tx.send(RunnerEvent::Thinking).await;
            info!(iteration, messages_count = messages.len(), "calling LLM (streaming)");

            let openrouter_client = self.llm_provider.acquire_client().await?;

            // ---- Phase 1: Build and send streaming request ----
            let mut request_builder =
                openrouter_rs::api::chat::ChatCompletionRequest::builder();
            request_builder
                .model(model)
                .messages(messages.clone())
                .temperature(0.7);

            if let Some(tool_defs) = &request_tools {
                request_builder.tools(tool_defs.clone());
                request_builder.tool_choice(openrouter_rs::types::ToolChoice::auto());
                // Allow the model to request multiple tool calls in a single turn.
                request_builder.parallel_tool_calls(true);
            }

            let request = request_builder.build().context(OpenRouterSnafu)?;

            // `stream_chat_completion` returns a `BoxStream` of
            // `CompletionsResponse` chunks. Each chunk contains a
            // `StreamingChoice` with incremental deltas.
            let mut stream = openrouter_client
                .stream_chat_completion(&request)
                .await
                .context(OpenRouterSnafu)?;

            // ---- Phase 2: Consume streaming chunks ----
            // We accumulate three things from the stream:
            //   - `accumulated_text`: full assistant text built from TextDelta fragments.
            //   - `pending_tool_calls`: tool calls being assembled from incremental deltas.
            //     Each tool call arrives across multiple chunks — first the id/name, then
            //     argument fragments. We key them by `index` (position in the tool_calls array).
            //   - `has_tool_calls`: set to true when finish_reason == ToolCalls.
            let mut accumulated_text = String::new();
            let mut pending_tool_calls: HashMap<u32, PendingToolCall> = HashMap::new();
            let mut has_tool_calls = false;

            while let Some(chunk_result) = stream.next().await {
                let response = match chunk_result {
                    Ok(r) => r,
                    Err(openrouter_rs::error::OpenRouterError::Serialization(e)) => {
                        // The model returned a streaming chunk with an
                        // unexpected JSON shape. This typically means the
                        // model/provider is incompatible with openrouter-rs's
                        // streaming parser. Abort and tell the user.
                        error!(
                            iteration,
                            model,
                            error = %e,
                            "model returned unparseable streaming response"
                        );
                        return Err(Error::Other {
                            message: format!(
                                "Model \"{model}\" returned an incompatible streaming response. \
                                 Please switch to a different model."
                            ).into(),
                        });
                    }
                    Err(e) => {
                        return Err(e).context(OpenRouterSnafu);
                    }
                };
                let Some(choice) = response.choices.first() else {
                    continue;
                };

                if let Choice::Streaming(sc) = choice {
                    // --- Text content delta ---
                    // Each chunk may contain a fragment of the assistant's text response.
                    if let Some(ref text) = sc.delta.content {
                        if !text.is_empty() {
                            accumulated_text.push_str(text);
                            let _ = tx.send(RunnerEvent::TextDelta(text.clone())).await;
                        }
                    }

                    // --- Reasoning delta ---
                    // Some models (e.g. o1) emit chain-of-thought reasoning in a
                    // separate field. We forward it as a distinct event type.
                    if let Some(ref reasoning) = sc.delta.reasoning {
                        if !reasoning.is_empty() {
                            let _ =
                                tx.send(RunnerEvent::ReasoningDelta(reasoning.clone())).await;
                        }
                    }

                    // --- Tool call delta accumulation ---
                    // Tool call fragments arrive incrementally:
                    //   Chunk 1: { index: 0, id: "call_abc", function: { name: "search", arguments: "" } }
                    //   Chunk 2: { index: 0, id: "",          function: { name: "",       arguments: '{"q' } }
                    //   Chunk 3: { index: 0, id: "",          function: { name: "",       arguments: '":"hello"}' } }
                    // We accumulate by `index` until `finish_reason == ToolCalls`.
                    if let Some(ref tool_calls_delta) = sc.delta.tool_calls {
                        for tc in tool_calls_delta {
                            let idx = tc.index.unwrap_or(0);
                            let entry =
                                pending_tool_calls.entry(idx).or_insert_with(|| {
                                    PendingToolCall {
                                        id:              String::new(),
                                        name:            String::new(),
                                        arguments_buf:   String::new(),
                                    }
                                });
                            // id and name are sent once (in the first chunk for this index);
                            // subsequent chunks have empty strings.
                            if !tc.id.is_empty() {
                                entry.id = tc.id.clone();
                            }
                            if !tc.function.name.is_empty() {
                                entry.name = tc.function.name.clone();
                            }
                            // Arguments are always appended — they arrive as JSON fragments.
                            entry.arguments_buf.push_str(&tc.function.arguments);
                        }
                    }

                    // --- Check finish_reason ---
                    // `ToolCalls` means the model wants us to execute tools before continuing.
                    // `Stop` or `Length` means the model has finished its response.
                    if let Some(ref reason) = sc.finish_reason {
                        match reason {
                            openrouter_rs::types::completion::FinishReason::ToolCalls => {
                                has_tool_calls = true;
                                break;
                            }
                            openrouter_rs::types::completion::FinishReason::Stop
                            | openrouter_rs::types::completion::FinishReason::Length => {
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }

            let _ = tx.send(RunnerEvent::ThinkingDone).await;

            // ---- Phase 3: Handle terminal response (no tool calls) ----
            if !has_tool_calls {
                let _ = tx
                    .send(RunnerEvent::Done {
                        text:            accumulated_text,
                        iterations:      iteration + 1,
                        tool_calls_made,
                    })
                    .await;
                return Ok(());
            }

            // ---- Phase 4: Assemble and execute tool calls ----
            // Sort by index to preserve the model's intended ordering.
            let mut sorted_indices: Vec<u32> = pending_tool_calls.keys().copied().collect();
            sorted_indices.sort_unstable();

            // Parse accumulated JSON argument strings into serde_json::Value.
            let tool_call_list: Vec<(String, String, serde_json::Value)> = sorted_indices
                .into_iter()
                .filter_map(|idx| pending_tool_calls.remove(&idx))
                .map(|ptc| {
                    let args = serde_json::from_str::<serde_json::Value>(&ptc.arguments_buf)
                        .unwrap_or(serde_json::json!({}));
                    (ptc.id, ptc.name, args)
                })
                .collect();

            // Reconstruct the assistant message with tool_calls for message history.
            // This is required by the OpenRouter/OpenAI protocol — the assistant turn
            // that requests tool calls must be in the message array, followed by the
            // tool response messages.
            let openrouter_tool_calls: Vec<openrouter_rs::types::completion::ToolCall> =
                tool_call_list
                    .iter()
                    .map(|(id, name, args)| openrouter_rs::types::completion::ToolCall {
                        id:        id.clone(),
                        type_:     "function".to_string(),
                        function:  openrouter_rs::types::completion::FunctionCall {
                            name:      name.clone(),
                            arguments: serde_json::to_string(args).unwrap_or_default(),
                        },
                        index: None,
                    })
                    .collect();

            let assistant_msg = Message {
                role:         Role::Assistant,
                content:      if accumulated_text.is_empty() {
                    Content::Text(String::new())
                } else {
                    Content::Text(accumulated_text.clone())
                },
                name:         None,
                tool_call_id: None,
                tool_calls:   Some(openrouter_tool_calls),
            };
            messages.push(assistant_msg);

            // Emit ToolCallStart events so the caller can show tool execution progress.
            for (id, name, args) in &tool_call_list {
                tool_calls_made += 1;
                let _ = tx
                    .send(RunnerEvent::ToolCallStart {
                        id:        id.clone(),
                        name:      name.clone(),
                        arguments: args.clone(),
                    })
                    .await;
            }

            // Execute all tool calls concurrently via `join_all`.
            // Unlike `run_with_model` which runs tools sequentially, this
            // significantly reduces latency when multiple tools are independent
            // (e.g. parallel web searches).
            let tool_futures: Vec<_> = tool_call_list
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

            // Emit ToolCallEnd events and append tool response messages to history.
            // The response messages use the OpenRouter `tool_response_named` format
            // so the model can see each tool's output keyed by call id.
            for ((id, name, _args), (success, result, err)) in
                tool_call_list.iter().zip(results)
            {
                let _ = tx
                    .send(RunnerEvent::ToolCallEnd {
                        id:      id.clone(),
                        name:    name.clone(),
                        success,
                        error:   err,
                        result:  if success { Some(result.clone()) } else { None },
                    })
                    .await;

                messages.push(Message::tool_response_named(id, name, result.to_string()));
            }
            // Loop continues — the model will see tool outputs and can call more
            // tools or produce a final text response.
        }

        Err(Error::Other {
            message: "agent loop exceeded max iterations".into(),
        })
    }
}

/// A tool call being incrementally assembled from streaming SSE chunks.
///
/// During streaming, tool call information arrives in fragments across
/// multiple `StreamingChoice` chunks. The first chunk for a given `index`
/// typically carries the `id` and `name`, while subsequent chunks append
/// to `arguments_buf` with JSON fragments. Once `finish_reason == ToolCalls`,
/// we parse `arguments_buf` as complete JSON and execute the tool.
struct PendingToolCall {
    /// Unique tool call identifier assigned by the model (e.g. "call_abc123").
    id:            String,
    /// Tool function name (e.g. "web_search").
    name:          String,
    /// Accumulated JSON argument string, built by concatenating fragments.
    arguments_buf: String,
}

fn build_assistant_tool_call_message(choice: &Choice, assistant_text: &str) -> Message {
    let content = if assistant_text.is_empty() {
        Content::Text(String::new())
    } else {
        Content::Text(assistant_text.to_owned())
    };

    Message {
        role: Role::Assistant,
        content,
        name: None,
        tool_call_id: None,
        tool_calls: choice.tool_calls().map(ToOwned::to_owned),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use test_case::test_case;

    use super::*;
    use crate::model::EnvOpenRouterLoader;

    /// Simple echo tool for testing.
    struct EchoTool;

    #[async_trait]
    impl crate::tool_registry::AgentTool for EchoTool {
        fn name(&self) -> &str { "echo_tool" }

        fn description(&self) -> &str { "Echoes input" }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {"text": {"type": "string"}}})
        }

        async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
            Ok(params)
        }
    }

    #[test_case(
        "plain_text",
        "You are a concise assistant.",
        "Reply with exactly one short greeting.",
        false,
        false
        ; "plain_text"
    )]
    #[test_case(
        "tool_call",
        "You are a tool-using assistant. Always call echo_tool exactly once before replying.",
        "Call echo_tool with {\"text\":\"hello-tool\"} and then answer with one sentence.",
        true,
        true
        ; "tool_call"
    )]
    #[tokio::test]
    // #[ignore = "requires real OpenRouter API key; runs real OpenRouter
    // integration cases"]
    async fn run_real_openrouter_table_driven(
        case_name: &'static str,
        system_prompt: &'static str,
        user_prompt: &'static str,
        register_echo: bool,
        expect_tool_call: bool,
    ) {
        common_telemetry::logging::init_default_ut_logging();
        let model_name = std::env::var("OPENROUTER_MODEL")
            .unwrap_or_else(|_| "z-ai/glm-4.5-air:free".to_owned());

        let mut tools = ToolRegistry::default();
        if register_echo {
            tools.register_builtin(Arc::new(EchoTool));
        }

        let events: Arc<Mutex<Vec<RunnerEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let events_ref = Arc::clone(&events);
        let on_event: OnEvent = Box::new(move |event| {
            events_ref.lock().expect("event lock poisoned").push(event);
        });

        let runner = AgentRunner::builder()
            .llm_provider(Arc::new(EnvOpenRouterLoader::default()))
            .model_name(model_name.clone())
            .system_prompt(system_prompt)
            .user_content(Content::Text(user_prompt.to_owned()))
            .build();

        let result = runner
            .run(&tools, Some(&on_event))
            .await
            .unwrap_or_else(|err| panic!("case `{}` failed: {err}", case_name));

        if expect_tool_call {
            assert!(
                result.tool_calls_made > 0,
                "case `{}` expected at least one tool call",
                case_name
            );
            let has_tool_start = events
                .lock()
                .expect("event lock poisoned")
                .iter()
                .any(|event| matches!(event, RunnerEvent::ToolCallStart { .. }));
            assert!(
                has_tool_start,
                "case `{}` expected ToolCallStart event",
                case_name
            );
        } else {
            assert_eq!(
                result.tool_calls_made, 0,
                "case `{}` should not execute tools",
                case_name
            );
        }

        let text = result
            .provider_response
            .choices
            .first()
            .and_then(|choice| choice.content())
            .unwrap_or_default();
        assert!(
            !text.trim().is_empty(),
            "case `{}` expected non-empty final response",
            case_name
        );
    }
}
