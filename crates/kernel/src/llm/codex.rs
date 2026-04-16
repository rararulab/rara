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

//! Codex backend driver — thin wrapper around [`OpenAiDriver`] configured
//! with [`ApiFormat::Responses`].
//!
//! Authentication uses OAuth tokens from `rara login codex`, resolved via
//! [`LlmCredentialResolverRef`].  The Codex backend endpoint
//! (`chatgpt.com/backend-api/codex/responses`) uses the OpenAI Responses API,
//! so all request/response handling is delegated to `OpenAiDriver`.

use async_trait::async_trait;
use tokio::sync::mpsc;

use super::{
    CompletionRequest, CompletionResponse, LlmCredentialResolverRef, StreamDelta,
    driver::LlmDriver,
    openai::{ApiFormat, OpenAiDriver},
};
use crate::error::Result;

/// Base URL for the ChatGPT backend API.
const CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api";

/// Codex driver that calls the ChatGPT backend Responses API.
///
/// Delegates all LLM operations to an inner [`OpenAiDriver`] configured
/// with [`ApiFormat::Responses`] and the Codex-specific request path.
pub struct CodexDriver {
    inner: OpenAiDriver,
}

impl CodexDriver {
    /// Create a new Codex driver with a credential resolver.
    ///
    /// The resolver must return credentials with `extra_headers` containing
    /// `chatgpt-account-id` (set up by `CodexCredentialResolver`).
    pub fn new(resolver: LlmCredentialResolverRef) -> Self {
        let inner =
            OpenAiDriver::with_credential_resolver(resolver, std::time::Duration::from_secs(120))
                .with_api_format(ApiFormat::Responses)
                .with_base_url_override(CODEX_BASE_URL)
                .with_request_path_override("/codex/responses");
        Self { inner }
    }
}

#[async_trait]
impl LlmDriver for CodexDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        self.inner.complete(request).await
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        tx: mpsc::Sender<StreamDelta>,
    ) -> Result<CompletionResponse> {
        self.inner.stream(request, tx).await
    }

    async fn model_context_length(&self, _model: &str) -> Option<usize> {
        // Codex models typically support 192k+ context.
        Some(192_000)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tokio::sync::mpsc;

    use super::super::{
        CompletionRequest, StreamDelta,
        openai::{ApiFormat, ResponsesStreamState, build_responses_request, parse_responses_event},
        types::{Message, StopReason, ThinkingConfig, ToolDefinition},
    };

    #[test]
    fn build_request_user_message_uses_input_text_format() {
        let request = CompletionRequest {
            model:               "codex-mini".into(),
            messages:            vec![Message::user("hello world")],
            tools:               vec![],
            temperature:         None,
            max_tokens:          None,
            thinking:            None,
            tool_choice:         Default::default(),
            parallel_tool_calls: false,
            frequency_penalty:   None,
            top_p:               None,
            emit_reasoning:      false,
        };

        let body = build_responses_request(&request, ApiFormat::Responses);
        let input = body["input"].as_array().expect("input should be array");
        assert_eq!(input.len(), 1);

        let msg = &input[0];
        assert_eq!(msg["type"], "message");
        assert_eq!(msg["role"], "user");

        let content = msg["content"].as_array().expect("content should be array");
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "input_text");
        assert_eq!(content[0]["text"], "hello world");
    }

    #[test]
    fn build_request_assistant_message_uses_output_text_format() {
        let request = CompletionRequest {
            model:               "codex-mini".into(),
            messages:            vec![Message::assistant("I can help")],
            tools:               vec![],
            temperature:         None,
            max_tokens:          None,
            thinking:            None,
            tool_choice:         Default::default(),
            parallel_tool_calls: false,
            frequency_penalty:   None,
            top_p:               None,
            emit_reasoning:      false,
        };

        let body = build_responses_request(&request, ApiFormat::Responses);
        let input = body["input"].as_array().expect("input should be array");
        let content = input[0]["content"]
            .as_array()
            .expect("content should be array");
        assert_eq!(content[0]["type"], "output_text");
        assert_eq!(content[0]["text"], "I can help");
    }

    #[test]
    fn build_request_includes_truncation_and_store() {
        let request = CompletionRequest {
            model:               "codex-mini".into(),
            messages:            vec![],
            tools:               vec![],
            temperature:         None,
            max_tokens:          None,
            thinking:            None,
            tool_choice:         Default::default(),
            parallel_tool_calls: false,
            frequency_penalty:   None,
            top_p:               None,
            emit_reasoning:      false,
        };

        let body = build_responses_request(&request, ApiFormat::Responses);
        assert_eq!(body["store"], false);
        assert!(body.get("truncation").is_none());
        assert_eq!(body["reasoning"]["effort"], "medium");
        assert_eq!(body["reasoning"]["summary"], "auto");
    }

    #[test]
    fn build_request_max_tokens_and_tools() {
        let request = CompletionRequest {
            model:               "codex-mini".into(),
            messages:            vec![],
            tools:               vec![ToolDefinition {
                name:        "read_file".into(),
                description: "Read a file".into(),
                parameters:  json!({"type": "object"}),
            }],
            temperature:         None,
            max_tokens:          Some(4096),
            thinking:            None,
            tool_choice:         Default::default(),
            parallel_tool_calls: true,
            frequency_penalty:   None,
            top_p:               None,
            emit_reasoning:      false,
        };

        let body = build_responses_request(&request, ApiFormat::Responses);
        assert!(body.get("max_output_tokens").is_none());
        assert!(body.get("parallel_tool_calls").is_none());
        assert!(body["tools"].as_array().expect("tools array").len() == 1);
    }

    #[test]
    fn build_request_high_thinking_budget_maps_to_high_effort() {
        let request = CompletionRequest {
            model:               "codex-mini".into(),
            messages:            vec![],
            tools:               vec![],
            temperature:         None,
            max_tokens:          None,
            thinking:            Some(ThinkingConfig {
                enabled:       true,
                budget_tokens: Some(20_000),
            }),
            tool_choice:         Default::default(),
            parallel_tool_calls: false,
            frequency_penalty:   None,
            top_p:               None,
            emit_reasoning:      false,
        };

        let body = build_responses_request(&request, ApiFormat::Responses);
        assert_eq!(body["reasoning"]["effort"], "high");
    }

    #[test]
    fn build_request_tool_call_and_tool_result_format() {
        let request = CompletionRequest {
            model:               "codex-mini".into(),
            messages:            vec![
                Message::assistant_with_tool_calls(
                    "",
                    vec![super::super::types::ToolCallRequest {
                        id:        "call_123".into(),
                        name:      "read_file".into(),
                        arguments: r#"{"path":"foo.rs"}"#.into(),
                    }],
                ),
                Message::tool_result("call_123", "file contents here"),
            ],
            tools:               vec![],
            temperature:         None,
            max_tokens:          None,
            thinking:            None,
            tool_choice:         Default::default(),
            parallel_tool_calls: false,
            frequency_penalty:   None,
            top_p:               None,
            emit_reasoning:      false,
        };

        let body = build_responses_request(&request, ApiFormat::Responses);
        let input = body["input"].as_array().expect("input should be array");

        assert_eq!(input[0]["type"], "function_call");
        assert_eq!(input[0]["name"], "read_file");
        assert_eq!(input[0]["call_id"], "call_123");

        assert_eq!(input[1]["type"], "function_call_output");
        assert_eq!(input[1]["call_id"], "call_123");
        assert_eq!(input[1]["output"], "file contents here");
    }

    #[test]
    fn parse_text_delta_event() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut state = ResponsesStreamState::new();

        let data = r#"{"delta":"hello ","item_id":"item_1","output_index":0}"#;
        let result = parse_responses_event("response.output_text.delta", data, &tx, &mut state);
        assert!(result.is_none());

        let delta = rx.try_recv().expect("should receive delta");
        match delta {
            StreamDelta::TextDelta { text } => assert_eq!(text, "hello "),
            other => panic!("unexpected delta: {other:?}"),
        }
        assert_eq!(state.accumulated_text, "hello ");
    }

    #[test]
    fn parse_reasoning_delta_event() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut state = ResponsesStreamState::new();

        let data = r#"{"delta":"thinking...","summary_index":0}"#;
        let result = parse_responses_event(
            "response.reasoning_summary_text.delta",
            data,
            &tx,
            &mut state,
        );
        assert!(result.is_none());

        let delta = rx.try_recv().expect("should receive reasoning delta");
        match delta {
            StreamDelta::ReasoningDelta { text } => assert_eq!(text, "thinking..."),
            other => panic!("unexpected delta: {other:?}"),
        }
        assert_eq!(state.accumulated_reasoning, "thinking...");
    }

    #[test]
    fn parse_tool_call_flow() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut state = ResponsesStreamState::new();

        let added = r#"{"item":{"type":"function_call","id":"item_1","call_id":"call_abc","name":"read_file","arguments":""},"output_index":2}"#;
        parse_responses_event("response.output_item.added", added, &tx, &mut state);

        let start = rx.try_recv().expect("should receive ToolCallStart");
        match start {
            StreamDelta::ToolCallStart { index, id, name } => {
                assert_eq!(index, 0);
                assert_eq!(id, "call_abc");
                assert_eq!(name, "read_file");
            }
            other => panic!("unexpected: {other:?}"),
        }

        let args_delta = r#"{"delta":"{\"path\"","output_index":2}"#;
        parse_responses_event(
            "response.function_call_arguments.delta",
            args_delta,
            &tx,
            &mut state,
        );

        let arg = rx.try_recv().expect("should receive args delta");
        match arg {
            StreamDelta::ToolCallArgumentsDelta { index, arguments } => {
                assert_eq!(index, 0);
                assert_eq!(arguments, r#"{"path""#);
            }
            other => panic!("unexpected: {other:?}"),
        }

        let done = r#"{"item":{"type":"function_call","id":"item_1","call_id":"call_abc","name":"read_file","arguments":"{\"path\":\"foo.rs\"}"},"output_index":2}"#;
        parse_responses_event("response.output_item.done", done, &tx, &mut state);
        assert!(state.has_function_call);
    }

    #[test]
    fn parse_completed_event_with_usage() {
        let (tx, _rx) = mpsc::channel(16);
        let mut state = ResponsesStreamState::new();

        let data = r#"{"response":{"status":"completed","usage":{"input_tokens":100,"output_tokens":50}}}"#;
        let result = parse_responses_event("response.completed", data, &tx, &mut state);
        assert_eq!(result, Some(true));
        assert_eq!(state.final_stop, StopReason::Stop);

        let usage = state.final_usage.expect("should have usage");
        assert_eq!(usage.prompt_tokens, 100);
        assert_eq!(usage.completion_tokens, 50);
        assert_eq!(usage.total_tokens, 150);
    }

    #[test]
    fn parse_completed_with_function_call_yields_tool_calls_stop() {
        let (tx, _rx) = mpsc::channel(16);
        let mut state = ResponsesStreamState::new();
        state.has_function_call = true;

        let data =
            r#"{"response":{"status":"completed","usage":{"input_tokens":10,"output_tokens":5}}}"#;
        let result = parse_responses_event("response.completed", data, &tx, &mut state);
        assert_eq!(result, Some(true));
        assert_eq!(state.final_stop, StopReason::ToolCalls);
    }

    #[test]
    fn parse_incomplete_max_output_tokens() {
        let (tx, _rx) = mpsc::channel(16);
        let mut state = ResponsesStreamState::new();

        let data = r#"{"response":{"incomplete_details":{"reason":"max_output_tokens"},"usage":{"input_tokens":50,"output_tokens":200}}}"#;
        let result = parse_responses_event("response.incomplete", data, &tx, &mut state);
        assert_eq!(result, Some(true));
        assert_eq!(state.final_stop, StopReason::Length);
    }

    #[test]
    fn parse_incomplete_content_filter() {
        let (tx, _rx) = mpsc::channel(16);
        let mut state = ResponsesStreamState::new();

        let data = r#"{"response":{"incomplete_details":{"reason":"content_filter"},"usage":{"input_tokens":50,"output_tokens":10}}}"#;
        let result = parse_responses_event("response.incomplete", data, &tx, &mut state);
        assert_eq!(result, Some(true));
        assert_eq!(state.final_stop, StopReason::ContentFilter);
    }

    #[test]
    fn unknown_event_type_does_not_crash() {
        let (tx, _rx) = mpsc::channel(16);
        let mut state = ResponsesStreamState::new();

        let data = r#"{"some":"data"}"#;
        let result = parse_responses_event("response.something.new", data, &tx, &mut state);
        assert!(result.is_none());
    }
}
