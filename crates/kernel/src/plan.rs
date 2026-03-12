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

//! Plan-execute core data structures.
//!
//! A [`Plan`] captures a goal decomposed into sequential [`PlanStep`]s.
//! As steps execute, their results are recorded as [`PastStep`]s with a
//! [`StepOutcome`].  The overall lifecycle is tracked by [`PlanStatus`].

use serde::{Deserialize, Serialize};

/// Plan status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Active,
    Completed,
    Failed,
    Replanned,
}

/// Step execution mode.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
