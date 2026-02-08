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

//! AI provider trait and core request/response types.

use serde::{Deserialize, Serialize};

/// A single message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Role of the message author (system, user, assistant).
    pub role: String,
    /// Content of the message.
    pub content: String,
}

/// Response from an AI provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiResponse {
    /// The generated text.
    pub content: String,
    /// Model identifier that produced the response.
    pub model: String,
    /// Token usage statistics.
    pub usage: Option<TokenUsage>,
}

/// Token usage statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    /// Number of tokens in the prompt.
    pub prompt_tokens: u32,
    /// Number of tokens in the completion.
    pub completion_tokens: u32,
}

/// Trait that every AI provider backend must implement.
#[async_trait::async_trait]
pub trait AiProvider: Send + Sync {
    /// Human-readable name of this provider (e.g. "OpenAI", "Anthropic").
    fn name(&self) -> &str;

    /// Send a chat completion request and return the response.
    async fn chat(
        &self,
        messages: &[ChatMessage],
    ) -> Result<AiResponse, Box<dyn std::error::Error + Send + Sync>>;
}
