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

//! OpenAI-compatible LLM driver.
//!
//! Uses `reqwest` directly for HTTP + SSE parsing, supporting fields
//! like `reasoning_content` that `async-openai` doesn't expose.

use std::{collections::HashMap, sync::Arc, time::Duration};

use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use super::{
    driver::LlmDriver,
    stream::StreamDelta,
    types::{
        CompletionRequest, CompletionResponse, ContentBlock, Message, MessageContent, Role,
        StopReason, ToolCallRequest, ToolChoice, Usage,
    },
};
use crate::error::{KernelError, Result};

// ---------------------------------------------------------------------------
// OpenAiDriver
// ---------------------------------------------------------------------------

/// OpenAI-compatible LLM driver.
///
/// Uses `reqwest` directly for HTTP + SSE parsing, supporting fields
/// like `reasoning_content` that `async-openai` doesn't expose.
pub struct OpenAiDriver {
    client:        reqwest::Client,
    config_source: OpenAiDriverConfigSource,
}

enum OpenAiDriverConfigSource {
    Static {
        base_url: String,
        api_key:  String,
    },
    SettingsBacked {
        settings:      Arc<dyn rara_domain_shared::settings::SettingsProvider>,
        provider_name: String,
    },
}

#[derive(Debug)]
struct ResolvedConfig {
    base_url: String,
    api_key:  String,
}

/// SSE idle timeout — if no event is received within this duration, abort.
const SSE_IDLE_TIMEOUT: Duration = Duration::from_secs(90);

/// Maximum number of retries for rate-limited (429) requests.
const RATE_LIMIT_MAX_RETRIES: u32 = 4;
/// Initial backoff delay for rate-limited retries.
const RATE_LIMIT_INITIAL_DELAY: Duration = Duration::from_secs(5);
/// Maximum backoff delay for rate-limited retries.
const RATE_LIMIT_MAX_DELAY: Duration = Duration::from_secs(60);

impl OpenAiDriver {
    /// Build a reqwest client with connect and overall read timeouts.
    fn build_http_client() -> reqwest::Client {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(300))
            .build()
            .expect("failed to build HTTP client")
    }

    /// Create a new driver targeting the given API base URL.
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            client:        Self::build_http_client(),
            config_source: OpenAiDriverConfigSource::Static {
                base_url: base_url.into(),
                api_key:  api_key.into(),
            },
        }
    }

    /// Create a driver that resolves its base URL and API key from runtime
    /// settings on every request.
    ///
    /// Looks up `llm.providers.{provider_name}.base_url` and
    /// `llm.providers.{provider_name}.api_key` from the settings provider.
    pub fn from_settings(
        settings: Arc<dyn rara_domain_shared::settings::SettingsProvider>,
        provider_name: impl Into<String>,
    ) -> Self {
        Self {
            client:        Self::build_http_client(),
            config_source: OpenAiDriverConfigSource::SettingsBacked {
                settings,
                provider_name: provider_name.into(),
            },
        }
    }

    async fn resolve_config(&self) -> Result<ResolvedConfig> {
        match &self.config_source {
            OpenAiDriverConfigSource::Static { base_url, api_key } => Ok(ResolvedConfig {
                base_url: base_url.clone(),
                api_key:  api_key.clone(),
            }),
            OpenAiDriverConfigSource::SettingsBacked {
                settings,
                provider_name,
            } => {
                let base_url_key = format!("llm.providers.{provider_name}.base_url");
                let api_key_key = format!("llm.providers.{provider_name}.api_key");

                let base_url =
                    settings
                        .get(&base_url_key)
                        .await
                        .ok_or_else(|| KernelError::Provider {
                            message: format!(
                                "LLM provider base URL is not configured (checked: {base_url_key})"
                            )
                            .into(),
                        })?;
                let api_key =
                    settings
                        .get(&api_key_key)
                        .await
                        .ok_or_else(|| KernelError::Provider {
                            message: format!(
                                "LLM provider API key is not configured (checked: {api_key_key})"
                            )
                            .into(),
                        })?;

                Ok(ResolvedConfig { base_url, api_key })
            }
        }
    }

    /// Send a request and return the successful HTTP response.
    ///
    /// Automatically retries on HTTP 429 (rate limited) with exponential
    /// backoff, respecting the `Retry-After` header when present.
    async fn send_request(
        &self,
        request: &CompletionRequest,
        stream: bool,
    ) -> Result<reqwest::Response> {
        let config = self.resolve_config().await?;
        let body = ChatRequest::from_completion(request, stream);

        tracing::debug!(
            model = request.model.as_str(),
            messages = request.messages.len(),
            tools = request.tools.len(),
            stream,
            "sending LLM request"
        );

        let mut attempt = 0u32;

        loop {
            let response = self
                .client
                .post(format!("{}/chat/completions", config.base_url))
                .bearer_auth(&config.api_key)
                .json(&body)
                .send()
                .await
                .map_err(|e| KernelError::Provider {
                    message: format!("LLM provider request failed: {e}").into(),
                })?;

            if response.status().is_success() {
                return Ok(response);
            }

            let status = response.status();

            // Rate limited — retry with backoff
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS && attempt < RATE_LIMIT_MAX_RETRIES
            {
                let retry_after = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .map(Duration::from_secs);

                let backoff = retry_after.unwrap_or_else(|| {
                    (RATE_LIMIT_INITIAL_DELAY * 2u32.saturating_pow(attempt))
                        .min(RATE_LIMIT_MAX_DELAY)
                });

                tracing::warn!(
                    attempt = attempt + 1,
                    max_retries = RATE_LIMIT_MAX_RETRIES,
                    backoff_secs = backoff.as_secs(),
                    model = request.model.as_str(),
                    "rate limited (429), backing off"
                );

                tokio::time::sleep(backoff).await;
                attempt += 1;
                continue;
            }

            // Non-429 error — fail immediately
            let text = response.text().await.unwrap_or_default();

            if let Ok(request_body) = serde_json::to_string(&body) {
                tracing::warn!(
                    %status,
                    response_body = %text,
                    request_body = %request_body,
                    "LLM provider returned error"
                );
            }

            return Err(crate::error::classify_provider_error(
                &format!("HTTP {status}: {text}"),
                Some(status.as_u16()),
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// LlmDriver implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl LlmDriver for OpenAiDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        let response = self.send_request(&request, false).await?;

        let raw: RawCompletionResponse =
            response.json().await.map_err(|e| KernelError::Provider {
                message: format!("failed to parse LLM response: {e}").into(),
            })?;

        let choice = raw
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| KernelError::Provider {
                message: "no choices in response".into(),
            })?;

        let stop_reason = parse_stop_reason(choice.finish_reason.as_deref());

        let tool_calls = choice
            .message
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .map(|tc| ToolCallRequest {
                id:        tc.id,
                name:      tc.function.name,
                arguments: tc.function.arguments,
            })
            .collect();

        let usage = raw.usage.map(|u| Usage {
            prompt_tokens:     u.prompt_tokens.unwrap_or(0),
            completion_tokens: u.completion_tokens.unwrap_or(0),
            total_tokens:      u.total_tokens.unwrap_or(0),
        });

        Ok(CompletionResponse {
            content: choice.message.content,
            reasoning_content: choice.message.reasoning_content,
            tool_calls,
            stop_reason,
            usage,
            model: raw.model.unwrap_or_else(|| request.model.clone()),
        })
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        tx: mpsc::Sender<StreamDelta>,
    ) -> Result<CompletionResponse> {
        let response = self.send_request(&request, true).await?;

        let mut event_stream = response.bytes_stream().eventsource();
        let mut acc = StreamAccumulator::new();

        loop {
            let maybe_event = tokio::time::timeout(SSE_IDLE_TIMEOUT, event_stream.next()).await;

            match maybe_event {
                Ok(Some(event_result)) => {
                    let event = event_result.map_err(|e| KernelError::Provider {
                        message: format!("SSE stream error: {e}").into(),
                    })?;
                    if event.data == "[DONE]" {
                        break;
                    }
                    let Ok(chunk) = serde_json::from_str::<RawStreamChunk>(&event.data) else {
                        continue;
                    };
                    acc.process_chunk(&chunk, &tx).await;
                }
                Ok(None) => break,
                Err(_elapsed) => {
                    tracing::warn!(
                        timeout_secs = SSE_IDLE_TIMEOUT.as_secs(),
                        "SSE stream idle timeout — no event received, aborting stream"
                    );
                    return Err(KernelError::RetryableServer {
                        message: "SSE stream idle timeout".into(),
                    });
                }
            }
        }

        Ok(acc.finalize(&tx, request.model.clone()).await)
    }
}

// ---------------------------------------------------------------------------
// StreamAccumulator
// ---------------------------------------------------------------------------

struct StreamAccumulator {
    text:        String,
    reasoning:   String,
    tools:       HashMap<u32, PendingToolCall>,
    stop_reason: StopReason,
    usage:       Option<Usage>,
}

struct PendingToolCall {
    id:        String,
    name:      String,
    arguments: String,
    started:   bool,
}

impl StreamAccumulator {
    fn new() -> Self {
        Self {
            text:        String::new(),
            reasoning:   String::new(),
            tools:       HashMap::new(),
            stop_reason: StopReason::Stop,
            usage:       None,
        }
    }

    async fn process_chunk(&mut self, chunk: &RawStreamChunk, tx: &mpsc::Sender<StreamDelta>) {
        for choice in &chunk.choices {
            // Text delta
            if let Some(ref text) = choice.delta.content {
                if !text.is_empty() {
                    self.text.push_str(text);
                    let _ = tx.send(StreamDelta::TextDelta { text: text.clone() }).await;
                }
            }

            // Reasoning delta
            if let Some(ref reasoning) = choice.delta.reasoning_content {
                if !reasoning.is_empty() {
                    self.reasoning.push_str(reasoning);
                    let _ = tx
                        .send(StreamDelta::ReasoningDelta {
                            text: reasoning.clone(),
                        })
                        .await;
                }
            }

            // Tool call deltas
            if let Some(ref tcs) = choice.delta.tool_calls {
                for tc in tcs {
                    let entry = self
                        .tools
                        .entry(tc.index)
                        .or_insert_with(|| PendingToolCall {
                            id:        String::new(),
                            name:      String::new(),
                            arguments: String::new(),
                            started:   false,
                        });

                    if let Some(ref id) = tc.id {
                        if !id.is_empty() {
                            entry.id = id.clone();
                        }
                    }

                    // Collect new arguments but defer sending the delta until
                    // after ToolCallStart has been emitted.  Some providers
                    // (e.g. OpenRouter) deliver name + arguments in a single
                    // SSE chunk; if we send ToolCallArgumentsDelta first the
                    // receiver has no pending entry yet and silently drops
                    // the arguments.
                    let mut new_args: Option<String> = None;

                    if let Some(ref func) = tc.function {
                        if let Some(ref name) = func.name {
                            if !name.is_empty() {
                                entry.name = name.clone();
                            }
                        }
                        if let Some(ref args) = func.arguments {
                            if !args.is_empty() {
                                entry.arguments.push_str(args);
                                new_args = Some(args.clone());
                            }
                        }
                    }

                    // Emit ToolCallStart exactly once when we first get both id and name.
                    // This MUST happen before ToolCallArgumentsDelta so the
                    // receiver registers the pending entry before accumulating
                    // arguments.
                    if !entry.started && !entry.id.is_empty() && !entry.name.is_empty() {
                        entry.started = true;
                        let _ = tx
                            .send(StreamDelta::ToolCallStart {
                                index: tc.index,
                                id:    entry.id.clone(),
                                name:  entry.name.clone(),
                            })
                            .await;
                    }

                    // Now send the argument delta (receiver entry is guaranteed
                    // to exist if ToolCallStart was emitted above).
                    if let Some(args) = new_args {
                        let _ = tx
                            .send(StreamDelta::ToolCallArgumentsDelta {
                                index:     tc.index,
                                arguments: args,
                            })
                            .await;
                    }
                }
            }

            // Finish reason
            if let Some(ref reason) = choice.finish_reason {
                self.stop_reason = parse_stop_reason(Some(reason.as_str()));
            }
        }

        // Usage (some providers send it in the last chunk)
        if let Some(ref usage) = chunk.usage {
            self.usage = Some(Usage {
                prompt_tokens:     usage.prompt_tokens.unwrap_or(0),
                completion_tokens: usage.completion_tokens.unwrap_or(0),
                total_tokens:      usage.total_tokens.unwrap_or(0),
            });
        }
    }

    fn collect_tools(tools: HashMap<u32, PendingToolCall>) -> Vec<ToolCallRequest> {
        let mut entries: Vec<(u32, PendingToolCall)> = tools.into_iter().collect();
        entries.sort_by_key(|(idx, _)| *idx);
        entries
            .into_iter()
            .map(|(_, pt)| ToolCallRequest {
                id:        pt.id,
                name:      pt.name,
                arguments: pt.arguments,
            })
            .collect()
    }

    async fn finalize(self, tx: &mpsc::Sender<StreamDelta>, model: String) -> CompletionResponse {
        let Self {
            text,
            reasoning,
            tools,
            stop_reason,
            usage,
        } = self;
        let tool_calls = Self::collect_tools(tools);

        let _ = tx.send(StreamDelta::Done { stop_reason, usage }).await;

        CompletionResponse {
            content: non_empty(text),
            reasoning_content: non_empty(reasoning),
            tool_calls,
            stop_reason,
            usage,
            model,
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn non_empty(s: String) -> Option<String> { if s.is_empty() { None } else { Some(s) } }

fn parse_stop_reason(reason: Option<&str>) -> StopReason {
    match reason {
        Some("stop") => StopReason::Stop,
        Some("tool_calls") => StopReason::ToolCalls,
        Some("length") => StopReason::Length,
        Some("content_filter") => StopReason::ContentFilter,
        _ => StopReason::Stop,
    }
}

// ---------------------------------------------------------------------------
// Wire types — typed request serialization
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ChatRequest<'a> {
    model:               &'a str,
    messages:            Vec<WireMessage<'a>>,
    stream:              bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature:         Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens:          Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools:               Option<Vec<WireTool<'a>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice:         Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parallel_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking:            Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options:      Option<WireStreamOptions>,
}

#[derive(Serialize)]
struct WireStreamOptions {
    include_usage: bool,
}

#[derive(Serialize)]
struct WireTool<'a> {
    r#type:   &'static str,
    function: WireToolFunction<'a>,
}

#[derive(Serialize)]
struct WireToolFunction<'a> {
    name:        &'a str,
    description: &'a str,
    parameters:  &'a serde_json::Value,
}

#[derive(Serialize)]
#[serde(untagged)]
enum WireContent<'a> {
    Text(&'a str),
    Multimodal(Vec<WireContentPart<'a>>),
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum WireContentPart<'a> {
    #[serde(rename = "text")]
    Text { text: &'a str },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: WireImageUrl<'a> },
}

#[derive(Serialize)]
struct WireImageUrl<'a> {
    url: std::borrow::Cow<'a, str>,
}

#[derive(Serialize)]
struct WireMessage<'a> {
    role:         &'static str,
    content:      WireContent<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls:   Option<Vec<WireToolCallRef<'a>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<&'a str>,
}

#[derive(Serialize)]
struct WireToolCallRef<'a> {
    id:       &'a str,
    r#type:   &'static str,
    function: WireFunctionRef<'a>,
}

#[derive(Serialize)]
struct WireFunctionRef<'a> {
    name:      &'a str,
    arguments: &'a str,
}

impl<'a> ChatRequest<'a> {
    fn from_completion(request: &'a CompletionRequest, stream: bool) -> Self {
        let messages: Vec<WireMessage<'a>> = request
            .messages
            .iter()
            .map(WireMessage::from_message)
            .collect();

        let (tools, tool_choice, parallel_tool_calls) = if !request.tools.is_empty() {
            let tools: Vec<WireTool<'a>> = request
                .tools
                .iter()
                .map(|t| WireTool {
                    r#type:   "function",
                    function: WireToolFunction {
                        name:        &t.name,
                        description: &t.description,
                        parameters:  &t.parameters,
                    },
                })
                .collect();

            let tool_choice = match &request.tool_choice {
                ToolChoice::Auto => None,
                ToolChoice::None => Some(serde_json::json!("none")),
                ToolChoice::Required => Some(serde_json::json!("required")),
                ToolChoice::Specific(name) => {
                    Some(serde_json::json!({"type": "function", "function": {"name": name}}))
                }
            };

            let parallel = if request.parallel_tool_calls {
                Some(true)
            } else {
                None
            };

            (Some(tools), tool_choice, parallel)
        } else {
            (None, None, None)
        };

        let thinking = request.thinking.as_ref().and_then(|t| {
            if t.enabled {
                let mut obj = serde_json::json!({"type": "enabled"});
                if let Some(budget) = t.budget_tokens {
                    obj["budget_tokens"] = serde_json::json!(budget);
                }
                Some(obj)
            } else {
                None
            }
        });

        let stream_options = if stream {
            Some(WireStreamOptions {
                include_usage: true,
            })
        } else {
            None
        };

        Self {
            model: &request.model,
            messages,
            stream,
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            tools,
            tool_choice,
            parallel_tool_calls,
            thinking,
            stream_options,
        }
    }
}

impl<'a> WireMessage<'a> {
    fn from_message(msg: &'a Message) -> Self {
        let role = match msg.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        };

        let content = match &msg.content {
            MessageContent::Text(text) => WireContent::Text(text),
            MessageContent::Multimodal(blocks) => {
                let parts = blocks
                    .iter()
                    .map(|b| match b {
                        ContentBlock::Text { text } => WireContentPart::Text { text },
                        ContentBlock::ImageUrl { url } => WireContentPart::ImageUrl {
                            image_url: WireImageUrl {
                                url: std::borrow::Cow::Borrowed(url),
                            },
                        },
                        ContentBlock::ImageBase64 { media_type, data } => {
                            let data_uri = format!("data:{media_type};base64,{data}");
                            WireContentPart::ImageUrl {
                                image_url: WireImageUrl {
                                    url: std::borrow::Cow::Owned(data_uri),
                                },
                            }
                        }
                    })
                    .collect();
                WireContent::Multimodal(parts)
            }
        };

        let tool_calls = if msg.tool_calls.is_empty() {
            None
        } else {
            Some(
                msg.tool_calls
                    .iter()
                    .map(|tc| WireToolCallRef {
                        id:       &tc.id,
                        r#type:   "function",
                        function: WireFunctionRef {
                            name:      &tc.name,
                            arguments: &tc.arguments,
                        },
                    })
                    .collect(),
            )
        };

        Self {
            role,
            content,
            tool_calls,
            tool_call_id: msg.tool_call_id.as_deref(),
        }
    }
}

// ---------------------------------------------------------------------------
// Raw SSE deserialization types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RawStreamChunk {
    #[allow(dead_code)]
    id:      Option<String>,
    choices: Vec<RawStreamChoice>,
    usage:   Option<RawUsage>,
}

#[derive(Debug, Deserialize)]
struct RawStreamChoice {
    #[allow(dead_code)]
    index:         u32,
    delta:         RawDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawDelta {
    content:           Option<String>,
    #[serde(alias = "reasoning")]
    reasoning_content: Option<String>,
    tool_calls:        Option<Vec<RawToolCallChunk>>,
    #[allow(dead_code)]
    role:              Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawToolCallChunk {
    index:    u32,
    id:       Option<String>,
    function: Option<RawFunctionChunk>,
}

#[derive(Debug, Deserialize)]
struct RawFunctionChunk {
    name:      Option<String>,
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawUsage {
    prompt_tokens:     Option<u32>,
    completion_tokens: Option<u32>,
    total_tokens:      Option<u32>,
}

// --- Non-streaming response types ---

#[derive(Debug, Deserialize)]
struct RawCompletionResponse {
    choices: Vec<RawResponseChoice>,
    usage:   Option<RawUsage>,
    model:   Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawResponseChoice {
    message:       RawResponseMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawResponseMessage {
    content:           Option<String>,
    #[serde(alias = "reasoning")]
    reasoning_content: Option<String>,
    tool_calls:        Option<Vec<RawResponseToolCall>>,
}

#[derive(Debug, Deserialize)]
struct RawResponseToolCall {
    id:       String,
    function: RawResponseFunction,
}

#[derive(Debug, Deserialize)]
struct RawResponseFunction {
    name:      String,
    arguments: String,
}
