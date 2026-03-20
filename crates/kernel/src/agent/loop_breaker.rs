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

//! Detects and breaks tool-call loops in agent execution.
//!
//! While [`super::repetition::RepetitionGuard`] handles text-level repetition,
//! this module detects repetitive **tool call** patterns:
//!
//! - **Exact duplicates**: the same tool + args called N times in a row
//! - **Ping-pong**: alternating A-B-A-B tool calls
//! - **Same-tool flooding**: one tool called far too many times overall

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use xxhash_rust::xxh3::xxh3_64;

/// Maximum number of recent fingerprints retained for pattern detection.
const MAX_RECENT_FINGERPRINTS: usize = 20;

/// Configuration for tool-call loop detection thresholds.
#[derive(Debug, Clone, bon::Builder)]
pub(crate) struct LoopBreakerConfig {
    /// Issue a warning after this many calls to the same tool.
    #[builder(default = 5)]
    pub warn_after:          usize,
    /// Disable a tool after this many calls.
    #[builder(default = 8)]
    pub disable_after:       usize,
    /// Consecutive identical (tool+args) calls that trigger immediate disable.
    #[builder(default = 3)]
    pub exact_dup_threshold: usize,
    /// Number of A-B alternation cycles to detect ping-pong (each cycle = 2
    /// calls).
    #[builder(default = 4)]
    pub pingpong_cycles:     usize,
}

/// Intervention the caller should apply after [`ToolCallLoopBreaker::check`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LoopIntervention {
    /// No action needed.
    None,
    /// Warn the LLM to change strategy.
    Warn {
        /// Detection pattern that triggered: `"flooding"`.
        pattern: &'static str,
        /// Human-readable warning injected into the conversation.
        message: String,
    },
    /// Disable specific tools and inform the LLM.
    DisableTools {
        /// Detection pattern: `"exact_duplicate"`, `"flooding"`, or
        /// `"pingpong"`.
        pattern: &'static str,
        /// Tool names to disable.
        tools:   Vec<String>,
        /// Human-readable explanation injected into the conversation.
        message: String,
    },
}

/// Tracks tool-call patterns and detects loops.
///
/// Create via [`ToolCallLoopBreaker::new`] with a [`LoopBreakerConfig`], then
/// call [`record`](Self::record) for every tool invocation and
/// [`check`](Self::check) to obtain any necessary intervention.
#[derive(Debug)]
pub(crate) struct ToolCallLoopBreaker {
    /// Per-tool invocation counts (BTreeMap for deterministic iteration order).
    tool_counts:         BTreeMap<String, usize>,
    /// Fingerprint of the most recent tool call.
    last_fingerprint:    Option<u64>,
    /// How many consecutive calls had the exact same fingerprint.
    consecutive_exact:   usize,
    /// Sliding window of recent call fingerprints (newest at back).
    recent_fingerprints: VecDeque<u64>,
    /// Tools that have already received a warning (at most once each).
    warned_tools:        HashSet<String>,
    /// Tools that have been disabled (at most once each).
    disabled_tools:      HashSet<String>,
    /// Detection thresholds.
    config:              LoopBreakerConfig,
    /// Name of the most recently recorded tool.
    last_tool_name:      Option<String>,
    /// Maps fingerprint → tool name for reverse lookups in pattern detection.
    fp_to_name:          HashMap<u64, String>,
}

impl ToolCallLoopBreaker {
    /// Create a new breaker with the given thresholds.
    pub(crate) fn new(config: LoopBreakerConfig) -> Self {
        Self {
            tool_counts: BTreeMap::new(),
            last_fingerprint: None,
            consecutive_exact: 0,
            recent_fingerprints: VecDeque::with_capacity(MAX_RECENT_FINGERPRINTS),
            warned_tools: HashSet::new(),
            disabled_tools: HashSet::new(),
            config,
            last_tool_name: None,
            fp_to_name: HashMap::new(),
        }
    }

    /// Record a tool invocation.
    ///
    /// **Calling convention**: call `record()` for each tool call in the
    /// iteration, then call [`check`](Self::check) exactly once.  Do NOT
    /// call `check` from within `record` — `check` reads state that
    /// `record` mutates (e.g. `recent_fingerprints` for ping-pong).
    pub(crate) fn record(&mut self, tool_name: &str, args: &str) {
        // Increment per-tool counter
        *self.tool_counts.entry(tool_name.to_owned()).or_insert(0) += 1;

        let fp = fingerprint(tool_name, args);

        // Track consecutive exact duplicates
        if self.last_fingerprint == Some(fp) {
            self.consecutive_exact += 1;
        } else {
            self.consecutive_exact = 1;
        }
        self.last_fingerprint = Some(fp);

        // Maintain sliding window; evict stale fp_to_name entries to bound memory.
        if self.recent_fingerprints.len() >= MAX_RECENT_FINGERPRINTS {
            if let Some(evicted) = self.recent_fingerprints.pop_front() {
                // Only remove if no other slot still references this fingerprint.
                if !self.recent_fingerprints.contains(&evicted) {
                    self.fp_to_name.remove(&evicted);
                }
            }
        }
        self.recent_fingerprints.push_back(fp);

        self.last_tool_name = Some(tool_name.to_owned());
        self.fp_to_name.insert(fp, tool_name.to_owned());
    }

    /// Evaluate recorded history and return an intervention if a loop is
    /// detected.
    ///
    /// Priority order: exact duplicate > ping-pong > same-tool flooding.
    pub(crate) fn check(&mut self) -> LoopIntervention {
        // --- 1. Exact duplicate detection (highest priority) ---
        if self.consecutive_exact >= self.config.exact_dup_threshold {
            if let Some(ref name) = self.last_tool_name {
                if !self.disabled_tools.contains(name) {
                    self.disabled_tools.insert(name.clone());
                    return LoopIntervention::DisableTools {
                        pattern: "exact_duplicate",
                        tools:   vec![name.clone()],
                        message: format!(
                            "Tool `{}` has been called {} times in a row with identical \
                             arguments. It is now disabled. Please use a different approach.",
                            name, self.consecutive_exact,
                        ),
                    };
                }
            }
        }

        // --- 2. Ping-pong detection ---
        let required_len = self.config.pingpong_cycles * 2;
        if self.recent_fingerprints.len() >= required_len {
            let tail: Vec<u64> = self
                .recent_fingerprints
                .iter()
                .rev()
                .take(required_len)
                .copied()
                .collect();

            // Check if tail alternates between exactly two distinct fingerprints
            let a = tail[0];
            let b = tail[1];
            if a != b
                && tail
                    .iter()
                    .enumerate()
                    .all(|(i, &fp)| fp == if i % 2 == 0 { a } else { b })
            {
                let name_a = self.fp_to_name.get(&a).cloned().unwrap_or_default();
                let name_b = self.fp_to_name.get(&b).cloned().unwrap_or_default();

                // Fire if at least one tool is not yet disabled.  Re-inserting an
                // already-disabled tool into the set is a no-op (HashSet idempotent).
                if !self.disabled_tools.contains(&name_a) || !self.disabled_tools.contains(&name_b)
                {
                    self.disabled_tools.insert(name_a.clone());
                    self.disabled_tools.insert(name_b.clone());
                    return LoopIntervention::DisableTools {
                        pattern: "pingpong",
                        tools:   vec![name_a.clone(), name_b.clone()],
                        message: format!(
                            "Ping-pong loop detected: `{}` and `{}` have been alternating for {} \
                             cycles. Both are now disabled. Please try a fundamentally different \
                             strategy.",
                            name_a, name_b, self.config.pingpong_cycles,
                        ),
                    };
                }
            }
        }

        // --- 3. Same-tool flooding ---
        // Check ALL tools, not just the last one, because multiple tool calls
        // may be recorded in a single iteration (parallel tool calls).
        for (name, &count) in &self.tool_counts {
            if self.disabled_tools.contains(name) {
                continue;
            }

            if count >= self.config.disable_after {
                self.disabled_tools.insert(name.clone());
                return LoopIntervention::DisableTools {
                    pattern: "flooding",
                    tools:   vec![name.clone()],
                    message: format!(
                        "Tool `{name}` has been called {count} times this turn. It is now \
                         disabled. Please adopt a different approach.",
                    ),
                };
            }

            if count >= self.config.warn_after && !self.warned_tools.contains(name) {
                self.warned_tools.insert(name.clone());
                return LoopIntervention::Warn {
                    pattern: "flooding",
                    message: format!(
                        "Tool `{name}` has been called {count} times. Consider whether you are \
                         making progress or stuck in a loop. Try a different approach if needed.",
                    ),
                };
            }
        }

        LoopIntervention::None
    }
}

/// Compute a fast, deterministic fingerprint for a (tool_name, args) pair.
///
/// Uses xxh3-64 with a `0xFF` separator to avoid collisions between
/// `("ab", "cd")` and `("abc", "d")`.
fn fingerprint(tool_name: &str, args: &str) -> u64 {
    let mut buf = Vec::with_capacity(tool_name.len() + 1 + args.len());
    buf.extend_from_slice(tool_name.as_bytes());
    buf.push(0xFF);
    buf.extend_from_slice(args.as_bytes());
    xxh3_64(&buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> LoopBreakerConfig { LoopBreakerConfig::builder().build() }

    // ---- Config defaults ----

    #[test]
    fn config_defaults() {
        let cfg = default_config();
        assert_eq!(cfg.warn_after, 5);
        assert_eq!(cfg.disable_after, 8);
        assert_eq!(cfg.exact_dup_threshold, 3);
        assert_eq!(cfg.pingpong_cycles, 4);
    }

    // ---- Fingerprinting ----

    #[test]
    fn fingerprint_deterministic() {
        let a = fingerprint("read", r#"{"path":"/tmp/x"}"#);
        let b = fingerprint("read", r#"{"path":"/tmp/x"}"#);
        assert_eq!(a, b);
    }

    #[test]
    fn fingerprint_differentiates_tool_name() {
        let a = fingerprint("read", "{}");
        let b = fingerprint("write", "{}");
        assert_ne!(a, b);
    }

    #[test]
    fn fingerprint_differentiates_args() {
        let a = fingerprint("read", r#"{"path":"a"}"#);
        let b = fingerprint("read", r#"{"path":"b"}"#);
        assert_ne!(a, b);
    }

    #[test]
    fn fingerprint_no_separator_collision() {
        // "ab" + 0xFF + "cd" vs "abc" + 0xFF + "d"
        let a = fingerprint("ab", "cd");
        let b = fingerprint("abc", "d");
        assert_ne!(a, b);
    }

    // ---- Record ----

    #[test]
    fn record_increments_counts() {
        let mut lb = ToolCallLoopBreaker::new(default_config());
        lb.record("read", "{}");
        lb.record("read", "{}");
        lb.record("write", "{}");
        assert_eq!(lb.tool_counts["read"], 2);
        assert_eq!(lb.tool_counts["write"], 1);
    }

    #[test]
    fn record_tracks_consecutive_exact() {
        let mut lb = ToolCallLoopBreaker::new(default_config());
        lb.record("read", "{}");
        assert_eq!(lb.consecutive_exact, 1);
        lb.record("read", "{}");
        assert_eq!(lb.consecutive_exact, 2);
        lb.record("read", r#"{"different": true}"#);
        assert_eq!(lb.consecutive_exact, 1); // reset on different args
    }

    // ---- Warn at threshold ----

    #[test]
    fn warn_at_threshold() {
        let mut lb = ToolCallLoopBreaker::new(default_config());
        // Call same tool with different args to avoid exact-dup trigger
        for i in 0..5 {
            lb.record("read", &format!("{{{}}}", i));
            let intervention = lb.check();
            if i < 4 {
                assert_eq!(
                    intervention,
                    LoopIntervention::None,
                    "should not warn at call {}",
                    i + 1
                );
            } else {
                assert!(
                    matches!(intervention, LoopIntervention::Warn { .. }),
                    "should warn at call 5"
                );
            }
        }
    }

    // ---- Disable at threshold ----

    #[test]
    fn disable_at_threshold() {
        let mut lb = ToolCallLoopBreaker::new(default_config());
        let mut last = LoopIntervention::None;
        for i in 0..8 {
            lb.record("read", &format!("{{{}}}", i));
            last = lb.check();
        }
        assert!(
            matches!(last, LoopIntervention::DisableTools { .. }),
            "should disable at call 8, got {:?}",
            last,
        );
    }

    // ---- Exact duplicate fires before flooding ----

    #[test]
    fn exact_dup_fires_before_flooding() {
        let cfg = LoopBreakerConfig::builder()
            .exact_dup_threshold(3)
            .warn_after(10)
            .disable_after(20)
            .build();
        let mut lb = ToolCallLoopBreaker::new(cfg);

        // 3 identical calls — should trigger exact dup, not wait for flooding
        for _ in 0..3 {
            lb.record("read", "{}");
        }
        let intervention = lb.check();
        match intervention {
            LoopIntervention::DisableTools { tools, .. } => {
                assert_eq!(tools, vec!["read"]);
            }
            other => panic!("expected DisableTools, got {:?}", other),
        }
    }

    // ---- Ping-pong detection ----

    #[test]
    fn pingpong_detection() {
        let cfg = LoopBreakerConfig::builder()
            .pingpong_cycles(4)
            .warn_after(100) // high so flooding doesn't interfere
            .disable_after(200)
            .exact_dup_threshold(100)
            .build();
        let mut lb = ToolCallLoopBreaker::new(cfg);

        // 4 cycles of A-B = 8 calls
        for _ in 0..4 {
            lb.record("read", r#"{"path":"a"}"#);
            lb.record("write", r#"{"path":"a"}"#);
        }
        let intervention = lb.check();
        match intervention {
            LoopIntervention::DisableTools {
                pattern,
                tools,
                message,
            } => {
                assert_eq!(pattern, "pingpong");
                assert!(tools.contains(&"read".to_owned()) || tools.contains(&"write".to_owned()));
                assert_eq!(tools.len(), 2);
                assert!(
                    message.contains("Ping-pong"),
                    "message should mention ping-pong"
                );
            }
            other => panic!("expected DisableTools for ping-pong, got {:?}", other),
        }
    }

    // ---- No false positive ----

    #[test]
    fn no_false_positive_mixed_tools() {
        let mut lb = ToolCallLoopBreaker::new(default_config());
        // Varied tools, each called only a few times
        for i in 0..4 {
            lb.record("read", &format!("{{{}}}", i));
            lb.record("write", &format!("{{{}}}", i));
            lb.record("list", &format!("{{{}}}", i));
        }
        // 4 calls per tool, under warn_after=5
        let intervention = lb.check();
        assert_eq!(intervention, LoopIntervention::None);
    }

    // ---- Warn fires only once per tool ----

    #[test]
    fn warn_only_once_per_tool() {
        let mut lb = ToolCallLoopBreaker::new(default_config());
        // Reach warn threshold
        for i in 0..5 {
            lb.record("read", &format!("{{{}}}", i));
        }
        let first = lb.check();
        assert!(matches!(first, LoopIntervention::Warn { .. }));

        // One more call — should NOT warn again
        lb.record("read", "{5}");
        let second = lb.check();
        assert_eq!(second, LoopIntervention::None);

        // One more — still no second warning
        lb.record("read", "{6}");
        let third = lb.check();
        assert_eq!(third, LoopIntervention::None);
    }

    // ---- Disable fires only once per tool ----

    #[test]
    fn disable_only_once_per_tool() {
        let mut lb = ToolCallLoopBreaker::new(default_config());
        let mut last = LoopIntervention::None;
        for i in 0..8 {
            lb.record("read", &format!("{{{}}}", i));
            last = lb.check();
        }
        // The 8th call should have triggered disable
        assert!(
            matches!(last, LoopIntervention::DisableTools { .. }),
            "expected DisableTools, got {:?}",
            last,
        );

        // Additional call — should NOT disable again
        lb.record("read", "{8}");
        let second = lb.check();
        assert_eq!(second, LoopIntervention::None);
    }
}
