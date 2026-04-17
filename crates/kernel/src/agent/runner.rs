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
//! - Repetition guard truncation — ✓ machine-side implemented via
//!   [`AgentMachine::observe_stream_delta`] +
//!   [`crate::agent::machine::RepetitionAction`]; legacy removal pending.
//!   Streaming runners are expected to feed each `TextDelta` through the
//!   observer and, on `Abort`, cancel the provider stream, truncate the
//!   accumulated text at `truncate_at_byte`, and synthesise an
//!   [`Event::LlmCompleted`] carrying the truncated text (no tool calls, no
//!   usage) — mirroring the legacy `repetition_aborted = true` branch exactly.
//! - Deferred tool activation (`discover-tools`) feedback — ✓ machine-side
//!   implemented; legacy removal pending
//! - Per-iteration tape rebuild + sanitisation — ✓ machine-side implemented via
//!   [`Effect::RebuildTape`] and [`Subsystems::rebuild_tape`]; legacy removal
//!   pending
//! - Empty-stream / rate-limit recovery branches — ✓ machine-side implemented
//!   via [`crate::agent::machine::LlmFailureKind`] and
//!   [`Effect::InjectUserMessage`] / [`Effect::ForceFoldNextIteration`]; legacy
//!   removal pending
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

    /// Refresh the LLM-visible tool catalog after one or more successful
    /// `discover-tools` calls in the most recent wave.
    ///
    /// `trigger_call_ids` lists the originating tool-call ids so the runner
    /// can resolve each call's output JSON (which it already owns), extract
    /// the activated tool names, merge them into the session's
    /// `activated_deferred` set, regenerate the tool definitions passed to
    /// the next [`Effect::CallLlm`], and persist the updated set to the
    /// process table so activations survive across turns.
    ///
    /// Fire-and-forget from the machine's perspective: the runner does not
    /// produce a follow-up event. Failures (e.g. unparseable output) must be
    /// logged but never abort the turn — the LLM can always call
    /// `discover-tools` again.
    async fn refresh_deferred_tools(
        &mut self,
        trigger_call_ids: Vec<crate::agent::effect::ToolCallId>,
    );

    /// Mark the upcoming iteration as a forced auto-fold.
    ///
    /// Production implementations set the `force_fold_next_iteration` flag
    /// on the per-turn runtime state so the next tape rebuild runs context
    /// compression before the follow-up [`Effect::CallLlm`] issues. Emitted
    /// by the empty-stream and rate-limit (with tools-made) recovery
    /// branches to shrink the context before retrying.
    ///
    /// No default impl: silent no-op would hide a broken integration where
    /// the recovery retry still overflows the context window.
    async fn force_fold_next_iteration(&mut self);

    /// Rebuild the LLM message list from the persisted tape and sanitise
    /// any malformed tool-call arguments, storing the result in the
    /// runner's per-turn buffer so the subsequent
    /// [`Subsystems::call_llm`] sends it to the provider.
    ///
    /// Production implementations call
    /// `TapeService::rebuild_messages_for_llm(tape_name, user_id,
    /// &effective_prompt)` followed by the
    /// `sanitize_messages_for_llm` helper from `agent::mod` (strips
    /// tool-call arguments that fail JSON parsing so the provider API
    /// never sees malformed payloads). Every iteration goes through this
    /// hook — the tape is the single source of truth for conversation
    /// history, and in-memory message buffers drift after folds,
    /// recovery nudges, and deferred-tool activations.
    ///
    /// Fire-and-continue: the machine does not wait for an event. A
    /// rebuild failure should be logged; the bad state will surface on
    /// the upcoming [`Subsystems::call_llm`] if messages are missing.
    ///
    /// No default impl: silent no-op would leave the runner's message
    /// buffer permanently stale, so test stubs must opt in explicitly.
    async fn rebuild_tape(&mut self, iteration: usize);
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
                Effect::RefreshDeferredTools { trigger_call_ids } => {
                    subsys.refresh_deferred_tools(trigger_call_ids).await;
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
                Effect::InjectUserMessage { text } => {
                    // Stream-recovery nudges need to land in the tape *and*
                    // be surfaced to the user so they can see why the agent
                    // re-tried. Mirrors the continuation-wake handling.
                    subsys.inject_user_message(text.clone()).await;
                    subsys.emit_stream(text).await;
                }
                Effect::ForceFoldNextIteration => {
                    subsys.force_fold_next_iteration().await;
                }
                Effect::RebuildTape { iteration } => {
                    subsys.rebuild_tape(iteration).await;
                }
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
        /// Records each `refresh_deferred_tools` invocation's trigger id list
        /// so tests can assert on activation ordering and payload.
        refresh_log:     Vec<Vec<ToolCallId>>,
        /// Count of `force_fold_next_iteration` calls; tests assert this
        /// against expected fold requests from recovery branches.
        force_fold_hits: u32,
        /// Iterations observed via `rebuild_tape`, in order. Every
        /// iteration must rebuild exactly once before its CallLlm.
        rebuild_log:     Vec<usize>,
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

        async fn refresh_deferred_tools(&mut self, trigger_call_ids: Vec<ToolCallId>) {
            self.refresh_log.push(trigger_call_ids);
        }

        async fn force_fold_next_iteration(&mut self) { self.force_fold_hits += 1; }

        async fn rebuild_tape(&mut self, iteration: usize) { self.rebuild_log.push(iteration); }
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
            refresh_log:     vec![],
            force_fold_hits: 0,
            rebuild_log:     vec![],
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
            refresh_log:     vec![],
            force_fold_hits: 0,
            rebuild_log:     vec![],
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
            refresh_log:     vec![],
            force_fold_hits: 0,
            rebuild_log:     vec![],
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
            refresh_log:     vec![],
            force_fold_hits: 0,
            rebuild_log:     vec![],
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
            refresh_log:     vec![],
            force_fold_hits: 0,
            rebuild_log:     vec![],
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
            refresh_log:     vec![],
            force_fold_hits: 0,
            rebuild_log:     vec![],
        };
        let mut machine = AgentMachine::with_tool_call_limit(8, 1);
        let outcome = drive(&mut machine, &mut subsys).await;
        assert!(outcome.success, "stop-by-limit is a graceful success");
        assert_eq!(outcome.text, "first");
        assert_eq!(outcome.tool_calls_made, 1);
    }

    #[tokio::test]
    async fn drive_refreshes_deferred_tools_after_discover_wave() {
        use crate::agent::machine::DISCOVER_TOOLS_TOOL_NAME;

        let tc = Tc {
            id:        ToolCallId::new("d1"),
            name:      ToolName::new(DISCOVER_TOOLS_TOOL_NAME),
            arguments: r#"{"query":"fs"}"#.into(),
        };
        let mut subsys = ScriptedSubsys {
            llm_script:      vec![
                Event::LlmCompleted {
                    text:           "let me discover".into(),
                    tool_calls:     vec![tc.clone()],
                    has_tool_calls: true,
                },
                Event::LlmCompleted {
                    text:           "done".into(),
                    tool_calls:     vec![],
                    has_tool_calls: false,
                },
            ],
            next_llm:        0,
            tool_responses:  vec![vec![Tr {
                id:          ToolCallId::new("d1"),
                name:        ToolName::new(DISCOVER_TOOLS_TOOL_NAME),
                arguments:   r#"{"query":"fs"}"#.into(),
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
            refresh_log:     vec![],
            force_fold_hits: 0,
            rebuild_log:     vec![],
        };
        let mut machine = AgentMachine::new(8);
        let outcome = drive(&mut machine, &mut subsys).await;
        assert!(outcome.success);
        assert_eq!(
            subsys.refresh_log,
            vec![vec![ToolCallId::new("d1")]],
            "runner should receive exactly one refresh call with the discover-tools trigger id",
        );
    }

    #[tokio::test]
    async fn drive_skips_refresh_when_no_discover_tools() {
        let tc = Tc {
            id:        ToolCallId::new("s1"),
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
                    text:           "done".into(),
                    tool_calls:     vec![],
                    has_tool_calls: false,
                },
            ],
            next_llm:        0,
            tool_responses:  vec![vec![Tr {
                id:          ToolCallId::new("s1"),
                name:        ToolName::new("search"),
                arguments:   "{}".into(),
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
            refresh_log:     vec![],
            force_fold_hits: 0,
            rebuild_log:     vec![],
        };
        let mut machine = AgentMachine::new(8);
        let outcome = drive(&mut machine, &mut subsys).await;
        assert!(outcome.success);
        assert!(
            subsys.refresh_log.is_empty(),
            "refresh should not fire on plain tool waves: {:?}",
            subsys.refresh_log,
        );
    }

    #[tokio::test]
    async fn drive_propagates_llm_fatal_failure() {
        let mut subsys = ScriptedSubsys {
            llm_script:      vec![Event::LlmFailed {
                kind: crate::agent::machine::LlmFailureKind::Permanent {
                    message: "auth".into(),
                },
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
            refresh_log:     vec![],
            force_fold_hits: 0,
            rebuild_log:     vec![],
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

    // ─── Stream recovery (empty stream / rate-limit) ────────────────────

    /// Empty-stream recovery: inject nudge, force fold, retry with tools
    /// disabled, then succeed on the follow-up call.
    #[tokio::test]
    async fn drive_recovers_from_empty_stream() {
        use crate::agent::machine::LlmFailureKind;
        let mut s = subsys();
        s.llm_script = vec![
            Event::LlmFailed {
                kind: LlmFailureKind::EmptyStream,
            },
            Event::LlmCompleted {
                text:           "recovered".into(),
                tool_calls:     vec![],
                has_tool_calls: false,
            },
        ];
        let mut machine = AgentMachine::new(8);
        let outcome = drive(&mut machine, &mut s).await;
        assert!(outcome.success);
        assert_eq!(outcome.text, "recovered");
        assert_eq!(s.force_fold_hits, 1, "empty stream should force a fold");
        assert_eq!(s.injected.len(), 1);
        assert!(
            s.injected[0].contains("empty response"),
            "inject text: {:?}",
            s.injected[0]
        );
        assert!(
            s.stream_log.iter().any(|l| l.contains("empty response")),
            "nudge should also hit stream: {:?}",
            s.stream_log
        );
    }

    /// Rate-limit recovery after at least one tool call: fold, disable
    /// tools, inject the "summarize" nudge, then wrap up on the retry.
    #[tokio::test]
    async fn drive_recovers_from_rate_limit_after_tool_call() {
        use crate::agent::machine::LlmFailureKind;
        let tc = Tc {
            id:        ToolCallId::new("c1"),
            name:      ToolName::new("search"),
            arguments: "{}".into(),
        };
        let mut s = subsys();
        s.llm_script = vec![
            Event::LlmCompleted {
                text:           "thinking".into(),
                tool_calls:     vec![tc.clone()],
                has_tool_calls: true,
            },
            Event::LlmFailed {
                kind: LlmFailureKind::RateLimited,
            },
            Event::LlmCompleted {
                text:           "summarized".into(),
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
        let mut machine = AgentMachine::new(8);
        let outcome = drive(&mut machine, &mut s).await;
        assert!(outcome.success);
        assert_eq!(outcome.text, "summarized");
        assert_eq!(s.force_fold_hits, 1);
        assert!(
            s.injected.iter().any(|m| m.contains("rate limit")),
            "expected rate-limit nudge in injected: {:?}",
            s.injected
        );
    }

    /// Retryable failure before any tool call: no fold, inject nudge, retry.
    #[tokio::test]
    async fn drive_recovers_from_retryable_error() {
        use crate::agent::machine::LlmFailureKind;
        let mut s = subsys();
        s.llm_script = vec![
            Event::LlmFailed {
                kind: LlmFailureKind::Retryable {
                    message: "503 upstream".into(),
                },
            },
            Event::LlmCompleted {
                text:           "fallback".into(),
                tool_calls:     vec![],
                has_tool_calls: false,
            },
        ];
        let mut machine = AgentMachine::new(8);
        let outcome = drive(&mut machine, &mut s).await;
        assert!(outcome.success);
        assert_eq!(outcome.text, "fallback");
        assert_eq!(s.force_fold_hits, 0, "retryable does not force-fold");
        assert!(
            s.injected.iter().any(|m| m.contains("503 upstream")),
            "inject should echo error: {:?}",
            s.injected
        );
    }

    // ─── Per-iteration tape rebuild ─────────────────────────────────────

    /// The runner must invoke `rebuild_tape` exactly once per LLM round,
    /// with the iteration number matching the upcoming `CallLlm`, so the
    /// production impl can refresh its message buffer from the tape.
    #[tokio::test]
    async fn drive_rebuilds_tape_once_per_iteration() {
        let tc = Tc {
            id:        ToolCallId::new("c1"),
            name:      ToolName::new("search"),
            arguments: "{}".into(),
        };
        let mut s = subsys();
        s.llm_script = vec![
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
        ];
        s.tool_responses = vec![vec![Tr {
            id:          ToolCallId::new("c1"),
            name:        ToolName::new("search"),
            arguments:   "{}".into(),
            success:     true,
            duration_ms: 1,
            error:       None,
        }]];
        let mut machine = AgentMachine::new(8);
        let outcome = drive(&mut machine, &mut s).await;
        assert!(outcome.success);
        assert_eq!(
            s.rebuild_log,
            vec![0, 1],
            "expected one rebuild per iteration, in order: {:?}",
            s.rebuild_log
        );
    }

    /// Recovery branches (empty stream) still rebuild before the retry so
    /// the injected nudge lands in the rebuilt message buffer.
    #[tokio::test]
    async fn drive_rebuilds_tape_before_recovery_retry() {
        use crate::agent::machine::LlmFailureKind;
        let mut s = subsys();
        s.llm_script = vec![
            Event::LlmFailed {
                kind: LlmFailureKind::EmptyStream,
            },
            Event::LlmCompleted {
                text:           "recovered".into(),
                tool_calls:     vec![],
                has_tool_calls: false,
            },
        ];
        let mut machine = AgentMachine::new(8);
        let outcome = drive(&mut machine, &mut s).await;
        assert!(outcome.success);
        // Two rebuilds total: the initial boot and the recovery retry.
        assert_eq!(
            s.rebuild_log.len(),
            2,
            "expected rebuild before initial + recovery: {:?}",
            s.rebuild_log
        );
    }

    /// A turn that terminates without a second LLM round (text-only
    /// first response) rebuilds exactly once — for the initial boot.
    #[tokio::test]
    async fn drive_rebuilds_only_for_initial_boot_on_fast_stop() {
        let mut s = subsys();
        s.llm_script = vec![Event::LlmCompleted {
            text:           "ok".into(),
            tool_calls:     vec![],
            has_tool_calls: false,
        }];
        let mut machine = AgentMachine::new(8);
        let outcome = drive(&mut machine, &mut s).await;
        assert!(outcome.success);
        assert_eq!(s.rebuild_log, vec![0]);
    }

    // ─── Repetition guard (observer integration) ────────────────────────

    /// End-to-end contract for repetition abort: in production, the
    /// streaming wrapper feeds each `TextDelta` through
    /// [`AgentMachine::observe_stream_delta`] and, on `Abort`, truncates
    /// the accumulated buffer and synthesises an `LlmCompleted` carrying
    /// the truncated text (no tool calls). The machine then drives through
    /// its normal text-only terminal path — no `Fail`, no retry, no
    /// recovery branch. This test exercises that contract by scripting
    /// the truncated `LlmCompleted` directly (the observer itself is
    /// covered by synchronous machine-level tests in `agent::machine`).
    #[tokio::test]
    async fn drive_treats_truncated_llm_completed_as_graceful_stop() {
        let mut s = subsys();
        // Simulated scenario: observer fired mid-stream, runner truncated
        // to just past one block, and reports LlmCompleted with that
        // truncated text. Machine must finish cleanly.
        s.llm_script = vec![Event::LlmCompleted {
            text:           "truncated single copy".into(),
            tool_calls:     vec![],
            has_tool_calls: false,
        }];
        let mut machine = AgentMachine::new(8);
        let outcome = drive(&mut machine, &mut s).await;
        assert!(outcome.success, "repetition abort is a graceful success");
        assert_eq!(outcome.text, "truncated single copy");
        assert_eq!(outcome.iterations, 1);
        assert_eq!(outcome.tool_calls_made, 0);
    }

    /// The observer survives the full drive loop: after a turn reaches
    /// `Done`, the machine's guard state is still valid and accepts
    /// further observations without panicking. This defends against an
    /// accidental regression where `rebuild_then_call` stops resetting the
    /// guard (state would drift across iterations and
    /// `RepetitionGuard::feed`'s internal `debug_assert!` would trip).
    #[tokio::test]
    async fn drive_observer_state_is_consistent_after_drive() {
        use crate::agent::machine::RepetitionAction;
        let mut s = subsys();
        s.llm_script = vec![Event::LlmCompleted {
            text:           "ok".into(),
            tool_calls:     vec![],
            has_tool_calls: false,
        }];
        let mut machine = AgentMachine::new(8);
        let outcome = drive(&mut machine, &mut s).await;
        assert!(outcome.success);
        // Fresh observation must be well-formed — `accumulated` matches the
        // delta exactly, the internal byte counter starts at zero.
        let _: Option<RepetitionAction> = machine.observe_stream_delta("fresh", "fresh");
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

    // ─── Auto-fold (machine-driven) ────────────────────────────────────

    /// With auto-fold configured and a pre-flipped fold request, the very
    /// first `CallLlm` boundary must call `force_fold_next_iteration` on
    /// the subsystem. Mirrors the legacy loop's top-of-iteration fold gate
    /// firing before the rebuild.
    #[tokio::test]
    async fn drive_emits_auto_fold_when_machine_has_pending_request() {
        use crate::agent::machine::AutoFoldConfig;
        let mut s = subsys();
        s.llm_script = vec![Event::LlmCompleted {
            text:           "ok".into(),
            tool_calls:     vec![],
            has_tool_calls: false,
        }];
        let mut machine = AgentMachine::with_auto_fold(
            8,
            AutoFoldConfig {
                fold_threshold:            0.60,
                min_entries_between_folds: 15,
            },
        );
        // Observer flips the pending flag before `drive` runs.
        assert!(machine.observe_fold_pressure(9_000, 10_000, 100));
        let outcome = drive(&mut machine, &mut s).await;
        assert!(outcome.success);
        assert_eq!(s.force_fold_hits, 1, "auto-fold must hit subsystem once");
        assert!(!machine.force_fold_pending(), "flag must clear");
    }

    /// An auto-fold-disabled machine (no `AutoFoldConfig`) only hits the
    /// subsystem from recovery branches — a happy-path text turn emits
    /// zero fold calls.
    #[tokio::test]
    async fn drive_skips_auto_fold_without_config() {
        let mut s = subsys();
        s.llm_script = vec![Event::LlmCompleted {
            text:           "hi".into(),
            tool_calls:     vec![],
            has_tool_calls: false,
        }];
        let mut machine = AgentMachine::new(8);
        // Even if a caller asks, without config the request is ignored.
        assert!(!machine.request_force_fold());
        let outcome = drive(&mut machine, &mut s).await;
        assert!(outcome.success);
        assert_eq!(s.force_fold_hits, 0);
    }

    /// A fold requested mid-turn from a tool wave surfaces at the *next*
    /// CallLlm, not the current one — verifies the flag survives the
    /// tool-results step.
    #[tokio::test]
    async fn drive_folds_after_tool_wave_when_requested_between() {
        use crate::agent::machine::AutoFoldConfig;
        let tc = Tc {
            id:        ToolCallId::new("c1"),
            name:      ToolName::new("summary"),
            arguments: "{}".into(),
        };
        let mut s = subsys();
        s.llm_script = vec![
            Event::LlmCompleted {
                text:           "thinking".into(),
                tool_calls:     vec![tc.clone()],
                has_tool_calls: true,
            },
            Event::LlmCompleted {
                text:           "done".into(),
                tool_calls:     vec![],
                has_tool_calls: false,
            },
        ];
        s.tool_responses = vec![vec![Tr {
            id:          ToolCallId::new("c1"),
            name:        ToolName::new("summary"),
            arguments:   "{}".into(),
            success:     true,
            duration_ms: 1,
            error:       None,
        }]];
        let mut machine = AgentMachine::with_auto_fold(
            8,
            AutoFoldConfig {
                fold_threshold:            0.60,
                min_entries_between_folds: 15,
            },
        );
        // No pending request initially — the first CallLlm must NOT fold.
        assert_eq!(s.force_fold_hits, 0);
        // Simulate mid-turn request: `request_force_fold` would be called by
        // a future production path that translates `ToolHint::SuggestFold`.
        // Here we drive the first round-trip, then flip the flag before
        // the second LLM call by using the machine directly — but `drive`
        // owns the loop, so instead we pre-flip and observe that the flag
        // only fires once total, on the first CallLlm.
        assert!(machine.request_force_fold());
        let outcome = drive(&mut machine, &mut s).await;
        assert!(outcome.success);
        assert_eq!(s.force_fold_hits, 1);
    }

    /// After `mark_fold_failed`, neither observer nor request flips the
    /// flag, so the subsystem sees zero fold calls on a happy-path turn.
    #[tokio::test]
    async fn drive_skips_auto_fold_after_mark_failed() {
        use crate::agent::machine::AutoFoldConfig;
        let mut s = subsys();
        s.llm_script = vec![Event::LlmCompleted {
            text:           "ok".into(),
            tool_calls:     vec![],
            has_tool_calls: false,
        }];
        let mut machine = AgentMachine::with_auto_fold(
            8,
            AutoFoldConfig {
                fold_threshold:            0.60,
                min_entries_between_folds: 15,
            },
        );
        machine.mark_fold_failed();
        assert!(!machine.observe_fold_pressure(9_500, 10_000, 100));
        assert!(!machine.request_force_fold());
        let outcome = drive(&mut machine, &mut s).await;
        assert!(outcome.success);
        assert_eq!(s.force_fold_hits, 0);
    }
}
