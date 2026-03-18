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

//! `create_plan` tool — validates and structures execution plans from
//! LLM-provided goal and steps.
//!
//! The tool only builds and returns the [`Plan`] as JSON. It does NOT write
//! to the tape or emit stream events — that responsibility belongs to the
//! plan executor (kernel loop).

use async_trait::async_trait;
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::plan::{ExecutionMode, Plan, PlanStatus, PlanStep};

/// LLM-callable tool that creates a structured execution plan.
#[derive(ToolDef)]
#[tool(
    name = "create-plan",
    description = "Create a structured execution plan for complex tasks. The plan will be \
                   executed step by step with independent context per step."
)]
pub struct CreatePlanTool;

// ============================================================================
// Parameter types
// ============================================================================

/// Execution mode for a plan step.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StepMode {
    /// Execute in the main agent context.
    #[default]
    Inline,
    /// Execute in an independent worker session.
    Worker,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StepInput {
    /// Natural language description of what this step should accomplish
    task:       String,
    /// Execution mode: inline (main agent) or worker (independent session)
    #[serde(default)]
    mode:       StepMode,
    /// Criteria for considering this step complete
    acceptance: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreatePlanParams {
    /// The overall goal of the plan
    goal:  String,
    /// The steps to execute
    steps: Vec<StepInput>,
}

// ============================================================================
// ToolExecute impl
// ============================================================================

#[async_trait]
impl super::ToolExecute for CreatePlanTool {
    type Output = serde_json::Value;
    type Params = CreatePlanParams;

    async fn run(
        &self,
        input: CreatePlanParams,
        _context: &super::ToolContext,
    ) -> anyhow::Result<serde_json::Value> {
        if input.steps.is_empty() {
            return Err(anyhow::anyhow!("plan must have at least one step"));
        }

        let steps: Vec<PlanStep> = input
            .steps
            .into_iter()
            .enumerate()
            .map(|(index, s)| {
                let mode = match s.mode {
                    StepMode::Worker => ExecutionMode::Worker,
                    StepMode::Inline => ExecutionMode::Inline,
                };
                PlanStep {
                    index,
                    task: s.task,
                    mode,
                    acceptance: s.acceptance,
                }
            })
            .collect();

        let plan = Plan {
            goal: input.goal,
            steps,
            past_steps: vec![],
            status: PlanStatus::Active,
        };

        let json = serde_json::to_value(&plan)
            .map_err(|e| anyhow::anyhow!("failed to serialize plan: {e}"))?;

        Ok(json)
    }
}
