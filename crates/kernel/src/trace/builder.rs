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

//! Incremental [`ExecutionTrace`] assembly.
//!
//! [`TraceBuilder`] observes the same [`StreamEvent`]s the agent runner emits
//! to channel adapters and accumulates the data needed for the persisted turn
//! summary. The builder is attached to the per-turn
//! [`StreamHandle`](crate::io::StreamHandle) so emission is a single call
//! site — the handle fans the event out to both broadcast subscribers and
//! the builder.
//!
//! The accumulation semantics match what the Telegram adapter used to do
//! inline in its progress-message bookkeeping (see git history for
//! `crates/channels/src/telegram/adapter.rs` pre-#1613), so trace contents
//! are visually identical for users regardless of channel:
//!
//! - `thinking_preview` is hard-truncated to 500 chars (char count, not bytes)
//!   to bound memory on long reasoning streams.
//! - `plan_steps` are saved as display strings (icon + step text) when
//!   `PlanCompleted` fires, matching the Telegram "📋 Plan" section.
//! - Tool entries derive `name` and `summary` from
//!   [`tool_display::tool_display_info`], producing the same shortened labels
//!   the live progress UI uses.
//!
//! [`tool_display`]: super::tool_display
//! [`tool_display::tool_display_info`]: super::tool_display::tool_display_info

use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use super::{ExecutionTrace, ToolTraceEntry, tool_display};
use crate::io::{PlanStepStatus, StreamEvent};

/// Hard cap on `thinking_preview` character count.
///
/// Matches the legacy Telegram-adapter limit. Keeps the JSON blob stored in
/// `execution_traces.trace_data` bounded regardless of how long the model
/// reasons, while still carrying enough text for the expanded detail view.
const THINKING_PREVIEW_MAX_CHARS: usize = 500;

/// Collector that accumulates trace fields across a single agent turn.
///
/// Construction: [`TraceBuilder::new`] at turn start (wall-clock timer
/// begins here). Observation: [`observe`](Self::observe) is called for
/// every [`StreamEvent`] the kernel emits. Finalization:
/// [`finalize`](Self::finalize) consumes the builder and returns a fully
/// populated [`ExecutionTrace`].
///
/// All mutation is behind a single [`Mutex`]. `observe` is cheap (no I/O,
/// small struct updates), so contention is negligible even at the peak
/// hundreds-of-deltas-per-second the LLM streams produce.
pub struct TraceBuilder {
    /// Wall-clock start of the turn. Used to compute `duration_secs` at
    /// finalize. Captured once at construction so partial-success traces
    /// still carry an accurate wall time even if the builder outlives
    /// the agent loop (e.g. a late error path).
    turn_started: Instant,
    state:        Mutex<State>,
}

/// Mutable accumulators. Private so the only valid mutation path is
/// [`TraceBuilder::observe`].
#[derive(Default)]
struct State {
    model:            String,
    iterations:       usize,
    input_tokens:     u32,
    output_tokens:    u32,
    thinking_ms:      u64,
    thinking_preview: String,
    turn_rationale:   Option<String>,
    plan_steps:       Vec<PlanStepAccum>,
    plan_completed:   bool,
    plan_summary:     String,
    tools:            Vec<ToolAccum>,
}

/// Per-step accumulation used to render the final `plan_steps` display list.
///
/// The kernel emits `PlanProgress` with `current_step: usize` + a human
/// `status_text` (e.g. `"第1步：research docs"`). We store just enough to
/// reproduce the TG format (`"{icon} 第{N}步：{task}"`).
struct PlanStepAccum {
    task:   String,
    status: PlanStepAccumStatus,
}

#[derive(Clone, Copy)]
enum PlanStepAccumStatus {
    Pending,
    Running,
    Done,
    Failed,
}

/// Per-tool accumulation. `started_at` is used to compute `duration_ms`
/// when the matching `ToolCallEnd` arrives, since `StreamEvent` does not
/// carry the start time.
struct ToolAccum {
    id:          String,
    /// The shortened display name (`tool_display_info.0`).
    name:        String,
    /// First-line summary of arguments (`tool_display_info.1`).
    summary:     String,
    started_at:  Instant,
    finished:    bool,
    success:     bool,
    duration_ms: Option<u64>,
    error:       Option<String>,
}

impl TraceBuilder {
    /// Start a new builder. The wall-clock timer begins now.
    pub fn new() -> Self {
        Self {
            turn_started: Instant::now(),
            state:        Mutex::new(State::default()),
        }
    }

    /// Observe a single stream event. Called by
    /// [`StreamHandle::emit`](crate::io::StreamHandle::emit) before
    /// broadcasting to subscribers.
    ///
    /// Events we do not consume (e.g. `TextDelta`, `Progress`, background
    /// task notifications) are silently ignored — the trace summarizes the
    /// LLM-level turn, not every UI ping.
    pub fn observe(&self, event: &StreamEvent) {
        let mut st = match self.state.lock() {
            Ok(g) => g,
            // A poisoned lock here means we panicked in another `observe`
            // call. The trace is best-effort observability, so we prefer
            // to drop data silently over propagating the panic into the
            // agent loop.
            Err(poisoned) => poisoned.into_inner(),
        };

        match event {
            StreamEvent::TurnStarted { model, .. } => {
                if st.model.is_empty() {
                    st.model = model.clone();
                }
            }
            StreamEvent::TurnMetrics {
                model, iterations, ..
            } => {
                st.model = model.clone();
                st.iterations = *iterations;
            }
            StreamEvent::UsageUpdate {
                input_tokens,
                output_tokens,
                thinking_ms,
            } => {
                st.input_tokens = *input_tokens;
                st.output_tokens = *output_tokens;
                st.thinking_ms = *thinking_ms;
            }
            StreamEvent::ReasoningDelta { text } => {
                append_capped_chars(&mut st.thinking_preview, text, THINKING_PREVIEW_MAX_CHARS);
            }
            StreamEvent::TurnRationale { text } => {
                st.turn_rationale = Some(text.clone());
            }
            StreamEvent::ToolCallStart {
                name,
                id,
                arguments,
            } => {
                let (display_name, summary) = tool_display::tool_display_info(name, arguments);
                st.tools.push(ToolAccum {
                    id: id.clone(),
                    name: display_name,
                    summary,
                    started_at: Instant::now(),
                    finished: false,
                    success: false,
                    duration_ms: None,
                    error: None,
                });
            }
            StreamEvent::ToolCallEnd {
                id, success, error, ..
            } => {
                if let Some(t) = st.tools.iter_mut().find(|t| t.id == *id) {
                    t.finished = true;
                    t.success = *success;
                    t.duration_ms = Some(t.started_at.elapsed().as_millis() as u64);
                    t.error = error.clone();
                }
            }
            StreamEvent::PlanCreated { total_steps, .. } => {
                st.plan_steps = (0..*total_steps)
                    .map(|_| PlanStepAccum {
                        task:   String::new(),
                        status: PlanStepAccumStatus::Pending,
                    })
                    .collect();
            }
            StreamEvent::PlanProgress {
                current_step,
                step_status,
                status_text,
                ..
            } => {
                while *current_step >= st.plan_steps.len() {
                    st.plan_steps.push(PlanStepAccum {
                        task:   String::new(),
                        status: PlanStepAccumStatus::Pending,
                    });
                }
                let step = &mut st.plan_steps[*current_step];
                step.status = match step_status {
                    PlanStepStatus::Running => PlanStepAccumStatus::Running,
                    PlanStepStatus::Done => PlanStepAccumStatus::Done,
                    PlanStepStatus::Failed { .. } | PlanStepStatus::NeedsReplan { .. } => {
                        PlanStepAccumStatus::Failed
                    }
                };
                if step.task.is_empty() {
                    // `status_text` is a human string like "第1步：research docs…".
                    // Strip the "第N步：" prefix (U+FF1A fullwidth colon) to
                    // isolate the task, mirroring the TG adapter.
                    let task = match status_text.find('\u{ff1a}') {
                        Some(pos) => status_text[pos + '\u{ff1a}'.len_utf8()..]
                            .trim_end_matches('\u{2026}')
                            .to_string(),
                        None => status_text.clone(),
                    };
                    step.task = task;
                }
            }
            StreamEvent::PlanCompleted { summary } => {
                // Mark any still-Running steps as Done (matches TG).
                for step in &mut st.plan_steps {
                    if matches!(step.status, PlanStepAccumStatus::Running) {
                        step.status = PlanStepAccumStatus::Done;
                    }
                }
                st.plan_completed = true;
                st.plan_summary = summary.clone();
            }
            // Events we explicitly ignore — either not trace-relevant or
            // covered by a sibling event above.
            StreamEvent::TextDelta { .. }
            | StreamEvent::TextClear
            | StreamEvent::ToolOutput { .. }
            | StreamEvent::Progress { .. }
            | StreamEvent::BackgroundTaskStarted { .. }
            | StreamEvent::BackgroundTaskDone { .. }
            | StreamEvent::TurnUsage { .. }
            | StreamEvent::PlanReplan { .. }
            | StreamEvent::DockTurnComplete { .. }
            | StreamEvent::ToolCallLimit { .. }
            | StreamEvent::ToolCallLimitResolved { .. }
            | StreamEvent::LoopBreakerTriggered { .. }
            | StreamEvent::TraceReady { .. }
            | StreamEvent::Attachment { .. }
            | StreamEvent::StreamClosed { .. } => {}
        }
    }

    /// Produce the final [`ExecutionTrace`] by draining the accumulators.
    ///
    /// Takes `&self` (not `self`) because the builder is shared through
    /// an [`Arc`](std::sync::Arc) with
    /// [`StreamHandle`](crate::io::StreamHandle); the stream handle is still
    /// live when the turn task calls `finalize`, so ownership cannot be
    /// transferred back out of the `Arc`. Internally the state is moved
    /// out via [`std::mem::take`]; calling `finalize` a second time
    /// would yield an empty trace, which is acceptable because the
    /// single caller (the kernel turn task) invokes it exactly once.
    ///
    /// `rara_message_id` is the [`crate::io::MessageId`] string form that
    /// correlates this turn back to the triggering inbound message; it is
    /// passed in rather than observed because it originates outside the
    /// stream (from the dispatch metadata).
    pub fn finalize(&self, rara_message_id: String) -> ExecutionTrace {
        let duration_secs = self.turn_started.elapsed().as_secs();
        let State {
            model,
            iterations,
            input_tokens,
            output_tokens,
            thinking_ms,
            thinking_preview,
            turn_rationale,
            plan_steps: plan_accum,
            plan_completed,
            plan_summary,
            tools: tool_accum,
        } = match self.state.lock() {
            Ok(mut g) => std::mem::take(&mut *g),
            Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
        };

        let mut plan_steps: Vec<String> = plan_accum
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let icon = match s.status {
                    PlanStepAccumStatus::Done => "\u{2705}",     // ✅
                    PlanStepAccumStatus::Failed => "\u{274c}",   // ❌
                    PlanStepAccumStatus::Running => "\u{1f7e1}", // 🟡
                    PlanStepAccumStatus::Pending => "\u{2b1c}",  // ⬜
                };
                format!("{icon} \u{7b2c}{}\u{6b65}\u{ff1a}{}", i + 1, s.task)
            })
            .collect();
        if plan_completed && !plan_summary.is_empty() {
            plan_steps.push(format!("\u{2705} {plan_summary}"));
        }

        let tools: Vec<ToolTraceEntry> = tool_accum
            .into_iter()
            .map(|t| ToolTraceEntry {
                name:        t.name,
                duration_ms: t.duration_ms,
                success:     t.success,
                summary:     t.summary,
                error:       t.error,
            })
            .collect();

        ExecutionTrace {
            duration_secs,
            iterations,
            model,
            input_tokens,
            output_tokens,
            thinking_ms,
            thinking_preview,
            plan_steps,
            turn_rationale,
            tools,
            rara_message_id,
        }
    }

    /// Elapsed wall-clock time since this builder was created.
    ///
    /// Exposed so the caller can sanity-log turn duration without having
    /// to finalize prematurely.
    #[allow(dead_code)]
    pub fn elapsed(&self) -> Duration { self.turn_started.elapsed() }
}

impl Default for TraceBuilder {
    fn default() -> Self { Self::new() }
}

/// Append `src` to `dst`, stopping once `dst` reaches `max_chars`.
///
/// Operates on char count (not byte count) so multi-byte UTF-8 (中文, emoji)
/// does not accidentally slice a code point in half. When the cap is hit
/// mid-append, a horizontal-ellipsis `…` is pushed to signal truncation.
fn append_capped_chars(dst: &mut String, src: &str, max_chars: usize) {
    let current = dst.chars().count();
    if current >= max_chars {
        return;
    }
    let remaining = max_chars - current;
    let src_chars = src.chars().count();
    if src_chars <= remaining {
        dst.push_str(src);
    } else {
        for c in src.chars().take(remaining) {
            dst.push(c);
        }
        dst.push('\u{2026}');
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::io::{MessageId, PlanStepStatus};

    fn mid() -> String { MessageId::new().to_string() }

    #[test]
    fn empty_builder_produces_zero_trace() {
        let b = TraceBuilder::new();
        let trace = b.finalize(mid());
        assert_eq!(trace.iterations, 0);
        assert_eq!(trace.tools.len(), 0);
        assert!(trace.plan_steps.is_empty());
        assert!(trace.thinking_preview.is_empty());
    }

    #[test]
    fn observes_turn_metrics_and_usage() {
        let b = TraceBuilder::new();
        b.observe(&StreamEvent::TurnStarted {
            model:                 "gpt-5".into(),
            context_window_tokens: Some(128_000),
        });
        b.observe(&StreamEvent::UsageUpdate {
            input_tokens:  1_200,
            output_tokens: 340,
            thinking_ms:   1_500,
        });
        b.observe(&StreamEvent::TurnMetrics {
            duration_ms:           10_000,
            iterations:            3,
            tool_calls:            2,
            model:                 "gpt-5".into(),
            rara_message_id:       "ignored".into(),
            context_window_tokens: Some(128_000),
        });
        let trace = b.finalize(mid());
        assert_eq!(trace.model, "gpt-5");
        assert_eq!(trace.iterations, 3);
        assert_eq!(trace.input_tokens, 1_200);
        assert_eq!(trace.output_tokens, 340);
        assert_eq!(trace.thinking_ms, 1_500);
    }

    #[test]
    fn reasoning_delta_caps_preview_with_ellipsis() {
        let b = TraceBuilder::new();
        let long: String = "a".repeat(THINKING_PREVIEW_MAX_CHARS + 200);
        b.observe(&StreamEvent::ReasoningDelta { text: long });
        let trace = b.finalize(mid());
        assert_eq!(
            trace.thinking_preview.chars().count(),
            THINKING_PREVIEW_MAX_CHARS + 1, // +1 for the … marker
        );
        assert!(trace.thinking_preview.ends_with('\u{2026}'));
    }

    #[test]
    fn tool_round_trip_records_duration_and_name() {
        let b = TraceBuilder::new();
        b.observe(&StreamEvent::ToolCallStart {
            name:      "web_search".into(),
            id:        "call-1".into(),
            arguments: json!({"query": "rust async"}),
        });
        // Small sleep is unnecessary — `started_at.elapsed()` will still
        // produce a small positive duration on ToolCallEnd.
        b.observe(&StreamEvent::ToolCallEnd {
            id:             "call-1".into(),
            result_preview: String::new(),
            success:        true,
            error:          None,
        });
        let trace = b.finalize(mid());
        assert_eq!(trace.tools.len(), 1);
        let t = &trace.tools[0];
        assert_eq!(t.name, "search");
        assert_eq!(t.summary, "rust async");
        assert!(t.success);
        assert!(t.duration_ms.is_some());
    }

    #[test]
    fn plan_lifecycle_builds_step_strings() {
        let b = TraceBuilder::new();
        b.observe(&StreamEvent::PlanCreated {
            goal:                    "reach green CI".into(),
            total_steps:             2,
            compact_summary:         String::new(),
            estimated_duration_secs: None,
        });
        b.observe(&StreamEvent::PlanProgress {
            current_step: 0,
            total_steps:  2,
            step_status:  PlanStepStatus::Running,
            status_text:  "\u{7b2c}1\u{6b65}\u{ff1a}compile".into(),
        });
        b.observe(&StreamEvent::PlanProgress {
            current_step: 0,
            total_steps:  2,
            step_status:  PlanStepStatus::Done,
            status_text:  "\u{7b2c}1\u{6b65}\u{ff1a}compile".into(),
        });
        b.observe(&StreamEvent::PlanProgress {
            current_step: 1,
            total_steps:  2,
            step_status:  PlanStepStatus::Running,
            status_text:  "\u{7b2c}2\u{6b65}\u{ff1a}test".into(),
        });
        b.observe(&StreamEvent::PlanCompleted {
            summary: "all good".into(),
        });
        let trace = b.finalize(mid());
        assert_eq!(trace.plan_steps.len(), 3); // 2 steps + completion line
        assert!(trace.plan_steps[0].contains("compile"));
        assert!(trace.plan_steps[1].contains("test"));
        assert!(trace.plan_steps[2].contains("all good"));
    }

    #[test]
    fn rationale_captured() {
        let b = TraceBuilder::new();
        b.observe(&StreamEvent::TurnRationale {
            text: "checking logs".into(),
        });
        let trace = b.finalize(mid());
        assert_eq!(trace.turn_rationale.as_deref(), Some("checking logs"));
    }
}
