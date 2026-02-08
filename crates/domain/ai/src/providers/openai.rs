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

//! OpenAI provider implementation.
//!
//! This module contains a stub implementation that has the correct
//! structure for making HTTP calls to the OpenAI chat completions API.
//! The actual HTTP client integration is left as a TODO for the
//! infrastructure layer.

use serde::{Deserialize, Serialize};

use crate::{
    error::AiError,
    provider::{AiModelProvider, AiProvider},
    types::{CompletionRequest, CompletionResponse, FinishReason, Message, TokenUsage},
};

// -----------------------------------------------------------------------
// Configuration
// -----------------------------------------------------------------------

/// Configuration for the OpenAI provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiConfig {
    /// OpenAI API key.
    pub api_key:       String,
    /// Base URL for the API (defaults to
    /// `https://api.openai.com/v1`).
    pub base_url:      String,
    /// Default model to use when the request does not specify one.
    pub default_model: String,
    /// Optional organization ID for multi-org accounts.
    pub org_id:        Option<String>,
}

impl Default for OpenAiConfig {
    fn default() -> Self {
        Self {
            api_key:       String::new(),
            base_url:      "https://api.openai.com/v1".to_owned(),
            default_model: "gpt-4o".to_owned(),
            org_id:        None,
        }
    }
}

// -----------------------------------------------------------------------
// OpenAI API request / response types
// -----------------------------------------------------------------------

/// A message in the OpenAI chat format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiMessage {
    pub role:    String,
    pub content: String,
}

impl From<&Message> for OpenAiMessage {
    fn from(msg: &Message) -> Self {
        Self {
            role:    msg.role.to_string(),
            content: msg.content.clone(),
        }
    }
}

/// Request body for the OpenAI `/chat/completions` endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiChatRequest {
    pub model:           String,
    pub messages:        Vec<OpenAiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature:     Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens:      Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<serde_json::Value>,
}

impl OpenAiChatRequest {
    /// Build an OpenAI-specific request from a generic
    /// [`CompletionRequest`].
    #[must_use]
    pub fn from_completion_request(req: &CompletionRequest, default_model: &str) -> Self {
        let model = if req.model.is_empty() {
            default_model.to_owned()
        } else {
            req.model.clone()
        };

        let messages = req.messages.iter().map(OpenAiMessage::from).collect();

        let response_format = req.output_schema.as_ref().map(|schema| {
            serde_json::json!({
                "type": "json_schema",
                "json_schema": {
                    "name": "output",
                    "schema": schema
                }
            })
        });

        Self {
            model,
            messages,
            temperature: req.temperature,
            max_tokens: req.max_tokens,
            response_format,
        }
    }
}

/// Token usage returned by the OpenAI API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiUsage {
    pub prompt_tokens:     u32,
    pub completion_tokens: u32,
    pub total_tokens:      u32,
}

/// A single choice in the OpenAI chat response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiChoice {
    pub index:         u32,
    pub message:       OpenAiMessage,
    pub finish_reason: Option<String>,
}

/// Response body from the OpenAI `/chat/completions` endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiChatResponse {
    pub id:      String,
    pub object:  String,
    pub model:   String,
    pub choices: Vec<OpenAiChoice>,
    pub usage:   Option<OpenAiUsage>,
}

impl OpenAiChatResponse {
    /// Convert this OpenAI-specific response into a generic
    /// [`CompletionResponse`].
    pub fn into_completion_response(self) -> Result<CompletionResponse, AiError> {
        let choice = self
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| AiError::InvalidResponse {
                provider: "openai".to_owned(),
                message:  "response contained no choices".to_owned(),
            })?;

        let finish_reason = match choice.finish_reason.as_deref() {
            Some("length") => FinishReason::Length,
            Some("tool_calls") => FinishReason::ToolCall,
            Some("content_filter") => FinishReason::ContentFilter,
            // "stop" and any unknown reason default to Stop.
            _ => FinishReason::Stop,
        };

        let usage = self.usage.map_or(
            TokenUsage {
                input_tokens:  0,
                output_tokens: 0,
                total_tokens:  0,
            },
            |u| TokenUsage {
                input_tokens:  u.prompt_tokens,
                output_tokens: u.completion_tokens,
                total_tokens:  u.total_tokens,
            },
        );

        Ok(CompletionResponse {
            content: choice.message.content,
            model: self.model,
            usage,
            finish_reason,
        })
    }
}

// -----------------------------------------------------------------------
// Provider implementation
// -----------------------------------------------------------------------

/// OpenAI provider backed by the chat completions API.
#[derive(Debug, Clone)]
pub struct OpenAiProvider {
    config: OpenAiConfig,
}

impl OpenAiProvider {
    /// Create a new `OpenAiProvider` with the given configuration.
    #[must_use]
    pub const fn new(config: OpenAiConfig) -> Self { Self { config } }
}

#[async_trait::async_trait]
impl AiProvider for OpenAiProvider {
    fn provider_name(&self) -> AiModelProvider { AiModelProvider::Openai }

    fn default_model(&self) -> &str { &self.config.default_model }

    async fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse, AiError> {
        let _api_request =
            OpenAiChatRequest::from_completion_request(request, &self.config.default_model);

        // TODO: Send `_api_request` to `{base_url}/chat/completions`
        //       using an HTTP client (reqwest).
        //
        // let url = format!("{}/chat/completions", self.config.base_url);
        // let mut http_req = client.post(&url)
        //     .bearer_auth(&self.config.api_key)
        //     .json(&_api_request);
        // if let Some(org) = &self.config.org_id {
        //     http_req = http_req.header("OpenAI-Organization", org);
        // }
        // let resp = http_req.send().await?;
        // let body: OpenAiChatResponse = resp.json().await?;
        // body.into_completion_response()

        tracing::warn!("OpenAI provider is stubbed -- returning placeholder response");

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
        // TODO: Make a lightweight API call (e.g. list models) to
        //       verify connectivity and auth.
        tracing::debug!("OpenAI health check stubbed");

        if self.config.api_key.is_empty() {
            return Err(AiError::AuthFailed {
                provider: "openai".to_owned(),
                message:  "API key is not configured".to_owned(),
            });
        }

        Ok(())
    }
}
