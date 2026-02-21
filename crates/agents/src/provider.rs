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

//! LLM provider abstraction.
//!
//! [`LlmProvider`] defines the interface for chat completions (streaming and
//! non-streaming).  [`OpenAiProvider`] is the default implementation backed by
//! `async_openai::Client`, targeting the OpenRouter API.
//!
//! [`LlmProviderLoader`] is a factory trait that creates providers on demand,
//! reading credentials from environment variables, runtime settings, etc.

use std::sync::Arc;

use async_openai::{
    Client,
    config::OpenAIConfig,
    types::chat::{
        ChatCompletionResponseStream, CreateChatCompletionRequest,
        CreateChatCompletionResponse,
    },
};
use async_trait::async_trait;
use snafu::OptionExt;

use crate::err::prelude::*;

pub type LlmProviderRef = Arc<dyn LlmProvider>;

pub const OPENROUTER_API_KEY_ENV: &str = "OPENROUTER_KEY";

/// Default OpenRouter API base URL used by [`OpenAiProvider`].
const OPENROUTER_API_BASE: &str = "https://openrouter.ai/api/v1";

/// Trait abstracting an LLM provider capable of chat completions.
///
/// Implementors wrap a concrete HTTP client (e.g. `async_openai::Client`)
/// and expose non-streaming and streaming chat completion methods.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Send a chat completion request and wait for the full response.
    async fn chat_completion(
        &self,
        request: CreateChatCompletionRequest,
    ) -> Result<CreateChatCompletionResponse>;

    /// Send a chat completion request and return a stream of response chunks.
    async fn chat_completion_stream(
        &self,
        request: CreateChatCompletionRequest,
    ) -> Result<ChatCompletionResponseStream>;
}

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
            .map_err(|e| Error::Provider {
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
            .map_err(|e| Error::Provider {
                message: e.to_string().into(),
            })
    }
}

// -- Provider loader ---------------------------------------------------------

/// Factory trait for acquiring an [`LlmProvider`].
///
/// Each call to [`acquire_provider`](LlmProviderLoader::acquire_provider)
/// should return a provider configured with the **current** credentials.
/// Implementations must NOT cache the provider permanently — API keys can
/// change at runtime (e.g. via the settings UI).
#[async_trait]
pub trait LlmProviderLoader: Send + Sync {
    async fn acquire_provider(&self) -> Result<Arc<dyn LlmProvider>>;
}

/// Convenience alias used across the codebase.
pub type LlmProviderLoaderRef = Arc<dyn LlmProviderLoader>;

/// [`LlmProviderLoader`] that reads the API key from the `OPENROUTER_KEY`
/// environment variable.  Since env vars don't change at runtime this loader
/// caches the provider after the first successful initialisation.
#[derive(Clone, Default)]
pub struct EnvLlmProviderLoader {
    provider: Arc<tokio::sync::OnceCell<Arc<dyn LlmProvider>>>,
}

impl std::fmt::Debug for EnvLlmProviderLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvLlmProviderLoader")
            .field("initialized", &self.provider.initialized())
            .finish()
    }
}

#[async_trait]
impl LlmProviderLoader for EnvLlmProviderLoader {
    async fn acquire_provider(&self) -> Result<Arc<dyn LlmProvider>> {
        let provider_ref = self
            .provider
            .get_or_try_init(|| async {
                let api_key = base::env::required_var(OPENROUTER_API_KEY_ENV)
                    .ok()
                    .context(ProviderNotConfiguredSnafu)?;

                let provider: Arc<dyn LlmProvider> = Arc::new(OpenAiProvider::new(api_key));
                Ok::<_, Error>(provider)
            })
            .await?;

        Ok(Arc::clone(provider_ref))
    }
}
