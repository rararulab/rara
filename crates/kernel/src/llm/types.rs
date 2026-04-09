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
    /// Developer role (GPT-4.1+); semantically equivalent to system for most
    /// providers.
    Developer,
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
    /// Inline base64-encoded audio data (transcribed server-side by STT).
    /// Should never reach the LLM — the adapter transcribes before submission.
    AudioBase64 {
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

    /// Return a copy of this message with image content blocks replaced by
    /// a text placeholder. Text-only messages are returned as-is.
    #[must_use]
    pub fn strip_images(&self) -> Self {
        let content = match &self.content {
            MessageContent::Multimodal(blocks) => {
                let has_non_text = blocks.iter().any(|b| {
                    matches!(
                        b,
                        ContentBlock::ImageUrl { .. }
                            | ContentBlock::ImageBase64 { .. }
                            | ContentBlock::AudioBase64 { .. }
                    )
                });
                if !has_non_text {
                    return self.clone();
                }
                let text_parts: Vec<&str> = blocks
                    .iter()
                    .map(|b| match b {
                        ContentBlock::Text { text } => text.as_str(),
                        ContentBlock::ImageUrl { .. } | ContentBlock::ImageBase64 { .. } => {
                            "[image: current model does not support vision]"
                        }
                        ContentBlock::AudioBase64 { .. } => "[audio]",
                    })
                    .collect();
                MessageContent::Text(text_parts.join("\n"))
            }
            MessageContent::Text(_) => return self.clone(),
        };
        Self {
            content,
            ..self.clone()
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
                    // Audio blocks are transcribed before reaching the LLM;
                    // estimate a small placeholder size.
                    ContentBlock::AudioBase64 { .. } => 100,
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
    // TODO: support me later
    // AllowedTools(Vec<String>),
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
    /// Nucleus sampling threshold. GLM defaults to 0.95; pass through for all
    /// providers.
    pub top_p:               Option<f32>,
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
    /// 智谱 BigModel (GLM series).
    Glm,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelCapabilities {
    pub provider:                     LlmProviderFamily,
    pub supports_tools:               bool,
    pub supports_parallel_tool_calls: bool,
    pub tools_disabled_reason:        Option<&'static str>,
    pub context_window_tokens:        usize,
    /// Whether the model accepts image/vision content in messages.
    pub supports_vision:              bool,
}

/// Conservative fallback context window size used when the provider API
/// does not report `context_length` and no manifest override is set.
pub const DEFAULT_CONTEXT_WINDOW_TOKENS: usize = 128_000;

impl ModelCapabilities {
    /// Override the context window size (e.g., from agent manifest config or
    /// runtime model metadata).
    #[must_use]
    pub fn with_context_window(mut self, tokens: usize) -> Self {
        self.context_window_tokens = tokens;
        self
    }

    /// Set whether the model supports vision/image content.
    #[must_use]
    pub fn with_vision(mut self, supported: bool) -> Self {
        self.supports_vision = supported;
        self
    }

    /// Detect model capabilities from provider hint and model name.
    ///
    /// The `context_window_tokens` field is set to
    /// [`DEFAULT_CONTEXT_WINDOW_TOKENS`] and should be overridden at runtime
    /// via the priority chain: manifest override > provider API > default.
    #[must_use]
    pub fn detect(provider_hint: Option<&str>, model_name: &str) -> Self {
        let provider = detect_provider_family(provider_hint, model_name);
        let lower = model_name.to_ascii_lowercase();

        // Ollama serves many raw models whose chat templates/tool-calling support
        // varies. Keep the deny-list small and explicit so unsupported models
        // degrade gracefully without breaking tool-capable ones.
        if matches!(provider, LlmProviderFamily::Ollama) && lower.contains("deepseek-r1") {
            return Self {
                provider,
                supports_tools: false,
                supports_parallel_tool_calls: false,
                tools_disabled_reason: Some(
                    "ollama deepseek-r1 variants do not support function/tool calling",
                ),
                context_window_tokens: DEFAULT_CONTEXT_WINDOW_TOKENS,
                supports_vision: false,
            };
        }

        Self {
            provider,
            supports_tools: true,
            supports_parallel_tool_calls: !matches!(provider, LlmProviderFamily::Ollama),
            tools_disabled_reason: None,
            context_window_tokens: DEFAULT_CONTEXT_WINDOW_TOKENS,
            supports_vision: is_known_vision_model(&lower),
        }
    }
}

/// Best-effort vision support detection by model name substring.
///
/// The OpenRouter `/models` endpoint carries authoritative modality
/// metadata, and the driver layer overrides this via
/// [`ModelCapabilities::with_vision`] when that cache is populated. This
/// helper is the fallback for providers that do not publish modality
/// metadata (local Ollama, custom OpenAI-compatible endpoints, models
/// not yet in the OpenRouter cache).
///
/// Matches are lowercased substring checks so variants
/// (`gpt-4o-mini`, `gpt-4o-2024-08-06`, `anthropic/claude-3.5-sonnet`)
/// all resolve to the same verdict.
fn is_known_vision_model(lower_model: &str) -> bool {
    const VISION_MARKERS: &[&str] = &[
        // OpenAI
        "gpt-4o",
        "gpt-4-vision",
        "gpt-4-turbo",
        "o1",
        "o3",
        "o4",
        // Anthropic Claude 3+ (all support vision)
        "claude-3",
        "claude-opus",
        "claude-sonnet",
        "claude-haiku",
        // Google Gemini
        "gemini-1.5",
        "gemini-2",
        "gemini-pro-vision",
        // Qwen vision variants
        "qwen-vl",
        "qwen2-vl",
        "qwen2.5-vl",
        // Meta Llama vision
        "llama-3.2-vision",
        "llama-4",
        // Local vision models
        "llava",
        "bakllava",
        "moondream",
        "minicpm-v",
        // MiniMax
        "minimax-m",
    ];
    VISION_MARKERS.iter().any(|m| lower_model.contains(m))
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

pub(super) fn detect_provider_family(
    provider_hint: Option<&str>,
    model_name: &str,
) -> LlmProviderFamily {
    let provider_hint = provider_hint.map(str::trim).map(str::to_ascii_lowercase);
    match provider_hint.as_deref() {
        Some("ollama") => return LlmProviderFamily::Ollama,
        Some("openrouter") => return LlmProviderFamily::OpenRouter,
        Some("codex") => return LlmProviderFamily::Codex,
        Some("glm" | "zhipu" | "bigmodel") => return LlmProviderFamily::Glm,
        _ => {}
    }

    let lower = model_name.to_ascii_lowercase();
    if lower.starts_with("glm-") || lower.starts_with("glm4") {
        return LlmProviderFamily::Glm;
    }

    let trimmed = model_name.trim();
    // Common Ollama local model syntax: `name:tag` with no provider prefix.
    if trimmed.contains(':') && !trimmed.contains('/') {
        return LlmProviderFamily::Ollama;
    }

    LlmProviderFamily::Unknown
}

// ---------------------------------------------------------------------------
// ModelInfo / EmbeddingRequest / EmbeddingResponse
// ---------------------------------------------------------------------------

/// Metadata for a model available from the provider.
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Model identifier (e.g. "gpt-4o", "llama3:latest").
    pub id:       String,
    /// Organization that owns/provides the model.
    pub owned_by: String,
    /// Unix timestamp when the model was created.
    pub created:  Option<u64>,
}

/// Request to generate text embeddings.
#[derive(Debug, Clone, bon::Builder)]
pub struct EmbeddingRequest {
    /// The embedding model to use (e.g. "text-embedding-3-small").
    pub model:      String,
    /// Input texts to embed.
    pub input:      Vec<String>,
    /// Optional output dimensions (for models that support it).
    pub dimensions: Option<usize>,
}

/// Response from an embedding request.
#[derive(Debug, Clone, bon::Builder)]
pub struct EmbeddingResponse {
    /// One embedding vector per input text, in order.
    pub embeddings: Vec<Vec<f32>>,
    /// The model that generated the embeddings.
    pub model:      String,
    /// Token usage statistics.
    pub usage:      Option<Usage>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_defaults_to_128k_context_window() {
        let caps = ModelCapabilities::detect(None, "some-unknown-model");
        assert_eq!(caps.context_window_tokens, DEFAULT_CONTEXT_WINDOW_TOKENS);
        assert!(caps.supports_tools);
    }

    #[test]
    fn detect_ollama_deepseek_r1_disables_tools() {
        let caps = ModelCapabilities::detect(Some("ollama"), "deepseek-r1:latest");
        assert!(!caps.supports_tools);
        assert_eq!(caps.provider, LlmProviderFamily::Ollama);
        assert_eq!(caps.context_window_tokens, DEFAULT_CONTEXT_WINDOW_TOKENS);
    }

    #[test]
    fn strip_images_replaces_image_blocks_with_notice() {
        let msg = Message {
            role:         Role::User,
            content:      MessageContent::Multimodal(vec![
                ContentBlock::Text {
                    text: "look at this".into(),
                },
                ContentBlock::ImageBase64 {
                    media_type: "image/jpeg".into(),
                    data:       "AAAA".into(),
                },
            ]),
            tool_calls:   vec![],
            tool_call_id: None,
        };
        let stripped = msg.strip_images();
        match &stripped.content {
            MessageContent::Text(t) => {
                assert!(t.contains("look at this"), "text should be preserved");
                assert!(t.contains("[image:"), "image placeholder should be present");
            }
            MessageContent::Multimodal(_) => panic!("should be text after stripping"),
        }
    }

    #[test]
    fn strip_images_preserves_text_only_message() {
        let msg = Message::user("hello");
        let stripped = msg.strip_images();
        assert_eq!(stripped.content.as_text(), "hello");
    }

    #[test]
    fn with_context_window_overrides_default() {
        let caps = ModelCapabilities::detect(None, "gpt-4o").with_context_window(200_000);
        assert_eq!(caps.context_window_tokens, 200_000);
    }

    #[test]
    fn detect_identifies_vision_models_by_name() {
        let vision_models = [
            "gpt-4o",
            "gpt-4o-mini",
            "openai/gpt-4o-2024-08-06",
            "gpt-4-turbo",
            "gpt-4-vision-preview",
            "claude-3-opus-20240229",
            "anthropic/claude-3.5-sonnet",
            "claude-3-haiku",
            "gemini-1.5-pro",
            "gemini-2.0-flash",
            "google/gemini-pro-vision",
            "qwen2-vl-7b",
            "qwen2.5-vl-72b-instruct",
            "llama-3.2-vision-11b",
            "llava",
            "llava-1.6",
            "MiniMax-M2.7",
            "minimax-m1",
        ];
        for model in vision_models {
            let caps = ModelCapabilities::detect(None, model);
            assert!(
                caps.supports_vision,
                "{model} should be detected as vision-capable"
            );
        }
    }

    #[test]
    fn detect_does_not_flag_non_vision_models() {
        let text_only = [
            "gpt-3.5-turbo",
            "claude-2",
            "deepseek-r1",
            "qwen2-7b",
            "llama-3-8b",
            "mistral-7b",
            "some-unknown-model",
        ];
        for model in text_only {
            let caps = ModelCapabilities::detect(None, model);
            assert!(
                !caps.supports_vision,
                "{model} should NOT be detected as vision-capable"
            );
        }
    }
}
