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

//! Plan-Execute architecture data types.
//!
//! A [`Plan`] is a structured execution plan for complex tasks, consisting of
//! a goal and a sequence of [`PlanStep`]s. Each step can be executed inline
//! (by the main agent) or by a worker (independent session).

use serde::{Deserialize, Serialize};

/// Execution mode for a plan step.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    /// Executed by the main agent in the current session.
    Inline,
    /// Executed by an independent worker session.
    Worker,
}

impl Default for ExecutionMode {
    fn default() -> Self { Self::Inline }
}

/// Outcome of a completed plan step.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StepOutcome {
    /// Step completed successfully.
    Success,
    /// Step failed.
    Failed,
    /// Step was skipped.
    Skipped,
}

/// Overall status of a plan.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    /// Plan is actively being executed.
    Active,
    /// All steps completed successfully.
    Completed,
    /// Plan was aborted due to failure or user cancellation.
    Aborted,
}

/// A single step in an execution plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    /// Zero-based index of the step within the plan.
    pub index:      usize,
    /// Natural language description of what this step should accomplish.
    pub task:       String,
    /// Execution mode: inline (main agent) or worker (independent session).
    #[serde(default)]
    pub mode:       ExecutionMode,
    /// Criteria for considering this step complete.
    pub acceptance: String,
}

/// Record of a completed plan step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PastStep {
    /// Index of the completed step.
    pub index:   usize,
    /// Outcome of the step execution.
    pub outcome: StepOutcome,
    /// Summary of what was accomplished or why it failed.
    pub summary: String,
}

/// A structured execution plan for complex tasks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    /// The overall goal of the plan.
    pub goal:       String,
    /// Ordered list of steps to execute.
    pub steps:      Vec<PlanStep>,
    /// Steps that have already been executed.
    pub past_steps: Vec<PastStep>,
    /// Current status of the plan.
    pub status:     PlanStatus,
}
