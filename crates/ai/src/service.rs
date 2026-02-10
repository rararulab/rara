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

//! AI service — a factory for creating task-specific agents.
//!
//! [`AiService`] holds the rig-core OpenAI client and default model.
//! Each agent method returns a lightweight borrowing agent that
//! executes a specific AI task.

use std::sync::atomic::{AtomicU64, Ordering};

use rig::providers::openai;

use crate::{
    agents::{
        cover_letter::CoverLetterAgent,
        follow_up::FollowUpDraftAgent,
        interview_prep::InterviewPrepAgent,
        job_fit::JobFitAgent,
        resume_optimizer::ResumeOptimizerAgent,
    },
    error::AiError,
};

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

/// The AI service — a factory for creating task-specific agents.
///
/// Holds the rig-core OpenAI client and default model. Each agent
/// method returns a lightweight borrowing agent that executes a
/// specific AI task.
pub struct AiService {
    /// The rig-core OpenAI client.
    client:        openai::Client,
    /// Default model to use.
    default_model: String,
    /// Optional rate limiter.
    rate_limiter:  Option<RateLimiter>,
}

impl AiService {
    /// Create a new `AiService`.
    ///
    /// - `api_key` -- the OpenAI API key.
    /// - `default_model` -- model identifier to use by default.
    /// - `rate_limiter` -- optional rate limiter.
    #[must_use]
    pub fn new(api_key: &str, default_model: String, rate_limiter: Option<RateLimiter>) -> Self {
        let client = openai::Client::builder()
            .api_key(api_key)
            .build()
            .expect("failed to build OpenAI client");

        Self {
            client,
            default_model,
            rate_limiter,
        }
    }

    /// Create a job-fit analysis agent.
    pub fn job_fit(&self) -> JobFitAgent<'_> {
        JobFitAgent::new(&self.client, &self.default_model)
    }

    /// Create a resume optimization agent.
    pub fn resume_optimizer(&self) -> ResumeOptimizerAgent<'_> {
        ResumeOptimizerAgent::new(&self.client, &self.default_model)
    }

    /// Create an interview preparation agent.
    pub fn interview_prep(&self) -> InterviewPrepAgent<'_> {
        InterviewPrepAgent::new(&self.client, &self.default_model)
    }

    /// Create a follow-up email drafting agent.
    pub fn follow_up(&self) -> FollowUpDraftAgent<'_> {
        FollowUpDraftAgent::new(&self.client, &self.default_model)
    }

    /// Create a cover letter generation agent.
    pub fn cover_letter(&self) -> CoverLetterAgent<'_> {
        CoverLetterAgent::new(&self.client, &self.default_model)
    }

    /// Access the rate limiter, if configured.
    pub fn rate_limiter(&self) -> Option<&RateLimiter> {
        self.rate_limiter.as_ref()
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
