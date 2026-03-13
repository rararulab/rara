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

//! Core LLM types — messages, requests, responses, and related types.
//!
//! These types are independent of any specific LLM provider and form the
//! canonical request/response model for the [`LlmDriver`](super::LlmDriver)
//! trait.

use base::shared_string::SharedString;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Role
// ---------------------------------------------------------------------------

/// Role of the entity in the conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

// ---------------------------------------------------------------------------
// ContentBlock / MessageContent
// ---------------------------------------------------------------------------

/// A content block within a message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ImageUrl {
        url: String,
    },
    ImageBase64 {
        media_type: String,
        data:       String,
    },
}

/// Message content — either plain text or multimodal blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Multimodal(Vec<ContentBlock>),
}

impl MessageContent {
    pub fn as_text(&self) -> &str {
        match self {
            Self::Text(s) => s,
            Self::Multimodal(blocks) => blocks
                .iter()
                .find_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .unwrap_or(""),
        }
    }
}

impl From<String> for MessageContent {
    fn from(s: String) -> Self { Self::Text(s) }
}

impl From<&str> for MessageContent {
    fn from(s: &str) -> Self { Self::Text(s.to_owned()) }
}

// ---------------------------------------------------------------------------
// ToolCallRequest
// ---------------------------------------------------------------------------

/// A tool call requested by the model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRequest {
    pub id:        String,
    pub name:      String,
    /// JSON-encoded arguments string.
    pub arguments: String,
}

// ---------------------------------------------------------------------------
// Message
// ---------------------------------------------------------------------------

/// A single message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role:         Role,
    pub content:      MessageContent,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls:   Vec<ToolCallRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    pub fn system(text: impl Into<String>) -> Self {
        Self {
            role:         Role::System,
            content:      MessageContent::Text(text.into()),
            tool_calls:   Vec::new(),
            tool_call_id: None,
        }
    }

    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role:         Role::User,
            content:      MessageContent::Text(text.into()),
            tool_calls:   Vec::new(),
            tool_call_id: None,
        }
    }

    pub fn user_multimodal(blocks: Vec<ContentBlock>) -> Self {
        Self {
            role:         Role::User,
            content:      MessageContent::Multimodal(blocks),
            tool_calls:   Vec::new(),
            tool_call_id: None,
        }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role:         Role::Assistant,
            content:      MessageContent::Text(text.into()),
            tool_calls:   Vec::new(),
            tool_call_id: None,
        }
    }

    pub fn assistant_with_tool_calls(
        text: impl Into<String>,
        tool_calls: Vec<ToolCallRequest>,
    ) -> Self {
        Self {
            role: Role::Assistant,
            content: MessageContent::Text(text.into()),
            tool_calls,
            tool_call_id: None,
        }
    }

    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role:         Role::Tool,
            content:      MessageContent::Text(content.into()),
            tool_calls:   Vec::new(),
            tool_call_id: Some(tool_call_id.into()),
        }
    }

    pub fn tool_result_multimodal(
        tool_call_id: impl Into<String>,
        blocks: Vec<ContentBlock>,
    ) -> Self {
        Self {
            role:         Role::Tool,
            content:      MessageContent::Multimodal(blocks),
            tool_calls:   Vec::new(),
            tool_call_id: Some(tool_call_id.into()),
        }
    }

    /// Rough character-count estimate for context size budgeting.
    pub fn estimated_char_len(&self) -> usize {
        let content_len = match &self.content {
            MessageContent::Text(s) => s.len(),
            MessageContent::Multimodal(blocks) => blocks
                .iter()
                .map(|b| match b {
                    ContentBlock::Text { text } => text.len(),
                    ContentBlock::ImageUrl { url } => url.len(),
                    // base64 images are large but already counted by the provider;
                    // use a small constant so we don't over-count.
                    ContentBlock::ImageBase64 { .. } => 256,
                })
                .sum(),
        };
        let tool_calls_len: usize = self
            .tool_calls
            .iter()
            .map(|tc| tc.name.len() + tc.arguments.len())
            .sum();
        content_len + tool_calls_len
    }
}

// ---------------------------------------------------------------------------
// ToolDefinition
// ---------------------------------------------------------------------------

/// Definition of a tool the model can call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name:        String,
    pub description: String,
    pub parameters:  serde_json::Value,
}

// ---------------------------------------------------------------------------
// ThinkingConfig
// ---------------------------------------------------------------------------

/// Thinking/reasoning budget configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkingConfig {
    pub enabled:       bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_tokens: Option<u32>,
}

// ---------------------------------------------------------------------------
// ToolChoice
// ---------------------------------------------------------------------------

/// Tool choice strategy.
#[derive(Debug, Clone, Default)]
pub enum ToolChoice {
    #[default]
    Auto,
    None,
    Required,
    Specific(String),
}

// ---------------------------------------------------------------------------
// CompletionRequest
// ---------------------------------------------------------------------------

/// A chat completion request.
#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub model:               String,
    pub messages:            Vec<Message>,
    pub tools:               Vec<ToolDefinition>,
    pub temperature:         Option<f32>,
    pub max_tokens:          Option<u32>,
    pub thinking:            Option<ThinkingConfig>,
    pub tool_choice:         ToolChoice,
    pub parallel_tool_calls: bool,
    pub frequency_penalty:   Option<f32>,
}

// ---------------------------------------------------------------------------
// StopReason
// ---------------------------------------------------------------------------

/// Reason the model stopped generating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    Stop,
    ToolCalls,
    Length,
    ContentFilter,
}

// ---------------------------------------------------------------------------
// Usage
// ---------------------------------------------------------------------------

/// Token usage statistics.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens:     u32,
    pub completion_tokens: u32,
    pub total_tokens:      u32,
}

// ---------------------------------------------------------------------------
// CompletionResponse
// ---------------------------------------------------------------------------

/// A complete chat completion response.
#[derive(Debug, Clone)]
pub struct CompletionResponse {
    pub content:           Option<String>,
    pub reasoning_content: Option<String>,
    pub tool_calls:        Vec<ToolCallRequest>,
    pub stop_reason:       StopReason,
    pub usage:             Option<Usage>,
    pub model:             String,
}

// ---------------------------------------------------------------------------
// LlmProviderFamily / ModelCapabilities (migrated from model.rs)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmProviderFamily {
    OpenRouter,
    Ollama,
    Codex,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelCapabilities {
    pub provider:                     LlmProviderFamily,
    pub supports_tools:               bool,
    pub supports_parallel_tool_calls: bool,
    pub tools_disabled_reason:        Option<&'static str>,
    pub context_window_tokens:        usize,
}

impl ModelCapabilities {
    #[must_use]
    pub fn detect(provider_hint: Option<&str>, model_name: &str) -> Self {
        let provider = detect_provider_family(provider_hint, model_name);
        let canonical = canonical_model_name(model_name);

        // Ollama serves many raw models whose chat templates/tool-calling support
        // varies. Keep the deny-list small and explicit so unsupported models
        // degrade gracefully without breaking tool-capable ones.
        let context_window_tokens = estimate_context_window(&canonical);

        if matches!(provider, LlmProviderFamily::Ollama) && canonical.starts_with("deepseek-r1") {
            return Self {
                provider,
                supports_tools: false,
                supports_parallel_tool_calls: false,
                tools_disabled_reason: Some(
                    "ollama deepseek-r1 variants do not support function/tool calling",
                ),
                context_window_tokens,
            };
        }

        Self {
            provider,
            supports_tools: true,
            supports_parallel_tool_calls: !matches!(provider, LlmProviderFamily::Ollama),
            tools_disabled_reason: None,
            context_window_tokens,
        }
    }
}

// ---------------------------------------------------------------------------
// ToolCall (SharedString-based, migrated from model.rs)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id:        SharedString,
    pub name:      SharedString,
    pub arguments: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Helper functions (migrated from model.rs)
// ---------------------------------------------------------------------------

fn detect_provider_family(provider_hint: Option<&str>, model_name: &str) -> LlmProviderFamily {
    let provider_hint = provider_hint.map(str::trim).map(str::to_ascii_lowercase);
    match provider_hint.as_deref() {
        Some("ollama") => return LlmProviderFamily::Ollama,
        Some("openrouter") => return LlmProviderFamily::OpenRouter,
        Some("codex") => return LlmProviderFamily::Codex,
        _ => {}
    }

    let trimmed = model_name.trim();
    // Common Ollama local model syntax: `name:tag` with no provider prefix.
    if trimmed.contains(':') && !trimmed.contains('/') {
        return LlmProviderFamily::Ollama;
    }

    LlmProviderFamily::Unknown
}

/// Best-effort context window estimate based on the canonical model name.
fn estimate_context_window(canonical: &str) -> usize {
    if canonical.contains("gemini") {
        1_000_000
    } else if canonical.contains("claude") {
        200_000
    } else {
        128_000
    }
}

fn canonical_model_name(model_name: &str) -> String {
    let trimmed = model_name.trim().to_ascii_lowercase();
    trimmed
        .rsplit('/')
        .next()
        .unwrap_or(trimmed.as_str())
        .to_owned()
}
