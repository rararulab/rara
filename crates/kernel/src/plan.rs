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
//! The entry point is `run_plan_loop`, which has the same signature as
//! `run_agent_loop` so the kernel can route to either.

use std::{sync::Arc, time::Instant};

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::{
    agent::{AgentManifest, AgentRole, AgentTurnResult},
    error::{KernelError, Result},
    guard::pipeline::GuardPipeline,
    handle::KernelHandle,
    io::{StreamEvent, StreamHandle},
    llm,
    memory::{TapEntryKind, TapeService},
    notification::NotificationBusRef,
    session::SessionKey,
    tool::{AgentTool, create_plan::CreatePlanTool},
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
// Constants
// ---------------------------------------------------------------------------

/// Maximum replan attempts before giving up.
const MAX_REPLAN_ATTEMPTS: usize = 3;

/// System prompt for the planning LLM call.
const PLANNING_SYSTEM_PROMPT: &str = r#"You are a task planner. Analyze the user's request and decompose it into a structured execution plan.

You MUST call the `create-plan` tool with:
- `goal`: A concise summary of the overall objective
- `steps`: An ordered list of steps, each with:
  - `task`: Clear description of what this step should accomplish
  - `mode`: "inline" for tasks the main agent can handle directly, "worker" for independent tasks that can run in their own session
  - `acceptance`: Criteria for considering this step complete

Guidelines:
- Keep plans concise: 2-5 steps for most tasks
- Use "inline" mode for most steps (direct agent execution)
- Use "worker" mode only for truly independent, heavyweight sub-tasks
- Each step should be self-contained with clear acceptance criteria
- Order steps logically — later steps may depend on earlier ones"#;

/// System prompt for the replan LLM call.
const REPLAN_SYSTEM_PROMPT: &str = r#"You are a task planner performing a replan. A previous plan encountered an issue and needs revision.

You will be given:
- The original goal
- Steps already completed (with outcomes)
- The failure reason
- Remaining steps that were not executed

Based on this context, create a REVISED plan using the `create-plan` tool. The new plan should:
- Keep the same overall goal
- Account for work already done (do not repeat completed steps)
- Address the failure reason
- Include only the remaining work needed

You MUST call the `create-plan` tool with the revised plan."#;

// ---------------------------------------------------------------------------
// Tool summary for planning context
// ---------------------------------------------------------------------------

/// Build a compact tool summary for the planning LLM.
///
/// Lists each tool's name and description so the planner knows what
/// capabilities are available when decomposing tasks into steps.
#[allow(dead_code)]
fn build_tool_summary(tools: &crate::tool::ToolRegistry) -> String {
    if tools.is_empty() {
        return String::new();
    }
    let mut lines = vec!["Available tools:".to_string()];
    for (name, tool) in tools.iter() {
        lines.push(format!("- {}: {}", name, tool.description()));
    }
    lines.join("\n")
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
/// 1. **Plan phase** — call the LLM to produce a `Plan` from the user message
///    via the `create-plan` tool.
/// 2. **Execute loop** — for each step, run an inline agent sub-turn or spawn a
///    worker child session.
/// 3. **Replan** — if a step fails or requests replan, call the LLM to revise
///    the remaining steps.
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
    rara_message_id: crate::io::MessageId,
) -> Result<AgentTurnResult> {
    info!(session_key = %session_key, "plan executor: starting v2 plan-execute loop");
    let start = Instant::now();

    // -- Phase 1: Plan creation -----------------------------------------------

    let plan = create_plan_via_llm(handle, session_key, &user_text, &tool_context).await?;

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

    // Generate a compact natural-language summary from the steps.
    let compact_summary = plan
        .steps
        .iter()
        .map(|s| s.task.as_str())
        .collect::<Vec<_>>()
        .join("，");
    let estimated_duration_secs = Some((plan.steps.len() as u32) * 10);

    stream_handle.emit(StreamEvent::PlanCreated {
        goal: plan.goal.clone(),
        total_steps: plan.steps.len(),
        compact_summary,
        estimated_duration_secs,
    });

    // -- Phase 2: Execute steps -----------------------------------------------

    let mut past_steps: Vec<PastStep> = Vec::new();
    let mut plan = plan;
    let mut total_iterations = 0usize;
    let mut total_tool_calls = 0usize;
    let mut last_model = String::new();
    let mut final_texts: Vec<String> = Vec::new();
    let mut replan_count = 0usize;
    // ── Tool call limit circuit breaker (plan mode) ────────────────────
    // Same mechanism as the inline agent loop, but tracks cumulative tool
    // calls across *all* plan steps (total_tool_calls). 0 = disabled.
    // Note: each plan step's inner agent loop has its own independent
    // limit check — this outer layer provides cross-step protection.
    let limit_interval = handle
        .session_manifest(&session_key)
        .await
        .map(|m| m.tool_call_limit.unwrap_or(0))
        .unwrap_or(0);
    let mut next_limit_at: usize = limit_interval;
    let mut limit_id_counter: u64 = 0;

    // Use an index-based loop so we can replace plan.steps on replan.
    let mut step_idx = 0;
    while step_idx < plan.steps.len() {
        if turn_cancel.is_cancelled() {
            warn!(session_key = %session_key, step = step_idx, "plan executor: cancelled");
            break;
        }

        let step = plan.steps[step_idx].clone();

        stream_handle.emit(StreamEvent::PlanProgress {
            current_step: step.index,
            total_steps:  plan.steps.len(),
            status_text:  format!("正在执行第{}步：{}…", step.index + 1, step.task),
        });

        let (outcome, summary) = match step.mode {
            ExecutionMode::Inline => {
                execute_inline_step(
                    handle,
                    session_key,
                    &step,
                    stream_handle,
                    turn_cancel,
                    tape.clone(),
                    tape_name,
                    tool_context.clone(),
                    milestone_tx.clone(),
                    output_interceptor.clone(),
                    guard_pipeline.clone(),
                    notification_bus.clone(),
                    rara_message_id,
                    &mut total_iterations,
                    &mut total_tool_calls,
                    &mut last_model,
                    &mut final_texts,
                )
                .await
            }
            ExecutionMode::Worker => execute_worker_step(handle, session_key, &step).await,
        };

        let end_status = match &outcome {
            StepOutcome::Success => format!("第{}步完成", step.index + 1),
            StepOutcome::Failed { reason } => {
                format!("第{}步失败：{}", step.index + 1, reason)
            }
            StepOutcome::NeedsReplan { reason } => {
                format!("第{}步需要调整：{}", step.index + 1, reason)
            }
        };
        stream_handle.emit(StreamEvent::PlanProgress {
            current_step: step.index,
            total_steps:  plan.steps.len(),
            status_text:  end_status,
        });

        // If interrupted during step execution, exit immediately
        // without replan or further processing.
        if turn_cancel.is_cancelled() {
            break;
        }

        let needs_replan = matches!(
            outcome,
            StepOutcome::Failed { .. } | StepOutcome::NeedsReplan { .. }
        );

        past_steps.push(PastStep {
            index:   step.index,
            task:    step.task.clone(),
            summary: summary.clone(),
            outcome: outcome.clone(),
        });

        // ── Tool call limit check (cumulative across all plan steps) ────
        // Uses total_tool_calls (sum across steps) rather than per-step
        // counts. Same oneshot + 120s timeout pattern as inline agent loop.
        if limit_interval > 0 && total_tool_calls >= next_limit_at {
            limit_id_counter += 1;
            let current_limit_id = limit_id_counter;
            let elapsed_secs = start.elapsed().as_secs();
            stream_handle.emit(StreamEvent::ToolCallLimit {
                session_key: session_key.to_string(),
                limit_id: current_limit_id,
                tool_calls_made: total_tool_calls,
                elapsed_secs,
            });

            let (tx, rx) = tokio::sync::oneshot::channel();
            handle.register_tool_call_limit(&session_key, current_limit_id, tx);

            info!(
                total_tool_calls,
                next_limit_at,
                limit_id = current_limit_id,
                step = step_idx,
                "plan loop paused at tool call limit"
            );

            let decision = tokio::select! {
                result = tokio::time::timeout(
                    std::time::Duration::from_secs(120),
                    rx,
                ) => result,
                _ = turn_cancel.cancelled() => {
                    return Err(KernelError::Interrupted);
                }
            };

            match decision {
                Ok(Ok(crate::io::ToolCallLimitDecision::Continue)) => {
                    next_limit_at = total_tool_calls + limit_interval;
                    stream_handle.emit(StreamEvent::ToolCallLimitResolved {
                        session_key: session_key.to_string(),
                        limit_id:    current_limit_id,
                        continued:   true,
                    });
                }
                _ => {
                    warn!(
                        total_tool_calls,
                        step = step_idx,
                        "plan loop stopped by user or timeout"
                    );
                    stream_handle.emit(StreamEvent::ToolCallLimitResolved {
                        session_key: session_key.to_string(),
                        limit_id:    current_limit_id,
                        continued:   false,
                    });
                    plan.status = PlanStatus::Failed;
                    break;
                }
            }
        }

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
                replan_count,
                "plan executor: replan triggered"
            );

            if replan_count >= MAX_REPLAN_ATTEMPTS {
                warn!(
                    session_key = %session_key,
                    "plan executor: max replan attempts reached, aborting"
                );
                plan.status = PlanStatus::Failed;
                break;
            }

            // Collect remaining unexecuted steps.
            let remaining_steps: Vec<&PlanStep> = plan.steps.iter().skip(step_idx + 1).collect();

            match replan_via_llm(
                handle,
                session_key,
                &plan.goal,
                &past_steps,
                &remaining_steps,
                &reason,
                &tool_context,
            )
            .await
            {
                Ok(new_plan) => {
                    replan_count += 1;

                    stream_handle.emit(StreamEvent::PlanReplan {
                        reason: reason.clone(),
                    });

                    // Replace remaining steps with the new plan's steps.
                    // Re-index them starting after the current past_steps.
                    let base_index = past_steps.len();
                    let reindexed_steps: Vec<PlanStep> = new_plan
                        .steps
                        .into_iter()
                        .enumerate()
                        .map(|(i, mut s)| {
                            s.index = base_index + i;
                            s
                        })
                        .collect();

                    plan.steps = reindexed_steps;
                    plan.status = PlanStatus::Replanned;

                    // Persist updated plan to tape.
                    if let Ok(plan_json) = serde_json::to_value(&plan) {
                        let _ = tape
                            .store()
                            .append(tape_name, TapEntryKind::Plan, plan_json, None)
                            .await;
                    }

                    // Reset step_idx to 0 to start executing the new steps.
                    step_idx = 0;
                    continue;
                }
                Err(e) => {
                    warn!(
                        session_key = %session_key,
                        error = %e,
                        "plan executor: replan LLM call failed, aborting"
                    );
                    stream_handle.emit(StreamEvent::PlanReplan {
                        reason: reason.clone(),
                    });
                    plan.status = PlanStatus::Failed;
                    break;
                }
            }
        }

        step_idx += 1;
    }

    // If the loop exited due to user cancellation, propagate Interrupted
    // so the kernel suppresses the duplicate message.
    if turn_cancel.is_cancelled() {
        return Err(KernelError::Interrupted);
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
        replan_count,
        "plan executor: finished"
    );

    let final_text_len = summary.len();
    Ok(AgentTurnResult {
        text:       summary,
        iterations: total_iterations,
        tool_calls: total_tool_calls,
        model:      last_model.clone(),
        trace:      crate::agent::TurnTrace {
            duration_ms: start.elapsed().as_millis() as u64,
            model: last_model,
            input_text: Some(user_text),
            iterations: vec![],
            final_text_len,
            total_tool_calls,
            success: plan.status == PlanStatus::Completed,
            error: if plan.status == PlanStatus::Failed {
                Some(format!("plan failed: {}", plan.goal))
            } else {
                None
            },
            rara_message_id,
        },
    })
}

// ---------------------------------------------------------------------------
// LLM-driven plan creation
// ---------------------------------------------------------------------------

/// Call the LLM with the `create-plan` tool to produce a structured plan.
///
/// Falls back to a single-step inline plan if the LLM doesn't call the tool.
async fn create_plan_via_llm(
    handle: &KernelHandle,
    session_key: SessionKey,
    user_text: &str,
    tool_context: &crate::tool::ToolContext,
) -> Result<Plan> {
    let (driver, model) = handle
        .session_resolve_driver(session_key)
        .await
        .map_err(|e| KernelError::AgentExecution {
            message: format!("failed to resolve LLM driver for planning: {e}"),
        })?;

    let create_plan_tool = CreatePlanTool;
    let tool_def = llm::ToolDefinition {
        name:        create_plan_tool.name().to_string(),
        description: create_plan_tool.description().to_string(),
        parameters:  create_plan_tool.parameters_schema(),
    };

    let messages = vec![
        llm::Message::system(PLANNING_SYSTEM_PROMPT),
        llm::Message::user(user_text),
    ];

    let request = llm::CompletionRequest {
        model: model.clone(),
        messages,
        tools: vec![tool_def],
        temperature: Some(0.3),
        max_tokens: None,
        thinking: None,
        tool_choice: llm::ToolChoice::Required,
        parallel_tool_calls: false,
        frequency_penalty: None,
    };

    info!(session_key = %session_key, "plan executor: calling LLM for plan creation");

    let response = driver
        .complete(request)
        .await
        .map_err(|e| KernelError::AgentExecution {
            message: format!("LLM plan creation call failed: {e}"),
        })?;

    // Try to extract the create_plan tool call from the response.
    if let Some(tool_call) = response
        .tool_calls
        .iter()
        .find(|tc| tc.name == crate::tool::create_plan::CreatePlanTool::TOOL_NAME)
    {
        let params: serde_json::Value =
            serde_json::from_str(&tool_call.arguments).map_err(|e| {
                KernelError::AgentExecution {
                    message: format!("failed to parse create_plan arguments: {e}"),
                }
            })?;

        let tool_output = create_plan_tool
            .execute(params, tool_context)
            .await
            .map_err(|e| KernelError::AgentExecution {
                message: format!("create_plan tool execution failed: {e}"),
            })?;

        let plan: Plan =
            serde_json::from_value(tool_output.json).map_err(|e| KernelError::AgentExecution {
                message: format!("failed to deserialize plan from tool output: {e}"),
            })?;

        info!(
            session_key = %session_key,
            goal = %plan.goal,
            steps = plan.steps.len(),
            "plan executor: LLM created plan"
        );

        return Ok(plan);
    }

    // Fallback: LLM responded with text instead of tool call.
    // Wrap the user request as a single inline step.
    warn!(
        session_key = %session_key,
        "plan executor: LLM did not call create_plan, falling back to single-step plan"
    );
    Ok(create_fallback_plan(user_text))
}

/// Call the LLM to produce a revised plan after a step failure.
async fn replan_via_llm(
    handle: &KernelHandle,
    session_key: SessionKey,
    goal: &str,
    past_steps: &[PastStep],
    remaining_steps: &[&PlanStep],
    failure_reason: &str,
    tool_context: &crate::tool::ToolContext,
) -> Result<Plan> {
    let (driver, model) = handle
        .session_resolve_driver(session_key)
        .await
        .map_err(|e| KernelError::AgentExecution {
            message: format!("failed to resolve LLM driver for replan: {e}"),
        })?;

    let create_plan_tool = CreatePlanTool;
    let tool_def = llm::ToolDefinition {
        name:        create_plan_tool.name().to_string(),
        description: create_plan_tool.description().to_string(),
        parameters:  create_plan_tool.parameters_schema(),
    };

    // Build the replan context as a user message.
    let past_steps_desc: Vec<String> = past_steps
        .iter()
        .map(|s| {
            format!(
                "- Step {}: {} [{}] — {}",
                s.index,
                s.task,
                s.outcome.label(),
                s.summary
            )
        })
        .collect();

    let remaining_desc: Vec<String> = remaining_steps
        .iter()
        .map(|s| format!("- Step {}: {}", s.index, s.task))
        .collect();

    let replan_context = format!(
        "Original goal: {goal}\n\nCompleted steps:\n{past}\n\nFailure reason: \
         {failure_reason}\n\nRemaining (unexecuted) steps:\n{remaining}\n\nPlease create a \
         revised plan that addresses the failure and completes the goal.",
        past = if past_steps_desc.is_empty() {
            "(none)".to_string()
        } else {
            past_steps_desc.join("\n")
        },
        remaining = if remaining_desc.is_empty() {
            "(none)".to_string()
        } else {
            remaining_desc.join("\n")
        },
    );

    let messages = vec![
        llm::Message::system(REPLAN_SYSTEM_PROMPT),
        llm::Message::user(replan_context),
    ];

    let request = llm::CompletionRequest {
        model: model.clone(),
        messages,
        tools: vec![tool_def],
        temperature: Some(0.3),
        max_tokens: None,
        thinking: None,
        tool_choice: llm::ToolChoice::Required,
        parallel_tool_calls: false,
        frequency_penalty: None,
    };

    info!(session_key = %session_key, "plan executor: calling LLM for replan");

    let response = driver
        .complete(request)
        .await
        .map_err(|e| KernelError::AgentExecution {
            message: format!("LLM replan call failed: {e}"),
        })?;

    // Extract the create_plan tool call.
    if let Some(tool_call) = response
        .tool_calls
        .iter()
        .find(|tc| tc.name == crate::tool::create_plan::CreatePlanTool::TOOL_NAME)
    {
        let params: serde_json::Value =
            serde_json::from_str(&tool_call.arguments).map_err(|e| {
                KernelError::AgentExecution {
                    message: format!("failed to parse replan create_plan arguments: {e}"),
                }
            })?;

        let tool_output = create_plan_tool
            .execute(params, tool_context)
            .await
            .map_err(|e| KernelError::AgentExecution {
                message: format!("replan create_plan tool execution failed: {e}"),
            })?;

        let plan: Plan =
            serde_json::from_value(tool_output.json).map_err(|e| KernelError::AgentExecution {
                message: format!("failed to deserialize replan from tool output: {e}"),
            })?;

        info!(
            session_key = %session_key,
            goal = %plan.goal,
            new_steps = plan.steps.len(),
            "plan executor: LLM produced revised plan"
        );

        return Ok(plan);
    }

    Err(KernelError::AgentExecution {
        message: "LLM did not call create_plan during replan".to_string(),
    })
}

// ---------------------------------------------------------------------------
// Worker execution
// ---------------------------------------------------------------------------

/// Execute a plan step by spawning an independent worker child session.
///
/// Uses `KernelHandle::spawn_child` to create a child agent that runs
/// the step's task independently. Waits for completion and returns the
/// outcome.
async fn execute_worker_step(
    handle: &KernelHandle,
    session_key: SessionKey,
    step: &PlanStep,
) -> (StepOutcome, String) {
    // Look up the principal from the parent session.
    let principal = match handle
        .process_table()
        .with(&session_key, |p| p.principal.clone())
    {
        Some(p) => p,
        None => {
            let reason = format!("session {} not found for worker spawn", session_key);
            return (
                StepOutcome::Failed {
                    reason: reason.clone(),
                },
                reason,
            );
        }
    };

    // Build a minimal worker manifest.
    let worker_manifest = AgentManifest {
        name:                   format!("plan-worker-{}", step.index),
        role:                   AgentRole::Worker,
        description:            format!("Worker for plan step {}: {}", step.index, step.task),
        model:                  None, // inherit from parent via driver registry
        system_prompt:          format!(
            "You are a worker agent executing a specific task as part of a larger plan.\n\nYour \
             task: {}\n\nAcceptance criteria: {}\n\nFocus exclusively on this task. Report your \
             results clearly when done.",
            step.task, step.acceptance
        ),
        soul_prompt:            None,
        provider_hint:          None,
        max_iterations:         Some(20),
        tools:                  vec!["*".to_string()], // inherit all tools
        max_children:           None,
        max_context_tokens:     None,
        priority:               crate::agent::Priority::Normal,
        metadata:               serde_json::Value::Null,
        sandbox:                None,
        default_execution_mode: None,
        tool_call_limit:        None,
    };

    info!(
        session_key = %session_key,
        step = step.index,
        task = %step.task,
        "plan executor: spawning worker for step"
    );

    // Spawn the child agent.
    let agent_handle = match handle
        .spawn_child(&session_key, &principal, worker_manifest, step.task.clone())
        .await
    {
        Ok(h) => h,
        Err(e) => {
            let reason = format!("failed to spawn worker: {e}");
            warn!(session_key = %session_key, step = step.index, error = %e, "worker spawn failed");
            return (
                StepOutcome::Failed {
                    reason: reason.clone(),
                },
                reason,
            );
        }
    };

    // Wait for the child to complete, collecting milestones.
    let mut rx = agent_handle.result_rx;
    let mut milestones = Vec::new();

    while let Some(event) = rx.recv().await {
        match event {
            crate::io::AgentEvent::Milestone { stage, detail } => {
                milestones.push(format!("{}: {}", stage, detail.unwrap_or_default()));
            }
            crate::io::AgentEvent::Done(result) => {
                let summary = if result.output.is_empty() {
                    format!(
                        "Worker completed ({} iterations, {} tool calls)",
                        result.iterations, result.tool_calls
                    )
                } else {
                    // Truncate long worker outputs for the plan summary.
                    let max_summary_len = 2000;
                    if result.output.len() > max_summary_len {
                        format!("{}...(truncated)", &result.output[..max_summary_len])
                    } else {
                        result.output.clone()
                    }
                };

                info!(
                    session_key = %session_key,
                    step = step.index,
                    iterations = result.iterations,
                    tool_calls = result.tool_calls,
                    milestones = milestones.len(),
                    "plan executor: worker completed"
                );

                return (StepOutcome::Success, summary);
            }
        }
    }

    // Channel closed without a Done event — worker was dropped.
    let reason = format!(
        "worker for step {} was dropped without producing a result",
        step.index
    );
    warn!(session_key = %session_key, step = step.index, "worker dropped without result");
    (
        StepOutcome::Failed {
            reason: reason.clone(),
        },
        reason,
    )
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Create a fallback single-step plan when LLM doesn't produce one.
fn create_fallback_plan(user_text: &str) -> Plan {
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

/// Classified result of a step's agent turn — extracted for testability.
#[derive(Debug)]
struct StepClassification {
    step_outcome:        StepOutcome,
    iterations_consumed: usize,
    tool_calls_consumed: usize,
    model:               String,
}

/// Classify an `AgentTurnResult` (or error) into a [`StepOutcome`], deciding
/// whether to keep the text for `final_texts`.
///
/// Returns `(classification, summary_text, optional_text_to_keep)`.
fn classify_step_result(
    result: std::result::Result<crate::agent::AgentTurnResult, KernelError>,
    step_index: usize,
) -> (StepClassification, String, Option<String>) {
    match result {
        Ok(turn) => {
            let summary = turn.text.clone();
            let cls = StepClassification {
                iterations_consumed: turn.iterations,
                tool_calls_consumed: turn.tool_calls,
                model:               turn.model.clone(),
                step_outcome:        StepOutcome::Success, // may be overridden below
            };

            // When the agent loop exhausted its max iterations without
            // completing, `trace.success` is false.  Treat this as a replan
            // trigger instead of silently accepting the fallback error text
            // — otherwise every exhausted step pushes the same "[已达到最大
            // 迭代次数…]" message and the user sees it repeated N times.
            if !turn.trace.success {
                let reason = turn
                    .trace
                    .error
                    .clone()
                    .unwrap_or_else(|| "agent loop did not complete successfully".to_string());
                warn!(
                    step = step_index,
                    reason = %reason,
                    "step finished with trace.success=false, requesting replan"
                );
                let cls = StepClassification {
                    step_outcome: StepOutcome::NeedsReplan { reason },
                    ..cls
                };
                // Don't push fallback text — it would pollute final output.
                return (cls, summary, None);
            }

            let text_to_keep = if turn.text.is_empty() {
                None
            } else {
                Some(turn.text)
            };
            (cls, summary, text_to_keep)
        }
        Err(e) => {
            let summary = e.to_string();
            let cls = StepClassification {
                step_outcome:        StepOutcome::Failed {
                    reason: summary.clone(),
                },
                iterations_consumed: 0,
                tool_calls_consumed: 0,
                model:               String::new(),
            };
            (cls, summary, None)
        }
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
    rara_message_id: crate::io::MessageId,
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
        rara_message_id,
    )
    .await;

    let (outcome, summary, text_to_keep) = classify_step_result(result, step.index);
    *total_iterations += outcome.iterations_consumed;
    *total_tool_calls += outcome.tool_calls_consumed;
    if !outcome.model.is_empty() {
        *last_model = outcome.model;
    }
    if let Some(text) = text_to_keep {
        final_texts.push(text);
    }
    (outcome.step_outcome, summary)
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
    fn create_fallback_plan_wraps_user_text() {
        let plan = create_fallback_plan("fix the login bug");
        assert_eq!(plan.goal, "fix the login bug");
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].task, "fix the login bug");
        assert_eq!(plan.steps[0].mode, ExecutionMode::Inline);
        assert_eq!(plan.status, PlanStatus::Active);
    }

    /// Helper to build a minimal `AgentTurnResult` for classification tests.
    fn make_turn_result(
        text: &str,
        success: bool,
        error: Option<&str>,
    ) -> crate::agent::AgentTurnResult {
        use crate::io::MessageId;
        crate::agent::AgentTurnResult {
            text:       text.to_owned(),
            iterations: 25,
            tool_calls: 25,
            model:      "test-model".to_owned(),
            trace:      crate::agent::TurnTrace {
                duration_ms: 1000,
                model: "test-model".to_owned(),
                input_text: None,
                iterations: vec![],
                final_text_len: text.len(),
                total_tool_calls: 25,
                success,
                error: error.map(|s| s.to_owned()),
                rara_message_id: MessageId::new(),
            },
        }
    }

    #[test]
    fn classify_step_result_success_keeps_text() {
        let turn = make_turn_result("step output", true, None);
        let (cls, summary, text) = super::classify_step_result(Ok(turn), 0);
        assert_eq!(cls.step_outcome, StepOutcome::Success);
        assert_eq!(summary, "step output");
        assert_eq!(text, Some("step output".to_owned()));
    }

    #[test]
    fn classify_step_result_exhaustion_triggers_replan_and_drops_text() {
        let fallback = "[已达到最大迭代次数，任务未完成。已执行 25 次工具调用。]";
        let turn = make_turn_result(
            fallback,
            false,
            Some("max iterations exhausted (25 iterations, 25 tool calls)"),
        );
        let (cls, _summary, text) = super::classify_step_result(Ok(turn), 0);
        assert!(
            matches!(cls.step_outcome, StepOutcome::NeedsReplan { .. }),
            "expected NeedsReplan, got {:?}",
            cls.step_outcome
        );
        // Fallback text must NOT be kept — prevents repeated messages in
        // final_texts when multiple steps exhaust.
        assert_eq!(text, None);
    }

    #[test]
    fn classify_step_result_exhaustion_without_error_field() {
        let turn = make_turn_result("", false, None);
        let (cls, _summary, text) = super::classify_step_result(Ok(turn), 0);
        assert!(matches!(cls.step_outcome, StepOutcome::NeedsReplan { .. }));
        assert_eq!(text, None);
        // Should use default reason when trace.error is None.
        if let StepOutcome::NeedsReplan { reason } = cls.step_outcome {
            assert!(reason.contains("did not complete"), "reason: {reason}");
        }
    }

    #[test]
    fn classify_step_result_kernel_error_returns_failed() {
        let err = KernelError::Llm {
            message: "boom".into(),
        };
        let (cls, summary, text) = super::classify_step_result(Err(err), 0);
        assert!(matches!(cls.step_outcome, StepOutcome::Failed { .. }));
        assert!(summary.contains("boom"));
        assert_eq!(text, None);
    }

    #[test]
    fn step_outcome_labels() {
        assert_eq!(StepOutcome::Success.label(), "success");
        assert_eq!(
            StepOutcome::Failed {
                reason: "boom".into(),
            }
            .label(),
            "failed"
        );
        assert_eq!(
            StepOutcome::NeedsReplan {
                reason: "changed".into(),
            }
            .label(),
            "needs_replan"
        );
    }
}
