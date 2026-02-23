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

use async_openai::types::chat::{
    ChatCompletionMessageToolCall, ChatCompletionMessageToolCalls,
    ChatCompletionRequestAssistantMessageArgs,
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
    ChatCompletionRequestToolMessageArgs, ChatCompletionRequestUserMessageArgs,
    ChatCompletionToolChoiceOption, CreateChatCompletionRequestArgs,
    CreateChatCompletionResponse, FunctionCall, FinishReason, ToolChoiceOptions,
};
use backon::{ExponentialBuilder, Retryable};
use base::shared_string::SharedString;
use bon::Builder;
use futures::StreamExt;
use tokio::sync::mpsc;
use tracing::{error, info, trace, warn};

use crate::{err::prelude::*, model::LlmProviderLoaderRef, tool_registry::ToolRegistry};

/// Maximum number of tool-call loop iterations before giving up.
pub const MAX_ITERATIONS: usize = 25;

/// Result of running the agent loop.
#[derive(Debug, Clone)]
pub struct AgentRunResponse {
    /// Raw provider response for the terminal assistant turn.
    pub provider_response: CreateChatCompletionResponse,
    /// Number of loop iterations consumed before termination.
    pub iterations:        usize,
    /// Total number of tool calls executed across all iterations.
    pub tool_calls_made:   usize,
}

impl AgentRunResponse {
    /// Extract the assistant's text content from the terminal response.
    pub fn response_text(&self) -> String {
        self.provider_response
            .choices
            .first()
            .and_then(|c| c.message.content.as_deref())
            .unwrap_or_default()
            .to_owned()
    }
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

/// User content for the agent runner — text or multimodal.
#[derive(Debug, Clone)]
pub enum UserContent {
    /// Plain text content.
    Text(String),
    /// Multimodal content with text and image URLs.
    Multimodal {
        text:       String,
        image_urls: Vec<String>,
    },
}

impl UserContent {
    /// Return the text portion of the user content.
    pub fn text(&self) -> &str {
        match self {
            UserContent::Text(t) => t,
            UserContent::Multimodal { text, .. } => text,
        }
    }
}

impl From<String> for UserContent {
    fn from(text: String) -> Self { UserContent::Text(text) }
}

impl From<&str> for UserContent {
    fn from(text: &str) -> Self { UserContent::Text(text.to_owned()) }
}

#[derive(Builder)]
#[builder(on(SharedString, into))]
pub struct AgentRunner {
    llm_provider:    LlmProviderLoaderRef,
    model_name:      SharedString,
    system_prompt:   SharedString,
    user_content:    UserContent,
    history:         Option<Vec<ChatCompletionRequestMessage>>,
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
        let is_multimodal = matches!(self.user_content, UserContent::Multimodal { .. });
        info!(model_name = model, is_multimodal, "starting agent loop");

        // prepare messages with system prompt, optional history, and current user
        // message
        let mut messages: Vec<ChatCompletionRequestMessage> = {
            let mut messages = vec![
                ChatCompletionRequestSystemMessageArgs::default()
                    .content(self.system_prompt.as_ref())
                    .build()
                    .map_err(|e| Error::Other {
                        message: format!("failed to build system message: {e}").into(),
                    })?
                    .into(),
            ];
            // Insert conversation history before the current user message.
            if let Some(hist) = &self.history {
                messages.extend(hist.clone());
            }
            messages.push(build_user_message(&self.user_content)?);
            messages
        };

        let request_tools = if tools.is_empty() {
            None
        } else {
            Some(tools.to_chat_completion_tools()?)
        };
        let mut tool_calls_made = 0_usize;

        for iteration in 0..self.max_iterations {
            // ---- Phase 1: begin one loop tick ----
            if let Some(cb) = on_event {
                cb(RunnerEvent::Iteration(iteration));
            }
            info!(iteration, messages_count = messages.len(), "calling LLM");
            trace!(iteration = iteration, messages = ?messages, "LLM request messages");
            if let Some(cb) = on_event {
                cb(RunnerEvent::Thinking);
            }

            let provider = self.llm_provider.acquire_provider().await?;

            // ---- Phase 2: build/send request with retry ----
            let response = (|| async {
                let mut request_builder = CreateChatCompletionRequestArgs::default();
                request_builder
                    .model(model)
                    .messages(messages.clone())
                    .temperature(0.7_f32);

                if let Some(tool_defs) = &request_tools {
                    request_builder.tools(tool_defs.clone());
                    request_builder.tool_choice(ChatCompletionToolChoiceOption::Mode(ToolChoiceOptions::Auto));
                    request_builder.parallel_tool_calls(true);
                }

                let request = request_builder.build().map_err(|e| Error::Other {
                    message: format!("failed to build request: {e}").into(),
                })?;
                provider.chat_completion(request).await
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
            trace!(
                iteration = iteration,
                has_content = !response.choices.is_empty(),
                prompt_tokens = response.usage.as_ref().map_or(0, |it| it.prompt_tokens),
                completion_tokens = response.usage.as_ref().map_or(0, |it| it.completion_tokens),
                "LLM response received"
            );

            let Some(choice) = response.choices.first() else {
                return Err(Error::Other {
                    message: "LLM returned no choices".into(),
                });
            };

            let assistant_text = choice.message.content.clone().unwrap_or_default();
            let raw_tool_calls = choice.message.tool_calls.as_deref().unwrap_or(&[]);

            // Extract function tool calls, ignoring custom tool types.
            let tool_calls: Vec<&ChatCompletionMessageToolCall> = raw_tool_calls
                .iter()
                .filter_map(|tc| match tc {
                    ChatCompletionMessageToolCalls::Function(f) => Some(f),
                    _ => None,
                })
                .collect();

            if tool_calls.is_empty() {
                // Terminal path: model produced final assistant output.
                return Ok(AgentRunResponse {
                    provider_response: response,
                    iterations: iteration + 1,
                    tool_calls_made,
                });
            }

            // ---- Phase 4: execute model-requested tools ----
            info!(
                iteration,
                tool_calls = tool_calls.len(),
                "assistant requested tool calls"
            );

            // Append assistant message that contains `tool_calls`.
            let assistant_msg = ChatCompletionRequestAssistantMessageArgs::default()
                .content(assistant_text.as_str())
                .tool_calls(raw_tool_calls.to_vec())
                .build()
                .map_err(|e| Error::Other {
                    message: format!("failed to build assistant tool-call message: {e}").into(),
                })?;
            messages.push(assistant_msg.into());

            for tool_call in &tool_calls {
                tool_calls_made = tool_calls_made.saturating_add(1);
                let tool_name = &tool_call.function.name;
                let tool_id = &tool_call.id;

                // Parse function args as JSON.
                let tool_arguments =
                    match serde_json::from_str::<serde_json::Value>(&tool_call.function.arguments) {
                        Ok(value) => value,
                        Err(err) => {
                            let error_message = format!("invalid tool arguments: {err}");
                            if let Some(cb) = on_event {
                                cb(RunnerEvent::ToolCallEnd {
                                    id:      tool_id.clone(),
                                    name:    tool_name.clone(),
                                    success: false,
                                    error:   Some(error_message.clone()),
                                    result:  None,
                                });
                            }
                            messages.push(build_tool_response_message(
                                tool_id,
                                &serde_json::json!({ "error": error_message }).to_string(),
                            )?);
                            continue;
                        }
                    };

                // Emit start event.
                if let Some(cb) = on_event {
                    cb(RunnerEvent::ToolCallStart {
                        id:        tool_id.clone(),
                        name:      tool_name.clone(),
                        arguments: tool_arguments.clone(),
                    });
                }

                // Execute tool.
                let tool_response_payload = if let Some(tool) = tools.get(tool_name) {
                    match tool.execute(tool_arguments.clone()).await {
                        Ok(result) => {
                            if let Some(cb) = on_event {
                                cb(RunnerEvent::ToolCallEnd {
                                    id:      tool_id.clone(),
                                    name:    tool_name.clone(),
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
                                    id:      tool_id.clone(),
                                    name:    tool_name.clone(),
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
                            id:      tool_id.clone(),
                            name:    tool_name.clone(),
                            success: false,
                            error:   Some(error_message.clone()),
                            result:  None,
                        });
                    }
                    serde_json::json!({ "error": error_message })
                };

                messages.push(build_tool_response_message(
                    tool_id,
                    &tool_response_payload.to_string(),
                )?);
            }
        }

        Err(Error::Other {
            message: "agent loop exceeded max iterations".into(),
        })?
    }

    /// Streaming variant of [`run`]. Spawns the agent loop in a background
    /// task and returns a channel receiver yielding [`RunnerEvent`]s in
    /// real-time, including incremental text/reasoning deltas.
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
    async fn run_streaming_inner(
        &self,
        model: &str,
        tools: &ToolRegistry,
        tx: &mpsc::Sender<RunnerEvent>,
    ) -> Result<()> {
        info!(model_name = model, "starting streaming agent loop");

        // ---- Prepare messages ----
        let mut messages: Vec<ChatCompletionRequestMessage> = {
            let mut msgs = vec![
                ChatCompletionRequestSystemMessageArgs::default()
                    .content(self.system_prompt.as_ref())
                    .build()
                    .map_err(|e| Error::Other {
                        message: format!("failed to build system message: {e}").into(),
                    })?
                    .into(),
            ];
            if let Some(hist) = &self.history {
                msgs.extend(hist.clone());
            }
            msgs.push(build_user_message(&self.user_content)?);
            msgs
        };

        let request_tools = if tools.is_empty() {
            None
        } else {
            Some(tools.to_chat_completion_tools()?)
        };
        let mut tool_calls_made = 0_usize;

        // ---- Main agent loop ----
        for iteration in 0..self.max_iterations {
            let _ = tx.send(RunnerEvent::Iteration(iteration)).await;
            let _ = tx.send(RunnerEvent::Thinking).await;
            info!(iteration, messages_count = messages.len(), "calling LLM (streaming)");

            let provider = self.llm_provider.acquire_provider().await?;

            // ---- Phase 1: Build and send streaming request ----
            let mut request_builder = CreateChatCompletionRequestArgs::default();
            request_builder
                .model(model)
                .messages(messages.clone())
                .temperature(0.7_f32);

            if let Some(tool_defs) = &request_tools {
                request_builder.tools(tool_defs.clone());
                request_builder.tool_choice(ChatCompletionToolChoiceOption::Mode(ToolChoiceOptions::Auto));
                request_builder.parallel_tool_calls(true);
            }

            let request = request_builder.build().map_err(|e| Error::Other {
                message: format!("failed to build streaming request: {e}").into(),
            })?;

            let mut stream = provider.chat_completion_stream(request).await?;

            // ---- Phase 2: Consume streaming chunks ----
            let mut accumulated_text = String::new();
            let mut pending_tool_calls: HashMap<u32, PendingToolCall> = HashMap::new();
            let mut has_tool_calls = false;

            while let Some(chunk_result) = stream.next().await {
                let response = match chunk_result {
                    Ok(r) => r,
                    Err(e) => {
                        error!(
                            iteration,
                            model,
                            error = %e,
                            "streaming chunk error"
                        );
                        return Err(Error::Provider {
                            message: format!(
                                "Model \"{model}\" returned an error during streaming: {e}"
                            ).into(),
                        });
                    }
                };

                let Some(choice) = response.choices.first() else {
                    continue;
                };

                // --- Text content delta ---
                if let Some(ref text) = choice.delta.content {
                    if !text.is_empty() {
                        accumulated_text.push_str(text);
                        let _ = tx.send(RunnerEvent::TextDelta(text.clone())).await;
                    }
                }

                // --- Tool call delta accumulation ---
                if let Some(ref tool_calls_delta) = choice.delta.tool_calls {
                    for tc in tool_calls_delta {
                        let idx = tc.index;
                        let entry =
                            pending_tool_calls.entry(idx).or_insert_with(|| {
                                PendingToolCall {
                                    id:              String::new(),
                                    name:            String::new(),
                                    arguments_buf:   String::new(),
                                }
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

                // --- Check finish_reason ---
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
            let mut sorted_indices: Vec<u32> = pending_tool_calls.keys().copied().collect();
            sorted_indices.sort_unstable();

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
            let openai_tool_calls: Vec<ChatCompletionMessageToolCalls> = tool_call_list
                .iter()
                .map(|(id, name, args)| {
                    ChatCompletionMessageToolCalls::Function(ChatCompletionMessageToolCall {
                        id:       id.clone(),
                        function: FunctionCall {
                            name:      name.clone(),
                            arguments: serde_json::to_string(args).unwrap_or_default(),
                        },
                    })
                })
                .collect();

            let assistant_msg = ChatCompletionRequestAssistantMessageArgs::default()
                .content(accumulated_text.clone())
                .tool_calls(openai_tool_calls)
                .build()
                .map_err(|e| Error::Other {
                    message: format!("failed to build assistant message: {e}").into(),
                })?;
            messages.push(assistant_msg.into());

            // Emit ToolCallStart events.
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

            // Execute all tool calls concurrently.
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

            // Emit ToolCallEnd events and append tool response messages.
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

                messages.push(build_tool_response_message(id, &result.to_string())?);
            }
        }

        Err(Error::Other {
            message: "agent loop exceeded max iterations".into(),
        })
    }
}

/// A tool call being incrementally assembled from streaming SSE chunks.
struct PendingToolCall {
    /// Unique tool call identifier assigned by the model (e.g. "call_abc123").
    id:            String,
    /// Tool function name (e.g. "web_search").
    name:          String,
    /// Accumulated JSON argument string, built by concatenating fragments.
    arguments_buf: String,
}

/// Build a user message from [`UserContent`].
fn build_user_message(content: &UserContent) -> Result<ChatCompletionRequestMessage> {
    match content {
        UserContent::Text(text) => {
            let msg = ChatCompletionRequestUserMessageArgs::default()
                .content(text.as_str())
                .build()
                .map_err(|e| Error::Other {
                    message: format!("failed to build user message: {e}").into(),
                })?;
            Ok(msg.into())
        }
        UserContent::Multimodal { text, image_urls } => {
            use async_openai::types::chat::{
                ChatCompletionRequestUserMessageContentPart,
                ChatCompletionRequestMessageContentPartImage,
                ChatCompletionRequestMessageContentPartText, ImageUrlArgs,
            };

            let mut parts: Vec<ChatCompletionRequestUserMessageContentPart> = Vec::new();
            parts.push(ChatCompletionRequestUserMessageContentPart::Text(
                ChatCompletionRequestMessageContentPartText {
                    text: text.clone(),
                },
            ));
            for url in image_urls {
                let image_url = ImageUrlArgs::default()
                    .url(url.as_str())
                    .build()
                    .map_err(|e| Error::Other {
                        message: format!("failed to build image URL: {e}").into(),
                    })?;
                parts.push(ChatCompletionRequestUserMessageContentPart::ImageUrl(
                    ChatCompletionRequestMessageContentPartImage {
                        image_url,
                    },
                ));
            }

            let msg = ChatCompletionRequestUserMessageArgs::default()
                .content(parts)
                .build()
                .map_err(|e| Error::Other {
                    message: format!("failed to build multimodal user message: {e}").into(),
                })?;
            Ok(msg.into())
        }
    }
}

/// Build a tool response message.
fn build_tool_response_message(
    tool_call_id: &str,
    content: &str,
) -> Result<ChatCompletionRequestMessage> {
    let msg = ChatCompletionRequestToolMessageArgs::default()
        .tool_call_id(tool_call_id)
        .content(content)
        .build()
        .map_err(|e| Error::Other {
            message: format!("failed to build tool response message: {e}").into(),
        })?;
    Ok(msg.into())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use test_case::test_case;

    use super::*;
    use crate::model::EnvLlmProviderLoader;

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
    // #[ignore = "requires real OpenRouter API key; runs real integration cases"]
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
            .llm_provider(Arc::new(EnvLlmProviderLoader::default()) as LlmProviderLoaderRef)
            .model_name(model_name.clone())
            .system_prompt(system_prompt)
            .user_content(UserContent::Text(user_prompt.to_owned()))
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
            .and_then(|choice| choice.message.content.as_deref())
            .unwrap_or_default();
        assert!(
            !text.trim().is_empty(),
            "case `{}` expected non-empty final response",
            case_name
        );
    }
}
