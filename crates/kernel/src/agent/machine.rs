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

use crate::{
    agent::{
        effect::{
            Effect, FinishReason, LimitDecision, PressureLevel, TapeAppendKind, ToolCall,
            ToolResult,
        },
        loop_breaker::{LoopBreakerConfig, LoopIntervention, ToolCallLoopBreaker},
    },
    tool::ToolName,
};

/// Usage fraction at which the machine emits a `Warning`-level
/// `ContextPressureWarning` effect. Mirrors the legacy
/// `CONTEXT_WARN_THRESHOLD` constant in `agent::mod`.
pub const CONTEXT_WARN_THRESHOLD: f64 = 0.70;

/// Usage fraction at which the machine emits a `Critical`-level
/// `ContextPressureWarning` effect. Mirrors the legacy
/// `CONTEXT_CRITICAL_THRESHOLD` constant in `agent::mod`.
pub const CONTEXT_CRITICAL_THRESHOLD: f64 = 0.85;

/// High-level phases of one agent turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Initial state; no LLM call has been issued yet for this turn.
    Idle,
    /// An LLM streaming call is in flight.
    AwaitingLlm,
    /// LLM responded with tool calls; runner is dispatching them.
    ExecutingTools,
    /// Tool-call-limit tripped; runner is awaiting the user's continue/stop
    /// decision before another LLM call is issued.
    PausedForLimit,
    /// Terminal: turn finished successfully.
    Done,
    /// Terminal: turn failed.
    Failed,
}

/// Maximum number of LLM retry attempts (mirrors the legacy
/// `MAX_LLM_ERROR_RECOVERIES` constant in `agent::mod`).
pub const MAX_LLM_RECOVERIES: u32 = 3;

/// Default maximum self-elected continuations per turn.
pub const DEFAULT_MAX_CONTINUATIONS: usize = 10;

/// Mutable state carried across machine transitions for one turn.
#[derive(Debug)]
pub struct AgentMachine {
    phase:                Phase,
    iteration:            usize,
    max_iterations:       usize,
    tool_calls_made:      usize,
    last_assistant_text:  String,
    tools_enabled:        bool,
    llm_recoveries:       u32,
    /// Whether the most recent tool wave included a continue-work signal.
    continuation_pending: bool,
    /// How many self-elected continuations have been consumed this turn.
    continuation_count:   usize,
    /// Maximum allowed continuations per turn.
    max_continuations:    usize,
    /// Tool-call-loop detector. Fingerprints every tool call and, when
    /// patterns (exact duplicates, ping-pong, flooding) are detected,
    /// returns the set of tools to disable for the remainder of the turn.
    loop_breaker:         ToolCallLoopBreaker,
    /// Tools the loop breaker has disabled, accumulated across the turn.
    /// Threaded into every subsequent [`Effect::CallLlm`].
    disabled_tools:       Vec<ToolName>,
    /// Tool-call-limit circuit breaker: pause the turn every this many
    /// cumulative tool calls and ask the user whether to continue. Zero
    /// disables the circuit breaker entirely.
    limit_interval:       usize,
    /// Next `tool_calls_made` threshold at which to pause.
    next_limit_at:        usize,
    /// Monotonic counter for limit-pause ids, handed to the runner so it
    /// can key its oneshot channel and reject stale decisions.
    limit_id_counter:     u64,
    /// Whether a `PressureLevel::Warning` has already been emitted this
    /// turn.  The legacy loop nudged the LLM once at each threshold
    /// crossing to avoid spamming a repeating reminder every iteration.
    warned_at_warning:    bool,
    /// Whether a `PressureLevel::Critical` has already been emitted this
    /// turn.  Critical can fire even after Warning has fired (they are
    /// distinct thresholds), but each is still one-shot.
    warned_at_critical:   bool,
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
            continuation_pending: false,
            continuation_count: 0,
            max_continuations: DEFAULT_MAX_CONTINUATIONS,
            loop_breaker: ToolCallLoopBreaker::new(LoopBreakerConfig::builder().build()),
            disabled_tools: Vec::new(),
            limit_interval: 0,
            next_limit_at: usize::MAX,
            limit_id_counter: 0,
            warned_at_warning: false,
            warned_at_critical: false,
        }
    }

    /// Construct a machine with a tool-call-limit circuit breaker.
    ///
    /// The turn will pause every `limit_interval` cumulative tool calls
    /// and emit [`Effect::PauseForLimit`]; the runner awaits the user's
    /// decision and feeds it back via [`Event::LimitResolved`]. A
    /// `limit_interval` of zero disables the circuit breaker.
    pub(crate) fn with_tool_call_limit(max_iterations: usize, limit_interval: usize) -> Self {
        Self {
            limit_interval,
            next_limit_at: if limit_interval == 0 {
                usize::MAX
            } else {
                limit_interval
            },
            ..Self::new(max_iterations)
        }
    }

    /// Construct a machine with custom iteration and continuation limits.
    pub fn with_max_continuations(max_iterations: usize, max_continuations: usize) -> Self {
        Self {
            max_continuations,
            ..Self::new(max_iterations)
        }
    }

    /// Construct a machine with a custom [`LoopBreakerConfig`].
    ///
    /// Callers use this to pass a `flooding_exempt` set (e.g. the current
    /// turn's read-only tools) so the breaker does not disable them on long
    /// investigations with many distinct arguments — mirroring the
    /// `t.is_read_only(...)` exemption the legacy `run_agent_loop` builds.
    pub(crate) fn with_loop_breaker_config(
        max_iterations: usize,
        loop_breaker: LoopBreakerConfig,
    ) -> Self {
        Self {
            loop_breaker: ToolCallLoopBreaker::new(loop_breaker),
            ..Self::new(max_iterations)
        }
    }

    /// Current high-level phase.
    pub fn phase(&self) -> Phase { self.phase }

    /// Iteration counter (0-based).
    pub fn iteration(&self) -> usize { self.iteration }

    /// Cumulative tool calls executed in the turn so far.
    pub fn tool_calls_made(&self) -> usize { self.tool_calls_made }

    /// How many self-elected continuations have been consumed this turn.
    pub fn continuation_count(&self) -> usize { self.continuation_count }

    /// Whether the machine has reached a terminal state.
    pub fn is_terminal(&self) -> bool { matches!(self.phase, Phase::Done | Phase::Failed) }

    /// Synchronous observation: report the current context-window usage and
    /// return any resulting `ContextPressureWarning` effects.
    ///
    /// The runner is expected to call this once per LLM round — after
    /// rebuilding the tape context for the next iteration but before
    /// interpreting `Effect::CallLlm` — so the injected warning message lands
    /// in the conversation immediately ahead of the model's next turn.
    ///
    /// Unlike `step`, this method does **not** consume an [`Event`] and does
    /// **not** change [`Phase`]. It is a pure observation:
    ///
    /// - Returns `[Effect::ContextPressureWarning { level: Critical, .. }]` the
    ///   first time usage crosses `CONTEXT_CRITICAL_THRESHOLD`.
    /// - Returns `[Effect::ContextPressureWarning { level: Warning, .. }]` the
    ///   first time usage crosses `CONTEXT_WARN_THRESHOLD` (and Critical has
    ///   not yet fired).
    /// - Returns `[]` otherwise (already warned at the bucket, or below
    ///   threshold, or window is zero).
    ///
    /// Warning and Critical each fire at most once per turn — the legacy
    /// loop nudged the LLM once per crossing to avoid spam that would anchor
    /// the conversation on the reminder itself.
    pub fn observe_context_usage(
        &mut self,
        estimated_tokens: usize,
        context_window_tokens: usize,
    ) -> Vec<Effect> {
        if context_window_tokens == 0 {
            return Vec::new();
        }
        let usage_ratio = estimated_tokens as f64 / context_window_tokens as f64;

        if usage_ratio >= CONTEXT_CRITICAL_THRESHOLD && !self.warned_at_critical {
            self.warned_at_critical = true;
            // Ensure Warning is also marked as delivered once we jumped
            // straight past it — avoids emitting a stale Warning after a
            // Critical has already been surfaced.
            self.warned_at_warning = true;
            return vec![Effect::ContextPressureWarning {
                level: PressureLevel::Critical,
                estimated_tokens,
                context_window_tokens,
            }];
        }

        if usage_ratio >= CONTEXT_WARN_THRESHOLD && !self.warned_at_warning {
            self.warned_at_warning = true;
            return vec![Effect::ContextPressureWarning {
                level: PressureLevel::Warning,
                estimated_tokens,
                context_window_tokens,
            }];
        }

        Vec::new()
    }

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
                    iteration:      self.iteration,
                    tools_enabled:  self.tools_enabled,
                    disabled_tools: self.disabled_tools.clone(),
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

                // If a previous tool wave signaled continue-work AND budget remains,
                // loop back instead of stopping.
                if self.continuation_pending && self.continuation_count < self.max_continuations {
                    self.continuation_pending = false;
                    self.continuation_count += 1;
                    self.phase = Phase::AwaitingLlm;
                    return vec![
                        Effect::AppendTape {
                            kind: TapeAppendKind::AssistantIntermediate,
                        },
                        Effect::InjectContinuationWake {
                            turn: self.continuation_count,
                            max:  self.max_continuations,
                        },
                        Effect::CallLlm {
                            iteration:      self.iteration,
                            tools_enabled:  self.tools_enabled,
                            disabled_tools: self.disabled_tools.clone(),
                        },
                    ];
                }

                self.continuation_pending = false;
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
                        iteration:      self.iteration,
                        tools_enabled:  self.tools_enabled,
                        disabled_tools: self.disabled_tools.clone(),
                    }]
                } else {
                    self.phase = Phase::Failed;
                    vec![Effect::Fail { message }]
                }
            }

            // ── Tool wave finished ───────────────────────────────────────
            (Phase::ExecutingTools, Event::ToolsCompleted { results }) => {
                // Check if any tool in this wave signaled continuation.
                self.continuation_pending = results
                    .iter()
                    .any(|r| r.name == "continue-work" && r.success);

                // Feed every call from this wave into the loop breaker, then
                // consult it exactly once.  Fingerprints are (name, arguments)
                // pairs, which `ToolResult.arguments` now preserves.
                for r in &results {
                    self.loop_breaker.record(r.name.as_str(), &r.arguments);
                }
                let loop_breaker_effect = match self.loop_breaker.check() {
                    LoopIntervention::None => None,
                    LoopIntervention::DisableTools { pattern, tools, .. } => {
                        let newly_disabled: Vec<ToolName> = tools
                            .into_iter()
                            .map(ToolName::new)
                            .filter(|t| !self.disabled_tools.contains(t))
                            .collect();
                        self.disabled_tools.extend(newly_disabled.iter().cloned());
                        Some(Effect::LoopBreakerTriggered {
                            disabled_tools:  newly_disabled,
                            pattern:         pattern.to_owned(),
                            tool_calls_made: self.tool_calls_made,
                        })
                    }
                };

                self.iteration += 1;
                let mut effects = vec![Effect::AppendTape {
                    kind: TapeAppendKind::ToolResults,
                }];
                if let Some(e) = loop_breaker_effect {
                    effects.push(e);
                }

                if self.iteration >= self.max_iterations {
                    self.phase = Phase::Done;
                    let text = std::mem::take(&mut self.last_assistant_text);
                    effects.push(Effect::Finish {
                        text,
                        iterations: self.iteration,
                        tool_calls: self.tool_calls_made,
                        reason: FinishReason::MaxIterations,
                    });
                } else if self.limit_interval > 0 && self.tool_calls_made >= self.next_limit_at {
                    self.limit_id_counter += 1;
                    self.phase = Phase::PausedForLimit;
                    effects.push(Effect::PauseForLimit {
                        limit_id:        self.limit_id_counter,
                        tool_calls_made: self.tool_calls_made,
                    });
                } else {
                    self.phase = Phase::AwaitingLlm;
                    effects.push(Effect::CallLlm {
                        iteration:      self.iteration,
                        tools_enabled:  self.tools_enabled,
                        disabled_tools: self.disabled_tools.clone(),
                    });
                }
                effects
            }

            // ── User decided whether to continue after a limit pause ─────
            (Phase::PausedForLimit, Event::LimitResolved { limit_id, decision })
                if limit_id == self.limit_id_counter =>
            {
                match decision {
                    LimitDecision::Continue => {
                        self.next_limit_at = self.tool_calls_made + self.limit_interval;
                        self.phase = Phase::AwaitingLlm;
                        vec![Effect::CallLlm {
                            iteration:      self.iteration,
                            tools_enabled:  self.tools_enabled,
                            disabled_tools: self.disabled_tools.clone(),
                        }]
                    }
                    LimitDecision::Stop => {
                        self.phase = Phase::Done;
                        let text = std::mem::take(&mut self.last_assistant_text);
                        vec![Effect::Finish {
                            text,
                            iterations: self.iteration,
                            tool_calls: self.tool_calls_made,
                            reason: FinishReason::StoppedByLimit,
                        }]
                    }
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
            (
                Phase::AwaitingLlm | Phase::ExecutingTools | Phase::PausedForLimit,
                Event::Interrupted,
            ) => {
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
    /// User (or the pause timeout) decided whether to continue after a
    /// tool-call-limit pause.
    LimitResolved {
        /// Id of the pause the decision resolves. Stale ids are ignored
        /// by the machine to prevent a late decision from resuming a
        /// subsequent pause.
        limit_id: u64,
        /// Continue or stop.
        decision: LimitDecision,
    },
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

    fn tool_result(id: &str, name: &str, args: &str, success: bool) -> Tr {
        Tr {
            id: ToolCallId::new(id),
            name: ToolName::new(name),
            arguments: args.to_owned(),
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
            results: vec![tool_result("c1", "search", "{}", true)],
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
            results: vec![tool_result("c1", "broken", "{}", false)],
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
            results: vec![tool_result("c1", "t", "{}", true)],
        });
        assert_eq!(m.iteration(), 1);
        // Iteration 1: another tool call.
        let _ = m.step(Event::LlmCompleted {
            text:           "step 1".into(),
            tool_calls:     vec![tool_call("c2", "t")],
            has_tool_calls: true,
        });
        let effects = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c2", "t", "{}", true)],
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
    fn continuation_signal_loops_back_to_llm() {
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);

        // LLM calls continue-work tool
        let _ = m.step(Event::LlmCompleted {
            text:           "working on it".into(),
            tool_calls:     vec![tool_call("c1", "continue-work")],
            has_tool_calls: true,
        });
        let _ = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c1", "continue-work", "{}", true)],
        });

        // LLM responds with text only — BUT continuation was signaled
        let effects = m.step(Event::LlmCompleted {
            text:           "still working...".into(),
            tool_calls:     vec![],
            has_tool_calls: false,
        });

        // Should NOT terminate — should inject wake and loop back
        assert_eq!(m.phase(), Phase::AwaitingLlm);
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::InjectContinuationWake { .. }))
        );
        assert!(effects.iter().any(|e| matches!(e, Effect::CallLlm { .. })));
        assert_eq!(m.continuation_count(), 1);
    }

    #[test]
    fn continuation_respects_max_limit() {
        let mut m = AgentMachine::with_max_continuations(20, 2);
        let _ = m.step(Event::TurnStarted);

        // Use up 2 continuations
        for i in 0..2 {
            let _ = m.step(Event::LlmCompleted {
                text:           String::new(),
                tool_calls:     vec![tool_call(&format!("c{i}"), "continue-work")],
                has_tool_calls: true,
            });
            let _ = m.step(Event::ToolsCompleted {
                results: vec![tool_result(&format!("c{i}"), "continue-work", "{}", true)],
            });
            // Text-only response — continuation kicks in
            let _ = m.step(Event::LlmCompleted {
                text:           format!("working {i}"),
                tool_calls:     vec![],
                has_tool_calls: false,
            });
            assert_eq!(
                m.phase(),
                Phase::AwaitingLlm,
                "should continue at round {i}"
            );
        }

        // 3rd continue-work call
        let _ = m.step(Event::LlmCompleted {
            text:           String::new(),
            tool_calls:     vec![tool_call("c3", "continue-work")],
            has_tool_calls: true,
        });
        let _ = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c3", "continue-work", "{}", true)],
        });
        // Text-only — but limit reached, should stop
        let effects = m.step(Event::LlmCompleted {
            text:           "done".into(),
            tool_calls:     vec![],
            has_tool_calls: false,
        });
        assert_eq!(m.phase(), Phase::Done);
        assert!(matches!(
            effects.last(),
            Some(Effect::Finish {
                reason: FinishReason::Stopped,
                ..
            })
        ));
    }

    #[test]
    fn continuation_not_triggered_without_signal() {
        let mut m = AgentMachine::new(8);
        let _ = m.step(Event::TurnStarted);

        // Regular tool call (not continue-work)
        let _ = m.step(Event::LlmCompleted {
            text:           String::new(),
            tool_calls:     vec![tool_call("c1", "search")],
            has_tool_calls: true,
        });
        let _ = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c1", "search", "{}", true)],
        });

        // Text-only — should stop normally
        let effects = m.step(Event::LlmCompleted {
            text:           "here are the results".into(),
            tool_calls:     vec![],
            has_tool_calls: false,
        });
        assert_eq!(m.phase(), Phase::Done);
        assert!(matches!(
            effects.last(),
            Some(Effect::Finish {
                reason: FinishReason::Stopped,
                ..
            })
        ));
        assert_eq!(m.continuation_count(), 0);
    }

    #[test]
    fn invalid_transition_is_surfaced() {
        let mut m = AgentMachine::new(8);
        // Feed ToolsCompleted before any LLM call — pure logic bug.
        let effects = m.step(Event::ToolsCompleted { results: vec![] });
        assert_eq!(m.phase(), Phase::Failed);
        assert!(matches!(effects.as_slice(), [Effect::Fail { .. }]));
    }

    // ─── Loop breaker integration ────────────────────────────────────────

    /// Three identical tool calls in a row trip the exact-duplicate detector
    /// (default `exact_dup_threshold = 3`): we expect a `LoopBreakerTriggered`
    /// effect emitted before the next `CallLlm`, and the subsequent `CallLlm`
    /// must carry the newly-disabled tool in its `disabled_tools` field.
    #[test]
    fn loop_breaker_disables_tools_on_exact_duplicate() {
        let mut m = AgentMachine::new(16);
        let _ = m.step(Event::TurnStarted);

        // Drive three waves of the same tool+args to trip exact-duplicate.
        for i in 0..3 {
            let _ = m.step(Event::LlmCompleted {
                text:           "tick".into(),
                tool_calls:     vec![tool_call(&format!("c{i}"), "repeat")],
                has_tool_calls: true,
            });
            let effects = m.step(Event::ToolsCompleted {
                results: vec![tool_result(&format!("c{i}"), "repeat", "{}", true)],
            });
            if i < 2 {
                assert!(
                    !effects
                        .iter()
                        .any(|e| matches!(e, Effect::LoopBreakerTriggered { .. })),
                    "breaker fired too early at wave {i}",
                );
                continue;
            }
            // Wave 3 (i == 2): third identical call → trip.
            let trip = effects
                .iter()
                .find(|e| matches!(e, Effect::LoopBreakerTriggered { .. }))
                .expect("expected LoopBreakerTriggered on third identical wave");
            match trip {
                Effect::LoopBreakerTriggered {
                    pattern,
                    disabled_tools,
                    ..
                } => {
                    assert_eq!(pattern, "exact_duplicate");
                    assert_eq!(disabled_tools, &vec![ToolName::new("repeat")]);
                }
                _ => unreachable!(),
            }
            // The next CallLlm must carry the accumulated disabled_tools.
            let call = effects
                .iter()
                .find_map(|e| match e {
                    Effect::CallLlm { disabled_tools, .. } => Some(disabled_tools),
                    _ => None,
                })
                .expect("expected CallLlm after trip");
            assert_eq!(call, &vec![ToolName::new("repeat")]);
        }
    }

    /// Varying arguments across successive calls keeps the breaker quiet:
    /// different fingerprints, so no exact-duplicate trip and far below
    /// `disable_after = 25`.
    #[test]
    fn loop_breaker_quiet_on_varied_tools() {
        let mut m = AgentMachine::new(16);
        let _ = m.step(Event::TurnStarted);

        for i in 0..3 {
            let _ = m.step(Event::LlmCompleted {
                text:           "tick".into(),
                tool_calls:     vec![tool_call(&format!("c{i}"), "search")],
                has_tool_calls: true,
            });
            let effects = m.step(Event::ToolsCompleted {
                results: vec![tool_result(
                    &format!("c{i}"),
                    "search",
                    &format!(r#"{{"q":"{i}"}}"#),
                    true,
                )],
            });
            assert!(
                !effects
                    .iter()
                    .any(|e| matches!(e, Effect::LoopBreakerTriggered { .. })),
                "breaker fired unexpectedly on varied args at wave {i}",
            );
        }
    }

    /// Once the breaker trips, every subsequent `CallLlm` must continue to
    /// carry the accumulated `disabled_tools` set so the runner can keep
    /// filtering tool definitions across iterations.
    #[test]
    fn disabled_tools_persist_across_iterations() {
        let mut m = AgentMachine::new(16);
        let _ = m.step(Event::TurnStarted);

        // Trip the breaker with three identical calls.
        for i in 0..3 {
            let _ = m.step(Event::LlmCompleted {
                text:           "tick".into(),
                tool_calls:     vec![tool_call(&format!("c{i}"), "repeat")],
                has_tool_calls: true,
            });
            let _ = m.step(Event::ToolsCompleted {
                results: vec![tool_result(&format!("c{i}"), "repeat", "{}", true)],
            });
        }

        // Now run two more iterations with a different tool and verify the
        // disabled set is still threaded through every CallLlm.
        for i in 3..5 {
            let _ = m.step(Event::LlmCompleted {
                text:           "tock".into(),
                tool_calls:     vec![tool_call(&format!("c{i}"), "search")],
                has_tool_calls: true,
            });
            let effects = m.step(Event::ToolsCompleted {
                results: vec![tool_result(
                    &format!("c{i}"),
                    "search",
                    &format!(r#"{{"q":"{i}"}}"#),
                    true,
                )],
            });
            let disabled = effects
                .iter()
                .find_map(|e| match e {
                    Effect::CallLlm { disabled_tools, .. } => Some(disabled_tools.clone()),
                    _ => None,
                })
                .expect("expected CallLlm after iteration");
            assert_eq!(
                disabled,
                vec![ToolName::new("repeat")],
                "disabled set should persist at iteration {i}",
            );
        }
    }

    // ─── Tool-call-limit circuit breaker ────────────────────────────────

    /// With `limit_interval = 1`, the very first tool wave trips the
    /// pause. The machine emits `PauseForLimit` and transitions to
    /// `PausedForLimit`; feeding `Continue` back resumes the turn.
    #[test]
    fn pause_for_limit_fires_at_threshold_and_continue_resumes() {
        let mut m = AgentMachine::with_tool_call_limit(8, 1);
        let _ = m.step(Event::TurnStarted);
        let _ = m.step(Event::LlmCompleted {
            text:           "step".into(),
            tool_calls:     vec![tool_call("c1", "search")],
            has_tool_calls: true,
        });
        let effects = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c1", "search", "{}", true)],
        });
        assert_eq!(m.phase(), Phase::PausedForLimit);
        let pause = effects
            .iter()
            .find_map(|e| match e {
                Effect::PauseForLimit {
                    limit_id,
                    tool_calls_made,
                } => Some((*limit_id, *tool_calls_made)),
                _ => None,
            })
            .expect("expected PauseForLimit effect");
        assert_eq!(pause, (1, 1));
        assert!(
            !effects.iter().any(|e| matches!(e, Effect::CallLlm { .. })),
            "no CallLlm should be emitted while paused",
        );

        let effects = m.step(Event::LimitResolved {
            limit_id: pause.0,
            decision: LimitDecision::Continue,
        });
        assert_eq!(m.phase(), Phase::AwaitingLlm);
        assert!(matches!(effects.as_slice(), [Effect::CallLlm { .. }]));
    }

    /// `Stop` is a graceful termination: `Finish` with `StoppedByLimit`.
    #[test]
    fn pause_for_limit_stop_terminates_gracefully() {
        let mut m = AgentMachine::with_tool_call_limit(8, 1);
        let _ = m.step(Event::TurnStarted);
        let _ = m.step(Event::LlmCompleted {
            text:           "step".into(),
            tool_calls:     vec![tool_call("c1", "search")],
            has_tool_calls: true,
        });
        let _ = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c1", "search", "{}", true)],
        });
        let effects = m.step(Event::LimitResolved {
            limit_id: 1,
            decision: LimitDecision::Stop,
        });
        assert_eq!(m.phase(), Phase::Done);
        assert!(matches!(
            effects.as_slice(),
            [Effect::Finish {
                reason: FinishReason::StoppedByLimit,
                ..
            }]
        ));
    }

    /// Stale `LimitResolved` (mismatched id) must not advance the machine.
    /// In legacy code this is prevented by the session-scoped oneshot key,
    /// so the machine mirrors that guarantee with the id check.
    #[test]
    fn pause_for_limit_ignores_stale_resolution() {
        let mut m = AgentMachine::with_tool_call_limit(8, 1);
        let _ = m.step(Event::TurnStarted);
        let _ = m.step(Event::LlmCompleted {
            text:           "step".into(),
            tool_calls:     vec![tool_call("c1", "search")],
            has_tool_calls: true,
        });
        let _ = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c1", "search", "{}", true)],
        });
        // Feed a wrong id — falls into the invalid-transition arm.
        let effects = m.step(Event::LimitResolved {
            limit_id: 999,
            decision: LimitDecision::Continue,
        });
        assert_eq!(m.phase(), Phase::Failed);
        assert!(matches!(effects.as_slice(), [Effect::Fail { .. }]));
    }

    /// `limit_interval = 0` disables the circuit breaker entirely; the
    /// machine keeps looping and never emits `PauseForLimit`.
    #[test]
    fn zero_interval_disables_limit() {
        let mut m = AgentMachine::with_tool_call_limit(8, 0);
        let _ = m.step(Event::TurnStarted);
        for i in 0..3 {
            let _ = m.step(Event::LlmCompleted {
                text:           "t".into(),
                tool_calls:     vec![tool_call(&format!("c{i}"), "t")],
                has_tool_calls: true,
            });
            let effects = m.step(Event::ToolsCompleted {
                results: vec![tool_result(&format!("c{i}"), "t", "{}", true)],
            });
            assert!(
                !effects
                    .iter()
                    .any(|e| matches!(e, Effect::PauseForLimit { .. })),
                "limit should stay disabled at wave {i}",
            );
            assert_eq!(m.phase(), Phase::AwaitingLlm);
        }
    }

    /// Continue advances `next_limit_at` by `limit_interval`, so the next
    /// pause only fires after another full interval of tool calls.
    #[test]
    fn continue_advances_next_threshold_by_interval() {
        let mut m = AgentMachine::with_tool_call_limit(16, 2);
        let _ = m.step(Event::TurnStarted);

        // Two tool calls → crosses first threshold (2 ≥ 2) → pause.
        let _ = m.step(Event::LlmCompleted {
            text:           "x".into(),
            tool_calls:     vec![tool_call("a", "t"), tool_call("b", "t")],
            has_tool_calls: true,
        });
        let _ = m.step(Event::ToolsCompleted {
            results: vec![
                tool_result("a", "t", "{}", true),
                tool_result("b", "t", r#"{"q":"2"}"#, true),
            ],
        });
        assert_eq!(m.phase(), Phase::PausedForLimit);
        let _ = m.step(Event::LimitResolved {
            limit_id: 1,
            decision: LimitDecision::Continue,
        });

        // One more tool call → at 3, below new threshold (2 + 2 = 4), no pause.
        let _ = m.step(Event::LlmCompleted {
            text:           "y".into(),
            tool_calls:     vec![tool_call("c", "t")],
            has_tool_calls: true,
        });
        let effects = m.step(Event::ToolsCompleted {
            results: vec![tool_result("c", "t", r#"{"q":"3"}"#, true)],
        });
        assert_eq!(m.phase(), Phase::AwaitingLlm);
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::PauseForLimit { .. })),
            "below advanced threshold should not pause",
        );

        // Another call → reaches 4 → pause again with next id.
        let _ = m.step(Event::LlmCompleted {
            text:           "z".into(),
            tool_calls:     vec![tool_call("d", "t")],
            has_tool_calls: true,
        });
        let effects = m.step(Event::ToolsCompleted {
            results: vec![tool_result("d", "t", r#"{"q":"4"}"#, true)],
        });
        assert_eq!(m.phase(), Phase::PausedForLimit);
        let id = effects
            .iter()
            .find_map(|e| match e {
                Effect::PauseForLimit { limit_id, .. } => Some(*limit_id),
                _ => None,
            })
            .expect("expected second PauseForLimit");
        assert_eq!(id, 2, "limit id monotonically increases");
    }

    // ─── Context-pressure observation ───────────────────────────────────

    /// Below the warning threshold the observer stays silent.
    #[test]
    fn context_pressure_silent_below_warning() {
        let mut m = AgentMachine::new(8);
        assert!(m.observe_context_usage(500, 1_000).is_empty());
        assert!(m.observe_context_usage(699, 1_000).is_empty());
    }

    /// Crossing 0.70 (but not 0.85) emits a single Warning effect.
    #[test]
    fn context_pressure_fires_warning_at_threshold() {
        let mut m = AgentMachine::new(8);
        let effects = m.observe_context_usage(750, 1_000);
        match effects.as_slice() {
            [
                Effect::ContextPressureWarning {
                    level: PressureLevel::Warning,
                    estimated_tokens,
                    context_window_tokens,
                },
            ] => {
                assert_eq!(*estimated_tokens, 750);
                assert_eq!(*context_window_tokens, 1_000);
            }
            other => panic!("expected single Warning, got {other:?}"),
        }
    }

    /// Warning is one-shot: repeated observations in the same bucket are
    /// silent even when usage rises further within the Warning band.
    #[test]
    fn context_pressure_warning_is_one_shot() {
        let mut m = AgentMachine::new(8);
        assert_eq!(m.observe_context_usage(750, 1_000).len(), 1);
        assert!(m.observe_context_usage(800, 1_000).is_empty());
        assert!(m.observe_context_usage(849, 1_000).is_empty());
    }

    /// Crossing 0.85 emits Critical even if Warning has already fired; and
    /// the subsequent Warning-band observation is swallowed.
    #[test]
    fn context_pressure_upgrades_to_critical() {
        let mut m = AgentMachine::new(8);
        assert_eq!(m.observe_context_usage(750, 1_000).len(), 1); // Warning
        let effects = m.observe_context_usage(900, 1_000);
        match effects.as_slice() {
            [
                Effect::ContextPressureWarning {
                    level: PressureLevel::Critical,
                    ..
                },
            ] => {}
            other => panic!("expected Critical, got {other:?}"),
        }
        // No more warnings — critical is one-shot too.
        assert!(m.observe_context_usage(950, 1_000).is_empty());
    }

    /// Jumping straight past Warning into Critical emits Critical only and
    /// does not double up with a Warning.
    #[test]
    fn context_pressure_skips_warning_when_jumping_to_critical() {
        let mut m = AgentMachine::new(8);
        let effects = m.observe_context_usage(900, 1_000);
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::ContextPressureWarning {
                level: PressureLevel::Critical,
                ..
            }
        ));
        // A subsequent Warning-band observation must NOT produce a stale
        // Warning effect.
        assert!(m.observe_context_usage(750, 1_000).is_empty());
    }

    /// Zero context window is treated as "unknown" and never emits.
    #[test]
    fn context_pressure_zero_window_is_noop() {
        let mut m = AgentMachine::new(8);
        assert!(m.observe_context_usage(10_000, 0).is_empty());
    }

    /// Mirrors the legacy `run_agent_loop` exemption for read-only tools:
    /// callers pass a `flooding_exempt` set so tools like `search` / `read`
    /// are not disabled after 25 varied-argument invocations. Without this
    /// the machine would regress long read-only investigations once the
    /// runner replaces the legacy loop in production.
    #[test]
    fn loop_breaker_flooding_exempt_is_honoured() {
        use std::collections::HashSet;

        let cfg = LoopBreakerConfig::builder()
            .flooding_exempt(HashSet::from(["search".to_owned()]))
            .build();
        let mut m = AgentMachine::with_loop_breaker_config(200, cfg);
        let _ = m.step(Event::TurnStarted);

        // 30 varied-arg calls — would trip `disable_after = 25` without the
        // exemption.
        for i in 0..30 {
            let _ = m.step(Event::LlmCompleted {
                text:           "tick".into(),
                tool_calls:     vec![tool_call(&format!("c{i}"), "search")],
                has_tool_calls: true,
            });
            let effects = m.step(Event::ToolsCompleted {
                results: vec![tool_result(
                    &format!("c{i}"),
                    "search",
                    &format!(r#"{{"q":"{i}"}}"#),
                    true,
                )],
            });
            assert!(
                !effects
                    .iter()
                    .any(|e| matches!(e, Effect::LoopBreakerTriggered { .. })),
                "exempt tool should not flood at wave {i}",
            );
        }
    }
}
