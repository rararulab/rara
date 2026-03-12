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

/// The execution mode for a plan step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    /// Execute inline within the current session's agent loop.
    Inline,
    /// Spawn an independent child session via `KernelHandle::spawn_child`.
    Worker,
}

/// Outcome of executing a single plan step.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepOutcome {
    /// Step completed successfully.
    Success { summary: String },
    /// Step failed.
    Failed { error: String },
    /// Step requests a replan (the remaining steps may no longer be valid).
    NeedsReplan { reason: String },
}

impl StepOutcome {
    /// Returns a short label for stream event reporting.
    fn label(&self) -> &str {
        match self {
            Self::Success { .. } => "success",
            Self::Failed { .. } => "failed",
            Self::NeedsReplan { .. } => "needs_replan",
        }
    }
}

/// Current status of the overall plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    /// Plan is being executed.
    InProgress,
    /// All steps completed successfully.
    Completed,
    /// Plan was aborted due to unrecoverable failure.
    Failed,
}

/// A single step in an execution plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    /// Human-readable task description.
    pub task:                String,
    /// Acceptance criteria for this step.
    #[serde(default)]
    pub acceptance_criteria: Option<String>,
    /// How this step should be executed.
    #[serde(default = "default_execution_mode")]
    pub mode:                ExecutionMode,
    /// Optional agent name override for Worker mode.
    #[serde(default)]
    pub agent:               Option<String>,
}

fn default_execution_mode() -> ExecutionMode { ExecutionMode::Inline }

/// A completed step with its outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PastStep {
    /// The original step definition.
    pub step:    PlanStep,
    /// The execution outcome.
    pub outcome: StepOutcome,
}

/// A structured execution plan produced by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    /// High-level goal derived from the user request.
    pub goal:       String,
    /// Ordered steps to achieve the goal.
    pub steps:      Vec<PlanStep>,
    /// Steps that have already been executed (for replan context).
    #[serde(default)]
    pub past_steps: Vec<PastStep>,
    /// Current plan status.
    #[serde(default = "default_plan_status")]
    pub status:     PlanStatus,
}

fn default_plan_status() -> PlanStatus { PlanStatus::InProgress }

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
///    steps (currently stubbed with `todo!()`).
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
        .append(tape_name, TapEntryKind::Plan, plan_json.clone(), None)
        .await
        .map_err(|e| KernelError::AgentExecution {
            message: format!("failed to persist plan to tape: {e}"),
        })?;

    stream_handle.emit(StreamEvent::PlanCreated {
        plan: plan_json.clone(),
    });

    // -- Phase 2: Execute steps -----------------------------------------------

    let mut past_steps: Vec<PastStep> = Vec::new();
    let mut plan = plan;
    let mut total_iterations = 0usize;
    let mut total_tool_calls = 0usize;
    let mut last_model = String::new();
    let mut final_texts: Vec<String> = Vec::new();

    for (step_index, step) in plan.steps.iter().enumerate() {
        if turn_cancel.is_cancelled() {
            warn!(session_key = %session_key, step = step_index, "plan executor: cancelled");
            break;
        }

        stream_handle.emit(StreamEvent::PlanStepStart {
            step_index,
            task: step.task.clone(),
        });

        let outcome = match step.mode {
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
                // This would spawn an independent child session with the step's
                // task as input and wait for its completion.
                StepOutcome::Failed {
                    error: "worker execution mode not yet implemented".into(),
                }
            }
        };

        stream_handle.emit(StreamEvent::PlanStepEnd {
            step_index,
            outcome: outcome.label().to_owned(),
        });

        let needs_replan = matches!(
            outcome,
            StepOutcome::Failed { .. } | StepOutcome::NeedsReplan { .. }
        );

        past_steps.push(PastStep {
            step:    step.clone(),
            outcome: outcome.clone(),
        });

        // -- Replan check -----------------------------------------------------
        if needs_replan {
            let reason = match &outcome {
                StepOutcome::Failed { error } => format!("step {} failed: {}", step_index, error),
                StepOutcome::NeedsReplan { reason } => reason.clone(),
                _ => unreachable!(),
            };

            info!(
                session_key = %session_key,
                step = step_index,
                reason = %reason,
                "plan executor: replan triggered"
            );

            // TODO: Call LLM with past_steps + remaining steps to produce a
            // revised plan. For now, we abort the plan on failure.
            //
            // The replan flow would:
            // 1. Build a prompt with past_steps context and remaining steps
            // 2. Call LLM to get a new Plan
            // 3. Emit StreamEvent::PlanReplan
            // 4. Continue execution with the new plan's steps

            stream_handle.emit(StreamEvent::PlanReplan {
                reason: reason.clone(),
                new_plan: serde_json::Value::Null, // placeholder
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
            total_tool_calls: total_tool_calls,
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
            task:                user_text.to_owned(),
            acceptance_criteria: None,
            mode:                ExecutionMode::Inline,
            agent:               None,
        }],
        past_steps: Vec::new(),
        status:     PlanStatus::InProgress,
    }
}

/// Execute a single plan step inline using `run_agent_loop`.
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
) -> StepOutcome {
    // Delegate to run_agent_loop with the step's task as the user text.
    // The agent loop will read context from the tape (which already has the
    // user's original message and the plan entry) and execute normally.
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
            if !turn_result.text.is_empty() {
                final_texts.push(turn_result.text.clone());
            }
            StepOutcome::Success {
                summary: turn_result.text,
            }
        }
        Err(e) => StepOutcome::Failed {
            error: e.to_string(),
        },
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
                    task:                "build the binary".into(),
                    acceptance_criteria: Some("cargo build succeeds".into()),
                    mode:                ExecutionMode::Inline,
                    agent:               None,
                },
                PlanStep {
                    task:                "run tests".into(),
                    acceptance_criteria: None,
                    mode:                ExecutionMode::Worker,
                    agent:               Some("test-runner".into()),
                },
            ],
            past_steps: vec![],
            status:     PlanStatus::InProgress,
        };

        let json = serde_json::to_string(&plan).unwrap();
        let parsed: Plan = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.goal, "deploy the app");
        assert_eq!(parsed.steps.len(), 2);
        assert_eq!(parsed.steps[0].mode, ExecutionMode::Inline);
        assert_eq!(parsed.steps[1].mode, ExecutionMode::Worker);
        assert_eq!(parsed.status, PlanStatus::InProgress);
    }

    #[test]
    fn create_initial_plan_wraps_user_text() {
        let plan = create_initial_plan("fix the login bug");
        assert_eq!(plan.goal, "fix the login bug");
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].task, "fix the login bug");
        assert_eq!(plan.steps[0].mode, ExecutionMode::Inline);
        assert_eq!(plan.status, PlanStatus::InProgress);
    }

    #[test]
    fn step_outcome_labels() {
        assert_eq!(
            StepOutcome::Success {
                summary: "ok".into()
            }
            .label(),
            "success"
        );
        assert_eq!(
            StepOutcome::Failed {
                error: "boom".into()
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
