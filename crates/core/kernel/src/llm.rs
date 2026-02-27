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

//! LLM provider abstraction — unified interface for chat completion.

use std::pin::Pin;

use async_trait::async_trait;
use base::shared_string::SharedString;
use futures::Stream;
use serde::{Deserialize, Serialize};

use crate::model::ModelCapabilities;

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

/// Role in a chat conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChatRole {
    System,
    User,
    Assistant,
    Tool,
}

/// A tool call requested by the model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id:        SharedString,
    pub name:      SharedString,
    pub arguments: serde_json::Value,
}

/// Definition of a tool the model can call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name:        String,
    pub description: String,
    pub parameters:  serde_json::Value,
}

/// A single message in a chat conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role:         ChatRole,
    pub content:      Option<String>,
    #[serde(default)]
    pub tool_calls:   Vec<ToolCall>,
    #[serde(default)]
    pub tool_call_id: Option<String>,
}

/// A chat completion request.
#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model:         String,
    pub system_prompt: String,
    pub messages:      Vec<ChatMessage>,
    pub tools:         Option<Vec<ToolDefinition>>,
    pub temperature:   Option<f32>,
}

/// Reason the model stopped generating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FinishReason {
    Stop,
    ToolCalls,
    Length,
    ContentFilter,
}

/// Token usage statistics.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens:     u32,
    pub completion_tokens: u32,
    pub total_tokens:      u32,
}

/// A complete chat completion response.
#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub content:       Option<String>,
    pub tool_calls:    Vec<ToolCall>,
    pub finish_reason: FinishReason,
    pub usage:         Option<Usage>,
}

/// A streaming delta from a chat completion.
#[derive(Debug, Clone)]
pub struct ChatStreamDelta {
    pub content:       Option<String>,
    pub tool_calls:    Vec<ToolCall>,
    pub finish_reason: Option<FinishReason>,
}

// ---------------------------------------------------------------------------
// LlmApi trait
// ---------------------------------------------------------------------------

/// Unified LLM access — send chat completion requests.
#[async_trait]
pub trait LlmApi: Send + Sync {
    /// Send a chat completion request and return the full response.
    async fn chat(&self, request: ChatRequest) -> crate::error::Result<ChatResponse>;

    /// Send a streaming chat completion request.
    async fn chat_stream(
        &self,
        request: ChatRequest,
    ) -> crate::error::Result<
        Pin<Box<dyn Stream<Item = crate::error::Result<ChatStreamDelta>> + Send>>,
    >;

    /// Detect model capabilities (tool support, etc.).
    fn capabilities(&self, model: &str) -> ModelCapabilities;
}
