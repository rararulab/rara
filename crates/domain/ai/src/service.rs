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
//! [`AiService`] ties together providers, prompt templates, and task
//! routing.  It is the primary entry point for the rest of the
//! application to invoke AI operations.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use jiff::Timestamp;
use uuid::Uuid;

use crate::{
    error::AiError,
    kind::{AiTaskConfig, AiTaskKind},
    provider::{AiModelProvider, AiProvider},
    template::{self, PromptTemplateManager},
    types::{CompletionRequest, CompletionResponse, Message},
};

/// Result of running an AI task, combining the model response with
/// metadata for cost tracking and observability.
#[derive(Debug, Clone)]
pub struct AiRunResult {
    /// Unique identifier for this run (maps to `ai_run.id` in the
    /// store).
    pub run_id:      Uuid,
    /// The completion response from the provider.
    pub response:    CompletionResponse,
    /// Which provider handled the request.
    pub provider:    AiModelProvider,
    /// Wall-clock duration of the provider call in milliseconds.
    pub duration_ms: u64,
    /// Estimated cost in cents (integer to avoid floating-point
    /// issues).
    pub cost_cents:  i32,
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
/// Combines one or more [`AiProvider`] implementations with a
/// [`PromptTemplateManager`] and routes tasks to the appropriate
/// provider based on configuration.
pub struct AiService {
    /// Registered providers keyed by their discriminant.
    providers:        HashMap<AiModelProvider, Arc<dyn AiProvider>>,
    /// Template manager for loading and rendering prompt templates.
    template_manager: Arc<dyn PromptTemplateManager>,
    /// Mapping from task kind to the provider that should handle it.
    routing:          HashMap<AiTaskKind, AiModelProvider>,
    /// Optional rate limiter.
    rate_limiter:     Option<RateLimiter>,
}

impl AiService {
    /// Create a new `AiService`.
    ///
    /// - `providers` -- the set of available providers.
    /// - `template_manager` -- how to load prompt templates.
    /// - `routing` -- maps each task kind to a provider.
    /// - `rate_limiter` -- optional rate limiter.
    #[must_use]
    pub fn new(
        providers: HashMap<AiModelProvider, Arc<dyn AiProvider>>,
        template_manager: Arc<dyn PromptTemplateManager>,
        routing: HashMap<AiTaskKind, AiModelProvider>,
        rate_limiter: Option<RateLimiter>,
    ) -> Self {
        Self {
            providers,
            template_manager,
            routing,
            rate_limiter,
        }
    }

    /// Run an AI task.
    ///
    /// 1. Resolve the provider for the given task kind via the routing table.
    /// 2. Load and render the prompt template (falling back to the built-in
    ///    default).
    /// 3. Build a [`CompletionRequest`] and send it to the provider.
    /// 4. Build an [`AiRunResult`] with timing and cost metadata.
    pub async fn run_task(&self, config: &AiTaskConfig) -> Result<AiRunResult, AiError> {
        // 1. Resolve provider.
        let provider_kind = self.routing.get(&config.kind).copied().ok_or_else(|| {
            AiError::NoProviderConfigured {
                kind: config.kind.to_string(),
            }
        })?;

        let provider =
            self.providers
                .get(&provider_kind)
                .ok_or_else(|| AiError::NoProviderConfigured {
                    kind: format!(
                        "provider {provider_kind} registered in routing but not in providers map",
                    ),
                })?;

        // 2. Load / render prompt template.
        let system_prompt =
            if let Some(tpl) = self.template_manager.get_for_task_kind(config.kind).await? {
                template::render(&tpl.content, &config.variables)?
            } else {
                // Fall back to the built-in default and still apply
                // variable substitution in case the default contains
                // placeholders.
                let default = config.kind.default_system_prompt();
                template::render(default, &config.variables).unwrap_or_else(|_| default.to_owned())
            };

        // 3. Build request.
        let model = config
            .model_override
            .as_deref()
            .unwrap_or_else(|| provider.default_model())
            .to_owned();

        let user_content = config
            .variables
            .get("user_input")
            .cloned()
            .unwrap_or_default();

        let messages = vec![Message::system(&system_prompt), Message::user(user_content)];

        let request = CompletionRequest {
            model,
            messages,
            temperature: config.temperature,
            max_tokens: config.max_tokens,
            output_schema: config.kind.default_output_schema(),
        };

        // 4. Rate limiting.
        if let Some(limiter) = &self.rate_limiter {
            // Estimate input tokens very roughly as chars / 4.
            let estimated_input: u64 = request
                .messages
                .iter()
                .map(|m| m.content.len() as u64)
                .sum::<u64>()
                / 4;
            limiter.try_consume(estimated_input, &provider_kind.to_string())?;
        }

        // 5. Call provider.
        let start = Instant::now();
        let created_at = Timestamp::now();
        let run_id = Uuid::new_v4();

        let response = provider.complete(&request).await?;
        #[expect(clippy::cast_possible_truncation)]
        let duration_ms = start.elapsed().as_millis() as u64;

        // 6. Post-call rate limiter accounting.
        if let Some(limiter) = &self.rate_limiter {
            limiter
                .try_consume(
                    u64::from(response.usage.output_tokens),
                    &provider_kind.to_string(),
                )
                .ok(); // Best-effort; don't fail the request.
        }

        // 7. Estimate cost (placeholder logic -- real pricing tables will be added
        //    later).
        let cost_cents = estimate_cost(
            provider_kind,
            response.usage.input_tokens,
            response.usage.output_tokens,
        );

        tracing::info!(
            run_id = %run_id,
            provider = %provider_kind,
            model = %response.model,
            input_tokens = response.usage.input_tokens,
            output_tokens = response.usage.output_tokens,
            duration_ms = duration_ms,
            cost_cents = cost_cents,
            "AI task completed"
        );

        Ok(AiRunResult {
            run_id,
            response,
            provider: provider_kind,
            duration_ms,
            cost_cents,
            created_at,
        })
    }

    /// Check health of all registered providers.
    pub async fn check_health(&self) -> HashMap<AiModelProvider, Result<(), AiError>> {
        let mut results = HashMap::new();
        for (&kind, provider) in &self.providers {
            results.insert(kind, provider.check_health().await);
        }
        results
    }
}

/// Rough cost estimation in cents.
///
/// This uses very approximate per-token pricing. A proper
/// implementation should maintain a pricing table per model.
fn estimate_cost(provider: AiModelProvider, input_tokens: u32, output_tokens: u32) -> i32 {
    // Prices in cents per 1 000 tokens (very rough approximations).
    let (input_rate, output_rate): (f64, f64) = match provider {
        // GPT-4o: ~$2.50 / 1M input, ~$10 / 1M output
        AiModelProvider::Openai => (0.00025, 0.001),
        // Claude Sonnet: ~$3 / 1M input, ~$15 / 1M output
        AiModelProvider::Anthropic => (0.0003, 0.0015),
        // Local models are free.
        AiModelProvider::Local | AiModelProvider::Other => (0.0, 0.0),
    };

    let cost = f64::from(input_tokens).mul_add(input_rate, f64::from(output_tokens) * output_rate);

    // Convert to integer cents, rounding up.
    #[expect(clippy::cast_possible_truncation)]
    let cents = cost.ceil() as i32;
    cents
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

    #[test]
    fn estimate_cost_local_is_free() {
        assert_eq!(estimate_cost(AiModelProvider::Local, 1000, 1000), 0);
    }
}
