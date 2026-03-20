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
//! - [`StreamDelta`] — streaming event types (including `ReasoningDelta`)
//! - [`Message`] — conversation message type
//! - [`OpenAiDriver`] — reqwest-based OpenAI-compatible driver with SSE parsing

pub mod driver;
pub mod image;
pub mod openai;
pub mod registry;
pub mod stream;
pub mod types;

pub use driver::{
    LlmDriver, LlmDriverRef, LlmEmbedder, LlmEmbedderRef, LlmModelLister, LlmModelListerRef,
};
pub use openai::OpenAiDriver;
pub use registry::{DriverRegistry, DriverRegistryRef, ProviderModelConfig};
pub use stream::StreamDelta;
pub use types::*;
