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

//! Composable feature traits for agent execution contexts.
//!
//! Inspired by anda_core's context.rs -- decompose agent capabilities into
//! orthogonal Feature traits, then compose them via blanket-impl supertraits.

use std::sync::Arc;

use async_openai::types::chat::ChatCompletionRequestMessage;
use async_trait::async_trait;
use base::shared_string::SharedString;

use crate::{
    error,
    memory::Memory,
    prompt::PromptRepo,
    provider::LlmProviderLoaderRef,
    runner::{AgentRunner, UserContent},
    tool::ToolRegistry,
};

// ---------------------------------------------------------------------------
// Protocol types (recall engine)
// ---------------------------------------------------------------------------

/// Where to inject recalled content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectTarget {
    SystemPrompt,
    ContextMessage,
}

/// Events that trigger recall rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Compaction,
    NewSession,
    SessionResume,
}

/// Context for recall engine evaluation.
#[derive(Debug, Clone)]
pub struct RecallContext {
    pub user_text:               String,
    pub turn_count:              usize,
    pub events:                  Vec<EventKind>,
    pub elapsed_since_last_secs: u64,
    pub summary:                 Option<String>,
    pub session_topic:           Option<String>,
}

/// Recall result ready for prompt injection.
#[derive(Debug, Clone)]
pub struct InjectionPayload {
    pub rule_name: String,
    pub target:    InjectTarget,
    pub content:   String,
}

// ---------------------------------------------------------------------------
// Feature Traits
// ---------------------------------------------------------------------------

/// LLM provider access and runner construction.
#[async_trait]
pub trait CompletionFeatures: Send + Sync {
    /// Return the LLM provider loader.
    fn llm_provider(&self) -> &LlmProviderLoaderRef;

    /// Build a chat system prompt with memory + skills injection.
    async fn build_chat_system_prompt(
        &self,
        base_prompt: &str,
        user_text: &str,
        history_len: usize,
        recall_ctx: Option<&RecallContext>,
    ) -> String;

    /// Build worker policy prompt (for proactive/scheduled agents).
    async fn build_worker_policy(&self) -> String;

    /// Build an AgentRunner with current provider config.
    fn build_runner(
        &self,
        model: String,
        system_prompt: String,
        user_content: UserContent,
        chat_history: Vec<ChatCompletionRequestMessage>,
    ) -> AgentRunner;

    /// Summarize conversation history text into a compact form.
    async fn summarize_history(&self, history_text: &str, model: &str) -> error::Result<String>;
}

/// Static and dynamic tool access.
#[async_trait]
pub trait ToolFeatures: Send + Sync {
    /// Return the base (static) tool registry.
    fn tools(&self) -> &Arc<ToolRegistry>;

    /// Build effective tools (static + MCP dynamic).
    async fn build_effective_tools(&self) -> Arc<ToolRegistry>;
}

/// Prompt repository access.
pub trait PromptFeatures: Send + Sync {
    /// Return the prompt repository.
    fn prompt_repo(&self) -> &Arc<dyn PromptRepo>;
}

/// Runtime settings access.
pub trait SettingsFeatures: Send + Sync {
    /// Resolve model for a settings key (e.g. "chat", "proactive").
    fn model_for_key(&self, key: &str) -> String;

    /// Current default chat model.
    fn current_default_model(&self) -> String;

    /// Provider hint (e.g. "openrouter", "ollama").
    fn provider_hint(&self) -> Option<String>;

    /// Max agent-loop iterations.
    fn max_iterations(&self, key: &str) -> usize;

    /// Fallback models list.
    fn fallback_models(&self) -> Vec<SharedString>;

    /// Check if history needs compaction (token-based).
    fn needs_compaction(&self, history_tokens: usize, context_length: usize) -> bool;
}

/// Session/context management.
#[async_trait]
pub trait SessionFeatures: Send + Sync {
    /// Resolve current system prompt.
    async fn current_system_prompt(&self) -> String;
}

// ---------------------------------------------------------------------------
// Composed Traits
// ---------------------------------------------------------------------------

/// Base context for all agents.
pub trait BaseContext:
    CompletionFeatures + ToolFeatures + PromptFeatures + SettingsFeatures
{
}
impl<T: CompletionFeatures + ToolFeatures + PromptFeatures + SettingsFeatures> BaseContext for T {}

/// Full agent context with all features.
///
/// Provides access to the unified [`Memory`] layer via
/// [`memory()`](AgentContext::memory). Also exposes recall-engine and
/// session-consolidation operations that will eventually be replaced by direct
/// use of the [`Memory`] trait.
#[async_trait]
pub trait AgentContext: BaseContext + SessionFeatures {
    /// Return the unified memory layer (state + knowledge + learning).
    ///
    /// Returns `None` when no memory backend is configured.
    fn memory(&self) -> Option<Arc<dyn Memory>>;

    /// Run the recall engine for prompt injection.
    async fn run_recall_engine(&self, ctx: &RecallContext) -> Vec<InjectionPayload>;

    /// Fire-and-forget session consolidation into long-term memory.
    fn spawn_session_consolidation(&self, exchanges: Vec<(String, String)>);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Rough token estimate: ~3 chars per token.
pub fn estimate_tokens(text: &str) -> usize { text.chars().count().div_ceil(3) }
