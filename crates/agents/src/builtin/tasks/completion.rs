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

//! Shared completion helpers for task agents.
//!
//! Provides two execution modes:
//! - `run_completion`: single-round LLM call (no tools)
//! - `run_with_tools`: multi-round tool-calling loop for analysis agents
//!
//! Both include retry-on-empty: if the LLM returns an empty response, the
//! function retries once with a nudge message.

use rara_kernel::{provider::LlmProvider, tool::ToolRegistry};
use async_openai::types::chat::{
    ChatCompletionMessageToolCall, ChatCompletionMessageToolCalls,
    ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestMessage,
    ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestToolMessageArgs,
    ChatCompletionRequestUserMessageArgs, ChatCompletionToolChoiceOption, ChatCompletionTools,
    CreateChatCompletionRequestArgs, ToolChoiceOptions,
};
use tracing::{info, warn};

use crate::builtin::tasks::error::TaskAgentError;

/// Execution mode for a task agent.
#[derive(Debug, Clone)]
pub enum TaskAgentMode {
    /// Current behavior: single LLM call, no tools.
    SingleRound,
    /// Use a tool-calling loop with limited iterations.
    WithTools { max_iterations: usize },
}

/// Default max iterations for tool-calling task agents.
pub const DEFAULT_TASK_TOOL_ITERATIONS: usize = 5;

/// Nudge message appended when the LLM returns an empty response.
const EMPTY_RESPONSE_NUDGE: &str = "Your previous response was empty. Please provide your \
                                    analysis based on the information available.";

/// Send a single system + user message completion request and return the
/// assistant's text response. Retries once on empty response.
pub(crate) async fn run_completion(
    provider: &dyn LlmProvider,
    model: &str,
    system_prompt: &str,
    user_input: &str,
) -> Result<String, TaskAgentError> {
    let result = run_completion_inner(provider, model, system_prompt, user_input).await?;

    if !result.trim().is_empty() {
        return Ok(result);
    }

    // Retry once with a nudge.
    warn!("LLM returned empty response, retrying with nudge");
    let nudged_input = format!("{user_input}\n\n{EMPTY_RESPONSE_NUDGE}");
    let retry_result = run_completion_inner(provider, model, system_prompt, &nudged_input).await?;

    if retry_result.trim().is_empty() {
        return Err(TaskAgentError::EmptyResponse);
    }

    Ok(retry_result)
}

/// Inner single-round completion without retry logic.
async fn run_completion_inner(
    provider: &dyn LlmProvider,
    model: &str,
    system_prompt: &str,
    user_input: &str,
) -> Result<String, TaskAgentError> {
    let request = CreateChatCompletionRequestArgs::default()
        .model(model)
        .messages(vec![
            ChatCompletionRequestSystemMessageArgs::default()
                .content(system_prompt)
                .build()
                .expect("system message")
                .into(),
            ChatCompletionRequestUserMessageArgs::default()
                .content(user_input)
                .build()
                .expect("user message")
                .into(),
        ])
        .build()
        .expect("chat completion request");

    let response =
        provider
            .chat_completion(request)
            .await
            .map_err(|e| TaskAgentError::RequestFailed {
                message: e.to_string(),
            })?;

    let choice = response
        .choices
        .first()
        .ok_or(TaskAgentError::EmptyResponse)?;

    Ok(choice.message.content.clone().unwrap_or_default())
}

/// Run a tool-calling loop: send messages to the LLM, execute tool calls,
/// repeat until the model produces a final text response or the iteration
/// limit is reached.
///
/// If no tools are provided (empty registry), this degrades gracefully to a
/// single-round completion — exactly like [`run_completion`].
///
/// Retries once on empty final response.
pub(crate) async fn run_with_tools(
    provider: &dyn LlmProvider,
    model: &str,
    system_prompt: &str,
    user_input: &str,
    tools: &ToolRegistry,
    max_iterations: usize,
) -> Result<String, TaskAgentError> {
    // Fast path: if no tools, fall back to single-round completion.
    if tools.is_empty() {
        return run_completion(provider, model, system_prompt, user_input).await;
    }

    let tool_defs: Vec<ChatCompletionTools> =
        tools
            .to_chat_completion_tools()
            .map_err(|e| TaskAgentError::RequestFailed {
                message: format!("failed to build tool definitions: {e}"),
            })?;

    let result = run_tool_loop_inner(
        provider,
        model,
        system_prompt,
        user_input,
        tools,
        &tool_defs,
        max_iterations,
    )
    .await?;

    if !result.trim().is_empty() {
        return Ok(result);
    }

    // Retry once with a nudge.
    warn!("tool-calling loop returned empty response, retrying with nudge");
    let nudged_input = format!("{user_input}\n\n{EMPTY_RESPONSE_NUDGE}");
    let retry_result = run_tool_loop_inner(
        provider,
        model,
        system_prompt,
        &nudged_input,
        tools,
        &tool_defs,
        max_iterations,
    )
    .await?;

    if retry_result.trim().is_empty() {
        return Err(TaskAgentError::EmptyResponse);
    }

    Ok(retry_result)
}

/// Inner tool-calling loop without retry logic.
async fn run_tool_loop_inner(
    provider: &dyn LlmProvider,
    model: &str,
    system_prompt: &str,
    user_input: &str,
    tools: &ToolRegistry,
    tool_defs: &[ChatCompletionTools],
    max_iterations: usize,
) -> Result<String, TaskAgentError> {
    let mut messages: Vec<ChatCompletionRequestMessage> = vec![
        ChatCompletionRequestSystemMessageArgs::default()
            .content(system_prompt)
            .build()
            .expect("system message")
            .into(),
        ChatCompletionRequestUserMessageArgs::default()
            .content(user_input)
            .build()
            .expect("user message")
            .into(),
    ];

    for iteration in 0..max_iterations {
        info!(
            iteration,
            messages_count = messages.len(),
            "task agent tool-calling iteration"
        );

        let request = CreateChatCompletionRequestArgs::default()
            .model(model)
            .messages(messages.clone())
            .tools(tool_defs.to_vec())
            .tool_choice(ChatCompletionToolChoiceOption::Mode(
                ToolChoiceOptions::Auto,
            ))
            .build()
            .expect("chat completion request with tools");

        let response =
            provider
                .chat_completion(request)
                .await
                .map_err(|e| TaskAgentError::RequestFailed {
                    message: e.to_string(),
                })?;

        let choice = response
            .choices
            .first()
            .ok_or(TaskAgentError::EmptyResponse)?;

        let assistant_text = choice.message.content.clone().unwrap_or_default();
        let raw_tool_calls = choice.message.tool_calls.as_deref().unwrap_or(&[]);

        // Extract function tool calls.
        let tool_calls: Vec<&ChatCompletionMessageToolCall> = raw_tool_calls
            .iter()
            .filter_map(|tc| match tc {
                ChatCompletionMessageToolCalls::Function(f) => Some(f),
                _ => None,
            })
            .collect();

        if tool_calls.is_empty() {
            // Terminal: model produced final text output.
            info!(
                iteration,
                "task agent tool-calling loop completed (no more tool calls)"
            );
            return Ok(assistant_text);
        }

        // Append assistant message with tool_calls to conversation.
        let assistant_msg = ChatCompletionRequestAssistantMessageArgs::default()
            .content(assistant_text.as_str())
            .tool_calls(raw_tool_calls.to_vec())
            .build()
            .expect("assistant tool-call message");
        messages.push(assistant_msg.into());

        // Execute each tool call and append results.
        for tool_call in &tool_calls {
            let tool_name = &tool_call.function.name;
            let tool_id = &tool_call.id;

            let tool_arguments =
                match serde_json::from_str::<serde_json::Value>(&tool_call.function.arguments) {
                    Ok(value) => value,
                    Err(err) => {
                        let error_message = format!("invalid tool arguments: {err}");
                        warn!(tool_name, %err, "invalid tool arguments from LLM");
                        messages.push(build_tool_response_message(
                            tool_id,
                            &serde_json::json!({ "error": error_message }).to_string(),
                        ));
                        continue;
                    }
                };

            info!(tool_name, iteration, "executing tool call");

            let tool_response_payload = if let Some(tool) = tools.get(tool_name) {
                match tool.execute(tool_arguments).await {
                    Ok(result) => result,
                    Err(err) => {
                        warn!(tool_name, %err, "tool execution failed");
                        serde_json::json!({ "error": err.to_string() })
                    }
                }
            } else {
                warn!(tool_name, "tool not found in registry");
                serde_json::json!({ "error": format!("tool not found: {tool_name}") })
            };

            messages.push(build_tool_response_message(
                tool_id,
                &tool_response_payload.to_string(),
            ));
        }
    }

    Err(TaskAgentError::MaxIterationsExceeded {
        max: max_iterations,
    })
}

/// Build a tool response message for the conversation history.
fn build_tool_response_message(tool_call_id: &str, content: &str) -> ChatCompletionRequestMessage {
    ChatCompletionRequestToolMessageArgs::default()
        .tool_call_id(tool_call_id)
        .content(content)
        .build()
        .expect("tool response message")
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_agent_mode_debug() {
        let single = TaskAgentMode::SingleRound;
        let with_tools = TaskAgentMode::WithTools { max_iterations: 3 };
        // Ensure Debug is implemented.
        assert!(format!("{single:?}").contains("SingleRound"));
        assert!(format!("{with_tools:?}").contains("WithTools"));
    }

    #[test]
    fn empty_nudge_message_is_not_blank() {
        assert!(!EMPTY_RESPONSE_NUDGE.trim().is_empty());
    }
}
