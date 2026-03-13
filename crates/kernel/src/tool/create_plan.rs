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
use serde::Deserialize;
use serde_json::json;

use crate::plan::{ExecutionMode, Plan, PlanStatus, PlanStep};

/// LLM-callable tool that creates a structured execution plan.
pub(crate) struct CreatePlanTool;

// ============================================================================
// Parameter types
// ============================================================================

#[derive(Debug, Deserialize)]
struct StepInput {
    task:       String,
    #[serde(default)]
    mode:       Option<String>,
    acceptance: String,
}

#[derive(Debug, Deserialize)]
struct CreatePlanParams {
    goal:  String,
    steps: Vec<StepInput>,
}

// ============================================================================
// AgentTool impl
// ============================================================================

#[async_trait]
impl super::AgentTool for CreatePlanTool {
    fn name(&self) -> &str { "create_plan" }

    fn description(&self) -> &str {
        "Create a structured execution plan for complex tasks. The plan will be executed step by \
         step with independent context per step."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["goal", "steps"],
            "properties": {
                "goal": {
                    "type": "string",
                    "description": "The overall goal of the plan"
                },
                "steps": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["task", "acceptance"],
                        "properties": {
                            "task": {
                                "type": "string",
                                "description": "Natural language description of what this step should accomplish"
                            },
                            "mode": {
                                "type": "string",
                                "enum": ["inline", "worker"],
                                "default": "inline",
                                "description": "Execution mode: inline (main agent) or worker (independent session)"
                            },
                            "acceptance": {
                                "type": "string",
                                "description": "Criteria for considering this step complete"
                            }
                        }
                    }
                }
            }
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _context: &super::ToolContext,
    ) -> anyhow::Result<super::ToolOutput> {
        let input: CreatePlanParams = serde_json::from_value(params)
            .map_err(|e| anyhow::anyhow!("invalid create_plan params: {e}"))?;

        if input.steps.is_empty() {
            return Err(anyhow::anyhow!("plan must have at least one step"));
        }

        let steps: Vec<PlanStep> = input
            .steps
            .into_iter()
            .enumerate()
            .map(|(index, s)| {
                let mode = match s.mode.as_deref() {
                    Some("worker") => ExecutionMode::Worker,
                    _ => ExecutionMode::Inline,
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
            goal:       input.goal,
            steps,
            past_steps: vec![],
            status:     PlanStatus::Active,
        };

        let json = serde_json::to_value(&plan)
            .map_err(|e| anyhow::anyhow!("failed to serialize plan: {e}"))?;

        Ok(json.into())
    }
}
