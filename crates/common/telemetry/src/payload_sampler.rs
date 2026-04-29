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
//! Configuration is optional in YAML — when omitted the sampler still runs
//! with safe built-in defaults (the mechanism is default-on; suppressing
//! payloads requires an explicit `on_error: 0.0`):
//!
//! ```yaml
//! telemetry:
//!   payload_sampling:
//!     on_error: 1.0     # sample every error
//!     on_success: 1.0   # 1.0 needed to populate Langfuse Input/Output
//!     max_chars: 4000   # hard cap per attribute
//! ```

use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

/// Default per-attribute character cap when no explicit `max_chars` is given.
///
/// Tuned so trace ingest stays well under typical OTLP payload limits
/// (Langfuse ingest tolerates up to a few hundred KB per attribute, but
/// keeping the cap at 4 KB keeps the UI snappy and protects against
/// runaway tool outputs). Mechanism-level tuning lives next to the
/// mechanism per the rara `anti-patterns.md` guideline; not exposed as YAML.
pub const DEFAULT_MAX_CHARS: usize = 4_000;

/// Default sampling rate on errored operations. Errors are always
/// interesting; sample every one.
pub const DEFAULT_ON_ERROR: f64 = 1.0;

/// Default sampling rate on successful operations. The Langfuse UI needs
/// `langfuse.*.input` / `langfuse.*.output` populated to show the trace
/// content panel as non-empty (#2002), so the default-on rate is `1.0`.
/// Operators who want to dial this back set `on_success` explicitly in YAML.
pub const DEFAULT_ON_SUCCESS: f64 = 1.0;

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
/// Fields are optional in YAML; when absent the corresponding
/// `DEFAULT_ON_*` / `DEFAULT_MAX_CHARS` constant from this module is
/// used. Missing the whole `payload_sampling` block falls back to a
/// fully default-on sampler — see [`PayloadSampler::from_optional_config`].
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct PayloadSamplingConfig {
    /// Sampling probability when [`Outcome::Error`] (range `0.0..=1.0`).
    /// Defaults to [`DEFAULT_ON_ERROR`] when absent.
    #[serde(default)]
    pub on_error:   Option<f64>,
    /// Sampling probability when [`Outcome::Success`] (range `0.0..=1.0`).
    /// Defaults to [`DEFAULT_ON_SUCCESS`] when absent — the Langfuse UI
    /// needs the input / output attributes populated to render content,
    /// so the mechanism is default-on.
    #[serde(default)]
    pub on_success: Option<f64>,
    /// Maximum number of UTF-8 chars kept per sampled attribute. Any
    /// payload longer than this is truncated; the truncation marker
    /// `… [truncated]` is appended in-band so reviewers can tell the
    /// payload was cut.
    /// Defaults to [`DEFAULT_MAX_CHARS`] when absent.
    #[serde(default)]
    pub max_chars:  Option<usize>,
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
    config:        ResolvedConfig,
    success_count: AtomicU64,
    error_count:   AtomicU64,
}

/// Resolved sampler parameters with all `Option` fields filled in. Internal
/// only — the public surface remains [`PayloadSamplingConfig`] which keeps
/// fields optional so YAML can omit them.
#[derive(Debug, Clone, Copy)]
struct ResolvedConfig {
    on_error:   f64,
    on_success: f64,
    max_chars:  usize,
}

impl PayloadSampler {
    /// Construct a sampler from a config block. Missing fields fall back to
    /// the `DEFAULT_*` constants in this module.
    ///
    /// Probabilities outside `[0.0, 1.0]` are clamped to that range; this is
    /// a defense against typos like `on_error: 100` (meant `1.0`) silently
    /// causing the sampler to never fire.
    #[must_use]
    pub fn new(config: PayloadSamplingConfig) -> Self {
        let resolved = ResolvedConfig {
            on_error:   config.on_error.unwrap_or(DEFAULT_ON_ERROR).clamp(0.0, 1.0),
            on_success: config
                .on_success
                .unwrap_or(DEFAULT_ON_SUCCESS)
                .clamp(0.0, 1.0),
            max_chars:  config.max_chars.unwrap_or(DEFAULT_MAX_CHARS),
        };
        Self {
            config:        resolved,
            success_count: AtomicU64::new(0),
            error_count:   AtomicU64::new(0),
        }
    }

    /// Default-on sampler used when YAML omits the `payload_sampling` block.
    ///
    /// Mechanism-level tuning lives next to the mechanism per the rara
    /// `anti-patterns.md` guideline ("would a deploy operator have a real
    /// reason to pick a different value?" — for Langfuse trace UI population
    /// the answer is no, so the defaults are a `const`, not a YAML knob).
    #[must_use]
    pub fn from_optional_config(config: Option<PayloadSamplingConfig>) -> Self {
        Self::new(config.unwrap_or_else(|| PayloadSamplingConfig::builder().build()))
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

/// In-band truncation marker appended to a sampled payload that exceeded
/// `max_chars`. Set in-band rather than as a separate `*.truncated` boolean
/// because Langfuse does not recognize bespoke `*.truncated` keys (#2002),
/// and a reviewer reading the trace JSON or UI needs an obvious cue.
pub const TRUNCATION_MARKER: &str = " … [truncated]";

/// Truncate `s` to at most `max_chars` UTF-8 characters and, when truncation
/// occurred, append [`TRUNCATION_MARKER`]. Returned string is always valid
/// UTF-8 and at most `max_chars + TRUNCATION_MARKER.chars().count()` chars.
///
/// This is the helper agent-loop spans should call before writing
/// `langfuse.observation.input` / `langfuse.observation.output`: Langfuse
/// reads those keys verbatim, so the marker must live inside the value.
#[must_use]
pub fn truncate_with_marker(s: &str, max_chars: usize) -> String {
    let (out, truncated) = truncate(s, max_chars);
    if truncated {
        format!("{out}{TRUNCATION_MARKER}")
    } else {
        out
    }
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
    fn from_optional_config_none_falls_back_to_defaults_on() {
        // Mechanism is default-on per #2002 — `None` config must NOT mean
        // "sampler disabled". Both success and error fire on the first call.
        let sampler = PayloadSampler::from_optional_config(None);
        assert!(matches!(
            sampler.decide(Outcome::Success),
            SamplingDecision::Record { max_chars }
                if max_chars == DEFAULT_MAX_CHARS
        ));
        assert!(matches!(
            sampler.decide(Outcome::Error),
            SamplingDecision::Record { max_chars }
                if max_chars == DEFAULT_MAX_CHARS
        ));
    }

    #[test]
    fn config_omits_fields_falls_back_to_defaults() {
        // Operator wrote `payload_sampling: {}` — every field absent.
        let cfg = PayloadSamplingConfig::builder().build();
        let sampler = PayloadSampler::new(cfg);
        assert!(matches!(
            sampler.decide(Outcome::Success),
            SamplingDecision::Record { max_chars }
                if max_chars == DEFAULT_MAX_CHARS
        ));
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
    fn truncate_with_marker_appends_marker_only_when_cut() {
        // Short payload — marker MUST NOT appear, otherwise reviewers see
        // `[truncated]` on every short trace.
        let short = truncate_with_marker("hi", 10);
        assert_eq!(short, "hi");
        // Long payload — marker appended in-band so Langfuse UI shows it.
        let long = truncate_with_marker("hello world", 5);
        assert_eq!(long, format!("hello{TRUNCATION_MARKER}"));
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
