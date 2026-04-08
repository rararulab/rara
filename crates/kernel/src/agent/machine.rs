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

//! Pure (sans-IO) agent turn-loop state machine.
//!
//! [`AgentMachine`] models the high-level spine of `agent::run_agent_loop`:
//!
//! ```text
//!   TurnStarted ──▶ AwaitingLlm ──┬─▶ LlmCompleted{tool_calls=∅} ──▶ Finish(Stopped)
//!                                 ├─▶ LlmCompleted{tool_calls=≠∅} ──▶ ExecutingTools
//!                                 │                                   │
//!                                 │                          ToolsCompleted
//!                                 │                                   │
//!                                 │                                   ▼
//!                                 │                              AwaitingLlm
//!                                 ├─▶ LlmFailed{retryable=true,recoveries<MAX} ──▶ AwaitingLlm
//!                                 ├─▶ LlmFailed{retryable=false}             ──▶ Fail
//!                                 ├─▶ GuardRejected                          ──▶ Fail
//!                                 └─▶ Interrupted                            ──▶ Fail
//! ```
//!
//! The machine is **pure**: every transition is a synchronous function from
//! `(state, event)` to `(new state, Vec<Effect>)`. No I/O, no `.await`, no
//! globals.  This is what makes the unit tests at the bottom of this file
//! possible without mocking five subsystems.
//!
//! The runner ([`crate::agent::runner`]) is the async layer that interprets
//! the [`Effect`]s against real subsystems and feeds the outcomes back as
//! [`Event`]s.

use crate::agent::effect::{Effect, FinishReason, TapeAppendKind, ToolCall, ToolResult};

/// High-level phases of one agent turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Initial state; no LLM call has been issued yet for this turn.
    Idle,
    /// An LLM streaming call is in flight.
    AwaitingLlm,
    /// LLM responded with tool calls; runner is dispatching them.
    ExecutingTools,
    /// Terminal: turn finished successfully.
    Done,
    /// Terminal: turn failed.
    Failed,
}

/// Maximum number of LLM retry attempts (mirrors the legacy
/// `MAX_LLM_ERROR_RECOVERIES` constant in `agent::mod`).
pub const MAX_LLM_RECOVERIES: u32 = 3;

/// Mutable state carried across machine transitions for one turn.
#[derive(Debug, Clone)]
pub struct AgentMachine {
    phase:               Phase,
    iteration:           usize,
    max_iterations:      usize,
    tool_calls_made:     usize,
    last_assistant_text: String,
    tools_enabled:       bool,
    llm_recoveries:      u32,
}

impl AgentMachine {
    /// Construct a fresh machine with the configured iteration ceiling.
    pub fn new(max_iterations: usize) -> Self {
        Self {
            phase: Phase::Idle,
            iteration: 0,
            max_iterations,
            tool_calls_made: 0,
            last_assistant_text: String::new(),
            tools_enabled: true,
            llm_recoveries: 0,
        }
    }

    /// Current high-level phase.
    pub fn phase(&self) -> Phase { self.phase }

    /// Iteration counter (0-based).
    pub fn iteration(&self) -> usize { self.iteration }

    /// Cumulative tool calls executed in the turn so far.
    pub fn tool_calls_made(&self) -> usize { self.tool_calls_made }

    /// Whether the machine has reached a terminal state.
    pub fn is_terminal(&self) -> bool { matches!(self.phase, Phase::Done | Phase::Failed) }

    /// Drive the machine with one event.  Returns the side effects the runner
    /// must perform before feeding the next event back in.
    ///
    /// Calling `step` after the machine has reached [`Phase::Done`] or
    /// [`Phase::Failed`] is a logic error and produces no effects.
    pub fn step(&mut self, event: Event) -> Vec<Effect> {
        match (self.phase, event) {
            // ── Turn boot ────────────────────────────────────────────────
            (Phase::Idle, Event::TurnStarted) => {
                self.phase = Phase::AwaitingLlm;
                vec![Effect::CallLlm {
                    iteration:     self.iteration,
                    tools_enabled: self.tools_enabled,
                }]
            }

            // ── LLM produced a terminal response ─────────────────────────
            (
                Phase::AwaitingLlm,
                Event::LlmCompleted {
                    text,
                    tool_calls,
                    has_tool_calls,
                },
            ) if !has_tool_calls => {
                self.last_assistant_text = text.clone();
                debug_assert!(tool_calls.is_empty());
                self.phase = Phase::Done;
                vec![
                    Effect::AppendTape {
                        kind: TapeAppendKind::AssistantFinal,
                    },
                    Effect::Finish {
                        text,
                        iterations: self.iteration + 1,
                        tool_calls: self.tool_calls_made,
                        reason: FinishReason::Stopped,
                    },
                ]
            }

            // ── LLM produced tool calls ──────────────────────────────────
            (
                Phase::AwaitingLlm,
                Event::LlmCompleted {
                    text,
                    tool_calls,
                    has_tool_calls: true,
                },
            ) => {
                self.last_assistant_text = text;
                self.tool_calls_made += tool_calls.len();
                self.phase = Phase::ExecutingTools;
                vec![
                    Effect::AppendTape {
                        kind: TapeAppendKind::AssistantIntermediate,
                    },
                    Effect::AppendTape {
                        kind: TapeAppendKind::ToolCalls,
                    },
                    Effect::RunTools { calls: tool_calls },
                ]
            }

            // ── LLM error: retry by disabling tools, fail when exhausted ─
            (Phase::AwaitingLlm, Event::LlmFailed { retryable, message }) => {
                if retryable && self.llm_recoveries < MAX_LLM_RECOVERIES {
                    self.llm_recoveries += 1;
                    self.tools_enabled = false;
                    // Stay in AwaitingLlm; runner re-issues CallLlm.
                    vec![Effect::CallLlm {
                        iteration:     self.iteration,
                        tools_enabled: self.tools_enabled,
                    }]
                } else {
                    self.phase = Phase::Failed;
                    vec![Effect::Fail { message }]
                }
            }

            // ── Tool wave finished ───────────────────────────────────────
            (Phase::ExecutingTools, Event::ToolsCompleted { results: _ }) => {
                self.iteration += 1;
                if self.iteration >= self.max_iterations {
                    self.phase = Phase::Done;
                    let text = std::mem::take(&mut self.last_assistant_text);
                    vec![
                        Effect::AppendTape {
                            kind: TapeAppendKind::ToolResults,
                        },
                        Effect::Finish {
                            text,
                            iterations: self.iteration,
                            tool_calls: self.tool_calls_made,
                            reason: FinishReason::MaxIterations,
                        },
                    ]
                } else {
                    self.phase = Phase::AwaitingLlm;
                    vec![
                        Effect::AppendTape {
                            kind: TapeAppendKind::ToolResults,
                        },
                        Effect::CallLlm {
                            iteration:     self.iteration,
                            tools_enabled: self.tools_enabled,
                        },
                    ]
                }
            }

            // ── Guard rejected the wave ──────────────────────────────────
            (Phase::ExecutingTools, Event::GuardRejected { reason }) => {
                self.phase = Phase::Failed;
                vec![Effect::Fail {
                    message: format!("guard rejected tool wave: {reason}"),
                }]
            }

            // ── Interruption from any non-terminal state ─────────────────
            (Phase::AwaitingLlm | Phase::ExecutingTools, Event::Interrupted) => {
                self.phase = Phase::Failed;
                vec![Effect::Fail {
                    message: "turn interrupted".to_owned(),
                }]
            }

            // Any other (phase, event) pair is a programming error in the
            // runner — surface it loudly so tests catch contract violations.
            (phase, event) => {
                self.phase = Phase::Failed;
                vec![Effect::Fail {
                    message: format!("invalid transition: phase={phase:?} event={event:?}"),
                }]
            }
        }
    }
}

/// Events fed to [`AgentMachine::step`] by the runner.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Begin a new turn (issued exactly once after construction).
    TurnStarted,
    /// LLM streaming call finished successfully.
    LlmCompleted {
        /// Concatenated assistant text.
        text:           String,
        /// Tool calls extracted from the response (may be empty).
        tool_calls:     Vec<ToolCall>,
        /// Whether the response indicates the LLM wants tools executed.
        has_tool_calls: bool,
    },
    /// LLM streaming call errored.
    LlmFailed {
        /// True for transient/provider errors that warrant a retry.
        retryable: bool,
        /// Human-readable failure description.
        message:   String,
    },
    /// All tool calls in the current wave have results (success or error).
    ToolsCompleted {
        /// Per-call outcome — order matches the originating
        /// [`Effect::RunTools::calls`].
        results: Vec<ToolResult>,
    },
    /// Security guard rejected the entire wave.
    GuardRejected {
        /// Reason string surfaced to the user / tape.
        reason: String,
    },
    /// User cancelled the turn (Ctrl-C, /stop, kernel shutdown, …).
    Interrupted,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agent::effect::{ToolCall as Tc, ToolCallId, ToolResult as Tr},
        tool::ToolName,
    };

    fn tool_call(id: &str, name: &str) -> Tc {
        Tc {
            id:        ToolCallId::new(id),
            name:      ToolName::new(name),
            arguments: "{}".to_owned(),
        }
    }

    fn tool_result(id: &str, name: &str, success: bool) -> Tr {
        Tr {
            id: ToolCallId::new(id),
            name: ToolName::new(name),
            success,
            duration_ms: 1,
            error: if success { None } else { Some("boom".into()) },
        }
    }

    #[test]
    fn happy_path_text_only() {
        let mut m = AgentMachine::new(8);
        let effects = m.step(Event::TurnStarted);
        assert!(matches!(effects.as_slice(), [Effect::CallLlm { .. }]));
        assert_eq!(m.phase(), Phase::AwaitingLlm);

        let effects = m.step(Event::LlmCompleted {
            text:           "hi user".into(),
            tool_calls:     vec![],
            has_tool_calls: false,
        });
        assert_eq!(m.phase(), Phase::Done);
        assert!(m.is_terminal());
        assert!(matches!(
            effects.as_slice(),
            [
                Effect::AppendTape {
                    kind: TapeAppendKind::AssistantFinal,
                },
                Effect::Finish {
                    reason: FinishReason::Stopped,
                    ..
                },
            ]
        ));
    }

    #[test]
    fn happy_path_with_tool_call_then_stop() {
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);

        let effects = m.step(Event::LlmCompleted {
            text:           "thinking".into(),
            tool_calls:     vec![tool_call("c1", "search")],
            has_tool_calls: true,
        });
        assert_eq!(m.phase(), Phase::ExecutingTools);
        assert_eq!(m.tool_calls_made(), 1);
        assert!(matches!(
            effects.as_slice(),
            [
                Effect::AppendTape {
                    kind: TapeAppendKind::AssistantIntermediate,
                },
                Effect::AppendTape {
                    kind: TapeAppendKind::ToolCalls,
                },
                Effect::RunTools { .. },
            ]
        ));

        let effects = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c1", "search", true)],
        });
        // Loop continues — runner gets a fresh CallLlm.
        assert_eq!(m.phase(), Phase::AwaitingLlm);
        assert_eq!(m.iteration(), 1);
        assert!(matches!(
            effects.as_slice(),
            [
                Effect::AppendTape {
                    kind: TapeAppendKind::ToolResults,
                },
                Effect::CallLlm { iteration: 1, .. },
            ]
        ));

        // Final LLM call wraps up the turn.
        let _ = m.step(Event::LlmCompleted {
            text:           "done".into(),
            tool_calls:     vec![],
            has_tool_calls: false,
        });
        assert_eq!(m.phase(), Phase::Done);
    }

    #[test]
    fn llm_error_retryable_falls_back_to_no_tools() {
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);

        let effects = m.step(Event::LlmFailed {
            retryable: true,
            message:   "503".into(),
        });
        // Recovery: machine stays in AwaitingLlm and re-issues CallLlm with tools off.
        assert_eq!(m.phase(), Phase::AwaitingLlm);
        match &effects[..] {
            [Effect::CallLlm { tools_enabled, .. }] => assert!(!tools_enabled),
            other => panic!("unexpected effects: {other:?}"),
        }
    }

    #[test]
    fn llm_error_non_retryable_fails_immediately() {
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);

        let effects = m.step(Event::LlmFailed {
            retryable: false,
            message:   "auth".into(),
        });
        assert_eq!(m.phase(), Phase::Failed);
        assert!(matches!(effects.as_slice(), [Effect::Fail { .. }]));
    }

    #[test]
    fn llm_error_exhausts_retries() {
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);
        for _ in 0..MAX_LLM_RECOVERIES {
            let _ = m.step(Event::LlmFailed {
                retryable: true,
                message:   "x".into(),
            });
            assert_eq!(m.phase(), Phase::AwaitingLlm);
        }
        let effects = m.step(Event::LlmFailed {
            retryable: true,
            message:   "x".into(),
        });
        assert_eq!(m.phase(), Phase::Failed);
        assert!(matches!(effects.as_slice(), [Effect::Fail { .. }]));
    }

    #[test]
    fn tool_failure_still_loops_back_to_llm() {
        // Tool errors are *data*: the runner reports them to the machine via
        // ToolsCompleted, the machine forwards them to the LLM as the next
        // iteration's context.  Failures do NOT abort the turn.
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);
        let _ = m.step(Event::LlmCompleted {
            text:           String::new(),
            tool_calls:     vec![tool_call("c1", "broken")],
            has_tool_calls: true,
        });
        let effects = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c1", "broken", false)],
        });
        assert_eq!(m.phase(), Phase::AwaitingLlm);
        assert!(matches!(effects.last(), Some(Effect::CallLlm { .. })));
    }

    #[test]
    fn guard_rejection_aborts_turn() {
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);
        let _ = m.step(Event::LlmCompleted {
            text:           String::new(),
            tool_calls:     vec![tool_call("c1", "rm-rf")],
            has_tool_calls: true,
        });
        let effects = m.step(Event::GuardRejected {
            reason: "denied path".into(),
        });
        assert_eq!(m.phase(), Phase::Failed);
        assert!(matches!(effects.as_slice(), [Effect::Fail { .. }]));
    }

    #[test]
    fn interruption_from_awaiting_llm_fails() {
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);
        let effects = m.step(Event::Interrupted);
        assert_eq!(m.phase(), Phase::Failed);
        assert!(matches!(effects.as_slice(), [Effect::Fail { .. }]));
    }

    #[test]
    fn interruption_from_executing_tools_fails() {
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);
        let _ = m.step(Event::LlmCompleted {
            text:           String::new(),
            tool_calls:     vec![tool_call("c1", "search")],
            has_tool_calls: true,
        });
        let effects = m.step(Event::Interrupted);
        assert_eq!(m.phase(), Phase::Failed);
        assert!(matches!(effects.as_slice(), [Effect::Fail { .. }]));
    }

    #[test]
    fn max_iterations_terminates_with_max_reason() {
        let mut m = AgentMachine::new(2);
        let _ = m.step(Event::TurnStarted);
        // Iteration 0: tool call, loop back.
        let _ = m.step(Event::LlmCompleted {
            text:           "step 0".into(),
            tool_calls:     vec![tool_call("c1", "t")],
            has_tool_calls: true,
        });
        let _ = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c1", "t", true)],
        });
        assert_eq!(m.iteration(), 1);
        // Iteration 1: another tool call.
        let _ = m.step(Event::LlmCompleted {
            text:           "step 1".into(),
            tool_calls:     vec![tool_call("c2", "t")],
            has_tool_calls: true,
        });
        let effects = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c2", "t", true)],
        });
        // iteration is now 2 == max_iterations → Finish(MaxIterations).
        assert_eq!(m.phase(), Phase::Done);
        assert!(matches!(
            effects.last(),
            Some(Effect::Finish {
                reason: FinishReason::MaxIterations,
                ..
            })
        ));
    }

    #[test]
    fn invalid_transition_is_surfaced() {
        let mut m = AgentMachine::new(8);
        // Feed ToolsCompleted before any LLM call — pure logic bug.
        let effects = m.step(Event::ToolsCompleted { results: vec![] });
        assert_eq!(m.phase(), Phase::Failed);
        assert!(matches!(effects.as_slice(), [Effect::Fail { .. }]));
    }
}
