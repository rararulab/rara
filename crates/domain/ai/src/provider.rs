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

//! AI provider trait that every backend must implement.

use crate::{
    error::AiError,
    types::{CompletionRequest, CompletionResponse},
};

/// Enumeration of supported AI model providers.
///
/// This mirrors `AiModelProvider` in yunara-store but lives in the
/// domain layer so that domain code does not depend on the store
/// crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiModelProvider {
    /// OpenAI (GPT family).
    Openai,
    /// Anthropic (Claude family).
    Anthropic,
    /// A locally-hosted model (e.g. Ollama, vLLM).
    Local,
    /// Any other provider not explicitly listed.
    Other,
}

impl std::fmt::Display for AiModelProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Openai => write!(f, "openai"),
            Self::Anthropic => write!(f, "anthropic"),
            Self::Local => write!(f, "local"),
            Self::Other => write!(f, "other"),
        }
    }
}

/// Trait that every AI provider backend must implement.
///
/// Implementations are expected to be cheaply cloneable (typically
/// wrapping an `Arc` internally) so that they can be shared across
/// tasks.
#[async_trait::async_trait]
pub trait AiProvider: Send + Sync {
    /// Return which provider this implementation represents.
    fn provider_name(&self) -> AiModelProvider;

    /// Return the default model identifier for this provider.
    fn default_model(&self) -> &str;

    /// Send a completion request and return the response.
    async fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse, AiError>;

    /// Perform a lightweight health check against the provider.
    ///
    /// Implementations should verify connectivity and authentication
    /// without consuming significant quota.
    async fn check_health(&self) -> Result<(), AiError>;
}
