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

use std::{collections::HashMap, sync::Arc};

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

impl OpenAiDriver {
    /// Create a new driver targeting the given API base URL.
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            client:        reqwest::Client::new(),
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
            client:        reqwest::Client::new(),
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

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();

            if let Ok(request_body) = serde_json::to_string(&body) {
                tracing::warn!(
                    %status,
                    response_body = %text,
                    request_body = %request_body,
                    "LLM provider returned error"
                );
            }

            return Err(KernelError::Provider {
                message: format!("HTTP {status}: {text}").into(),
            });
        }

        Ok(response)
    }
}

// ---------------------------------------------------------------------------
// LlmDriver implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl LlmDriver for OpenAiDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        let response = self.send_request(&request, false).await?;

        let raw: RawCompletionResponse = response
            .json()
            .await
            .map_err(|e| KernelError::Provider {
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

        while let Some(event_result) = event_stream.next().await {
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

                    if let Some(ref func) = tc.function {
                        if let Some(ref name) = func.name {
                            if !name.is_empty() {
                                entry.name = name.clone();
                            }
                        }
                        if let Some(ref args) = func.arguments {
                            if !args.is_empty() {
                                entry.arguments.push_str(args);
                                let _ = tx
                                    .send(StreamDelta::ToolCallArgumentsDelta {
                                        index:     tc.index,
                                        arguments: args.clone(),
                                    })
                                    .await;
                            }
                        }
                    }

                    // Emit ToolCallStart exactly once when we first get both id and name
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
    url: &'a str,
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
                            image_url: WireImageUrl { url },
                        },
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rara_domain_shared::settings::{SettingsProvider, testing::MapSettingsProvider};

    use super::*;
    use crate::llm::types::ToolDefinition;

    /// Helper: serialize a `ChatRequest` to `serde_json::Value` for assertions.
    fn build_body(request: &CompletionRequest, stream: bool) -> serde_json::Value {
        let wire = ChatRequest::from_completion(request, stream);
        serde_json::to_value(&wire).unwrap()
    }

    /// Helper: serialize a `WireMessage` to `serde_json::Value`.
    fn msg_to_json(msg: &Message) -> serde_json::Value {
        let wire = WireMessage::from_message(msg);
        serde_json::to_value(&wire).unwrap()
    }

    #[test]
    fn test_build_request_body_simple() {
        let request = CompletionRequest {
            model:               "gpt-4".to_string(),
            messages:            vec![Message::system("You are helpful."), Message::user("Hello")],
            tools:               vec![],
            temperature:         Some(0.7),
            max_tokens:          None,
            thinking:            None,
            tool_choice:         ToolChoice::Auto,
            parallel_tool_calls: false,
        };
        let body = build_body(&request, false);
        assert_eq!(body["model"], "gpt-4");
        assert_eq!(body["messages"].as_array().unwrap().len(), 2);
        assert_eq!(body["stream"], false);
        let temp = body["temperature"].as_f64().unwrap();
        assert!((temp - 0.7).abs() < 0.001, "expected ~0.7, got {temp}");
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn test_build_request_body_with_tools() {
        let request = CompletionRequest {
            model:               "gpt-4".to_string(),
            messages:            vec![Message::user("Use the tool")],
            tools:               vec![ToolDefinition {
                name:        "read_file".to_string(),
                description: "Read a file".to_string(),
                parameters:  serde_json::json!({"type": "object", "properties": {"path": {"type": "string"}}}),
            }],
            temperature:         None,
            max_tokens:          Some(4096),
            thinking:            None,
            tool_choice:         ToolChoice::Auto,
            parallel_tool_calls: true,
        };
        let body = build_body(&request, true);
        assert_eq!(body["tools"].as_array().unwrap().len(), 1);
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["function"]["name"], "read_file");
        assert!(body.get("tool_choice").is_none());
        assert_eq!(body["parallel_tool_calls"], true);
        assert_eq!(body["stream"], true);
        assert_eq!(body["max_tokens"], 4096);
        assert!(body.get("max_completion_tokens").is_none());
    }

    #[test]
    fn test_build_request_body_tool_choice_variants() {
        let base = CompletionRequest {
            model:               "gpt-4".to_string(),
            messages:            vec![Message::user("test")],
            tools:               vec![ToolDefinition {
                name:        "t".to_string(),
                description: "d".to_string(),
                parameters:  serde_json::json!({}),
            }],
            temperature:         None,
            max_tokens:          None,
            thinking:            None,
            tool_choice:         ToolChoice::None,
            parallel_tool_calls: false,
        };

        let body = build_body(&base, false);
        assert_eq!(body["tool_choice"], "none");

        let mut req = base.clone();
        req.tool_choice = ToolChoice::Required;
        let body = build_body(&req, false);
        assert_eq!(body["tool_choice"], "required");

        let mut req = base.clone();
        req.tool_choice = ToolChoice::Specific("read_file".to_string());
        let body = build_body(&req, false);
        assert_eq!(body["tool_choice"]["function"]["name"], "read_file");
    }

    #[test]
    fn test_message_to_json_system() {
        let msg = Message::system("Be helpful");
        let json = msg_to_json(&msg);
        assert_eq!(json["role"], "system");
        assert_eq!(json["content"], "Be helpful");
    }

    #[test]
    fn test_message_to_json_tool_result() {
        let msg = Message::tool_result("call_1", "file contents here");
        let json = msg_to_json(&msg);
        assert_eq!(json["role"], "tool");
        assert_eq!(json["content"], "file contents here");
        assert_eq!(json["tool_call_id"], "call_1");
    }

    #[test]
    fn test_message_to_json_assistant_with_tool_calls() {
        let msg = Message::assistant_with_tool_calls(
            "thinking...",
            vec![ToolCallRequest {
                id:        "call_1".to_string(),
                name:      "read_file".to_string(),
                arguments: r#"{"path":"/tmp/test"}"#.to_string(),
            }],
        );
        let json = msg_to_json(&msg);
        assert_eq!(json["role"], "assistant");
        assert_eq!(json["tool_calls"].as_array().unwrap().len(), 1);
        assert_eq!(json["tool_calls"][0]["function"]["name"], "read_file");
    }

    #[test]
    fn test_message_to_json_multimodal() {
        let msg = Message::user_multimodal(vec![
            ContentBlock::Text {
                text: "What's this?".to_string(),
            },
            ContentBlock::ImageUrl {
                url: "https://example.com/img.png".to_string(),
            },
        ]);
        let json = msg_to_json(&msg);
        assert_eq!(json["role"], "user");
        let parts = json["content"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0]["type"], "text");
        assert_eq!(parts[1]["type"], "image_url");
    }

    #[test]
    fn test_parse_sse_text_delta() {
        let data = r#"{"id":"resp_1","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}"#;
        let chunk: RawStreamChunk = serde_json::from_str(data).unwrap();
        assert_eq!(chunk.choices[0].delta.content.as_deref(), Some("Hello"));
        assert!(chunk.choices[0].delta.reasoning_content.is_none());
    }

    #[test]
    fn test_parse_sse_reasoning_delta() {
        let data = r#"{"id":"resp_1","choices":[{"index":0,"delta":{"reasoning_content":"Let me think about this..."},"finish_reason":null}]}"#;
        let chunk: RawStreamChunk = serde_json::from_str(data).unwrap();
        assert!(chunk.choices[0].delta.content.is_none());
        assert_eq!(
            chunk.choices[0].delta.reasoning_content.as_deref(),
            Some("Let me think about this...")
        );
    }

    #[test]
    fn test_parse_sse_reasoning_delta_ollama_alias() {
        let data = r#"{"id":"resp_1","choices":[{"index":0,"delta":{"reasoning":"Let me think about this..."},"finish_reason":null}]}"#;
        let chunk: RawStreamChunk = serde_json::from_str(data).unwrap();
        assert!(chunk.choices[0].delta.content.is_none());
        assert_eq!(
            chunk.choices[0].delta.reasoning_content.as_deref(),
            Some("Let me think about this...")
        );
    }

    #[test]
    fn test_parse_sse_tool_call_delta() {
        let data = r#"{"id":"resp_1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"read_file","arguments":"{\"path\":"}}]},"finish_reason":null}]}"#;
        let chunk: RawStreamChunk = serde_json::from_str(data).unwrap();
        let tc = &chunk.choices[0].delta.tool_calls.as_ref().unwrap()[0];
        assert_eq!(tc.id.as_deref(), Some("call_1"));
        assert_eq!(
            tc.function.as_ref().unwrap().name.as_deref(),
            Some("read_file")
        );
    }

    #[test]
    fn test_parse_sse_finish_with_usage() {
        let data = r#"{"id":"resp_1","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":20,"total_tokens":30}}"#;
        let chunk: RawStreamChunk = serde_json::from_str(data).unwrap();
        assert_eq!(chunk.choices[0].finish_reason.as_deref(), Some("stop"));
        let usage = chunk.usage.unwrap();
        assert_eq!(usage.prompt_tokens, Some(10));
        assert_eq!(usage.completion_tokens, Some(20));
    }

    #[test]
    fn test_parse_non_streaming_response() {
        let data = r#"{"choices":[{"message":{"content":"Hello!","reasoning_content":"I should greet the user","tool_calls":null},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":10,"total_tokens":15},"model":"deepseek-r1"}"#;
        let resp: RawCompletionResponse = serde_json::from_str(data).unwrap();
        assert_eq!(resp.choices[0].message.content.as_deref(), Some("Hello!"));
        assert_eq!(
            resp.choices[0].message.reasoning_content.as_deref(),
            Some("I should greet the user")
        );
        assert_eq!(resp.model.as_deref(), Some("deepseek-r1"));
    }

    #[test]
    fn test_parse_non_streaming_response_with_ollama_reasoning_alias() {
        let data = r#"{"choices":[{"message":{"content":"Hello!","reasoning":"I should greet the user","tool_calls":null},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":10,"total_tokens":15},"model":"qwen3.5:cloud"}"#;
        let resp: RawCompletionResponse = serde_json::from_str(data).unwrap();
        assert_eq!(resp.choices[0].message.content.as_deref(), Some("Hello!"));
        assert_eq!(
            resp.choices[0].message.reasoning_content.as_deref(),
            Some("I should greet the user")
        );
        assert_eq!(resp.model.as_deref(), Some("qwen3.5:cloud"));
    }

    #[test]
    fn test_parse_non_streaming_with_tool_calls() {
        let data = r#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"search","arguments":"{\"q\":\"rust\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":20,"completion_tokens":15,"total_tokens":35},"model":"gpt-4"}"#;
        let resp: RawCompletionResponse = serde_json::from_str(data).unwrap();
        assert!(resp.choices[0].message.content.is_none());
        let tcs = resp.choices[0].message.tool_calls.as_ref().unwrap();
        assert_eq!(tcs.len(), 1);
        assert_eq!(tcs[0].id, "call_1");
        assert_eq!(tcs[0].function.name, "search");
    }

    #[test]
    fn test_parse_stop_reason_variants() {
        assert_eq!(parse_stop_reason(Some("stop")), StopReason::Stop);
        assert_eq!(parse_stop_reason(Some("tool_calls")), StopReason::ToolCalls);
        assert_eq!(parse_stop_reason(Some("length")), StopReason::Length);
        assert_eq!(
            parse_stop_reason(Some("content_filter")),
            StopReason::ContentFilter
        );
        assert_eq!(parse_stop_reason(Some("unknown")), StopReason::Stop);
        assert_eq!(parse_stop_reason(None), StopReason::Stop);
    }

    #[test]
    fn test_stream_options_only_when_streaming() {
        let request = CompletionRequest {
            model:               "gpt-4".to_string(),
            messages:            vec![Message::user("Hi")],
            tools:               vec![],
            temperature:         None,
            max_tokens:          None,
            thinking:            None,
            tool_choice:         ToolChoice::Auto,
            parallel_tool_calls: false,
        };

        let body_no_stream = build_body(&request, false);
        assert!(body_no_stream.get("stream_options").is_none());

        let body_stream = build_body(&request, true);
        assert!(body_stream.get("stream_options").is_some());
        assert_eq!(body_stream["stream_options"]["include_usage"], true);
    }

    #[tokio::test]
    async fn settings_backed_driver_reads_latest_settings_values() {
        let settings = Arc::new(MapSettingsProvider::default());
        settings
            .set(
                "llm.providers.openrouter.base_url",
                "https://one.example/v1",
            )
            .await
            .unwrap();
        settings
            .set("llm.providers.openrouter.api_key", "key-one")
            .await
            .unwrap();

        let driver = OpenAiDriver::from_settings(settings.clone(), "openrouter");

        let config = driver.resolve_config().await.unwrap();
        assert_eq!(config.base_url, "https://one.example/v1");
        assert_eq!(config.api_key, "key-one");

        settings
            .set(
                "llm.providers.openrouter.base_url",
                "https://two.example/v1",
            )
            .await
            .unwrap();
        settings
            .set("llm.providers.openrouter.api_key", "key-two")
            .await
            .unwrap();

        let config = driver.resolve_config().await.unwrap();
        assert_eq!(config.base_url, "https://two.example/v1");
        assert_eq!(config.api_key, "key-two");
    }

    #[tokio::test]
    async fn settings_backed_driver_requires_base_url_without_fallback() {
        let driver =
            OpenAiDriver::from_settings(Arc::new(MapSettingsProvider::default()), "ollama");

        let err = driver
            .resolve_config()
            .await
            .expect_err("missing base url should fail");
        assert!(matches!(err, KernelError::Provider { .. }));
        assert!(err.to_string().contains("base URL"));
    }

    #[tokio::test]
    async fn settings_backed_driver_requires_api_key_without_default() {
        let settings = Arc::new(MapSettingsProvider::default());
        settings
            .set(
                "llm.providers.openrouter.base_url",
                "https://openrouter.ai/api/v1",
            )
            .await
            .unwrap();

        let driver = OpenAiDriver::from_settings(settings, "openrouter");

        let err = driver
            .resolve_config()
            .await
            .expect_err("missing api key should fail");
        assert!(matches!(err, KernelError::Provider { .. }));
        assert!(err.to_string().contains("API key"));
    }
}
