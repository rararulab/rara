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

//! LLM driver abstraction — unified interface for chat completion.
//!
//! This module provides:
//! - [`LlmDriver`] trait — the primary interface for LLM providers
//! - [`CompletionRequest`] / [`CompletionResponse`] — request/response types
//! - [`StreamDelta`] — streaming event types
//! - [`Message`] — conversation message type
//!
//! ## Legacy compatibility
//!
//! The legacy [`LlmApi`] trait, [`ChatRequest`], [`ChatResponse`], and
//! [`ChatStreamDelta`] types are preserved for backward compatibility.
//! New code should use [`LlmDriver`] and the types in [`types`].

pub mod driver;
pub mod openai;
pub mod stream;
pub mod types;

// --- Legacy re-exports for backward compatibility ---
// The old llm.rs had these types. Keep them available until consumers migrate.
use std::pin::Pin;

use async_trait::async_trait;
pub use driver::{LlmDriver, LlmDriverRef};
use futures::Stream;
pub use openai::OpenAiDriver;
use serde::{Deserialize, Serialize};
pub use stream::StreamDelta;
pub use types::*;

pub use crate::channel::types::ToolCall;
use crate::model::ModelCapabilities;

/// Reason the model stopped generating (legacy alias).
///
/// New code should use [`StopReason`] instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FinishReason {
    Stop,
    ToolCalls,
    Length,
    ContentFilter,
}

/// A chat completion request (legacy — uses `channel::types::ChatMessage`).
///
/// New code should use [`CompletionRequest`] instead.
#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model:         String,
    pub system_prompt: String,
    pub messages:      Vec<crate::channel::types::ChatMessage>,
    pub tools:         Option<Vec<ToolDefinition>>,
    pub temperature:   Option<f32>,
}

/// A complete chat completion response (legacy).
///
/// New code should use [`CompletionResponse`] instead.
#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub content:       Option<String>,
    pub tool_calls:    Vec<ToolCall>,
    pub finish_reason: FinishReason,
    pub usage:         Option<Usage>,
}

/// A streaming delta from a chat completion (legacy).
///
/// New code should use [`StreamDelta`] instead.
#[derive(Debug, Clone)]
pub struct ChatStreamDelta {
    pub content:       Option<String>,
    pub tool_calls:    Vec<ToolCall>,
    pub finish_reason: Option<FinishReason>,
}

/// Unified LLM access (legacy trait — use [`LlmDriver`] for new code).
#[async_trait]
pub trait LlmApi: Send + Sync {
    async fn chat(&self, request: ChatRequest) -> crate::error::Result<ChatResponse>;
    async fn chat_stream(
        &self,
        request: ChatRequest,
    ) -> crate::error::Result<
        Pin<Box<dyn Stream<Item = crate::error::Result<ChatStreamDelta>> + Send>>,
    >;
    fn capabilities(&self, model: &str) -> ModelCapabilities;
}
