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

//! Layer B payload sampler.
//!
//! Layer A attributes are always-on and low-cardinality (the contract). Layer
//! B attributes — prompts, completions, tool input/output, error messages —
//! carry user content and are gated by this sampler. Defaults skew safe:
//! **off on success, 100% on error**, with a hard `max_chars` truncation so
//! a runaway tool output cannot blow up trace ingest.
//!
//! Configuration lives in YAML (no Rust-side defaults — see crate guidelines):
//!
//! ```yaml
//! telemetry:
//!   payload_sampling:
//!     on_error: 1.0     # sample every error
//!     on_success: 0.0   # off in prod; 0.05 in dev
//!     max_chars: 1000   # hard cap per attribute
//! ```

use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

/// Outcome of the operation being sampled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The operation succeeded.
    Success,
    /// The operation failed.
    Error,
}

/// Configuration for the Layer B payload sampler.
///
/// All fields are required in YAML — there is no `Default` impl per the
/// rara config guideline ("no hardcoded defaults in Rust").
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct PayloadSamplingConfig {
    /// Sampling probability when [`Outcome::Error`] (range `0.0..=1.0`).
    pub on_error:   f64,
    /// Sampling probability when [`Outcome::Success`] (range `0.0..=1.0`).
    pub on_success: f64,
    /// Maximum number of bytes (UTF-8 chars) kept per sampled attribute. Any
    /// payload longer than this is truncated and the corresponding
    /// `*.truncated` attribute is set to `true`.
    pub max_chars:  usize,
}

/// Decision about whether to attach Layer B payloads to a span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SamplingDecision {
    /// Skip this span entirely — Layer B attributes MUST NOT be set.
    Skip,
    /// Record this span — Layer B attributes MAY be set, truncated to
    /// `max_chars`.
    Record {
        /// Hard cap on attribute string length.
        max_chars: usize,
    },
}

/// Layer B payload sampler.
///
/// Implements deterministic, lock-free fractional sampling using a counter
/// and the configured probabilities. Two independent counters track success
/// and error decisions so changing one rate doesn't perturb the other.
#[derive(Debug)]
pub struct PayloadSampler {
    config:        PayloadSamplingConfig,
    success_count: AtomicU64,
    error_count:   AtomicU64,
}

impl PayloadSampler {
    /// Construct a sampler from validated config.
    ///
    /// Probabilities outside `[0.0, 1.0]` are clamped to that range; this is
    /// a defense against typos like `on_error: 100` (meant `1.0`) silently
    /// causing the sampler to never fire.
    #[must_use]
    pub fn new(config: PayloadSamplingConfig) -> Self {
        let config = PayloadSamplingConfig {
            on_error:   config.on_error.clamp(0.0, 1.0),
            on_success: config.on_success.clamp(0.0, 1.0),
            max_chars:  config.max_chars,
        };
        Self {
            config,
            success_count: AtomicU64::new(0),
            error_count: AtomicU64::new(0),
        }
    }

    /// Decide whether to attach Layer B payloads for an operation with the
    /// given outcome. Pure function of the configured rate plus an internal
    /// counter — does not consult any RNG, so behaviour is reproducible
    /// under unit tests.
    #[must_use]
    pub fn decide(&self, outcome: Outcome) -> SamplingDecision {
        let (rate, counter) = match outcome {
            Outcome::Error => (self.config.on_error, &self.error_count),
            Outcome::Success => (self.config.on_success, &self.success_count),
        };

        if rate <= 0.0 {
            return SamplingDecision::Skip;
        }
        if rate >= 1.0 {
            return SamplingDecision::Record {
                max_chars: self.config.max_chars,
            };
        }

        // Counter-based fractional sampling: every Nth call fires, where
        // N = round(1 / rate). Stable, no entropy required, and trivial to
        // assert against in tests.
        let stride = (1.0 / rate).round() as u64;
        let n = counter.fetch_add(1, Ordering::Relaxed);
        if stride > 0 && n.is_multiple_of(stride) {
            SamplingDecision::Record {
                max_chars: self.config.max_chars,
            }
        } else {
            SamplingDecision::Skip
        }
    }
}

/// Truncate `s` to at most `max_chars` UTF-8 characters, returning the
/// possibly-truncated string and a flag indicating whether truncation
/// occurred. Splitting at a character boundary keeps the result valid UTF-8.
#[must_use]
pub fn truncate(s: &str, max_chars: usize) -> (String, bool) {
    if s.chars().count() <= max_chars {
        return (s.to_owned(), false);
    }
    let truncated: String = s.chars().take(max_chars).collect();
    (truncated, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(on_error: f64, on_success: f64, max_chars: usize) -> PayloadSamplingConfig {
        PayloadSamplingConfig::builder()
            .on_error(on_error)
            .on_success(on_success)
            .max_chars(max_chars)
            .build()
    }

    #[test]
    fn off_on_success_means_skip() {
        let sampler = PayloadSampler::new(cfg(1.0, 0.0, 1000));
        for _ in 0..100 {
            assert_eq!(sampler.decide(Outcome::Success), SamplingDecision::Skip);
        }
    }

    #[test]
    fn full_on_error_means_record_every_time() {
        let sampler = PayloadSampler::new(cfg(1.0, 0.0, 1000));
        for _ in 0..100 {
            assert!(matches!(
                sampler.decide(Outcome::Error),
                SamplingDecision::Record { max_chars: 1000 }
            ));
        }
    }

    #[test]
    fn fractional_rate_fires_on_expected_stride() {
        // 1 in 4 successes recorded.
        let sampler = PayloadSampler::new(cfg(0.0, 0.25, 100));
        let recorded: Vec<bool> = (0..8)
            .map(|_| {
                matches!(
                    sampler.decide(Outcome::Success),
                    SamplingDecision::Record { .. }
                )
            })
            .collect();
        // First call (n=0, n % 4 == 0) records; then every 4th.
        assert_eq!(
            recorded,
            vec![true, false, false, false, true, false, false, false]
        );
    }

    #[test]
    fn out_of_range_probabilities_are_clamped() {
        // Typo: on_error: 100 (meant 1.0) must NOT silently disable sampling.
        let sampler = PayloadSampler::new(cfg(100.0, -0.5, 100));
        assert!(matches!(
            sampler.decide(Outcome::Error),
            SamplingDecision::Record { .. }
        ));
        assert_eq!(sampler.decide(Outcome::Success), SamplingDecision::Skip);
    }

    #[test]
    fn truncate_keeps_short_strings_intact() {
        let (out, truncated) = truncate("hello", 10);
        assert_eq!(out, "hello");
        assert!(!truncated);
    }

    #[test]
    fn truncate_cuts_long_strings_at_char_boundary() {
        let (out, truncated) = truncate("hello world", 5);
        assert_eq!(out, "hello");
        assert!(truncated);
    }

    #[test]
    fn truncate_handles_multibyte_chars_safely() {
        let (out, truncated) = truncate("你好世界rara", 3);
        assert_eq!(out, "你好世");
        assert!(truncated);
        // No panic, valid UTF-8.
        let _ = out.as_bytes();
    }

    #[test]
    fn success_and_error_counters_are_independent() {
        let sampler = PayloadSampler::new(cfg(1.0, 0.5, 100));
        // Drain a few successes; errors still always fire.
        for _ in 0..5 {
            let _ = sampler.decide(Outcome::Success);
        }
        for _ in 0..3 {
            assert!(matches!(
                sampler.decide(Outcome::Error),
                SamplingDecision::Record { .. }
            ));
        }
    }
}
