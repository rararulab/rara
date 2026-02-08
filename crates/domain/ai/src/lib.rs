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

//! # job-domain-ai
//!
//! AI provider abstraction for the Job Automation platform.
//!
//! This crate encapsulates all interactions with large-language-model
//! (LLM) providers (OpenAI, Anthropic, local models, etc.).  It
//! provides:
//!
//! - The [`AiProvider`] trait that concrete backends must implement.
//! - Typed request/response types ([`CompletionRequest`],
//!   [`CompletionResponse`]).
//! - Error types via [`AiError`].
//! - AI task kinds ([`AiTaskKind`]) with default prompts and output schemas.
//! - Prompt template management and rendering.
//! - Provider stubs for OpenAI and Anthropic.
//! - An [`AiService`] orchestrator that routes tasks to the appropriate
//!   provider.

/// Error types for the AI domain.
pub mod error;
/// AI task kinds and their default configurations.
pub mod kind;
/// AI provider trait and provider discriminant.
pub mod provider;
/// Concrete provider implementations (OpenAI, Anthropic).
pub mod providers;
/// AI service orchestrator.
pub mod service;
/// Prompt template management and rendering.
pub mod template;
/// Core request/response types.
pub mod types;

// Re-exports for convenience.
pub use error::AiError;
pub use kind::{AiTaskConfig, AiTaskKind};
pub use provider::{AiModelProvider, AiProvider};
pub use service::{AiRunResult, AiService, RateLimiter};
pub use template::{InMemoryTemplateManager, PromptTemplateManager};
pub use types::{
    CompletionRequest, CompletionResponse, FinishReason, Message, MessageRole, TokenUsage,
};
