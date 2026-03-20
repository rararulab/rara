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

//! Pure rule-based filter for proactive signals. Zero LLM cost.
//!
//! Checks quiet hours, per-kind cooldowns, and global hourly rate limits
//! before allowing a signal through to Mita.

use std::{collections::HashMap, time::Duration};

use jiff::Timestamp;
use tracing::debug;

use super::{config::ProactiveConfig, signal::ProactiveSignal};

/// Pure rule-based filter for proactive signals.
///
/// All checks are deterministic with zero LLM cost. Signals that fail
/// any check are silently dropped.
pub struct ProactiveFilter {
    /// Filter configuration (quiet hours, cooldowns, rate limits).
    config:              ProactiveConfig,
    /// Last fire time per signal kind (for cooldown dedup).
    last_fired:          HashMap<String, Timestamp>,
    /// Number of signals passed in the current hourly window.
    hourly_count:        u32,
    /// Start of the current hourly window.
    hourly_window_start: Timestamp,
}

impl ProactiveFilter {
    /// Create a new filter from the given configuration.
    pub fn new(config: ProactiveConfig) -> Self {
        Self {
            config,
            last_fired: HashMap::new(),
            hourly_count: 0,
            hourly_window_start: Timestamp::now(),
        }
    }

    /// Check whether a signal should pass through all filter rules.
    ///
    /// Returns `true` if the signal passes quiet hours, cooldown, and
    /// rate limit checks. Note: the hourly window may be reset as a
    /// side effect. Call [`Self::record_fired`] after successfully
    /// emitting the signal.
    pub fn should_pass(&mut self, signal: &ProactiveSignal, session_key: Option<&str>) -> bool {
        let now = Timestamp::now();

        // 1. Quiet hours check.
        if self.is_quiet_hours(now) {
            debug!(
                kind = signal.kind_name(),
                "proactive filter: suppressed by quiet hours"
            );
            return false;
        }

        // 2. Per-kind cooldown dedup (session-scoped for session signals).
        let cooldown_key = signal.cooldown_key(session_key);
        if let Some(cooldown) = self.config.cooldowns.get(signal.kind_name()) {
            if let Some(last) = self.last_fired.get(&cooldown_key) {
                let elapsed_secs = now
                    .since(*last)
                    .ok()
                    .and_then(|s| s.total(jiff::Unit::Second).ok())
                    .unwrap_or(0.0);
                if Duration::from_secs_f64(elapsed_secs) < *cooldown {
                    debug!(
                        kind = signal.kind_name(),
                        cooldown_key = cooldown_key.as_str(),
                        elapsed_secs = elapsed_secs as u64,
                        cooldown_secs = cooldown.as_secs(),
                        "proactive filter: suppressed by cooldown"
                    );
                    return false;
                }
            }
        }

        // 3. Global hourly rate limit.
        self.maybe_reset_hourly_window(now);
        if self.hourly_count >= self.config.max_hourly {
            debug!(
                kind = signal.kind_name(),
                hourly_count = self.hourly_count,
                max_hourly = self.config.max_hourly,
                "proactive filter: suppressed by hourly rate limit"
            );
            return false;
        }

        true
    }

    /// Record that a signal was successfully emitted.
    ///
    /// Updates the cooldown timestamp and hourly counter.
    pub fn record_fired(&mut self, signal: &ProactiveSignal, session_key: Option<&str>) {
        let now = Timestamp::now();
        self.last_fired
            .insert(signal.cooldown_key(session_key), now);
        self.maybe_reset_hourly_window(now);
        self.hourly_count += 1;
    }

    /// Check if the current time falls within configured quiet hours.
    ///
    /// Handles midnight wrap-around: if start > end (e.g. 23:00–08:00),
    /// quiet hours span midnight.
    fn is_quiet_hours(&self, now: Timestamp) -> bool {
        let (start, end) = match (
            self.config.parsed_quiet_start(),
            self.config.parsed_quiet_end(),
        ) {
            (Some(s), Some(e)) => (s, e),
            _ => return false,
        };

        let tz = match self.config.parsed_timezone() {
            Some(tz) => tz,
            None => return false,
        };

        let local = now.to_zoned(tz);
        let current_time = local.time();

        if start <= end {
            // Normal range (e.g. 01:00–06:00).
            current_time >= start && current_time < end
        } else {
            // Midnight wrap-around (e.g. 23:00–08:00).
            current_time >= start || current_time < end
        }
    }

    /// Reset the hourly window if more than one hour has elapsed.
    fn maybe_reset_hourly_window(&mut self, now: Timestamp) {
        let elapsed_secs = now
            .since(self.hourly_window_start)
            .and_then(|s| s.total(jiff::Unit::Second))
            .unwrap_or(0.0);
        if elapsed_secs >= 3600.0 {
            self.hourly_window_start = now;
            self.hourly_count = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> ProactiveConfig {
        ProactiveConfig::builder()
            .cooldowns(HashMap::from([(
                "session_idle".to_string(),
                Duration::from_secs(3600),
            )]))
            .max_hourly(3)
            .work_hours_start("09:00".to_string())
            .work_hours_end("18:00".to_string())
            .timezone("UTC".to_string())
            .idle_threshold_secs(1800)
            .build()
    }

    #[test]
    fn pass_when_no_cooldown_hit() {
        let mut filter = ProactiveFilter::new(test_config());
        let signal = ProactiveSignal::MorningGreeting;
        assert!(filter.should_pass(&signal, None));
    }

    #[test]
    fn block_after_rate_limit() {
        let mut filter = ProactiveFilter::new(test_config());
        let signal = ProactiveSignal::MorningGreeting;

        // Exhaust the hourly limit.
        for _ in 0..3 {
            assert!(filter.should_pass(&signal, None));
            filter.record_fired(&signal, None);
        }
        // Fourth should be blocked.
        assert!(!filter.should_pass(&signal, None));
    }

    #[test]
    fn cooldown_blocks_same_session_only() {
        let mut filter = ProactiveFilter::new(test_config());
        let signal = ProactiveSignal::SessionIdle {
            idle_duration: Duration::from_secs(600),
        };

        assert!(filter.should_pass(&signal, Some("session-a")));
        filter.record_fired(&signal, Some("session-a"));

        // Same session within cooldown should be blocked.
        assert!(!filter.should_pass(&signal, Some("session-a")));

        // Different session should still pass.
        assert!(filter.should_pass(&signal, Some("session-b")));
    }
}
