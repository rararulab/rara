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

//! Codex backend driver — calls `chatgpt.com/backend-api/codex/responses`
//! using OAuth tokens from `rara login codex`.
//!
//! This is a separate driver from [`super::openai::OpenAiDriver`] because
//! the Codex backend uses the OpenAI **Responses API** (not Chat
//! Completions). The request/response formats differ fundamentally:
//!
//! - **Request**: `input[]` items (not `messages[]`), with `type` tags
//!   (`message`, `function_call`, `function_call_output`)
//! - **Response**: SSE with `response.*` events (not `chat.completion.chunk`)
//! - **Auth**: OAuth token + `ChatGPT-Account-Id` header (not API key)
//! - **Endpoint**: `/codex/responses` (not `/chat/completions`)
//!
//! Reference: <https://github.com/numman-ali/opencode-openai-codex-auth>

use std::{collections::HashMap, time::Duration};

use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::{
    CompletionRequest, CompletionResponse, LlmCredentialResolverRef, StreamDelta, Usage,
    driver::LlmDriver, types::StopReason,
};
use crate::error::{KernelError, Result};

/// Base URL for the ChatGPT backend API.
const CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api";

/// SSE idle timeout — if no event arrives for this long, abort the stream.
const SSE_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

/// Codex driver that calls the ChatGPT backend Responses API.
pub struct CodexDriver {
    resolver:      LlmCredentialResolverRef,
    client:        Client,
    stream_client: Client,
}

impl CodexDriver {
    /// Create a new Codex driver with a credential resolver.
    ///
    /// The resolver must return credentials with `extra_headers` containing
    /// `chatgpt-account-id` (set up by `CodexCredentialResolver`).
    pub fn new(resolver: LlmCredentialResolverRef) -> Self {
        let proxy = std::env::var("HTTPS_PROXY")
            .or_else(|_| std::env::var("https_proxy"))
            .or_else(|_| std::env::var("ALL_PROXY"))
            .or_else(|_| std::env::var("all_proxy"))
            .ok();

        let mut client_builder = Client::builder().timeout(Duration::from_secs(300));
        let mut stream_builder = Client::builder(); // no total timeout for SSE

        if let Some(ref url) = proxy {
            tracing::info!(%url, "CodexDriver: using proxy");
            if let Ok(p) = reqwest::Proxy::all(url) {
                client_builder = client_builder.proxy(p.clone());
                stream_builder = stream_builder.proxy(p);
            }
        }

        let client = client_builder.build().unwrap_or_default();
        let stream_client = stream_builder.build().unwrap_or_default();
        Self {
            resolver,
            client,
            stream_client,
        }
    }

    /// Resolve credential and build headers.
    async fn resolve(&self) -> Result<(String, reqwest::header::HeaderMap)> {
        let cred = self.resolver.resolve().await?;
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", cred.api_key())
                .parse()
                .expect("valid header value"),
        );
        headers.insert(
            "openai-beta",
            "responses=experimental"
                .parse()
                .expect("valid header value"),
        );
        headers.insert(
            "originator",
            "codex_cli_rs".parse().expect("valid header value"),
        );
        for (name, value) in cred.extra_headers() {
            if let (Ok(n), Ok(v)) = (
                name.parse::<reqwest::header::HeaderName>(),
                value.parse::<reqwest::header::HeaderValue>(),
            ) {
                headers.insert(n, v);
            }
        }
        let base = cred.base_url().to_owned();
        // Override to Codex backend regardless of what the credential says.
        let _ = base;
        Ok((CODEX_BASE_URL.to_owned(), headers))
    }
}

// ---------------------------------------------------------------------------
// Request body construction
// ---------------------------------------------------------------------------

/// Convert our internal `CompletionRequest` (messages-based) into
/// the Responses API `input[]` format.
///
/// The Responses API expects structured content arrays with explicit type
/// tags (`input_text`, `output_text`) rather than plain strings.
fn build_codex_request(request: &CompletionRequest) -> Value {
    let mut input = Vec::new();

    for msg in &request.messages {
        match msg.role {
            super::types::Role::System
            | super::types::Role::Developer
            | super::types::Role::User => {
                let role_str = match msg.role {
                    super::types::Role::System | super::types::Role::Developer => "developer",
                    super::types::Role::User => "user",
                    _ => unreachable!(),
                };
                let content_type = if role_str == "user" {
                    "input_text"
                } else {
                    // developer messages use plain string content
                    "input_text"
                };
                input.push(json!({
                    "type": "message",
                    "role": role_str,
                    "content": [{
                        "type": content_type,
                        "text": msg.content.as_text(),
                    }],
                }));
            }
            super::types::Role::Assistant => {
                if msg.tool_calls.is_empty() {
                    input.push(json!({
                        "type": "message",
                        "role": "assistant",
                        "content": [{
                            "type": "output_text",
                            "text": msg.content.as_text(),
                        }],
                    }));
                } else {
                    let text = msg.content.as_text();
                    if !text.is_empty() {
                        input.push(json!({
                            "type": "message",
                            "role": "assistant",
                            "content": [{
                                "type": "output_text",
                                "text": text,
                            }],
                        }));
                    }
                    for tc in &msg.tool_calls {
                        input.push(json!({
                            "type": "function_call",
                            "name": tc.name,
                            "arguments": tc.arguments,
                            "call_id": tc.id,
                        }));
                    }
                }
            }
            super::types::Role::Tool => {
                // Tool results become function_call_output.
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": msg.tool_call_id.as_deref().unwrap_or(""),
                    "output": msg.content.as_text(),
                }));
            }
        }
    }

    // Build tools array in Responses API format.
    let tools: Vec<Value> = request
        .tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "name": t.name,
                "description": t.description,
                "parameters": t.parameters,
            })
        })
        .collect();

    // Determine reasoning effort from thinking config, default to "medium".
    let reasoning_effort = request
        .thinking
        .as_ref()
        .and_then(|t| {
            t.budget_tokens.map(|b| {
                if b >= 10_000 {
                    "high"
                } else if b >= 3_000 {
                    "medium"
                } else {
                    "low"
                }
            })
        })
        .unwrap_or("medium");

    let mut body = json!({
        "model": request.model,
        "input": input,
        "stream": true,
        "store": true,
        "truncation": "auto",
    });

    if !tools.is_empty() {
        body["tools"] = json!(tools);
        body["parallel_tool_calls"] = json!(request.parallel_tool_calls);
    }

    if let Some(max_tokens) = request.max_tokens {
        body["max_output_tokens"] = json!(max_tokens);
    }

    if let Some(temp) = request.temperature {
        body["temperature"] = json!(temp);
    }

    // Reasoning config — Codex models support this.
    body["reasoning"] = json!({
        "effort": reasoning_effort,
        "summary": "auto",
    });

    body
}

// ---------------------------------------------------------------------------
// SSE event parsing
// ---------------------------------------------------------------------------

/// Tracks tool calls by `output_index` (the Responses API's positional
/// index in the `output[]` array), mapping them to sequential indices for
/// our `StreamDelta::ToolCallStart`.
struct ToolIndexTracker {
    map:  HashMap<u64, u32>,
    next: u32,
}

impl ToolIndexTracker {
    fn new() -> Self {
        Self {
            map:  HashMap::new(),
            next: 0,
        }
    }

    fn index_for(&mut self, output_index: u64) -> u32 {
        *self.map.entry(output_index).or_insert_with(|| {
            let idx = self.next;
            self.next += 1;
            idx
        })
    }
}

/// Accumulated state while processing the SSE stream.
struct StreamState {
    tool_tracker:          ToolIndexTracker,
    accumulated_text:      String,
    accumulated_reasoning: String,
    has_function_call:     bool,
    final_stop:            StopReason,
    final_usage:           Option<Usage>,
}

impl StreamState {
    fn new() -> Self {
        Self {
            tool_tracker:          ToolIndexTracker::new(),
            accumulated_text:      String::new(),
            accumulated_reasoning: String::new(),
            has_function_call:     false,
            final_stop:            StopReason::Stop,
            final_usage:           None,
        }
    }
}

/// Parse a single Codex SSE event and emit corresponding `StreamDelta`s.
///
/// Returns `Some(true)` when the stream should terminate (response finished),
/// `Some(false)` on parse failure, `None` to continue.
fn parse_codex_event(
    event_type: &str,
    data: &str,
    tx: &mpsc::Sender<StreamDelta>,
    state: &mut StreamState,
) -> Option<bool> {
    let parsed: Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(_) => return Some(false),
    };

    match event_type {
        // --- Text output ---
        "response.output_text.delta" => {
            if let Some(delta) = parsed.get("delta").and_then(|d| d.as_str()) {
                state.accumulated_text.push_str(delta);
                let _ = tx.try_send(StreamDelta::TextDelta {
                    text: delta.to_owned(),
                });
            }
        }

        // --- Refusal (emit as text so the user sees it) ---
        "response.refusal.delta" => {
            if let Some(delta) = parsed.get("delta").and_then(|d| d.as_str()) {
                let _ = tx.try_send(StreamDelta::TextDelta {
                    text: delta.to_owned(),
                });
            }
        }

        // --- Reasoning summary text ---
        "response.reasoning_summary_text.delta" => {
            if let Some(delta) = parsed.get("delta").and_then(|d| d.as_str()) {
                state.accumulated_reasoning.push_str(delta);
                let _ = tx.try_send(StreamDelta::ReasoningDelta {
                    text: delta.to_owned(),
                });
            }
        }

        // Track new reasoning summary part (logged for visibility).
        "response.reasoning_summary_part.added" => {
            tracing::trace!("reasoning summary part added");
        }

        // --- Output items (tool calls, messages, reasoning) ---
        "response.output_item.added" => {
            if let Some(item) = parsed.get("item") {
                let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                let output_index = parsed
                    .get("output_index")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                match item_type {
                    "function_call" => {
                        let name = item
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_owned();
                        let call_id = item
                            .get("call_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_owned();
                        let index = state.tool_tracker.index_for(output_index);
                        let _ = tx.try_send(StreamDelta::ToolCallStart {
                            index,
                            id: call_id,
                            name,
                        });
                    }
                    "reasoning" => {
                        tracing::trace!(output_index, "reasoning item started");
                    }
                    "message" => {
                        tracing::trace!(output_index, "message item started");
                    }
                    other => {
                        tracing::debug!(item_type = other, "unhandled output_item.added type");
                    }
                }
            }
        }

        "response.output_item.done" => {
            if let Some(item) = parsed.get("item") {
                let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if item_type == "function_call" {
                    state.has_function_call = true;
                    // The agent loop reconstructs the full tool call from
                    // ToolCallStart + ToolCallArgumentsDelta events, so we
                    // don't need to emit anything extra here.
                }
            }
        }

        // --- Tool call argument streaming ---
        "response.function_call_arguments.delta" => {
            if let Some(delta) = parsed.get("delta").and_then(|d| d.as_str()) {
                let output_index = parsed
                    .get("output_index")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let index = state.tool_tracker.index_for(output_index);
                let _ = tx.try_send(StreamDelta::ToolCallArgumentsDelta {
                    index,
                    arguments: delta.to_owned(),
                });
            }
        }

        // --- Annotations (URL citations → inline markdown) ---
        "response.output_text.annotation.added" => {
            if let Some(annotation) = parsed.get("annotation") {
                let ann_type = annotation
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if ann_type == "url_citation" {
                    let url = annotation.get("url").and_then(|v| v.as_str()).unwrap_or("");
                    let title = annotation
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or(url);
                    if !url.is_empty() {
                        let citation = format!(" [{title}]({url})");
                        let _ = tx.try_send(StreamDelta::TextDelta { text: citation });
                    }
                }
            }
        }

        // --- Response lifecycle ---
        "response.created" => {
            if let Some(response) = parsed.get("response") {
                let id = response.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                let model = response
                    .get("model")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                tracing::debug!(response_id = id, model, "Codex response created");
            }
        }

        "response.completed" | "response.incomplete" => {
            let is_incomplete = event_type == "response.incomplete";

            // Extract usage from response.usage
            state.final_usage = parsed.get("response").and_then(|r| {
                let u = r.get("usage")?;
                let input = u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let output = u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                Some(Usage {
                    prompt_tokens:     input,
                    completion_tokens: output,
                    total_tokens:      input + output,
                })
            });

            // Determine stop reason.
            // For incomplete responses, check incomplete_details.reason.
            // For completed responses with function calls, use ToolCalls.
            if is_incomplete {
                let reason = parsed
                    .get("response")
                    .and_then(|r| r.get("incomplete_details"))
                    .and_then(|d| d.get("reason"))
                    .and_then(|r| r.as_str());

                state.final_stop = match reason {
                    Some("max_output_tokens") => StopReason::Length,
                    Some("content_filter") => StopReason::ContentFilter,
                    Some(other) => {
                        tracing::warn!(
                            reason = other,
                            "Codex response incomplete with unexpected reason"
                        );
                        if state.has_function_call {
                            StopReason::ToolCalls
                        } else {
                            StopReason::Stop
                        }
                    }
                    None => {
                        if state.has_function_call {
                            StopReason::ToolCalls
                        } else {
                            StopReason::Stop
                        }
                    }
                };
            } else {
                // completed — check if there were tool calls
                state.final_stop = if state.has_function_call {
                    StopReason::ToolCalls
                } else {
                    StopReason::Stop
                };
            }

            return Some(true);
        }

        // --- Error event from the API ---
        "error" => {
            let code = parsed
                .get("code")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let message = parsed
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            tracing::error!(code, message, "Codex API error event");
        }

        _ => {
            tracing::debug!(event_type, "unhandled Codex SSE event");
        }
    }

    None
}

// ---------------------------------------------------------------------------
// LlmDriver implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl LlmDriver for CodexDriver {
    async fn stream(
        &self,
        request: CompletionRequest,
        tx: mpsc::Sender<StreamDelta>,
    ) -> Result<CompletionResponse> {
        let (base_url, headers) = self.resolve().await?;
        let body = build_codex_request(&request);
        let url = format!("{base_url}/codex/responses");

        tracing::debug!(
            model = request.model.as_str(),
            url = %url,
            "sending Codex Responses API request"
        );

        let response = self
            .stream_client
            .post(&url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|e| KernelError::Provider {
                message: format!("Codex request failed: {e}").into(),
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(crate::error::classify_provider_error(
                &format!("HTTP {status}: {text}"),
                Some(status.as_u16()),
            ));
        }

        let mut event_stream = response.bytes_stream().eventsource();
        let mut state = StreamState::new();

        loop {
            if tx.is_closed() {
                tracing::debug!("stream consumer disconnected, returning partial response");
                break;
            }

            let maybe = tokio::time::timeout(SSE_IDLE_TIMEOUT, event_stream.next()).await;

            match maybe {
                Ok(Some(Ok(event))) => {
                    if event.data == "[DONE]" {
                        break;
                    }
                    if let Some(terminal) =
                        parse_codex_event(&event.event, &event.data, &tx, &mut state)
                    {
                        if terminal {
                            break;
                        }
                        // Non-terminal return (parse failure) — skip this
                        // event.
                    }
                }
                Ok(Some(Err(e))) => {
                    return Err(KernelError::Provider {
                        message: format!("Codex SSE error: {e}").into(),
                    });
                }
                Ok(None) => break,
                Err(_) => {
                    return Err(KernelError::RetryableServer {
                        message: "Codex SSE idle timeout".into(),
                    });
                }
            }
        }

        let _ = tx
            .send(StreamDelta::Done {
                stop_reason: state.final_stop,
                usage:       state.final_usage,
            })
            .await;

        let reasoning_content = if state.accumulated_reasoning.is_empty() {
            None
        } else {
            Some(state.accumulated_reasoning)
        };

        Ok(CompletionResponse {
            content: if state.accumulated_text.is_empty() {
                None
            } else {
                Some(state.accumulated_text)
            },
            reasoning_content,
            tool_calls: vec![], // Populated by agent loop from StreamDelta events.
            stop_reason: state.final_stop,
            usage: state.final_usage,
            model: request.model,
        })
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        // Non-streaming: use stream() with a local channel and collect.
        let (tx, mut rx) = mpsc::channel(128);
        let result = self.stream(request, tx).await?;
        // Drain remaining events.
        while rx.recv().await.is_some() {}
        Ok(result)
    }

    async fn model_context_length(&self, _model: &str) -> Option<usize> {
        // Codex models typically support 192k+ context.
        Some(192_000)
    }

    async fn model_supports_vision(&self, _model: &str) -> Option<bool> { Some(false) }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::types::{Message, ThinkingConfig, ToolDefinition};

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
        };

        let body = build_codex_request(&request);
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
        };

        let body = build_codex_request(&request);
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
        };

        let body = build_codex_request(&request);
        assert_eq!(body["store"], true);
        assert_eq!(body["truncation"], "auto");
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
        };

        let body = build_codex_request(&request);
        assert_eq!(body["max_output_tokens"], 4096);
        assert_eq!(body["parallel_tool_calls"], true);
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
        };

        let body = build_codex_request(&request);
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
        };

        let body = build_codex_request(&request);
        let input = body["input"].as_array().expect("input should be array");

        // First item: function_call (assistant text was empty, so no message item)
        assert_eq!(input[0]["type"], "function_call");
        assert_eq!(input[0]["name"], "read_file");
        assert_eq!(input[0]["call_id"], "call_123");

        // Second item: function_call_output
        assert_eq!(input[1]["type"], "function_call_output");
        assert_eq!(input[1]["call_id"], "call_123");
        assert_eq!(input[1]["output"], "file contents here");
    }

    #[test]
    fn parse_text_delta_event() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut state = StreamState::new();

        let data = r#"{"delta":"hello ","item_id":"item_1","output_index":0}"#;
        let result = parse_codex_event("response.output_text.delta", data, &tx, &mut state);
        assert!(result.is_none()); // not terminal

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
        let mut state = StreamState::new();

        let data = r#"{"delta":"thinking...","summary_index":0}"#;
        let result = parse_codex_event(
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
        let mut state = StreamState::new();

        // output_item.added with function_call
        let added = r#"{"item":{"type":"function_call","id":"item_1","call_id":"call_abc","name":"read_file","arguments":""},"output_index":2}"#;
        parse_codex_event("response.output_item.added", added, &tx, &mut state);

        let start = rx.try_recv().expect("should receive ToolCallStart");
        match start {
            StreamDelta::ToolCallStart { index, id, name } => {
                assert_eq!(index, 0);
                assert_eq!(id, "call_abc");
                assert_eq!(name, "read_file");
            }
            other => panic!("unexpected: {other:?}"),
        }

        // arguments delta
        let args_delta = r#"{"delta":"{\"path\"","output_index":2}"#;
        parse_codex_event(
            "response.function_call_arguments.delta",
            args_delta,
            &tx,
            &mut state,
        );

        let arg = rx.try_recv().expect("should receive args delta");
        match arg {
            StreamDelta::ToolCallArgumentsDelta { index, arguments } => {
                assert_eq!(index, 0); // same output_index maps to same tracker index
                assert_eq!(arguments, r#"{"path""#);
            }
            other => panic!("unexpected: {other:?}"),
        }

        // output_item.done marks function_call flag
        let done = r#"{"item":{"type":"function_call","id":"item_1","call_id":"call_abc","name":"read_file","arguments":"{\"path\":\"foo.rs\"}"},"output_index":2}"#;
        parse_codex_event("response.output_item.done", done, &tx, &mut state);
        assert!(state.has_function_call);
    }

    #[test]
    fn parse_completed_event_with_usage() {
        let (tx, _rx) = mpsc::channel(16);
        let mut state = StreamState::new();

        let data = r#"{"response":{"status":"completed","usage":{"input_tokens":100,"output_tokens":50}}}"#;
        let result = parse_codex_event("response.completed", data, &tx, &mut state);
        assert_eq!(result, Some(true)); // terminal
        assert_eq!(state.final_stop, StopReason::Stop);

        let usage = state.final_usage.expect("should have usage");
        assert_eq!(usage.prompt_tokens, 100);
        assert_eq!(usage.completion_tokens, 50);
        assert_eq!(usage.total_tokens, 150);
    }

    #[test]
    fn parse_completed_with_function_call_yields_tool_calls_stop() {
        let (tx, _rx) = mpsc::channel(16);
        let mut state = StreamState::new();
        state.has_function_call = true;

        let data =
            r#"{"response":{"status":"completed","usage":{"input_tokens":10,"output_tokens":5}}}"#;
        let result = parse_codex_event("response.completed", data, &tx, &mut state);
        assert_eq!(result, Some(true));
        assert_eq!(state.final_stop, StopReason::ToolCalls);
    }

    #[test]
    fn parse_incomplete_max_output_tokens() {
        let (tx, _rx) = mpsc::channel(16);
        let mut state = StreamState::new();

        let data = r#"{"response":{"incomplete_details":{"reason":"max_output_tokens"},"usage":{"input_tokens":50,"output_tokens":200}}}"#;
        let result = parse_codex_event("response.incomplete", data, &tx, &mut state);
        assert_eq!(result, Some(true));
        assert_eq!(state.final_stop, StopReason::Length);
    }

    #[test]
    fn parse_incomplete_content_filter() {
        let (tx, _rx) = mpsc::channel(16);
        let mut state = StreamState::new();

        let data = r#"{"response":{"incomplete_details":{"reason":"content_filter"},"usage":{"input_tokens":50,"output_tokens":10}}}"#;
        let result = parse_codex_event("response.incomplete", data, &tx, &mut state);
        assert_eq!(result, Some(true));
        assert_eq!(state.final_stop, StopReason::ContentFilter);
    }

    #[test]
    fn unknown_event_type_does_not_crash() {
        let (tx, _rx) = mpsc::channel(16);
        let mut state = StreamState::new();

        let data = r#"{"some":"data"}"#;
        let result = parse_codex_event("response.something.new", data, &tx, &mut state);
        assert!(result.is_none()); // not terminal, just logged
    }
}
