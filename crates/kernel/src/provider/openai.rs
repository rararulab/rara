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

//! OpenAI-compatible [`LlmProvider`](super::LlmProvider) implementation.
//!
//! Uses `async_openai::Client` under the hood.  By default the client points
//! at the OpenRouter API base, but any OpenAI-compatible endpoint works.

use async_openai::{
    Client,
    config::OpenAIConfig,
    types::chat::{
        ChatCompletionResponseStream, CreateChatCompletionRequest, CreateChatCompletionResponse,
    },
};
use async_trait::async_trait;

use super::LlmProvider;
use crate::error::{KernelError, Result};

pub const OPENROUTER_API_KEY_ENV: &str = "OPENROUTER_KEY";

/// Default OpenRouter API base URL.
pub const OPENROUTER_API_BASE: &str = "https://openrouter.ai/api/v1";

/// [`LlmProvider`] backed by `async_openai::Client` with [`OpenAIConfig`].
///
/// This provider points at the OpenRouter API base by default but can be
/// configured to use any OpenAI-compatible endpoint.
pub struct OpenAiProvider {
    client: Client<OpenAIConfig>,
}

impl OpenAiProvider {
    /// Create a new provider from an API key, targeting OpenRouter.
    pub fn new(api_key: impl Into<String>) -> Self {
        let config = OpenAIConfig::new()
            .with_api_key(api_key)
            .with_api_base(OPENROUTER_API_BASE);
        Self {
            client: Client::with_config(config),
        }
    }

    /// Create a new provider with a fully custom [`OpenAIConfig`].
    pub fn with_config(config: OpenAIConfig) -> Self {
        Self {
            client: Client::with_config(config),
        }
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn chat_completion(
        &self,
        request: CreateChatCompletionRequest,
    ) -> Result<CreateChatCompletionResponse> {
        self.client
            .chat()
            .create(request)
            .await
            .map_err(|e| KernelError::Provider {
                message: e.to_string().into(),
            })
    }

    async fn chat_completion_stream(
        &self,
        request: CreateChatCompletionRequest,
    ) -> Result<ChatCompletionResponseStream> {
        self.client
            .chat()
            .create_stream(request)
            .await
            .map_err(|e| KernelError::Provider {
                message: e.to_string().into(),
            })
    }
}
