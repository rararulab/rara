//! Full data-flow taint tracking for agent sessions.
//!
//! Implements a lattice-based taint propagation model.
//! Labels are attached at tool-output boundaries and checked at tool-input
//! boundaries. The LLM is treated as a mixer — its output inherits the union
//! of all input labels in the session context.

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use tracing::instrument;

use crate::session::SessionKey;

/// Classification label applied to data flowing through the system.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, strum::Display)]
pub enum TaintLabel {
    /// Data from external network requests (web_fetch, browser_*).
    ExternalNetwork,
    /// Data from direct user input.
    UserInput,
    /// Personally identifiable information.
    Pii,
    /// Secret material (API keys, tokens, passwords).
    Secret,
    /// Data produced by an untrusted / sandboxed sub-agent.
    UntrustedAgent,
}

/// A taint policy violation.
#[derive(Debug, Clone, snafu::Snafu)]
#[snafu(display("taint violation: label '{label}' from source '{source}' is not allowed to reach sink '{sink_name}'"))]
pub struct TaintViolation {
    pub label: TaintLabel,
    pub sink_name: String,
    #[snafu(source(false))]
    pub source: String,
}

/// Session-level taint state — tracks accumulated labels in LLM context.
#[derive(Debug, Default)]
struct SessionTaintState {
    /// Union of all taint labels in this session's LLM context.
    context_labels: HashSet<TaintLabel>,
}

/// Tracks taint labels across all sessions.
pub struct TaintTracker {
    sessions: DashMap<SessionKey, SessionTaintState>,
}

impl TaintTracker {
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
        }
    }

    /// Record taint labels from a tool's output into the session.
    ///
    /// Called after every successful tool execution. If the tool produces
    /// tainted data (e.g. external network content), the corresponding labels
    /// are added to the session's context — all subsequent tool calls in this
    /// session will be checked against these labels.
    #[instrument(skip(self), fields(%session, tool_name))]
    pub fn record_tool_output(&self, session: &SessionKey, tool_name: &str) {
        let labels = Self::labels_for_tool_output(tool_name);
        if labels.is_empty() {
            return;
        }
        self.sessions
            .entry(*session)
            .or_default()
            .context_labels
            .extend(labels);
    }

    /// Check whether the session's taint state allows calling this tool.
    ///
    /// Returns `Err(TaintViolation)` if any label in the session context is
    /// blocked by the target tool's sink policy. Tools without a sink policy
    /// (e.g. `file_read`) are always allowed.
    #[instrument(skip(self), fields(%session, tool_name))]
    pub fn check_tool_input(
        &self,
        session: &SessionKey,
        tool_name: &str,
    ) -> Result<(), TaintViolation> {
        let blocked = match Self::sink_for_tool(tool_name) {
            Some(b) => b,
            None => return Ok(()),
        };
        let state = match self.sessions.get(session) {
            Some(s) => s,
            None => return Ok(()),
        };
        for label in &state.context_labels {
            if blocked.contains(label) {
                return Err(TaintViolation {
                    label: label.clone(),
                    sink_name: tool_name.to_string(),
                    source: "session context".to_string(),
                });
            }
        }
        Ok(())
    }

    /// Get current taint labels for a session.
    pub fn get_labels(&self, session: &SessionKey) -> HashSet<TaintLabel> {
        self.sessions
            .get(session)
            .map(|s| s.context_labels.clone())
            .unwrap_or_default()
    }

    /// Fork taint state from parent to child session.
    ///
    /// The child inherits the full set of parent labels so that sub-agent
    /// sessions cannot bypass taint restrictions established earlier.
    #[instrument(skip(self), fields(parent = %parent, child = %child))]
    pub fn fork_session(&self, parent: &SessionKey, child: &SessionKey) {
        if let Some(parent_labels) = self
            .sessions
            .get(parent)
            .map(|parent_state| parent_state.context_labels.clone())
        {
            self.sessions.insert(
                *child,
                SessionTaintState {
                    context_labels: parent_labels,
                },
            );
        }
    }

    /// Remove taint state for a completed session.
    #[instrument(skip(self), fields(%session))]
    pub fn clear_session(&self, session: &SessionKey) {
        self.sessions.remove(session);
    }

    /// Manually inject a Secret label (for env/secret sources).
    #[instrument(skip(self), fields(%session))]
    pub fn record_secret(&self, session: &SessionKey) {
        self.sessions
            .entry(*session)
            .or_default()
            .context_labels
            .insert(TaintLabel::Secret);
    }

    /// Tool output → taint label mapping.
    ///
    /// Determines which labels a tool's output introduces into the session
    /// context. Only tools that bring external/untrusted data need labels;
    /// internal tools (file_read, search, etc.) produce no taint.
    fn labels_for_tool_output(tool_name: &str) -> HashSet<TaintLabel> {
        match tool_name {
            // Network tools fetch content from untrusted external sources.
            "web_fetch" | "browser_navigate" | "browser_snapshot" | "browser_click"
            | "browser_fill_form" | "browser_evaluate" => {
                HashSet::from([TaintLabel::ExternalNetwork])
            }
            // Sub-agent output is untrusted because we don't control its prompt.
            "agent_send" | "agent_spawn" => HashSet::from([TaintLabel::UntrustedAgent]),
            _ => HashSet::new(),
        }
    }

    /// Tool → sink policy mapping.
    ///
    /// Returns the set of taint labels that are **forbidden** from flowing
    /// into this tool. `None` means the tool has no restrictions.
    ///
    /// Policy rationale:
    /// - Shell: blocks external/untrusted/user data → prevents RCE via injection.
    /// - File write: blocks external/untrusted data → prevents disk poisoning.
    /// - Network out: blocks secrets/PII → prevents data exfiltration.
    /// - Agent messaging: blocks secrets → prevents leaks to sub-agents.
    fn sink_for_tool(tool_name: &str) -> Option<HashSet<TaintLabel>> {
        match tool_name {
            "bash" | "shell_exec" => Some(HashSet::from([
                TaintLabel::ExternalNetwork,
                TaintLabel::UntrustedAgent,
                TaintLabel::UserInput,
            ])),
            "file_write" | "file_delete" | "edit" | "write" => Some(HashSet::from([
                TaintLabel::ExternalNetwork,
                TaintLabel::UntrustedAgent,
            ])),
            "web_fetch" => Some(HashSet::from([
                TaintLabel::Secret,
                TaintLabel::Pii,
            ])),
            "agent_send" | "agent_message" => Some(HashSet::from([
                TaintLabel::Secret,
            ])),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::session::SessionKey;

    use super::*;

    #[test]
    fn tracker_clean_session_passes() {
        let tracker = TaintTracker::new();
        let sk = SessionKey::new();
        assert!(tracker.check_tool_input(&sk, "bash").is_ok());
    }

    #[test]
    fn tracker_blocks_after_web_fetch() {
        let tracker = TaintTracker::new();
        let sk = SessionKey::new();
        tracker.record_tool_output(&sk, "web_fetch");

        let result = tracker.check_tool_input(&sk, "bash");
        assert!(result.is_err());
        let violation = result.unwrap_err();
        assert_eq!(violation.label, TaintLabel::ExternalNetwork);
        assert_eq!(violation.sink_name, "bash");
    }

    #[test]
    fn tracker_web_fetch_does_not_block_file_read() {
        let tracker = TaintTracker::new();
        let sk = SessionKey::new();
        tracker.record_tool_output(&sk, "web_fetch");
        assert!(tracker.check_tool_input(&sk, "file_read").is_ok());
    }

    #[test]
    fn tracker_labels_accumulate() {
        let tracker = TaintTracker::new();
        let sk = SessionKey::new();
        tracker.record_tool_output(&sk, "web_fetch");
        tracker.record_tool_output(&sk, "agent_spawn");

        let state = tracker.get_labels(&sk);
        assert!(state.contains(&TaintLabel::ExternalNetwork));
        assert!(state.contains(&TaintLabel::UntrustedAgent));
    }

    #[test]
    fn tracker_clear_session() {
        let tracker = TaintTracker::new();
        let sk = SessionKey::new();
        tracker.record_tool_output(&sk, "web_fetch");
        assert!(tracker.check_tool_input(&sk, "bash").is_err());

        tracker.clear_session(&sk);
        assert!(tracker.check_tool_input(&sk, "bash").is_ok());
    }

    #[test]
    fn tracker_fork_inherits_parent_taint() {
        let tracker = TaintTracker::new();
        let parent = SessionKey::new();
        let child = SessionKey::new();

        tracker.record_tool_output(&parent, "web_fetch");
        tracker.fork_session(&parent, &child);
        assert!(tracker.check_tool_input(&child, "bash").is_err());
    }
}
