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
//! This crate encapsulates all interactions with large-language-model (LLM)
//! providers (OpenAI, Anthropic, local models, etc.).  It provides:
//!
//! - The [`AiProvider`] trait that concrete backends must implement.
//! - Prompt template management and rendering.
//! - Model routing logic (cost/latency/capability-based selection).
//! - Token budget helpers.
//!
//! No provider-specific HTTP clients live here yet -- they will be added as
//! feature-gated modules during the migration phases.

/// AI provider trait and supporting types.
pub mod provider;

pub use provider::AiProvider;
