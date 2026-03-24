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
    io::{PlanStepStatus, StreamEvent, StreamHandle},
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
// TODO: add `tools: Vec<String>` field for per-step tool scoping (e.g.,
// `["*"]` = all tools, `["read_file", "grep"]` = restricted). Requires
// schema changes to `CreatePlanTool` and LLM prompt updates.
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

/// Per-step trace for plan-mode observability.
///
/// Collected during the step loop and embedded into `TurnTrace.iterations`
/// as synthetic `IterationTrace` entries (one per step).
///
/// **Temporary**: ideally `TurnTrace` would have a dedicated
/// `plan_steps: Vec<PlanStepTrace>` field instead of reusing
/// `iterations`. The current approach fills `IterationTrace` with
/// placeholder values (`stream_ms: 0`, `tool_calls: vec![]`). This
/// should be revisited when `TurnTrace` is next refactored.
#[derive(Debug, Clone, Serialize)]
pub struct PlanStepTrace {
    pub step_index: usize,
    pub task:       String,
    pub outcome:    String,
    pub iterations: usize,
    pub tool_calls: usize,
    pub model:      String,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum replan attempts before giving up.
const MAX_REPLAN_ATTEMPTS: usize = 3;
/// Max LLM iterations per worker step — keeps impossible tasks from burning
/// time.
const WORKER_MAX_ITERATIONS: usize = 12;
/// Max cumulative LLM iterations across all plan steps. Prevents runaway
/// execution when multiple steps each consume close to their per-step limit.
const PLAN_MAX_TOTAL_ITERATIONS: usize = 50;
/// Default timeout (seconds) for a worker step when not configured via
/// `AgentManifest.worker_timeout_secs`. Prevents stuck workers from
/// blocking the plan loop indefinitely.
const DEFAULT_WORKER_TIMEOUT_SECS: u64 = 300;

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
- Order steps logically — later steps may depend on earlier ones
- Only plan steps that use the tools listed in the tool summary below. Do not assume capabilities that are not listed."#;

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
///
/// Returns an empty string when the registry is empty; callers should
/// skip injection in that case to avoid a blank block in the prompt.
fn build_tool_summary(tools: &crate::tool::ToolRegistry) -> String {
    if tools.is_empty() {
        return String::new();
    }
    // Sort by name for deterministic output (stable prompt caching).
    let mut entries: Vec<_> = tools.iter().collect();
    entries.sort_by_key(|(name, _)| *name);
    let mut lines = vec!["Available tools:".to_string()];
    for (name, tool) in entries {
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
    guard_pipeline: Arc<GuardPipeline>,
    notification_bus: NotificationBusRef,
    rara_message_id: crate::io::MessageId,
) -> Result<AgentTurnResult> {
    info!(session_key = %session_key, "plan executor: starting v2 plan-execute loop");
    let start = Instant::now();

    // -- Phase 1: Plan creation -----------------------------------------------

    // Build agent context for the planner (same identity as reactive loop).
    let manifest =
        handle
            .session_manifest(&session_key)
            .await
            .map_err(|e| KernelError::AgentExecution {
                message: format!("failed to get manifest for planning: {e}"),
            })?;
    let full_tools = handle
        .session_tool_registry(session_key)
        .await
        .map_err(|e| KernelError::AgentExecution {
            message: format!("failed to get tool registry for planning: {e}"),
        })?;
    let tools_for_plan = full_tools.filtered_for_manifest(&manifest.tools);
    let (agent_prompt, _) = crate::agent::build_agent_system_prompt(&manifest, &tools_for_plan);

    let plan = create_plan_via_llm(
        handle,
        session_key,
        &user_text,
        &tool_context,
        &agent_prompt,
        &tools_for_plan,
    )
    .await?;

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
    let estimated_duration_secs = None;

    stream_handle.emit(StreamEvent::PlanCreated {
        goal: plan.goal.clone(),
        total_steps: plan.steps.len(),
        compact_summary,
        estimated_duration_secs,
    });

    // -- Phase 2: Execute steps -----------------------------------------------

    let worker_timeout_secs = manifest
        .worker_timeout_secs
        .unwrap_or(DEFAULT_WORKER_TIMEOUT_SECS);

    let mut past_steps: Vec<PastStep> = Vec::new();
    let mut plan = plan;
    let mut total_iterations = 0usize;
    let mut total_tool_calls = 0usize;
    let mut step_traces: Vec<PlanStepTrace> = Vec::new();
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

        // Total iteration cap across all steps — prevents runaway execution.
        if total_iterations >= PLAN_MAX_TOTAL_ITERATIONS {
            warn!(
                session_key = %session_key,
                total_iterations,
                step = step_idx,
                "plan executor: total iteration cap reached, aborting"
            );
            stream_handle.emit(StreamEvent::PlanProgress {
                current_step: step_idx,
                total_steps:  plan.steps.len(),
                step_status:  PlanStepStatus::Failed {
                    reason: format!(
                        "total iteration cap reached ({PLAN_MAX_TOTAL_ITERATIONS} iterations)"
                    ),
                },
                status_text:  format!(
                    "计划终止：累计迭代次数已达上限（{PLAN_MAX_TOTAL_ITERATIONS}）"
                ),
            });
            plan.status = PlanStatus::Failed;
            break;
        }

        let step = plan.steps[step_idx].clone();

        stream_handle.emit(StreamEvent::PlanProgress {
            current_step: step.index,
            total_steps:  plan.steps.len(),
            step_status:  PlanStepStatus::Running,
            status_text:  format!("正在执行第{}步：{}…", step.index + 1, step.task),
        });

        let iters_before = total_iterations;
        let tools_before = total_tool_calls;

        let (outcome, summary) = match step.mode {
            ExecutionMode::Inline => {
                execute_inline_step(
                    handle,
                    session_key,
                    &step,
                    &plan.goal,
                    &past_steps,
                    stream_handle,
                    turn_cancel,
                    tape.clone(),
                    tape_name,
                    tool_context.clone(),
                    milestone_tx.clone(),
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
            ExecutionMode::Worker => {
                let worker_result = execute_worker_step(
                    handle,
                    session_key,
                    &step,
                    turn_cancel,
                    worker_timeout_secs,
                )
                .await;
                // Accumulate worker metrics into the plan-level totals.
                total_iterations += worker_result.iterations;
                total_tool_calls += worker_result.tool_calls;
                (worker_result.outcome, worker_result.summary)
            }
        };

        let (step_status, end_status) = match &outcome {
            StepOutcome::Success => (PlanStepStatus::Done, format!("第{}步完成", step.index + 1)),
            StepOutcome::Failed { reason } => (
                PlanStepStatus::Failed {
                    reason: reason.clone(),
                },
                format!("第{}步失败：{}", step.index + 1, reason),
            ),
            StepOutcome::NeedsReplan { reason } => (
                PlanStepStatus::NeedsReplan {
                    reason: reason.clone(),
                },
                format!("第{}步需要调整：{}", step.index + 1, reason),
            ),
        };
        stream_handle.emit(StreamEvent::PlanProgress {
            current_step: step.index,
            total_steps: plan.steps.len(),
            step_status,
            status_text: end_status,
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

        step_traces.push(PlanStepTrace {
            step_index: step.index,
            task:       step.task.clone(),
            outcome:    outcome.label().to_owned(),
            iterations: total_iterations - iters_before,
            tool_calls: total_tool_calls - tools_before,
            model:      last_model.clone(),
        });

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
                &agent_prompt,
                &tools_for_plan,
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
                        "plan executor: replan LLM call failed, falling back to remaining steps"
                    );
                    stream_handle.emit(StreamEvent::PlanReplan {
                        reason: reason.clone(),
                    });

                    // If this was the last step, there is nothing left to try.
                    if step_idx + 1 >= plan.steps.len() {
                        plan.status = PlanStatus::Failed;
                        break;
                    }

                    // Otherwise skip the failed step and continue with the
                    // remaining original steps.
                    step_idx += 1;
                    continue;
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
            iterations: step_traces
                .iter()
                .map(|st| crate::agent::IterationTrace {
                    index:          st.step_index,
                    first_token_ms: None,
                    stream_ms:      0,
                    text_preview:   format!(
                        "[step {}] {} → {}",
                        st.step_index, st.task, st.outcome
                    ),
                    reasoning_text: None,
                    tool_calls:     Vec::new(),
                })
                .collect(),
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
        cascade:    crate::cascade::CascadeTrace::empty(),
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
    agent_system_prompt: &str,
    tools: &crate::tool::ToolRegistry,
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

    // Compose planning prompt with agent identity and available tools so
    // the planner understands the agent's capabilities.
    let tool_summary = build_tool_summary(tools);
    let mut planning_prompt = format!(
        "{PLANNING_SYSTEM_PROMPT}\n\n<agent_context>\n{agent_system_prompt}\n</agent_context>"
    );
    if !tool_summary.is_empty() {
        planning_prompt.push_str("\n\n");
        planning_prompt.push_str(&tool_summary);
    }

    let messages = vec![
        llm::Message::system(&planning_prompt),
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
    agent_system_prompt: &str,
    tools: &crate::tool::ToolRegistry,
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

    // Compose replan prompt with agent identity and available tools so
    // the replanner understands the agent's capabilities.
    let tool_summary = build_tool_summary(tools);
    let mut replan_prompt = format!(
        "{REPLAN_SYSTEM_PROMPT}\n\n<agent_context>\n{agent_system_prompt}\n</agent_context>"
    );
    if !tool_summary.is_empty() {
        replan_prompt.push_str("\n\n");
        replan_prompt.push_str(&tool_summary);
    }

    let messages = vec![
        llm::Message::system(&replan_prompt),
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

/// Result of executing a worker step, including metrics for the caller.
struct WorkerStepResult {
    outcome:    StepOutcome,
    summary:    String,
    iterations: usize,
    tool_calls: usize,
}

/// Execute a plan step by spawning an independent worker child session.
///
/// Uses `KernelHandle::spawn_child` to create a child agent that runs
/// the step's task independently. Waits for completion (with timeout and
/// cancellation support) and returns the outcome plus metrics.
async fn execute_worker_step(
    handle: &KernelHandle,
    session_key: SessionKey,
    step: &PlanStep,
    turn_cancel: &CancellationToken,
    worker_timeout_secs: u64,
) -> WorkerStepResult {
    let failed = |reason: String| WorkerStepResult {
        outcome:    StepOutcome::Failed {
            reason: reason.clone(),
        },
        summary:    reason,
        iterations: 0,
        tool_calls: 0,
    };

    // Look up the principal from the parent session.
    let principal = match handle
        .process_table()
        .with(&session_key, |p| p.principal.clone())
    {
        Some(p) => p,
        None => {
            let reason = format!("session {} not found for worker spawn", session_key);
            return failed(reason);
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
             task: {task}\n\nAcceptance criteria: {acceptance}\n\nFocus exclusively on this \
             task.\n\nIf a step requires interactive human action (e.g., browser login, manual \
             approval) that you cannot perform, report what is needed and stop immediately \
             instead of retrying.{output_suffix}",
            task = step.task,
            acceptance = step.acceptance,
            output_suffix = crate::agent::STRUCTURED_OUTPUT_SUFFIX,
        ),
        soul_prompt:            None,
        provider_hint:          None,
        max_iterations:         Some(WORKER_MAX_ITERATIONS),
        tools:                  vec!["*".to_string()], // inherit all tools
        excluded_tools:         vec![],
        max_children:           None,
        max_context_tokens:     None,
        priority:               crate::agent::Priority::Normal,
        metadata:               serde_json::Value::Null,
        sandbox:                None,
        default_execution_mode: None,
        tool_call_limit:        None,
        worker_timeout_secs:    None,
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
            return failed(reason);
        }
    };

    let child_key = agent_handle.session_key.clone();

    // Wait for the child to complete, with timeout and cancellation.
    let mut rx = agent_handle.result_rx;
    let mut milestones = Vec::new();
    let timeout = tokio::time::Duration::from_secs(worker_timeout_secs);

    let recv_result = tokio::time::timeout(timeout, async {
        loop {
            tokio::select! {
                event = rx.recv() => {
                    match event {
                        Some(crate::io::AgentEvent::Milestone { stage, detail }) => {
                            milestones.push(format!("{}: {}", stage, detail.unwrap_or_default()));
                        }
                        Some(crate::io::AgentEvent::Done(result)) => return Some(result),
                        // Channel closed without Done — worker dropped.
                        None => return None,
                    }
                }
                _ = turn_cancel.cancelled() => return None,
            }
        }
    })
    .await;

    match recv_result {
        Ok(Some(result)) => {
            let summary = if result.output.is_empty() {
                format!(
                    "Worker completed ({} iterations, {} tool calls)",
                    result.iterations, result.tool_calls
                )
            } else {
                crate::agent::truncate_preview(
                    &result.output,
                    crate::agent::CHILD_RESULT_SAFETY_LIMIT_BYTES,
                )
            };

            info!(
                session_key = %session_key,
                step = step.index,
                success = result.success,
                iterations = result.iterations,
                tool_calls = result.tool_calls,
                milestones = milestones.len(),
                "plan executor: worker completed"
            );

            // Mirror classify_step_result logic: treat unsuccessful
            // completion (e.g. max iterations exhausted) as replan trigger.
            let outcome = if result.success {
                StepOutcome::Success
            } else {
                let reason = format!(
                    "worker did not complete successfully ({} iterations, {} tool calls)",
                    result.iterations, result.tool_calls
                );
                warn!(
                    session_key = %session_key,
                    step = step.index,
                    "plan executor: worker finished with success=false, requesting replan"
                );
                StepOutcome::NeedsReplan { reason }
            };

            WorkerStepResult {
                outcome,
                summary,
                iterations: result.iterations,
                tool_calls: result.tool_calls,
            }
        }
        Ok(None) if turn_cancel.is_cancelled() => {
            // Parent was cancelled — propagate without extra noise.
            let reason = format!("worker for step {} cancelled", step.index);
            warn!(session_key = %session_key, step = step.index, "worker cancelled by user");
            failed(reason)
        }
        Ok(None) => {
            // Channel closed without Done — worker dropped.
            let reason = format!(
                "worker for step {} was dropped without producing a result",
                step.index
            );
            warn!(session_key = %session_key, step = step.index, "worker dropped without result");
            failed(reason)
        }
        Err(_) => {
            // Timeout — terminate the child and give it a short grace
            // period to shut down. This is best-effort: if the signal is
            // ignored or the child hangs, we log and move on.
            warn!(session_key = %session_key, step = step.index, "worker timed out, sending terminate signal");
            if let Err(e) = handle.send_signal(child_key, crate::session::Signal::Terminate) {
                warn!(
                    session_key = %session_key,
                    step = step.index,
                    error = %e,
                    "plan executor: failed to send terminate to timed-out worker"
                );
            } else {
                // Drain remaining events for up to 5s so the child has a
                // chance to clean up. If it doesn't exit, we proceed anyway.
                let _ = tokio::time::timeout(tokio::time::Duration::from_secs(5), async {
                    while rx.recv().await.is_some() {}
                })
                .await;
            }
            let reason = format!(
                "worker for step {} timed out after {}s",
                step.index, worker_timeout_secs
            );
            failed(reason)
        }
    }
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

/// Build an enriched user prompt that includes plan context for an inline step.
///
/// When a step runs in a forked tape, it has no prior conversation history.
/// This function injects enough context (goal + completed step summaries)
/// so the agent can continue coherently.
fn build_step_prompt(plan_goal: &str, past_steps: &[PastStep], step: &PlanStep) -> String {
    let mut parts = Vec::with_capacity(3);

    parts.push(format!("Plan goal: {plan_goal}"));

    if !past_steps.is_empty() {
        let steps_desc: String = past_steps
            .iter()
            .map(|s| {
                format!(
                    "- Step {}: {} [{}] — {}",
                    s.index + 1,
                    s.task,
                    s.outcome.label(),
                    s.summary,
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        parts.push(format!("Completed steps:\n{steps_desc}"));
    }

    parts.push(format!(
        "Current task: {}\nAcceptance criteria: {}",
        step.task, step.acceptance,
    ));

    parts.join("\n\n")
}

/// Execute a single plan step inline using `run_agent_loop`.
///
/// For steps after step 0, the tape is partially forked so the agent loop
/// runs in an isolated context containing only plan-level summaries —
/// not the full history of previous steps. The fork is merged back after
/// completion to preserve the audit trail.
///
/// Returns `(StepOutcome, summary_text)`.
#[allow(clippy::too_many_arguments)]
async fn execute_inline_step(
    handle: &KernelHandle,
    session_key: SessionKey,
    step: &PlanStep,
    plan_goal: &str,
    past_steps: &[PastStep],
    stream_handle: &StreamHandle,
    turn_cancel: &CancellationToken,
    tape: TapeService,
    tape_name: &str,
    tool_context: crate::tool::ToolContext,
    milestone_tx: Option<tokio::sync::mpsc::Sender<crate::io::AgentEvent>>,
    guard_pipeline: Arc<GuardPipeline>,
    notification_bus: NotificationBusRef,
    rara_message_id: crate::io::MessageId,
    total_iterations: &mut usize,
    total_tool_calls: &mut usize,
    last_model: &mut String,
    final_texts: &mut Vec<String>,
) -> (StepOutcome, String) {
    // For step 0, run directly against the parent tape so the original
    // user message provides natural context. For subsequent steps, fork
    // the tape at the current position to isolate each step's context.
    let (effective_tape_name, fork_name) = if step.index == 0 {
        (tape_name.to_owned(), None)
    } else {
        match tape.last_entry_id(tape_name).await {
            Ok(last_id) => match tape.store().fork(tape_name, Some(last_id)).await {
                Ok(name) => {
                    info!(
                        step = step.index,
                        fork = %name,
                        "plan executor: forked tape for inline step"
                    );
                    (name.clone(), Some(name))
                }
                Err(e) => {
                    warn!(
                        step = step.index,
                        error = %e,
                        "plan executor: fork failed, falling back to shared tape"
                    );
                    (tape_name.to_owned(), None)
                }
            },
            Err(e) => {
                warn!(
                    step = step.index,
                    error = %e,
                    "plan executor: last_entry_id failed, falling back to shared tape"
                );
                (tape_name.to_owned(), None)
            }
        }
    };

    // When running in a fork, enrich the user prompt with plan context
    // since the forked tape has no prior conversation.
    let user_text = if fork_name.is_some() {
        build_step_prompt(plan_goal, past_steps, step)
    } else {
        step.task.clone()
    };

    let result = crate::agent::run_agent_loop(
        handle,
        session_key,
        user_text,
        stream_handle,
        turn_cancel,
        tape.clone(),
        &effective_tape_name,
        tool_context,
        milestone_tx,
        guard_pipeline,
        notification_bus,
        rara_message_id,
    )
    .await;

    // Always clean up the fork — merge on success/failure, discard only
    // if merge itself fails. This prevents orphan fork files from
    // accumulating on disk (e.g. after panics or cancellation).
    if let Some(ref fork) = fork_name {
        if let Err(e) = tape.store().merge(fork, tape_name).await {
            warn!(
                step = step.index,
                error = %e,
                "plan executor: merge failed, discarding fork to avoid leak"
            );
            if let Err(e2) = tape.store().discard(fork).await {
                warn!(
                    step = step.index,
                    error = %e2,
                    "plan executor: discard also failed, fork file may be orphaned"
                );
            }
        }
    }

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
            cascade:    crate::cascade::CascadeTrace {
                message_id: String::new(),
                ticks:      Vec::new(),
                summary:    crate::cascade::CascadeSummary {
                    tick_count:      0,
                    tool_call_count: 0,
                    total_entries:   0,
                },
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
