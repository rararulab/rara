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

//! Terminal-state reconciliation for the Telegram stream forwarder.
//!
//! The stream forwarder in [`super::adapter`] renders a progress "thinking"
//! bubble while the agent turn runs. When the kernel stream closes, the bubble
//! MUST be reconciled to a terminal state regardless of whether any
//! `TextDelta` arrived — otherwise a turn that ends empty (salvage failure,
//! provider protocol error, or a kernel-side `TurnError`) leaves the user
//! staring at the thinking indicator forever.
//!
//! Production symptom this module fixes: kernel logs show
//! `turn completed reply_len=119` but the Telegram bubble stays on
//! `discombobulating…` — the forwarder saw only reasoning/tool events and
//! never a `TextDelta`, so the pre-existing close handler had nothing to
//! edit and left the bubble intact.
//!
//! This module provides a pure state-machine ([`reconcile_terminal_state`])
//! that maps the terminal state `(accumulated_text, turn_error)` to a
//! [`TerminalOutcome`] describing how the adapter should render the final
//! message. The adapter is responsible for translating the outcome into
//! Telegram API calls; keeping the decision logic pure makes it unit-testable
//! without a live bot.

use rara_kernel::agent::TurnFailureKind;

/// Adapter-local summary of a kernel `TurnError` that is safe to render into
/// a user-facing Telegram message. Mirrors [`TurnFailureKind`] plus the model
/// identifier, which the adapter already tracks via `StreamEvent::TurnMetrics`.
///
/// Kept as a distinct struct (rather than reusing
/// [`rara_kernel::agent::TurnError`]) so reconcile logic does not depend on
/// `SessionKey` or other kernel-only fields that are not available at the
/// adapter stream-close site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TurnFailureSummary {
    /// Classification of the failure (`EmptyContent`, `ProtocolError`,
    /// `EmptyTurn`), mirroring the kernel enum.
    pub kind: TurnFailureKind,
}

impl TurnFailureSummary {
    /// One-line user-facing rendering of the failure, suitable for an inline
    /// error footer appended to salvaged content or as the sole body when no
    /// content was produced.
    pub(super) fn render_line(&self) -> String {
        match &self.kind {
            TurnFailureKind::EmptyContent { reasoning_len } => {
                format!("\u{26a0}\u{fe0f} empty content from model (reasoning_len={reasoning_len})")
            }
            TurnFailureKind::ProtocolError { code, message } => {
                // Provider messages are free-form — truncate to keep the line
                // compact for the bubble.
                let short: String = message.chars().take(120).collect();
                let ellipsis = if message.chars().count() > 120 {
                    "\u{2026}"
                } else {
                    ""
                };
                format!("\u{26a0}\u{fe0f} protocol error [{code}]: {short}{ellipsis}")
            }
            TurnFailureKind::EmptyTurn => {
                "\u{26a0}\u{fe0f} model ended turn with no content and no tool calls".to_owned()
            }
        }
    }
}

/// Terminal action the stream forwarder should take when the kernel stream
/// closes. Pure data — no side effects, no Telegram API types — so the
/// decision can be unit-tested in isolation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TerminalOutcome {
    /// Render the salvaged assistant reply as the final message body.
    ///
    /// The adapter's existing flush path already handles the "happy path"
    /// (edit the current streaming message with `accumulated`), so this
    /// variant carries the text and an optional footer line the adapter
    /// appends as a small error note when a failure accompanied salvaged
    /// content.
    Content {
        body:         String,
        error_footer: Option<String>,
    },
    /// The turn produced no content and the kernel published a structured
    /// failure. The adapter should replace the progress bubble with a single
    /// user-visible error line.
    Error { line: String },
    /// The turn produced no content and no structured failure was observed.
    /// This is a kernel-side bug (the agent loop should always emit a
    /// [`rara_kernel::agent::TurnError`] for empty terminal turns), so the
    /// adapter logs at ERROR and shows a neutral "(no reply)" marker rather
    /// than leaving the bubble stuck.
    Neutral { line: String },
}

/// Neutral placeholder shown when a turn ends with no content and no
/// structured failure. Public so the adapter can pattern-match tests against
/// the literal.
pub(super) const NEUTRAL_EMPTY_MARKER: &str = "(no reply)";

/// Decide how to reconcile the progress bubble at stream close.
///
/// Inputs:
/// - `accumulated`: raw text aggregated from `StreamEvent::TextDelta`. Callers
///   should pass the same value the existing flush path uses; we treat
///   whitespace-only strings as empty.
/// - `turn_error`: structured failure summary observed for this turn, or `None`
///   if the stream closed cleanly.
///
/// Truth table:
///
/// | content | error | outcome                                 |
/// |---------|-------|-----------------------------------------|
/// | non-∅   | None  | `Content { footer: None }`        |
/// | non-∅   | Some  | `Content { footer: Some(err) }`   |
/// | ∅       | Some  | `Error { line: err }`             |
/// | ∅       | None  | `Neutral { line: "(no reply)" }`  |
///
/// User-facing content always wins over the error line: if the turn managed
/// to produce *any* assistant text we prefer delivering it and append the
/// error as a small footer, rather than hiding the salvaged reply behind an
/// error banner.
pub(super) fn reconcile_terminal_state(
    accumulated: &str,
    turn_error: Option<&TurnFailureSummary>,
) -> TerminalOutcome {
    let has_content = !accumulated.trim().is_empty();
    match (has_content, turn_error) {
        (true, None) => TerminalOutcome::Content {
            body:         accumulated.to_owned(),
            error_footer: None,
        },
        (true, Some(err)) => TerminalOutcome::Content {
            body:         accumulated.to_owned(),
            error_footer: Some(err.render_line()),
        },
        (false, Some(err)) => TerminalOutcome::Error {
            line: err.render_line(),
        },
        (false, None) => TerminalOutcome::Neutral {
            line: NEUTRAL_EMPTY_MARKER.to_owned(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err_empty(reasoning_len: usize) -> TurnFailureSummary {
        TurnFailureSummary {
            kind: TurnFailureKind::EmptyContent { reasoning_len },
        }
    }

    fn err_protocol(code: &str, message: &str) -> TurnFailureSummary {
        TurnFailureSummary {
            kind: TurnFailureKind::ProtocolError {
                code:    code.to_owned(),
                message: message.to_owned(),
            },
        }
    }

    fn err_empty_turn() -> TurnFailureSummary {
        TurnFailureSummary {
            kind: TurnFailureKind::EmptyTurn,
        }
    }

    #[test]
    fn reconcile_content_without_error_renders_content_no_footer() {
        let outcome = reconcile_terminal_state("hello world", None);
        assert_eq!(
            outcome,
            TerminalOutcome::Content {
                body:         "hello world".to_owned(),
                error_footer: None,
            }
        );
    }

    #[test]
    fn reconcile_content_with_error_prefers_content_and_adds_footer() {
        let err = err_protocol("2013", "invalid message role: system");
        let outcome = reconcile_terminal_state("partial reply", Some(&err));
        match outcome {
            TerminalOutcome::Content { body, error_footer } => {
                assert_eq!(body, "partial reply");
                let footer = error_footer.expect("footer present");
                assert!(footer.contains("2013"), "footer={footer}");
                assert!(footer.contains("invalid message role"), "footer={footer}");
            }
            other => panic!("expected Content, got {other:?}"),
        }
    }

    #[test]
    fn reconcile_empty_content_with_empty_content_error_renders_error_line() {
        let err = err_empty(1234);
        let outcome = reconcile_terminal_state("", Some(&err));
        match outcome {
            TerminalOutcome::Error { line } => {
                assert!(line.contains("reasoning_len=1234"), "line={line}");
                assert!(line.contains("empty content"), "line={line}");
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn reconcile_empty_content_with_empty_turn_error_renders_error_line() {
        let err = err_empty_turn();
        let outcome = reconcile_terminal_state("   \n\t  ", Some(&err));
        match outcome {
            TerminalOutcome::Error { line } => {
                assert!(line.contains("no content"), "line={line}");
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn reconcile_empty_content_without_error_renders_neutral_marker() {
        let outcome = reconcile_terminal_state("", None);
        assert_eq!(
            outcome,
            TerminalOutcome::Neutral {
                line: NEUTRAL_EMPTY_MARKER.to_owned(),
            }
        );
    }

    #[test]
    fn reconcile_whitespace_only_content_treated_as_empty() {
        // Salvage sometimes produces a lone newline; it should not count as
        // a user-visible reply.
        let outcome = reconcile_terminal_state("\n  \t\n", None);
        assert!(matches!(outcome, TerminalOutcome::Neutral { .. }));
    }

    #[test]
    fn render_line_truncates_long_protocol_messages() {
        let long_msg = "x".repeat(500);
        let err = err_protocol("9999", &long_msg);
        let line = err.render_line();
        // Truncated to 120 chars of content + ellipsis, plus fixed prefix.
        assert!(line.contains("\u{2026}"), "expected ellipsis in {line}");
        assert!(line.len() < 300, "line too long: {}", line.len());
    }
}
