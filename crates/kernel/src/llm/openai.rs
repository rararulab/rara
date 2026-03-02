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

use std::collections::HashMap;

use async_trait::async_trait;
use futures::StreamExt;
use serde::Deserialize;
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
    client:   reqwest::Client,
    base_url: String,
    api_key:  String,
}

impl OpenAiDriver {
    /// Create a new driver targeting the given API base URL.
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            client:   reqwest::Client::new(),
            base_url: base_url.into(),
            api_key:  api_key.into(),
        }
    }

    /// Create a new driver targeting OpenRouter.
    pub fn openrouter(api_key: impl Into<String>) -> Self {
        Self::new("https://openrouter.ai/api/v1", api_key)
    }

    /// Create a new driver targeting a local Ollama instance.
    pub fn ollama(base_url: impl Into<String>) -> Self {
        let base = base_url.into();
        Self::new(format!("{base}/v1"), "ollama")
    }
}

// ---------------------------------------------------------------------------
// LlmDriver implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl LlmDriver for OpenAiDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        let body = build_request_body(&request, false);

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| KernelError::Provider {
                message: e.to_string().into(),
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(KernelError::Provider {
                message: format!("HTTP {status}: {text}").into(),
            });
        }

        let raw: RawCompletionResponse =
            response.json().await.map_err(|e| KernelError::Provider {
                message: format!("failed to parse response: {e}").into(),
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
        let body = build_request_body(&request, true);

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| KernelError::Provider {
                message: e.to_string().into(),
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(KernelError::Provider {
                message: format!("HTTP {status}: {text}").into(),
            });
        }

        let mut byte_stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut accumulated_text = String::new();
        let mut accumulated_reasoning = String::new();
        let mut pending_tools: HashMap<u32, PendingToolCall> = HashMap::new();
        let mut final_stop_reason = StopReason::Stop;
        let mut final_usage = None;

        while let Some(chunk_result) = byte_stream.next().await {
            let bytes = chunk_result.map_err(|e| KernelError::Provider {
                message: e.to_string().into(),
            })?;
            buffer.push_str(&String::from_utf8_lossy(&bytes));

            // Process complete lines
            while let Some(line_end) = buffer.find('\n') {
                let line = buffer[..line_end].trim().to_string();
                buffer = buffer[line_end + 1..].to_string();

                if line.is_empty() {
                    continue;
                }

                let Some(data) = line.strip_prefix("data: ") else {
                    continue;
                };

                if data == "[DONE]" {
                    let tool_call_results = collect_pending_tools(pending_tools);

                    let _ = tx
                        .send(StreamDelta::Done {
                            stop_reason: final_stop_reason,
                            usage:       final_usage,
                        })
                        .await;

                    return Ok(CompletionResponse {
                        content:           non_empty(accumulated_text),
                        reasoning_content: non_empty(accumulated_reasoning),
                        tool_calls:        tool_call_results,
                        stop_reason:       final_stop_reason,
                        usage:             final_usage,
                        model:             request.model.clone(),
                    });
                }

                // Parse JSON chunk
                let Ok(chunk) = serde_json::from_str::<RawStreamChunk>(data) else {
                    continue;
                };

                process_stream_chunk(
                    &chunk,
                    &tx,
                    &mut accumulated_text,
                    &mut accumulated_reasoning,
                    &mut pending_tools,
                    &mut final_stop_reason,
                    &mut final_usage,
                )
                .await;
            }
        }

        // Stream ended without [DONE] — still return what we have
        let tool_call_results = collect_pending_tools(pending_tools);

        let _ = tx
            .send(StreamDelta::Done {
                stop_reason: final_stop_reason,
                usage:       final_usage,
            })
            .await;

        Ok(CompletionResponse {
            content:           non_empty(accumulated_text),
            reasoning_content: non_empty(accumulated_reasoning),
            tool_calls:        tool_call_results,
            stop_reason:       final_stop_reason,
            usage:             final_usage,
            model:             request.model.clone(),
        })
    }
}

// ---------------------------------------------------------------------------
// Stream processing helpers
// ---------------------------------------------------------------------------

/// Accumulator for a tool call being assembled from streaming chunks.
struct PendingToolCall {
    id:        String,
    name:      String,
    arguments: String,
    /// Whether `ToolCallStart` has already been emitted for this call.
    started:   bool,
}

fn collect_pending_tools(pending: HashMap<u32, PendingToolCall>) -> Vec<ToolCallRequest> {
    let mut entries: Vec<(u32, PendingToolCall)> = pending.into_iter().collect();
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

async fn process_stream_chunk(
    chunk: &RawStreamChunk,
    tx: &mpsc::Sender<StreamDelta>,
    accumulated_text: &mut String,
    accumulated_reasoning: &mut String,
    pending_tools: &mut HashMap<u32, PendingToolCall>,
    final_stop_reason: &mut StopReason,
    final_usage: &mut Option<Usage>,
) {
    for choice in &chunk.choices {
        // Text delta
        if let Some(ref text) = choice.delta.content {
            if !text.is_empty() {
                accumulated_text.push_str(text);
                let _ = tx.send(StreamDelta::TextDelta { text: text.clone() }).await;
            }
        }

        // Reasoning delta
        if let Some(ref reasoning) = choice.delta.reasoning_content {
            if !reasoning.is_empty() {
                accumulated_reasoning.push_str(reasoning);
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
                let entry = pending_tools
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
            *final_stop_reason = parse_stop_reason(Some(reason.as_str()));
        }
    }

    // Usage (some providers send it in the last chunk)
    if let Some(ref usage) = chunk.usage {
        *final_usage = Some(Usage {
            prompt_tokens:     usage.prompt_tokens.unwrap_or(0),
            completion_tokens: usage.completion_tokens.unwrap_or(0),
            total_tokens:      usage.total_tokens.unwrap_or(0),
        });
    }
}

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
// Request body building
// ---------------------------------------------------------------------------

fn build_request_body(request: &CompletionRequest, stream: bool) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": request.model,
        "messages": request.messages.iter().map(message_to_json).collect::<Vec<_>>(),
        "stream": stream,
    });

    if let Some(temp) = request.temperature {
        body["temperature"] = serde_json::json!(temp);
    }
    if let Some(max) = request.max_tokens {
        body["max_completion_tokens"] = serde_json::json!(max);
    }
    if !request.tools.is_empty() {
        body["tools"] = serde_json::json!(
            request
                .tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.parameters,
                        }
                    })
                })
                .collect::<Vec<_>>()
        );
        body["tool_choice"] = match &request.tool_choice {
            ToolChoice::Auto => serde_json::json!("auto"),
            ToolChoice::None => serde_json::json!("none"),
            ToolChoice::Required => serde_json::json!("required"),
            ToolChoice::Specific(name) => {
                serde_json::json!({"type": "function", "function": {"name": name}})
            }
        };
        if request.parallel_tool_calls {
            body["parallel_tool_calls"] = serde_json::json!(true);
        }
    }
    if let Some(ref thinking) = request.thinking {
        if thinking.enabled {
            let mut thinking_obj = serde_json::json!({"type": "enabled"});
            if let Some(budget) = thinking.budget_tokens {
                thinking_obj["budget_tokens"] = serde_json::json!(budget);
            }
            body["thinking"] = thinking_obj;
        }
    }
    if stream {
        // Request usage in stream chunks
        body["stream_options"] = serde_json::json!({"include_usage": true});
    }
    body
}

fn message_to_json(msg: &Message) -> serde_json::Value {
    let role = match msg.role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    };

    let mut obj = serde_json::json!({ "role": role });

    match &msg.content {
        MessageContent::Text(text) => {
            obj["content"] = serde_json::json!(text);
        }
        MessageContent::Multimodal(blocks) => {
            let parts: Vec<serde_json::Value> = blocks
                .iter()
                .map(|b| match b {
                    ContentBlock::Text { text } => {
                        serde_json::json!({"type": "text", "text": text})
                    }
                    ContentBlock::ImageUrl { url } => {
                        serde_json::json!({"type": "image_url", "image_url": {"url": url}})
                    }
                })
                .collect();
            obj["content"] = serde_json::json!(parts);
        }
    }

    if !msg.tool_calls.is_empty() {
        obj["tool_calls"] = serde_json::json!(
            msg.tool_calls
                .iter()
                .map(|tc| {
                    serde_json::json!({
                        "id": tc.id,
                        "type": "function",
                        "function": {
                            "name": tc.name,
                            "arguments": tc.arguments,
                        }
                    })
                })
                .collect::<Vec<_>>()
        );
    }

    if let Some(ref id) = msg.tool_call_id {
        obj["tool_call_id"] = serde_json::json!(id);
    }

    obj
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
    use super::*;
    use crate::llm::types::ToolDefinition;

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
        let body = build_request_body(&request, false);
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
        let body = build_request_body(&request, true);
        assert_eq!(body["tools"].as_array().unwrap().len(), 1);
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["function"]["name"], "read_file");
        assert_eq!(body["tool_choice"], "auto");
        assert_eq!(body["parallel_tool_calls"], true);
        assert_eq!(body["stream"], true);
        assert_eq!(body["max_completion_tokens"], 4096);
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

        let body = build_request_body(&base, false);
        assert_eq!(body["tool_choice"], "none");

        let mut req = base.clone();
        req.tool_choice = ToolChoice::Required;
        let body = build_request_body(&req, false);
        assert_eq!(body["tool_choice"], "required");

        let mut req = base.clone();
        req.tool_choice = ToolChoice::Specific("read_file".to_string());
        let body = build_request_body(&req, false);
        assert_eq!(body["tool_choice"]["function"]["name"], "read_file");
    }

    #[test]
    fn test_message_to_json_system() {
        let msg = Message::system("Be helpful");
        let json = message_to_json(&msg);
        assert_eq!(json["role"], "system");
        assert_eq!(json["content"], "Be helpful");
    }

    #[test]
    fn test_message_to_json_tool_result() {
        let msg = Message::tool_result("call_1", "file contents here");
        let json = message_to_json(&msg);
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
        let json = message_to_json(&msg);
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
        let json = message_to_json(&msg);
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

        let body_no_stream = build_request_body(&request, false);
        assert!(body_no_stream.get("stream_options").is_none());

        let body_stream = build_request_body(&request, true);
        assert!(body_stream.get("stream_options").is_some());
        assert_eq!(body_stream["stream_options"]["include_usage"], true);
    }
}
