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

//! Anthropic (Claude) provider implementation.
//!
//! This module contains a stub implementation that has the correct
//! structure for making HTTP calls to the Anthropic messages API.
//! The actual HTTP client integration is left as a TODO for the
//! infrastructure layer.

use serde::{Deserialize, Serialize};

use crate::{
    error::AiError,
    provider::{AiModelProvider, AiProvider},
    types::{CompletionRequest, CompletionResponse, FinishReason, MessageRole, TokenUsage},
};

// -----------------------------------------------------------------------
// Configuration
// -----------------------------------------------------------------------

/// Configuration for the Anthropic provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicConfig {
    /// Anthropic API key.
    pub api_key:       String,
    /// Base URL for the API (defaults to
    /// `https://api.anthropic.com`).
    pub base_url:      String,
    /// Default model to use when the request does not specify one.
    pub default_model: String,
    /// The `anthropic-version` header value.
    pub api_version:   String,
}

impl Default for AnthropicConfig {
    fn default() -> Self {
        Self {
            api_key:       String::new(),
            base_url:      "https://api.anthropic.com".to_owned(),
            default_model: "claude-sonnet-4-20250514".to_owned(),
            api_version:   "2023-06-01".to_owned(),
        }
    }
}

// -----------------------------------------------------------------------
// Anthropic API request / response types
// -----------------------------------------------------------------------

/// A message in the Anthropic messages API format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicMessage {
    pub role:    String,
    pub content: String,
}

/// Request body for the Anthropic `/v1/messages` endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicMessagesRequest {
    pub model:       String,
    pub messages:    Vec<AnthropicMessage>,
    /// The system prompt (sent as a top-level field, not as a message).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system:      Option<String>,
    pub max_tokens:  u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
}

impl AnthropicMessagesRequest {
    /// Build an Anthropic-specific request from a generic
    /// [`CompletionRequest`].
    #[must_use]
    pub fn from_completion_request(req: &CompletionRequest, default_model: &str) -> Self {
        let model = if req.model.is_empty() {
            default_model.to_owned()
        } else {
            req.model.clone()
        };

        // Anthropic expects system prompt as a top-level field, not in
        // the messages array.
        let mut system: Option<String> = None;
        let mut messages = Vec::new();

        for msg in &req.messages {
            match msg.role {
                MessageRole::System => {
                    // Concatenate multiple system messages if present.
                    match &mut system {
                        Some(s) => {
                            s.push('\n');
                            s.push_str(&msg.content);
                        }
                        None => system = Some(msg.content.clone()),
                    }
                }
                MessageRole::User | MessageRole::Assistant => {
                    messages.push(AnthropicMessage {
                        role:    msg.role.to_string(),
                        content: msg.content.clone(),
                    });
                }
            }
        }

        Self {
            model,
            messages,
            system,
            max_tokens: req.max_tokens.unwrap_or(4096),
            temperature: req.temperature,
        }
    }
}

/// Token usage returned by the Anthropic API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicUsage {
    pub input_tokens:  u32,
    pub output_tokens: u32,
}

/// A content block in an Anthropic response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicContentBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    #[serde(default)]
    pub text:       String,
}

/// Response body from the Anthropic `/v1/messages` endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicMessagesResponse {
    pub id:          String,
    pub model:       String,
    pub role:        String,
    pub content:     Vec<AnthropicContentBlock>,
    pub stop_reason: Option<String>,
    pub usage:       AnthropicUsage,
}

impl AnthropicMessagesResponse {
    /// Convert this Anthropic-specific response into a generic
    /// [`CompletionResponse`].
    pub fn into_completion_response(self) -> Result<CompletionResponse, AiError> {
        let content = self
            .content
            .iter()
            .filter(|b| b.block_type == "text")
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join("");

        if content.is_empty() {
            return Err(AiError::InvalidResponse {
                provider: "anthropic".to_owned(),
                message:  "response contained no text content blocks".to_owned(),
            });
        }

        let finish_reason = match self.stop_reason.as_deref() {
            Some("max_tokens") => FinishReason::Length,
            Some("tool_use") => FinishReason::ToolCall,
            // "end_turn" and any unknown reason default to Stop.
            _ => FinishReason::Stop,
        };

        let total = self.usage.input_tokens + self.usage.output_tokens;

        Ok(CompletionResponse {
            content,
            model: self.model,
            usage: TokenUsage {
                input_tokens:  self.usage.input_tokens,
                output_tokens: self.usage.output_tokens,
                total_tokens:  total,
            },
            finish_reason,
        })
    }
}

// -----------------------------------------------------------------------
// Provider implementation
// -----------------------------------------------------------------------

/// Anthropic provider backed by the messages API.
#[derive(Debug, Clone)]
pub struct AnthropicProvider {
    config: AnthropicConfig,
}

impl AnthropicProvider {
    /// Create a new `AnthropicProvider` with the given configuration.
    #[must_use]
    pub const fn new(config: AnthropicConfig) -> Self { Self { config } }
}

#[async_trait::async_trait]
impl AiProvider for AnthropicProvider {
    fn provider_name(&self) -> AiModelProvider { AiModelProvider::Anthropic }

    fn default_model(&self) -> &str { &self.config.default_model }

    async fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse, AiError> {
        let _api_request =
            AnthropicMessagesRequest::from_completion_request(request, &self.config.default_model);

        // TODO: Send `_api_request` to `{base_url}/v1/messages`
        //       using an HTTP client (reqwest).
        //
        // let url = format!("{}/v1/messages", self.config.base_url);
        // let resp = client.post(&url)
        //     .header("x-api-key", &self.config.api_key)
        //     .header("anthropic-version", &self.config.api_version)
        //     .header("content-type", "application/json")
        //     .json(&_api_request)
        //     .send()
        //     .await?;
        // let body: AnthropicMessagesResponse = resp.json().await?;
        // body.into_completion_response()

        tracing::warn!("Anthropic provider is stubbed -- returning placeholder response");

        Ok(CompletionResponse {
            content:       String::new(),
            model:         request.model.clone(),
            usage:         TokenUsage {
                input_tokens:  0,
                output_tokens: 0,
                total_tokens:  0,
            },
            finish_reason: FinishReason::Stop,
        })
    }

    async fn check_health(&self) -> Result<(), AiError> {
        // TODO: Make a lightweight API call to verify connectivity
        //       and auth.
        tracing::debug!("Anthropic health check stubbed");

        if self.config.api_key.is_empty() {
            return Err(AiError::AuthFailed {
                provider: "anthropic".to_owned(),
                message:  "API key is not configured".to_owned(),
            });
        }

        Ok(())
    }
}
