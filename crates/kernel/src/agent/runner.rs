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
//! - Loop breaker (`crate::agent::loop_breaker`) interventions
//! - Context pressure warnings + session-length reminders injected as user
//!   messages
//! - Tool-call-limit circuit breaker with oneshot resume
//! - Repetition guard truncation
//! - Deferred tool activation (`discover-tools`) feedback
//! - Per-iteration tape rebuild + sanitisation
//! - Empty-stream / rate-limit recovery branches
//! - Cascade trace assembly + mood inference
//!
//! Each item maps to either an additional [`Effect`] variant or extra fields
//! on [`AgentMachine`].

use async_trait::async_trait;

use crate::agent::{
    effect::{Effect, ToolCall, ToolResult},
    machine::{AgentMachine, Event, Phase},
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
    async fn call_llm(&mut self, iteration: usize, tools_enabled: bool) -> Event;

    /// Execute a wave of tool calls, producing the next event.
    async fn run_tools(&mut self, calls: Vec<ToolCall>) -> Event;

    /// Persist some structured payload to the tape.  Failures here are
    /// best-effort — they should be logged but never abort the turn.
    async fn append_tape(&mut self, kind: crate::agent::effect::TapeAppendKind);

    /// Forward a stream event to the user-facing transport.
    async fn emit_stream(&mut self, kind: String);
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
                } => {
                    follow_up = Some(subsys.call_llm(iteration, tools_enabled).await);
                }
                Effect::RunTools { calls } => {
                    follow_up = Some(subsys.run_tools(calls).await);
                }
                Effect::AppendTape { kind } => subsys.append_tape(kind).await,
                Effect::InjectContinuationWake { turn, max } => {
                    subsys
                        .emit_stream(format!(
                            "[continuation:wake] Turn {turn}/{max}. The agent elected to continue \
                             working."
                        ))
                        .await;
                }
                Effect::EmitStream { kind } => subsys.emit_stream(kind).await,
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
        llm_script:     Vec<Event>,
        next_llm:       usize,
        tool_responses: Vec<Vec<Tr>>,
        next_tool:      usize,
        tape_log:       Vec<TapeAppendKind>,
        stream_log:     Vec<String>,
    }

    #[async_trait]
    impl Subsystems for ScriptedSubsys {
        async fn call_llm(&mut self, _iteration: usize, _tools_enabled: bool) -> Event {
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
    }

    #[tokio::test]
    async fn drive_terminates_on_text_only_response() {
        let mut subsys = ScriptedSubsys {
            llm_script:     vec![Event::LlmCompleted {
                text:           "ok".into(),
                tool_calls:     vec![],
                has_tool_calls: false,
            }],
            next_llm:       0,
            tool_responses: vec![],
            next_tool:      0,
            tape_log:       vec![],
            stream_log:     vec![],
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
            llm_script:     vec![
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
            next_llm:       0,
            tool_responses: vec![vec![Tr {
                id:          ToolCallId::new("c1"),
                name:        ToolName::new("search"),
                success:     true,
                duration_ms: 5,
                error:       None,
            }]],
            next_tool:      0,
            tape_log:       vec![],
            stream_log:     vec![],
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
            llm_script:     vec![
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
            next_llm:       0,
            tool_responses: vec![vec![Tr {
                id:          ToolCallId::new("c1"),
                name:        ToolName::new("continue-work"),
                success:     true,
                duration_ms: 1,
                error:       None,
            }]],
            next_tool:      0,
            tape_log:       vec![],
            stream_log:     vec![],
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
    async fn drive_propagates_llm_fatal_failure() {
        let mut subsys = ScriptedSubsys {
            llm_script:     vec![Event::LlmFailed {
                retryable: false,
                message:   "auth".into(),
            }],
            next_llm:       0,
            tool_responses: vec![],
            next_tool:      0,
            tape_log:       vec![],
            stream_log:     vec![],
        };
        let mut machine = AgentMachine::new(8);
        let outcome = drive(&mut machine, &mut subsys).await;
        assert!(!outcome.success);
        assert!(outcome.failure_message.unwrap().contains("auth"));
    }
}
