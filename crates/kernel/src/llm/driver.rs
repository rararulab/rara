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

//! [`LlmDriver`] trait — the unified interface for LLM providers.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;

use super::{
    stream::StreamDelta,
    types::{
        CompletionRequest, CompletionResponse, EmbeddingRequest, EmbeddingResponse, ModelInfo,
    },
};
use crate::error::Result;

/// Unified LLM driver interface.
///
/// Implementors translate between rara's types and provider-specific
/// formats (OpenAI, Anthropic, Ollama, etc.).
#[async_trait]
pub trait LlmDriver: Send + Sync {
    /// Send a completion request and wait for the full response.
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse>;

    /// Stream a completion request, sending incremental events to `tx`.
    ///
    /// Returns the accumulated response when complete. The driver
    /// MUST send `StreamDelta::Done` as the last event before returning.
    async fn stream(
        &self,
        request: CompletionRequest,
        tx: mpsc::Sender<StreamDelta>,
    ) -> Result<CompletionResponse>;

    /// Optional capability: return the context window size (in tokens) for
    /// the given model, if the provider exposes this metadata (e.g.,
    /// OpenRouter's `/models` endpoint returns `context_length`).
    ///
    /// Returns `None` when the provider does not support model metadata
    /// queries.  Callers fall back to a conservative default (128 K).
    async fn model_context_length(&self, _model: &str) -> Option<usize> { None }

    /// Whether the given model supports image/vision input.
    /// Returns `None` when the provider does not expose modality metadata.
    async fn model_supports_vision(&self, _model: &str) -> Option<bool> { None }
}

/// Shared reference to an [`LlmDriver`].
pub type LlmDriverRef = Arc<dyn LlmDriver>;

/// Trait for listing models available from a provider.
#[async_trait]
pub trait LlmModelLister: Send + Sync {
    /// List all models available from the provider.
    async fn list_models(&self) -> Result<Vec<ModelInfo>>;
}

/// Trait for generating text embeddings.
#[async_trait]
pub trait LlmEmbedder: Send + Sync {
    /// Generate embeddings for the given input texts.
    async fn embed(&self, request: EmbeddingRequest) -> Result<EmbeddingResponse>;
}

/// Reference-counted model lister.
pub type LlmModelListerRef = Arc<dyn LlmModelLister>;

/// Reference-counted embedder.
pub type LlmEmbedderRef = Arc<dyn LlmEmbedder>;

/// Credential resolved by a [`LlmCredentialResolver`].
///
/// Fields are private so that callers go through accessor methods,
/// making it straightforward to add auditing or redaction later.
#[derive(Debug, Clone)]
pub struct LlmCredential {
    base_url:      String,
    api_key:       String,
    /// Extra headers sent with every LLM request (e.g. `ChatGPT-Account-Id`
    /// for Codex OAuth). Keys are lowercased header names.
    extra_headers: Vec<(String, String)>,
}

impl LlmCredential {
    /// Create a new credential pair.
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            base_url:      base_url.into(),
            api_key:       api_key.into(),
            extra_headers: Vec::new(),
        }
    }

    /// Add an extra header to every request made with this credential.
    #[must_use]
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_headers.push((name.into(), value.into()));
        self
    }

    /// Provider base URL (e.g. `https://api.openai.com/v1`).
    pub fn base_url(&self) -> &str { &self.base_url }

    /// Bearer token or API key.
    pub fn api_key(&self) -> &str { &self.api_key }

    /// Extra headers for this credential (e.g. account ID).
    pub fn extra_headers(&self) -> &[(String, String)] { &self.extra_headers }
}

/// Dynamic credential resolver for LLM providers that need runtime
/// token refresh (e.g. OAuth flows).
#[async_trait]
pub trait LlmCredentialResolver: Send + Sync {
    /// Resolve the current credential, refreshing if necessary.
    async fn resolve(&self) -> Result<LlmCredential>;
}

/// Shared reference to an [`LlmCredentialResolver`].
pub type LlmCredentialResolverRef = Arc<dyn LlmCredentialResolver>;
