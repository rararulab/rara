// Copyright 2025 Crrow
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

//! Domain types for scheduler task management.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use job_domain_core::id::SchedulerTaskId;

/// Status of a task run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskRunStatus {
    /// The run completed successfully.
    Success,
    /// The run failed.
    Failed,
    /// The run is currently in progress.
    Running,
}

/// A scheduled task registered in the system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTask {
    /// Unique identifier.
    pub id: SchedulerTaskId,
    /// Human-readable name.
    pub name: String,
    /// Cron expression (e.g. "0 */5 * * * *").
    pub cron_expr: String,
    /// Whether the task is currently enabled.
    pub enabled: bool,
    /// When the task was last run.
    pub last_run_at: Option<DateTime<Utc>>,
    /// Status of the last run.
    pub last_status: Option<TaskRunStatus>,
    /// Error message from the last run (if failed).
    pub last_error: Option<String>,
    /// Total number of runs.
    pub run_count: i64,
    /// Total number of failed runs.
    pub failure_count: i64,
    /// When the record was created.
    pub created_at: DateTime<Utc>,
    /// When the record was last updated.
    pub updated_at: DateTime<Utc>,
}

/// A record of a single task execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRunRecord {
    /// Unique identifier for this run.
    pub id: Uuid,
    /// The task that was executed.
    pub task_id: SchedulerTaskId,
    /// Outcome of the run.
    pub status: TaskRunStatus,
    /// When the run started.
    pub started_at: DateTime<Utc>,
    /// When the run finished.
    pub finished_at: Option<DateTime<Utc>>,
    /// Duration in milliseconds.
    pub duration_ms: Option<i64>,
    /// Error message (if failed).
    pub error: Option<String>,
    /// Structured output from the run.
    pub output: Option<serde_json::Value>,
    /// When this record was created.
    pub created_at: DateTime<Utc>,
}

/// Parameters for creating (registering) a new scheduler task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTaskRequest {
    /// Human-readable name for the task.
    pub name: String,
    /// Cron expression for scheduling.
    pub cron_expr: String,
}

/// Criteria for listing/filtering scheduler tasks.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskFilter {
    /// Filter by enabled/disabled status.
    pub enabled: Option<bool>,
    /// Filter by tasks whose name contains this string.
    pub name_contains: Option<String>,
}
