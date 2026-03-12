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

//! Plan-execute architecture — v2 agent execution mode.
//!
//! Instead of a single reactive agent loop, the plan executor:
//! 1. Asks the LLM to produce a structured [`Plan`] from the user request.
//! 2. Executes each [`PlanStep`] sequentially, using `run_agent_loop` for
//!    inline steps (or `KernelHandle::spawn_child` for worker steps).
//! 3. Supports replanning when a step fails or requests revision.
//!
//! The entry point is [`run_plan_loop`], which has the same signature as
//! [`crate::agent::run_agent_loop`] so the kernel can route to either.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::{
    agent::AgentTurnResult,
    error::{KernelError, Result},
    guard::pipeline::GuardPipeline,
    handle::KernelHandle,
    io::{StreamEvent, StreamHandle},
    memory::{TapEntryKind, TapeService},
    notification::NotificationBusRef,
    session::SessionKey,
};

// ---------------------------------------------------------------------------
// Plan data structures
// ---------------------------------------------------------------------------

/// Plan status.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Active,
    Completed,
    Failed,
    Replanned,
}

/// Step execution mode.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    /// Execute in the main agent loop.
    Inline,
    /// Spawn an independent worker session.
    Worker,
}

impl Default for ExecutionMode {
    fn default() -> Self { Self::Inline }
}

/// Step outcome after execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum StepOutcome {
    Success,
    Failed { reason: String },
    NeedsReplan { reason: String },
}

impl StepOutcome {
    /// Returns a short label for stream event reporting.
    fn label(&self) -> &str {
        match self {
            Self::Success => "success",
            Self::Failed { .. } => "failed",
            Self::NeedsReplan { .. } => "needs_replan",
        }
    }
}

/// A planned step to be executed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    pub index:      usize,
    pub task:       String,
    #[serde(default)]
    pub mode:       ExecutionMode,
    pub acceptance: String,
}

/// A completed step with its result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PastStep {
    pub index:   usize,
    pub task:    String,
    pub summary: String,
    pub outcome: StepOutcome,
}

/// Plan intermediate representation, stored in tape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub goal:       String,
    pub steps:      Vec<PlanStep>,
    pub past_steps: Vec<PastStep>,
    pub status:     PlanStatus,
}

// ---------------------------------------------------------------------------
// Plan executor
// ---------------------------------------------------------------------------

/// Run the plan-execute loop (v2 execution mode).
///
/// This function has the same parameter list as `run_agent_loop` so the kernel
/// can dispatch to either based on routing rules.
///
/// # Execution phases
///
/// 1. **Plan phase** — call the LLM to produce a `Plan` from the user message.
/// 2. **Execute loop** — for each step, run an inline agent sub-turn or spawn
///    a worker child session.
/// 3. **Replan** — if a step fails or requests replan, revise the remaining
///    steps (currently stubbed).
/// 4. **Completion** — emit `PlanCompleted` and produce a final summary.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_plan_loop(
    handle: &KernelHandle,
    session_key: SessionKey,
    user_text: String,
    stream_handle: &StreamHandle,
    turn_cancel: &CancellationToken,
    tape: TapeService,
    tape_name: &str,
    tool_context: crate::tool::ToolContext,
    milestone_tx: Option<tokio::sync::mpsc::Sender<crate::io::AgentEvent>>,
    output_interceptor: crate::tool::DynamicOutputInterceptor,
    guard_pipeline: Arc<GuardPipeline>,
    notification_bus: NotificationBusRef,
) -> Result<AgentTurnResult> {
    info!(session_key = %session_key, "plan executor: starting v2 plan-execute loop");

    // -- Phase 1: Plan creation -----------------------------------------------
    //
    // In the full implementation this would call the LLM with a system prompt
    // instructing it to produce a Plan JSON via the `create_plan` tool.
    // For now we generate a single-step plan that wraps the entire user request
    // so the structure is exercised end-to-end.

    let plan = create_initial_plan(&user_text);

    // Persist plan to tape as a Plan entry.
    let plan_json = serde_json::to_value(&plan).map_err(|e| KernelError::AgentExecution {
        message: format!("failed to serialize plan: {e}"),
    })?;

    tape.store()
        .append(tape_name, TapEntryKind::Plan, plan_json, None)
        .await
        .map_err(|e| KernelError::AgentExecution {
            message: format!("failed to persist plan to tape: {e}"),
        })?;

    stream_handle.emit(StreamEvent::PlanCreated {
        goal:  plan.goal.clone(),
        steps: plan.steps.iter().map(|s| s.task.clone()).collect(),
    });

    // -- Phase 2: Execute steps -----------------------------------------------

    let mut past_steps: Vec<PastStep> = Vec::new();
    let mut plan = plan;
    let mut total_iterations = 0usize;
    let mut total_tool_calls = 0usize;
    let mut last_model = String::new();
    let mut final_texts: Vec<String> = Vec::new();

    for step in plan.steps.iter() {
        if turn_cancel.is_cancelled() {
            warn!(session_key = %session_key, step = step.index, "plan executor: cancelled");
            break;
        }

        let mode_label = match step.mode {
            ExecutionMode::Inline => "inline",
            ExecutionMode::Worker => "worker",
        };

        stream_handle.emit(StreamEvent::PlanStepStart {
            index: step.index,
            task:  step.task.clone(),
            mode:  mode_label.to_owned(),
        });

        let (outcome, summary) = match step.mode {
            ExecutionMode::Inline => {
                execute_inline_step(
                    handle,
                    session_key,
                    step,
                    stream_handle,
                    turn_cancel,
                    tape.clone(),
                    tape_name,
                    tool_context.clone(),
                    milestone_tx.clone(),
                    output_interceptor.clone(),
                    guard_pipeline.clone(),
                    notification_bus.clone(),
                    &mut total_iterations,
                    &mut total_tool_calls,
                    &mut last_model,
                    &mut final_texts,
                )
                .await
            }
            ExecutionMode::Worker => {
                // TODO: Implement worker mode using KernelHandle::spawn_child.
                let reason = "worker execution mode not yet implemented".to_owned();
                (StepOutcome::Failed { reason: reason.clone() }, reason)
            }
        };

        stream_handle.emit(StreamEvent::PlanStepEnd {
            index:   step.index,
            outcome: outcome.label().to_owned(),
            summary: summary.clone(),
        });

        let needs_replan = matches!(
            outcome,
            StepOutcome::Failed { .. } | StepOutcome::NeedsReplan { .. }
        );

        past_steps.push(PastStep {
            index:   step.index,
            task:    step.task.clone(),
            summary,
            outcome: outcome.clone(),
        });

        // -- Replan check -----------------------------------------------------
        if needs_replan {
            let reason = match &outcome {
                StepOutcome::Failed { reason } => {
                    format!("step {} failed: {}", step.index, reason)
                }
                StepOutcome::NeedsReplan { reason } => reason.clone(),
                _ => unreachable!(),
            };

            info!(
                session_key = %session_key,
                step = step.index,
                reason = %reason,
                "plan executor: replan triggered"
            );

            // TODO: Call LLM with past_steps + remaining steps to produce a
            // revised plan. For now, we abort the plan on failure.
            stream_handle.emit(StreamEvent::PlanReplan {
                reason:    reason.clone(),
                new_steps: vec![],
            });

            plan.status = PlanStatus::Failed;
            break;
        }
    }

    // -- Phase 3: Completion --------------------------------------------------

    if plan.status != PlanStatus::Failed {
        plan.status = PlanStatus::Completed;
    }
    plan.past_steps = past_steps;

    let summary = if plan.status == PlanStatus::Completed {
        if final_texts.is_empty() {
            format!("Plan completed: {}", plan.goal)
        } else {
            final_texts.join("\n\n")
        }
    } else {
        format!("Plan failed: {}", plan.goal)
    };

    stream_handle.emit(StreamEvent::PlanCompleted {
        summary: summary.clone(),
    });

    info!(
        session_key = %session_key,
        status = ?plan.status,
        steps = plan.steps.len(),
        past_steps = plan.past_steps.len(),
        "plan executor: finished"
    );

    let final_text_len = summary.len();
    Ok(AgentTurnResult {
        text:       summary,
        iterations: total_iterations,
        tool_calls: total_tool_calls,
        model:      last_model.clone(),
        trace:      crate::agent::TurnTrace {
            duration_ms:      0,
            model:            last_model,
            input_text:       Some(user_text),
            iterations:       vec![],
            final_text_len,
            total_tool_calls,
            success:          plan.status == PlanStatus::Completed,
            error:            if plan.status == PlanStatus::Failed {
                Some(format!("plan failed: {}", plan.goal))
            } else {
                None
            },
        },
    })
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Create an initial single-step plan from the user request.
///
/// In the full implementation, this would call the LLM to decompose the
/// request into multiple steps. For v1, we wrap the entire request as one
/// inline step so the execution structure is exercised.
fn create_initial_plan(user_text: &str) -> Plan {
    Plan {
        goal:       user_text.to_owned(),
        steps:      vec![PlanStep {
            index:      0,
            task:       user_text.to_owned(),
            mode:       ExecutionMode::Inline,
            acceptance: "task completed successfully".to_owned(),
        }],
        past_steps: Vec::new(),
        status:     PlanStatus::Active,
    }
}

/// Execute a single plan step inline using `run_agent_loop`.
///
/// Returns `(StepOutcome, summary_text)`.
#[allow(clippy::too_many_arguments)]
async fn execute_inline_step(
    handle: &KernelHandle,
    session_key: SessionKey,
    step: &PlanStep,
    stream_handle: &StreamHandle,
    turn_cancel: &CancellationToken,
    tape: TapeService,
    tape_name: &str,
    tool_context: crate::tool::ToolContext,
    milestone_tx: Option<tokio::sync::mpsc::Sender<crate::io::AgentEvent>>,
    output_interceptor: crate::tool::DynamicOutputInterceptor,
    guard_pipeline: Arc<GuardPipeline>,
    notification_bus: NotificationBusRef,
    total_iterations: &mut usize,
    total_tool_calls: &mut usize,
    last_model: &mut String,
    final_texts: &mut Vec<String>,
) -> (StepOutcome, String) {
    // Delegate to run_agent_loop with the step's task as the user text.
    let result = crate::agent::run_agent_loop(
        handle,
        session_key,
        step.task.clone(),
        stream_handle,
        turn_cancel,
        tape,
        tape_name,
        tool_context,
        milestone_tx,
        output_interceptor,
        guard_pipeline,
        notification_bus,
    )
    .await;

    match result {
        Ok(turn_result) => {
            *total_iterations += turn_result.iterations;
            *total_tool_calls += turn_result.tool_calls;
            *last_model = turn_result.model.clone();
            let summary = turn_result.text.clone();
            if !turn_result.text.is_empty() {
                final_texts.push(turn_result.text);
            }
            (StepOutcome::Success, summary)
        }
        Err(e) => {
            let reason = e.to_string();
            (StepOutcome::Failed { reason: reason.clone() }, reason)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_serialization_roundtrip() {
        let plan = Plan {
            goal:       "deploy the app".into(),
            steps:      vec![
                PlanStep {
                    index:      0,
                    task:       "build the binary".into(),
                    acceptance: "cargo build succeeds".into(),
                    mode:       ExecutionMode::Inline,
                },
                PlanStep {
                    index:      1,
                    task:       "run tests".into(),
                    acceptance: "all tests pass".into(),
                    mode:       ExecutionMode::Worker,
                },
            ],
            past_steps: vec![],
            status:     PlanStatus::Active,
        };

        let json = serde_json::to_string(&plan).unwrap();
        let parsed: Plan = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.goal, "deploy the app");
        assert_eq!(parsed.steps.len(), 2);
        assert_eq!(parsed.steps[0].mode, ExecutionMode::Inline);
        assert_eq!(parsed.steps[1].mode, ExecutionMode::Worker);
        assert_eq!(parsed.status, PlanStatus::Active);
    }

    #[test]
    fn create_initial_plan_wraps_user_text() {
        let plan = create_initial_plan("fix the login bug");
        assert_eq!(plan.goal, "fix the login bug");
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].task, "fix the login bug");
        assert_eq!(plan.steps[0].mode, ExecutionMode::Inline);
        assert_eq!(plan.status, PlanStatus::Active);
    }

    #[test]
    fn step_outcome_labels() {
        assert_eq!(StepOutcome::Success.label(), "success");
        assert_eq!(
            StepOutcome::Failed {
                reason: "boom".into()
            }
            .label(),
            "failed"
        );
        assert_eq!(
            StepOutcome::NeedsReplan {
                reason: "changed".into()
            }
            .label(),
            "needs_replan"
        );
    }
}
