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
    types::{CompletionRequest, CompletionResponse},
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
}

/// Shared reference to an [`LlmDriver`].
pub type LlmDriverRef = Arc<dyn LlmDriver>;
