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

//! AI service orchestrator.
//!
//! [`AiService`] wraps a rig-core OpenAI client with prompt template
//! management and rate limiting. It is the primary entry point for the
//! rest of the application to invoke AI operations.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use jiff::Timestamp;
use rig::{client::CompletionClient, completion::Prompt, providers::openai};
use uuid::Uuid;

use crate::{
    error::AiError,
    kind::AiTaskConfig,
    template::{self, PromptTemplateManager},
};

/// Result of running an AI task, combining the model response with
/// metadata for observability.
#[derive(Debug, Clone)]
pub struct AiRunResult {
    /// Unique identifier for this run.
    pub run_id:      Uuid,
    /// The generated text content.
    pub content:     String,
    /// Model identifier that produced this response.
    pub model:       String,
    /// Wall-clock duration of the provider call in milliseconds.
    pub duration_ms: u64,
    /// Timestamp when the run started.
    pub created_at:  Timestamp,
}

/// Simple rate limiter based on a token counter.
///
/// Tracks how many tokens have been consumed and rejects requests once
/// a configurable budget is exhausted. A more sophisticated
/// implementation (sliding window, token bucket) can replace this
/// later.
#[derive(Debug)]
pub struct RateLimiter {
    /// Maximum number of tokens allowed.
    budget:   u64,
    /// Tokens consumed so far.
    consumed: AtomicU64,
}

impl RateLimiter {
    /// Create a new rate limiter with the given token budget.
    #[must_use]
    pub const fn new(budget: u64) -> Self {
        Self {
            budget,
            consumed: AtomicU64::new(0),
        }
    }

    /// Try to consume `tokens` from the budget.
    ///
    /// Returns `Ok(())` if the budget allows it, or an
    /// [`AiError::RateLimited`] if the budget is exhausted.
    pub fn try_consume(&self, tokens: u64, provider: &str) -> Result<(), AiError> {
        let prev = self.consumed.fetch_add(tokens, Ordering::Relaxed);
        if prev + tokens > self.budget {
            // Roll back the optimistic addition.
            self.consumed.fetch_sub(tokens, Ordering::Relaxed);
            return Err(AiError::RateLimited {
                provider:         provider.to_owned(),
                retry_after_secs: 60,
            });
        }
        Ok(())
    }

    /// Return the number of tokens consumed so far.
    #[must_use]
    pub fn consumed(&self) -> u64 { self.consumed.load(Ordering::Relaxed) }

    /// Reset the consumed counter to zero.
    pub fn reset(&self) { self.consumed.store(0, Ordering::Relaxed); }
}

/// The AI service orchestrator.
///
/// Wraps a rig-core OpenAI client with a [`PromptTemplateManager`]
/// for template resolution and an optional [`RateLimiter`].
pub struct AiService {
    /// The rig-core OpenAI client.
    client:           openai::Client,
    /// Default model to use when no override is specified.
    default_model:    String,
    /// Template manager for loading and rendering prompt templates.
    template_manager: Arc<dyn PromptTemplateManager>,
    /// Optional rate limiter.
    rate_limiter:     Option<RateLimiter>,
}

impl AiService {
    /// Create a new `AiService`.
    ///
    /// - `api_key` -- the OpenAI API key.
    /// - `default_model` -- model identifier to use when none is
    ///   overridden per-task.
    /// - `template_manager` -- how to load prompt templates.
    /// - `rate_limiter` -- optional rate limiter.
    #[must_use]
    pub fn new(
        api_key: &str,
        default_model: String,
        template_manager: Arc<dyn PromptTemplateManager>,
        rate_limiter: Option<RateLimiter>,
    ) -> Self {
        let client = openai::Client::builder()
            .api_key(api_key)
            .build()
            .expect("failed to build OpenAI client");

        Self {
            client,
            default_model,
            template_manager,
            rate_limiter,
        }
    }

    /// Run an AI task.
    ///
    /// 1. Load and render the prompt template (falling back to the
    ///    built-in default).
    /// 2. Build a rig agent with the resolved system prompt.
    /// 3. Apply rate limiting.
    /// 4. Call the agent and return an [`AiRunResult`].
    pub async fn run_task(&self, config: &AiTaskConfig) -> Result<AiRunResult, AiError> {
        // 1. Load / render prompt template.
        let system_prompt =
            if let Some(tpl) = self.template_manager.get_for_task_kind(config.kind).await? {
                template::render(&tpl.content, &config.variables)?
            } else {
                let default = config.kind.default_system_prompt();
                template::render(default, &config.variables)
                    .unwrap_or_else(|_| default.to_owned())
            };

        // 2. Build rig agent.
        let model = config
            .model_override
            .as_deref()
            .unwrap_or(&self.default_model);

        let mut builder = self.client.agent(model).preamble(&system_prompt);
        if let Some(temp) = config.temperature {
            builder = builder.temperature(f64::from(temp));
        }
        let agent = builder.build();

        // 3. Rate limiting (input estimation).
        let user_input = config
            .variables
            .get("user_input")
            .cloned()
            .unwrap_or_default();

        if let Some(limiter) = &self.rate_limiter {
            let estimated_tokens = (system_prompt.len() + user_input.len()) as u64 / 4;
            limiter.try_consume(estimated_tokens, model)?;
        }

        // 4. Call rig agent.
        let start = Instant::now();
        let created_at = Timestamp::now();
        let run_id = Uuid::new_v4();

        let content: String = agent
            .prompt(&user_input)
            .await
            .map_err(|e| AiError::RequestFailed {
                message: e.to_string(),
            })?;

        #[expect(clippy::cast_possible_truncation)]
        let duration_ms = start.elapsed().as_millis() as u64;

        tracing::info!(
            run_id = %run_id,
            model = model,
            duration_ms = duration_ms,
            "AI task completed"
        );

        Ok(AiRunResult {
            run_id,
            content,
            model: model.to_owned(),
            duration_ms,
            created_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limiter_allows_within_budget() {
        let limiter = RateLimiter::new(100);
        assert!(limiter.try_consume(50, "test").is_ok());
        assert!(limiter.try_consume(50, "test").is_ok());
        assert_eq!(limiter.consumed(), 100);
    }

    #[test]
    fn rate_limiter_rejects_over_budget() {
        let limiter = RateLimiter::new(100);
        assert!(limiter.try_consume(60, "test").is_ok());
        let err = limiter.try_consume(60, "test").unwrap_err();
        assert!(err.to_string().contains("Rate limited"));
    }

    #[test]
    fn rate_limiter_reset_clears_counter() {
        let limiter = RateLimiter::new(100);
        limiter.try_consume(80, "test").unwrap();
        limiter.reset();
        assert_eq!(limiter.consumed(), 0);
        assert!(limiter.try_consume(80, "test").is_ok());
    }
}
