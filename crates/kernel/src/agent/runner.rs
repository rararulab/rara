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

//! Async runner that drives the sans-IO [`AgentMachine`] against real
//! subsystems.
//!
//! # Status
//!
//! This module is a **partial migration scaffold** for issue #1145. It
//! contains:
//!
//! 1. A working `drive` loop that executes the high-level state machine spine
//!    against an in-memory [`Subsystems`] trait — used by the unit tests in
//!    [`crate::agent::machine`] to exercise the full machine end-to-end with
//!    zero mocks (the test impl is plain values, not a mock framework).
//! 2. Documentation of which secondary state from the legacy
//!    `agent::run_agent_loop` still needs to migrate before the new runner can
//!    replace it.
//!
//! The legacy `agent::run_agent_loop` remains the **production** turn loop
//! and is still wired into `crate::kernel` / `crate::plan`. Once the items
//! below land, the legacy loop is deleted and `drive` becomes the sole code
//! path.
//!
//! # Migration TODO (follow-up to #1145)
//!
//! Behaviour from `run_agent_loop` not yet expressed as machine effects:
//!
//! - Auto-fold (pressure-driven context compression) and
//!   `force_fold_next_iteration`
//! - Loop breaker (`crate::agent::loop_breaker`) interventions — ✓ machine-side
//!   implemented; legacy removal pending
//! - Context pressure warnings + session-length reminders injected as user
//!   messages — ✓ machine-side implemented via
//!   [`AgentMachine::observe_context_usage`] +
//!   [`Effect::ContextPressureWarning`]; legacy removal pending
//! - Tool-call-limit circuit breaker with oneshot resume — ✓ machine-side
//!   implemented; legacy removal pending
//! - Repetition guard truncation
//! - Deferred tool activation (`discover-tools`) feedback
//! - Per-iteration tape rebuild + sanitisation
//! - Empty-stream / rate-limit recovery branches
//! - Cascade trace assembly + mood inference
//!
//! Each item maps to either an additional [`Effect`] variant or extra fields
//! on [`AgentMachine`].

use async_trait::async_trait;

use crate::{
    agent::{
        effect::{Effect, PressureLevel, ToolCall, ToolResult},
        machine::{AgentMachine, Event, Phase},
    },
    tool::ToolName,
};

/// Outcome surfaced to the caller of [`drive`].
#[derive(Debug, Clone, PartialEq)]
pub struct DriveOutcome {
    /// Final assistant text (empty on failure).
    pub text:            String,
    /// Number of iterations the machine actually ran.
    pub iterations:      usize,
    /// Cumulative tool calls.
    pub tool_calls_made: usize,
    /// Whether the turn ended in a terminal success state.
    pub success:         bool,
    /// Optional failure message when `success == false`.
    pub failure_message: Option<String>,
}

/// Side-effect interpreter the runner uses to translate [`Effect`]s into
/// real subsystem calls.
///
/// Production code will provide an implementation that wraps the kernel
/// handle, tape service, guard pipeline, and stream handle.  The integration
/// tests in this module supply a synchronous in-memory implementation backed
/// by plain `Vec`s — *not* a mock framework — to demonstrate the contract.
#[async_trait]
pub trait Subsystems: Send + Sync {
    /// Issue an LLM completion request and produce the next event.
    ///
    /// `disabled_tools` lists tool names the loop breaker has removed from
    /// circulation for the remainder of the turn; the production impl must
    /// filter these out of the manifest passed to the LLM.
    async fn call_llm(
        &mut self,
        iteration: usize,
        tools_enabled: bool,
        disabled_tools: Vec<ToolName>,
    ) -> Event;

    /// Surface a loop-breaker trip to the user (stream event, tape entry,
    /// …). The machine has already updated its `disabled_tools` state
    /// before this is invoked, so implementations only need to *announce*
    /// the event — they do not need to track it.
    ///
    /// Default: no-op (test stubs that don't care about loop-breaker
    /// telemetry).
    async fn loop_breaker_triggered(
        &mut self,
        _disabled_tools: Vec<ToolName>,
        _pattern: String,
        _tool_calls_made: usize,
    ) {
    }

    /// Execute a wave of tool calls, producing the next event.
    async fn run_tools(&mut self, calls: Vec<ToolCall>) -> Event;

    /// Persist some structured payload to the tape.  Failures here are
    /// best-effort — they should be logged but never abort the turn.
    async fn append_tape(&mut self, kind: crate::agent::effect::TapeAppendKind);

    /// Forward a stream event to the user-facing transport.
    async fn emit_stream(&mut self, kind: String);

    /// Inject a user-role message into the conversation context so the LLM
    /// sees it on the next `call_llm`.
    ///
    /// Used by the continuation-wake path and by context-pressure warnings.
    /// Implementations append the message to the tape (persisted history),
    /// not just to an in-memory buffer — otherwise the nudge disappears on
    /// the next tape rebuild. Failures should be logged but must not abort
    /// the turn.
    ///
    /// No default impl: silent inject would hide broken integrations
    /// (anti-pattern: "Do NOT use noop trait implementations"). Test stubs
    /// keep a plain `Vec<String>` log; production wires the kernel
    /// `TapeService`.
    async fn inject_user_message(&mut self, text: String);

    /// Observe context-window usage for the current turn and return the
    /// estimated-tokens / window pair (in tokens).
    ///
    /// The runner calls this once per LLM round before interpreting
    /// `Effect::CallLlm` so the machine's
    /// [`AgentMachine::observe_context_usage`] can emit pressure warnings.
    ///
    /// Return `(0, 0)` — or any usage below `CONTEXT_WARN_THRESHOLD` —
    /// when sampling is unavailable or disabled; the machine treats a zero
    /// window as "unknown" and emits nothing. No default impl so test stubs
    /// have to opt in explicitly.
    async fn sample_context_usage(&mut self) -> (usize, usize);

    /// Pause the turn and await the user's continue/stop decision.
    ///
    /// Production implementations are expected to:
    /// - emit the user-facing `ToolCallLimit` stream event keyed by `limit_id`;
    /// - register a oneshot channel keyed by `(session, limit_id)`;
    /// - `tokio::select!` over the oneshot, a hard timeout (legacy used 120s),
    ///   and the turn cancel token;
    /// - emit `ToolCallLimitResolved` before returning;
    /// - map the outcome onto [`Event::LimitResolved`] (timeout / drop /
    ///   explicit `Stop` all produce `LimitDecision::Stop`).
    ///
    /// Treat a cancel-token firing as [`Event::Interrupted`] instead.
    async fn pause_for_limit(&mut self, limit_id: u64, tool_calls_made: usize) -> Event;
}

/// Drive the [`AgentMachine`] to completion against `subsys`.
///
/// This is the *runner* half of the sans-IO split: the loop owns the only
/// `.await` calls; the machine itself stays pure.
pub async fn drive<S: Subsystems>(machine: &mut AgentMachine, subsys: &mut S) -> DriveOutcome {
    let mut next_event = Event::TurnStarted;
    let mut outcome = DriveOutcome {
        text:            String::new(),
        iterations:      0,
        tool_calls_made: 0,
        success:         false,
        failure_message: None,
    };

    loop {
        let effects = machine.step(next_event);
        let mut follow_up: Option<Event> = None;
        for effect in effects {
            match effect {
                Effect::CallLlm {
                    iteration,
                    tools_enabled,
                    disabled_tools,
                } => {
                    // Sample context usage and let the machine emit any
                    // pressure warnings before the LLM is invoked. The
                    // generated `InjectUserMessage` is written straight to
                    // the tape so it becomes part of the next rebuild.
                    let (estimated_tokens, context_window_tokens) =
                        subsys.sample_context_usage().await;
                    for eff in
                        machine.observe_context_usage(estimated_tokens, context_window_tokens)
                    {
                        if let Effect::ContextPressureWarning {
                            level,
                            estimated_tokens: used,
                            context_window_tokens: window,
                        } = eff
                        {
                            let text = render_pressure_message(level, used, window);
                            subsys.inject_user_message(text.clone()).await;
                            subsys.emit_stream(text).await;
                        }
                    }
                    follow_up = Some(
                        subsys
                            .call_llm(iteration, tools_enabled, disabled_tools)
                            .await,
                    );
                }
                Effect::LoopBreakerTriggered {
                    disabled_tools,
                    pattern,
                    tool_calls_made,
                } => {
                    subsys
                        .loop_breaker_triggered(disabled_tools, pattern, tool_calls_made)
                        .await;
                }
                Effect::PauseForLimit {
                    limit_id,
                    tool_calls_made,
                } => {
                    follow_up = Some(subsys.pause_for_limit(limit_id, tool_calls_made).await);
                }
                Effect::RunTools { calls } => {
                    follow_up = Some(subsys.run_tools(calls).await);
                }
                Effect::AppendTape { kind } => subsys.append_tape(kind).await,
                Effect::InjectContinuationWake { turn, max } => {
                    let wake_msg = format!(
                        "[continuation:wake] Turn {turn}/{max}. The agent elected to continue \
                         working."
                    );
                    // Inject into conversation context so the LLM sees it.
                    subsys.inject_user_message(wake_msg.clone()).await;
                    // Also emit to stream for observability.
                    subsys.emit_stream(wake_msg).await;
                }
                Effect::EmitStream { kind } => subsys.emit_stream(kind).await,
                Effect::ContextPressureWarning {
                    level,
                    estimated_tokens,
                    context_window_tokens,
                } => {
                    // Handled inline during the CallLlm branch today; this
                    // arm keeps the match exhaustive in case `step` ever
                    // surfaces the effect directly.
                    let text =
                        render_pressure_message(level, estimated_tokens, context_window_tokens);
                    subsys.inject_user_message(text.clone()).await;
                    subsys.emit_stream(text).await;
                }
                Effect::Finish {
                    text,
                    iterations,
                    tool_calls,
                    ..
                } => {
                    outcome.text = text;
                    outcome.iterations = iterations;
                    outcome.tool_calls_made = tool_calls;
                    outcome.success = true;
                }
                Effect::Fail { message } => {
                    outcome.failure_message = Some(message);
                    outcome.success = false;
                }
            }
        }

        if machine.is_terminal() {
            return outcome;
        }
        // Sanity: a non-terminal step must always queue a follow-up event.
        // If it didn't, the machine has an unhandled (phase, event) pair —
        // we'd loop forever otherwise.
        next_event = follow_up.unwrap_or_else(|| {
            tracing::error!(
                phase = ?machine.phase(),
                "agent runner: machine in non-terminal phase but no follow-up event queued"
            );
            Event::Interrupted
        });
    }
}

/// Render the user-facing pressure-warning text the runner injects into the
/// tape. Kept in the runner (not the pure machine) so the wording stays an
/// I/O concern and the machine only tracks threshold crossings.
fn render_pressure_message(
    level: PressureLevel,
    estimated_tokens: usize,
    context_window_tokens: usize,
) -> String {
    let ratio = if context_window_tokens == 0 {
        0.0
    } else {
        estimated_tokens as f64 / context_window_tokens as f64
    };
    let pct = (ratio * 100.0).round() as u32;
    match level {
        PressureLevel::Warning => format!(
            "[context-pressure:warning] Context usage is at ~{pct}% \
             ({estimated_tokens}/{context_window_tokens} tokens). SHOULD begin summarising \
             long-tail details and prepare to hand off."
        ),
        PressureLevel::Critical => format!(
            "[context-pressure:critical] Context usage is at ~{pct}% \
             ({estimated_tokens}/{context_window_tokens} tokens). MUST hand off or summarise \
             before further tool calls."
        ),
    }
}

// Silence dead-code warnings until production wiring lands.
const _: fn() = || {
    let _ = Phase::Idle;
    let _: Option<ToolResult> = None;
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agent::effect::{TapeAppendKind, ToolCall as Tc, ToolCallId, ToolResult as Tr},
        tool::ToolName,
    };

    /// Pure in-memory subsystem stub for end-to-end runner tests.
    /// Scripts the LLM responses up front so each turn is fully deterministic.
    struct ScriptedSubsys {
        llm_script:      Vec<Event>,
        next_llm:        usize,
        tool_responses:  Vec<Vec<Tr>>,
        next_tool:       usize,
        tape_log:        Vec<TapeAppendKind>,
        stream_log:      Vec<String>,
        injected:        Vec<String>,
        /// Scripted user decisions for each `pause_for_limit` invocation,
        /// consumed in order. Leave empty for tests that never pause.
        limit_decisions: Vec<crate::agent::effect::LimitDecision>,
        next_limit:      usize,
        /// Scripted context-usage samples, consumed in order per CallLlm.
        /// A test can leave this empty to keep sampling off (zero window).
        context_samples: Vec<(usize, usize)>,
        next_sample:     usize,
    }

    #[async_trait]
    impl Subsystems for ScriptedSubsys {
        async fn call_llm(
            &mut self,
            _iteration: usize,
            _tools_enabled: bool,
            _disabled_tools: Vec<ToolName>,
        ) -> Event {
            let ev = self.llm_script[self.next_llm].clone();
            self.next_llm += 1;
            ev
        }

        async fn run_tools(&mut self, _calls: Vec<Tc>) -> Event {
            let results = self.tool_responses[self.next_tool].clone();
            self.next_tool += 1;
            Event::ToolsCompleted { results }
        }

        async fn append_tape(&mut self, kind: TapeAppendKind) { self.tape_log.push(kind); }

        async fn emit_stream(&mut self, kind: String) { self.stream_log.push(kind); }

        async fn pause_for_limit(&mut self, limit_id: u64, _tool_calls_made: usize) -> Event {
            let decision = self.limit_decisions[self.next_limit];
            self.next_limit += 1;
            Event::LimitResolved { limit_id, decision }
        }

        async fn inject_user_message(&mut self, text: String) { self.injected.push(text); }

        async fn sample_context_usage(&mut self) -> (usize, usize) {
            if self.next_sample < self.context_samples.len() {
                let s = self.context_samples[self.next_sample];
                self.next_sample += 1;
                s
            } else {
                (0, 0)
            }
        }
    }

    /// Factory producing a fresh stub with empty scripts for every field.
    fn subsys() -> ScriptedSubsys {
        ScriptedSubsys {
            llm_script:      vec![],
            next_llm:        0,
            tool_responses:  vec![],
            next_tool:       0,
            tape_log:        vec![],
            stream_log:      vec![],
            injected:        vec![],
            limit_decisions: vec![],
            next_limit:      0,
            context_samples: vec![],
            next_sample:     0,
        }
    }

    #[tokio::test]
    async fn drive_terminates_on_text_only_response() {
        let mut subsys = ScriptedSubsys {
            llm_script:      vec![Event::LlmCompleted {
                text:           "ok".into(),
                tool_calls:     vec![],
                has_tool_calls: false,
            }],
            next_llm:        0,
            tool_responses:  vec![],
            next_tool:       0,
            tape_log:        vec![],
            stream_log:      vec![],
            limit_decisions: vec![],
            next_limit:      0,
            injected:        vec![],
            context_samples: vec![],
            next_sample:     0,
        };
        let mut machine = AgentMachine::new(8);
        let outcome = drive(&mut machine, &mut subsys).await;
        assert!(outcome.success);
        assert_eq!(outcome.text, "ok");
        assert_eq!(outcome.iterations, 1);
        assert_eq!(subsys.tape_log, vec![TapeAppendKind::AssistantFinal]);
    }

    #[tokio::test]
    async fn drive_handles_one_tool_round_trip() {
        let tc = Tc {
            id:        ToolCallId::new("c1"),
            name:      ToolName::new("search"),
            arguments: "{}".into(),
        };
        let mut subsys = ScriptedSubsys {
            llm_script:      vec![
                Event::LlmCompleted {
                    text:           "thinking".into(),
                    tool_calls:     vec![tc.clone()],
                    has_tool_calls: true,
                },
                Event::LlmCompleted {
                    text:           "final".into(),
                    tool_calls:     vec![],
                    has_tool_calls: false,
                },
            ],
            next_llm:        0,
            tool_responses:  vec![vec![Tr {
                id:          ToolCallId::new("c1"),
                name:        ToolName::new("search"),
                arguments:   "{}".into(),
                success:     true,
                duration_ms: 5,
                error:       None,
            }]],
            next_tool:       0,
            tape_log:        vec![],
            stream_log:      vec![],
            limit_decisions: vec![],
            next_limit:      0,
            injected:        vec![],
            context_samples: vec![],
            next_sample:     0,
        };
        let mut machine = AgentMachine::new(8);
        let outcome = drive(&mut machine, &mut subsys).await;
        assert!(outcome.success);
        assert_eq!(outcome.text, "final");
        assert_eq!(outcome.tool_calls_made, 1);
        assert_eq!(
            subsys.tape_log,
            vec![
                TapeAppendKind::AssistantIntermediate,
                TapeAppendKind::ToolCalls,
                TapeAppendKind::ToolResults,
                TapeAppendKind::AssistantFinal,
            ]
        );
    }

    #[tokio::test]
    async fn drive_handles_continuation_wake() {
        let tc = Tc {
            id:        ToolCallId::new("c1"),
            name:      ToolName::new("continue-work"),
            arguments: r#"{"reason":"checking services"}"#.into(),
        };
        let mut subsys = ScriptedSubsys {
            llm_script:      vec![
                // Iteration 0: call continue-work
                Event::LlmCompleted {
                    text:           "checking...".into(),
                    tool_calls:     vec![tc.clone()],
                    has_tool_calls: true,
                },
                // Iteration 1: text-only (continuation fires, wake injected, another LLM call)
                Event::LlmCompleted {
                    text:           "still working".into(),
                    tool_calls:     vec![],
                    has_tool_calls: false,
                },
                // After wake: finishes for real
                Event::LlmCompleted {
                    text:           "all done".into(),
                    tool_calls:     vec![],
                    has_tool_calls: false,
                },
            ],
            next_llm:        0,
            tool_responses:  vec![vec![Tr {
                id:          ToolCallId::new("c1"),
                name:        ToolName::new("continue-work"),
                arguments:   r#"{"reason":"checking services"}"#.into(),
                success:     true,
                duration_ms: 1,
                error:       None,
            }]],
            next_tool:       0,
            tape_log:        vec![],
            stream_log:      vec![],
            limit_decisions: vec![],
            next_limit:      0,
            injected:        vec![],
            context_samples: vec![],
            next_sample:     0,
        };
        let mut machine = AgentMachine::with_max_continuations(8, 3);
        let outcome = drive(&mut machine, &mut subsys).await;
        assert!(outcome.success);
        assert_eq!(outcome.text, "all done");
        assert_eq!(machine.continuation_count(), 1);
        // Verify wake message was emitted
        assert!(
            subsys
                .stream_log
                .iter()
                .any(|s| s.contains("[continuation:wake]")),
            "expected continuation wake in stream log: {:?}",
            subsys.stream_log
        );
    }

    #[tokio::test]
    async fn drive_pauses_then_continues_on_limit() {
        use crate::agent::effect::LimitDecision;

        // Two tool waves with `limit_interval = 1` so the first wave trips
        // the circuit breaker. The user continues, the second wave runs,
        // and then the LLM wraps up.
        let tc_a = Tc {
            id:        ToolCallId::new("a"),
            name:      ToolName::new("search"),
            arguments: "{}".into(),
        };
        let tc_b = Tc {
            id:        ToolCallId::new("b"),
            name:      ToolName::new("search"),
            arguments: r#"{"q":"2"}"#.into(),
        };
        let mut subsys = ScriptedSubsys {
            llm_script:      vec![
                Event::LlmCompleted {
                    text:           "first".into(),
                    tool_calls:     vec![tc_a.clone()],
                    has_tool_calls: true,
                },
                Event::LlmCompleted {
                    text:           "second".into(),
                    tool_calls:     vec![tc_b.clone()],
                    has_tool_calls: true,
                },
                Event::LlmCompleted {
                    text:           "wrap".into(),
                    tool_calls:     vec![],
                    has_tool_calls: false,
                },
            ],
            next_llm:        0,
            tool_responses:  vec![
                vec![Tr {
                    id:          ToolCallId::new("a"),
                    name:        ToolName::new("search"),
                    arguments:   "{}".into(),
                    success:     true,
                    duration_ms: 1,
                    error:       None,
                }],
                vec![Tr {
                    id:          ToolCallId::new("b"),
                    name:        ToolName::new("search"),
                    arguments:   r#"{"q":"2"}"#.into(),
                    success:     true,
                    duration_ms: 1,
                    error:       None,
                }],
            ],
            next_tool:       0,
            tape_log:        vec![],
            stream_log:      vec![],
            limit_decisions: vec![LimitDecision::Continue, LimitDecision::Continue],
            next_limit:      0,
            injected:        vec![],
            context_samples: vec![],
            next_sample:     0,
        };
        let mut machine = AgentMachine::with_tool_call_limit(8, 1);
        let outcome = drive(&mut machine, &mut subsys).await;
        assert!(outcome.success);
        assert_eq!(outcome.text, "wrap");
        assert_eq!(outcome.tool_calls_made, 2);
        assert_eq!(subsys.next_limit, 2, "pause_for_limit fired once per wave");
    }

    #[tokio::test]
    async fn drive_stops_gracefully_when_user_declines_limit() {
        use crate::agent::effect::LimitDecision;

        let tc = Tc {
            id:        ToolCallId::new("a"),
            name:      ToolName::new("search"),
            arguments: "{}".into(),
        };
        let mut subsys = ScriptedSubsys {
            llm_script:      vec![Event::LlmCompleted {
                text:           "first".into(),
                tool_calls:     vec![tc.clone()],
                has_tool_calls: true,
            }],
            next_llm:        0,
            tool_responses:  vec![vec![Tr {
                id:          ToolCallId::new("a"),
                name:        ToolName::new("search"),
                arguments:   "{}".into(),
                success:     true,
                duration_ms: 1,
                error:       None,
            }]],
            next_tool:       0,
            tape_log:        vec![],
            stream_log:      vec![],
            limit_decisions: vec![LimitDecision::Stop],
            next_limit:      0,
            injected:        vec![],
            context_samples: vec![],
            next_sample:     0,
        };
        let mut machine = AgentMachine::with_tool_call_limit(8, 1);
        let outcome = drive(&mut machine, &mut subsys).await;
        assert!(outcome.success, "stop-by-limit is a graceful success");
        assert_eq!(outcome.text, "first");
        assert_eq!(outcome.tool_calls_made, 1);
    }

    #[tokio::test]
    async fn drive_propagates_llm_fatal_failure() {
        let mut subsys = ScriptedSubsys {
            llm_script:      vec![Event::LlmFailed {
                retryable: false,
                message:   "auth".into(),
            }],
            next_llm:        0,
            tool_responses:  vec![],
            next_tool:       0,
            tape_log:        vec![],
            stream_log:      vec![],
            limit_decisions: vec![],
            next_limit:      0,
            injected:        vec![],
            context_samples: vec![],
            next_sample:     0,
        };
        let mut machine = AgentMachine::new(8);
        let outcome = drive(&mut machine, &mut subsys).await;
        assert!(!outcome.success);
        assert!(outcome.failure_message.unwrap().contains("auth"));
    }

    // ─── Context pressure warnings (runner integration) ─────────────────

    /// When the sampled context usage crosses the Warning threshold on the
    /// first iteration, the runner must inject a pressure warning into the
    /// tape AND emit it on the stream before the LLM call returns.
    #[tokio::test]
    async fn drive_emits_pressure_warning_on_first_iteration() {
        let mut s = subsys();
        s.llm_script = vec![Event::LlmCompleted {
            text:           "ok".into(),
            tool_calls:     vec![],
            has_tool_calls: false,
        }];
        // 750 / 1000 = 0.75 → Warning.
        s.context_samples = vec![(750, 1_000)];

        let mut machine = AgentMachine::new(8);
        let outcome = drive(&mut machine, &mut s).await;
        assert!(outcome.success);
        assert_eq!(s.injected.len(), 1, "exactly one injection");
        assert!(
            s.injected[0].contains("[context-pressure:warning]"),
            "injection text missing marker: {:?}",
            s.injected[0]
        );
        assert!(
            s.stream_log
                .iter()
                .any(|line| line.contains("[context-pressure:warning]")),
            "stream log missing pressure warning: {:?}",
            s.stream_log
        );
    }

    /// Warning fires exactly once even when multiple iterations each sample
    /// pressure above the threshold — the one-shot latch lives in the
    /// machine.
    #[tokio::test]
    async fn drive_warning_is_one_shot_across_iterations() {
        let tc = Tc {
            id:        ToolCallId::new("c1"),
            name:      ToolName::new("search"),
            arguments: "{}".into(),
        };
        let mut s = subsys();
        s.llm_script = vec![
            Event::LlmCompleted {
                text:           "tool".into(),
                tool_calls:     vec![tc.clone()],
                has_tool_calls: true,
            },
            Event::LlmCompleted {
                text:           "final".into(),
                tool_calls:     vec![],
                has_tool_calls: false,
            },
        ];
        s.tool_responses = vec![vec![Tr {
            id:          ToolCallId::new("c1"),
            name:        ToolName::new("search"),
            arguments:   "{}".into(),
            success:     true,
            duration_ms: 1,
            error:       None,
        }]];
        // Both samples above 0.70 — should still only inject once.
        s.context_samples = vec![(750, 1_000), (800, 1_000)];

        let mut machine = AgentMachine::new(8);
        let outcome = drive(&mut machine, &mut s).await;
        assert!(outcome.success);
        assert_eq!(
            s.injected.len(),
            1,
            "Warning should fire exactly once across iterations, got: {:?}",
            s.injected
        );
    }

    /// Crossing from Warning into Critical across iterations produces two
    /// distinct injections — one per bucket.
    #[tokio::test]
    async fn drive_warning_then_critical_emits_two_injections() {
        let tc = Tc {
            id:        ToolCallId::new("c1"),
            name:      ToolName::new("search"),
            arguments: "{}".into(),
        };
        let mut s = subsys();
        s.llm_script = vec![
            Event::LlmCompleted {
                text:           "tool".into(),
                tool_calls:     vec![tc.clone()],
                has_tool_calls: true,
            },
            Event::LlmCompleted {
                text:           "final".into(),
                tool_calls:     vec![],
                has_tool_calls: false,
            },
        ];
        s.tool_responses = vec![vec![Tr {
            id:          ToolCallId::new("c1"),
            name:        ToolName::new("search"),
            arguments:   "{}".into(),
            success:     true,
            duration_ms: 1,
            error:       None,
        }]];
        // Iter 0: 0.75 → Warning. Iter 1: 0.90 → Critical.
        s.context_samples = vec![(750, 1_000), (900, 1_000)];

        let mut machine = AgentMachine::new(8);
        let outcome = drive(&mut machine, &mut s).await;
        assert!(outcome.success);
        assert_eq!(s.injected.len(), 2);
        assert!(s.injected[0].contains("[context-pressure:warning]"));
        assert!(s.injected[1].contains("[context-pressure:critical]"));
    }

    /// Sampling `(0, 0)` (unavailable / disabled) never produces a
    /// pressure injection.
    #[tokio::test]
    async fn drive_no_injection_when_sampling_disabled() {
        let mut s = subsys();
        s.llm_script = vec![Event::LlmCompleted {
            text:           "ok".into(),
            tool_calls:     vec![],
            has_tool_calls: false,
        }];
        // Default: context_samples is empty → returns (0, 0) each call.
        let mut machine = AgentMachine::new(8);
        let outcome = drive(&mut machine, &mut s).await;
        assert!(outcome.success);
        assert!(
            s.injected.is_empty(),
            "expected no injections, got: {:?}",
            s.injected
        );
    }
}
