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

use std::time::Duration;

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

/// Convert our internal `CompletionRequest` (messages-based) into
/// the Responses API `input[]` format.
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
                input.push(json!({
                    "type": "message",
                    "role": role_str,
                    "content": msg.content.as_text(),
                }));
            }
            super::types::Role::Assistant => {
                if msg.tool_calls.is_empty() {
                    input.push(json!({
                        "type": "message",
                        "role": "assistant",
                        "content": msg.content.as_text(),
                    }));
                } else {
                    let text = msg.content.as_text();
                    if !text.is_empty() {
                        input.push(json!({
                            "type": "message",
                            "role": "assistant",
                            "content": text,
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

    let mut body = json!({
        "model": request.model,
        "input": input,
        "stream": true,
        "store": false,
    });

    if !tools.is_empty() {
        body["tools"] = json!(tools);
    }

    if let Some(temp) = request.temperature {
        body["temperature"] = json!(temp);
    }

    // Reasoning config — Codex models support this.
    body["reasoning"] = json!({
        "effort": "medium",
        "summary": "auto",
    });

    body
}

/// Maps Responses API `item_id` strings to integer indices for the agent
/// loop's `pending_tool_calls` HashMap.
struct ToolIndexTracker {
    map:  std::collections::HashMap<String, u32>,
    next: u32,
}

impl ToolIndexTracker {
    fn new() -> Self {
        Self {
            map:  std::collections::HashMap::new(),
            next: 0,
        }
    }

    fn index_for(&mut self, item_id: &str) -> u32 {
        *self.map.entry(item_id.to_owned()).or_insert_with(|| {
            let idx = self.next;
            self.next += 1;
            idx
        })
    }
}

/// Parse a Codex SSE event into stream deltas.
fn parse_codex_event(
    event_type: &str,
    data: &str,
    tx: &mpsc::Sender<StreamDelta>,
    tracker: &mut ToolIndexTracker,
) -> Option<(StopReason, Option<Usage>)> {
    let parsed: Value = serde_json::from_str(data).ok()?;

    match event_type {
        "response.output_text.delta" => {
            if let Some(delta) = parsed.get("delta").and_then(|d| d.as_str()) {
                let _ = tx.try_send(StreamDelta::TextDelta {
                    text: delta.to_owned(),
                });
            }
        }
        "response.refusal.delta" => {
            if let Some(delta) = parsed.get("delta").and_then(|d| d.as_str()) {
                let _ = tx.try_send(StreamDelta::TextDelta {
                    text: delta.to_owned(),
                });
            }
        }
        "response.function_call_arguments.delta" => {
            if let Some(delta) = parsed.get("delta").and_then(|d| d.as_str()) {
                let item_id = parsed.get("item_id").and_then(|v| v.as_str()).unwrap_or("");
                let index = tracker.index_for(item_id);
                let _ = tx.try_send(StreamDelta::ToolCallArgumentsDelta {
                    index,
                    arguments: delta.to_owned(),
                });
            }
        }
        "response.output_item.added" => {
            if let Some(item) = parsed.get("item") {
                let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if item_type == "function_call" {
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
                    let item_id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let index = tracker.index_for(item_id);
                    let _ = tx.try_send(StreamDelta::ToolCallStart {
                        index,
                        id: call_id,
                        name,
                    });
                }
            }
        }
        "response.completed" => {
            let usage = parsed.get("response").and_then(|r| {
                let u = r.get("usage")?;
                let input = u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let output = u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                Some(Usage {
                    prompt_tokens:     input,
                    completion_tokens: output,
                    total_tokens:      input + output,
                })
            });
            let stop = parsed
                .get("response")
                .and_then(|r| r.get("status"))
                .and_then(|s| s.as_str())
                .map(|s| match s {
                    "completed" => StopReason::Stop,
                    _ => StopReason::Stop,
                })
                .unwrap_or(StopReason::Stop);
            return Some((stop, usage));
        }
        _ => {}
    }

    None
}

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
        let mut final_stop = StopReason::Stop;
        let mut final_usage = None;
        let mut accumulated_text = String::new();
        let mut tool_tracker = ToolIndexTracker::new();

        loop {
            if tx.is_closed() {
                break;
            }

            let maybe = tokio::time::timeout(SSE_IDLE_TIMEOUT, event_stream.next()).await;

            match maybe {
                Ok(Some(Ok(event))) => {
                    if event.data == "[DONE]" {
                        break;
                    }
                    if let Some((stop, usage)) =
                        parse_codex_event(&event.event, &event.data, &tx, &mut tool_tracker)
                    {
                        final_stop = stop;
                        final_usage = usage;
                        break;
                    }
                    // Track text for the final response.
                    if event.event == "response.output_text.delta" {
                        if let Ok(parsed) = serde_json::from_str::<Value>(&event.data) {
                            if let Some(delta) = parsed.get("delta").and_then(|d| d.as_str()) {
                                accumulated_text.push_str(delta);
                            }
                        }
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
                stop_reason: final_stop,
                usage:       final_usage,
            })
            .await;

        Ok(CompletionResponse {
            content:           if accumulated_text.is_empty() {
                None
            } else {
                Some(accumulated_text)
            },
            reasoning_content: None,
            tool_calls:        vec![], // Populated by agent loop from StreamDelta events.
            stop_reason:       final_stop,
            usage:             final_usage,
            model:             request.model,
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
