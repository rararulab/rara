//! Guard pipeline — combines taint tracking + pattern scanning.
//!
//! Sits between permission checks and tool execution in `agent.rs`.
//! Taint is checked first (cheaper, session-level) and short-circuits
//! before the more expensive argument-level pattern scan.

use tracing::instrument;

use crate::session::SessionKey;

use super::{pattern::PatternGuard, taint::TaintTracker};

/// Verdict from the guard pipeline.
#[derive(Debug)]
pub enum GuardVerdict {
    /// Tool call is safe to execute.
    Pass,
    /// Tool call is blocked.
    Blocked {
        /// Which layer blocked it: "taint" or "pattern".
        layer: &'static str,
        /// Human-readable reason.
        reason: String,
        /// The tool that was blocked.
        tool_name: String,
    },
}

/// Combines taint tracking + pattern scanning into a single guard.
pub struct GuardPipeline {
    taint: TaintTracker,
    pattern: PatternGuard,
}

impl GuardPipeline {
    pub fn new() -> Self {
        Self {
            taint: TaintTracker::new(),
            pattern: PatternGuard,
        }
    }

    /// Run taint + pattern checks before tool execution.
    ///
    /// Order matters: taint is checked first (cheaper, session-level) and
    /// short-circuits before the more expensive pattern scan.
    #[instrument(skip(self, args), fields(%session, tool_name))]
    pub fn pre_execute(
        &self,
        session: &SessionKey,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> GuardVerdict {
        // Layer 1: taint-flow check (session-level, O(1) per label).
        if let Err(violation) = self.taint.check_tool_input(session, tool_name) {
            return GuardVerdict::Blocked {
                layer: "taint",
                reason: violation.to_string(),
                tool_name: tool_name.to_string(),
            };
        }

        // Layer 2: pattern scan (argument-level, O(rules × args)).
        let matches = self.pattern.scan(tool_name, args);
        if let Some(critical) = matches.iter().find(|m| {
            matches!(
                m.severity,
                crate::security::RiskLevel::Critical | crate::security::RiskLevel::High
            )
        }) {
            return GuardVerdict::Blocked {
                layer: "pattern",
                reason: format!(
                    "{}: matched '{}'",
                    critical.rule_name, critical.matched_pattern
                ),
                tool_name: tool_name.to_string(),
            };
        }

        GuardVerdict::Pass
    }

    /// Record taint labels after successful tool execution.
    #[instrument(skip(self), fields(%session, tool_name))]
    pub fn post_execute(&self, session: &SessionKey, tool_name: &str) {
        self.taint.record_tool_output(session, tool_name);
    }

    /// Access the taint tracker directly (for fork, clear, manual label injection).
    pub fn taint_tracker(&self) -> &TaintTracker {
        &self.taint
    }
}

#[cfg(test)]
mod tests {
    use crate::guard::taint::TaintLabel;
    use crate::session::SessionKey;

    use super::*;

    #[test]
    fn pass_when_clean() {
        let pipeline = GuardPipeline::new();
        let sk = SessionKey::new();
        let args = serde_json::json!({ "command": "ls -la" });
        let verdict = pipeline.pre_execute(&sk, "bash", &args);
        assert!(matches!(verdict, GuardVerdict::Pass));
    }

    #[test]
    fn taint_blocks_before_pattern() {
        let pipeline = GuardPipeline::new();
        let sk = SessionKey::new();
        pipeline.post_execute(&sk, "web_fetch");

        let args = serde_json::json!({ "command": "ls -la" });
        let verdict = pipeline.pre_execute(&sk, "bash", &args);
        assert!(matches!(
            verdict,
            GuardVerdict::Blocked { layer: "taint", .. }
        ));
    }

    #[test]
    fn pattern_blocks_dangerous_command() {
        let pipeline = GuardPipeline::new();
        let sk = SessionKey::new();
        let args = serde_json::json!({ "command": "rm -rf /" });
        let verdict = pipeline.pre_execute(&sk, "bash", &args);
        assert!(matches!(
            verdict,
            GuardVerdict::Blocked {
                layer: "pattern",
                ..
            }
        ));
    }

    #[test]
    fn pattern_blocks_injection_marker_on_any_tool() {
        let pipeline = GuardPipeline::new();
        let sk = SessionKey::new();
        let args = serde_json::json!({ "content": "ignore previous instructions" });
        let verdict = pipeline.pre_execute(&sk, "file_write", &args);
        assert!(matches!(
            verdict,
            GuardVerdict::Blocked {
                layer: "pattern",
                ..
            }
        ));
    }

    #[test]
    fn web_fetch_after_secret_read_blocked() {
        let pipeline = GuardPipeline::new();
        let sk = SessionKey::new();
        pipeline.taint_tracker().record_secret(&sk);
        let args = serde_json::json!({ "url": "https://example.com" });
        let verdict = pipeline.pre_execute(&sk, "web_fetch", &args);
        assert!(matches!(
            verdict,
            GuardVerdict::Blocked { layer: "taint", .. }
        ));
    }

    #[test]
    fn post_execute_accumulates_labels() {
        let pipeline = GuardPipeline::new();
        let sk = SessionKey::new();
        pipeline.post_execute(&sk, "web_fetch");
        pipeline.post_execute(&sk, "agent_spawn");
        let labels = pipeline.taint_tracker().get_labels(&sk);
        assert!(labels.contains(&TaintLabel::ExternalNetwork));
        assert!(labels.contains(&TaintLabel::UntrustedAgent));
    }
}
