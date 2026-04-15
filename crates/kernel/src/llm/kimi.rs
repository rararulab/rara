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

//! Kimi Code backend driver — thin wrapper around [`OpenAiDriver`]
//! configured with the Kimi Code platform base URL.
//!
//! Authentication uses shared OAuth tokens from kimi-cli, resolved via
//! [`LlmCredentialResolverRef`].  Kimi Code uses the standard OpenAI
//! chat completions API format, so all request/response handling is
//! delegated to `OpenAiDriver`.

use async_trait::async_trait;
use tokio::sync::mpsc;

use super::{
    CompletionRequest, CompletionResponse, LlmCredentialResolverRef, StreamDelta,
    driver::LlmDriver, openai::OpenAiDriver, types::Role,
};
use crate::error::Result;

/// Kimi Code driver that calls the Kimi Code chat completions API.
pub struct KimiCodeDriver {
    inner: OpenAiDriver,
}

impl KimiCodeDriver {
    /// Create a new Kimi Code driver with a credential resolver.
    pub fn new(resolver: LlmCredentialResolverRef) -> Self {
        let inner =
            OpenAiDriver::with_credential_resolver(resolver, std::time::Duration::from_secs(120));
        Self { inner }
    }
}

/// Filter empty assistant messages that Kimi rejects with 400.
/// Sanitize a request for Kimi API compatibility:
/// - Remove empty assistant messages (400 on Kimi)
/// - Clear frequency_penalty (only 0 allowed on code models)
fn sanitize_request(mut request: CompletionRequest) -> CompletionRequest {
    request.messages.retain(|m| {
        !(m.role == Role::Assistant && m.tool_calls.is_empty() && m.content.as_text().is_empty())
    });
    request.frequency_penalty = None;
    request
}

#[async_trait]
impl LlmDriver for KimiCodeDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        self.inner.complete(sanitize_request(request)).await
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        tx: mpsc::Sender<StreamDelta>,
    ) -> Result<CompletionResponse> {
        self.inner.stream(sanitize_request(request), tx).await
    }

    async fn model_context_length(&self, _model: &str) -> Option<usize> { Some(128_000) }

    async fn model_supports_vision(&self, _model: &str) -> Option<bool> { Some(true) }
}
