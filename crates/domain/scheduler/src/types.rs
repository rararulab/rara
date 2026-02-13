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

use jiff::Timestamp;
use rara_domain_shared::id::SchedulerTaskId;
use serde::{Deserialize, Serialize};
use strum_macros::FromRepr;
use uuid::Uuid;

/// Status of a task run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
#[derive(FromRepr)]
pub enum TaskRunStatus {
    /// The run completed successfully.
    Success = 0,
    /// The run failed.
    Failed = 1,
    /// The run is currently in progress.
    Running = 2,
}

/// A scheduled task registered in the system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTask {
    /// Unique identifier.
    pub id:            SchedulerTaskId,
    /// Human-readable name.
    pub name:          String,
    /// Cron expression (e.g. "0 */5 * * * *").
    pub cron_expr:     String,
    /// Whether the task is currently enabled.
    pub enabled:       bool,
    /// When the task was last run.
    pub last_run_at:   Option<Timestamp>,
    /// Status of the last run.
    pub last_status:   Option<TaskRunStatus>,
    /// Error message from the last run (if failed).
    pub last_error:    Option<String>,
    /// Total number of runs.
    pub run_count:     i64,
    /// Total number of failed runs.
    pub failure_count: i64,
    /// When the record was created.
    pub created_at:    Timestamp,
    /// When the record was last updated.
    pub updated_at:    Timestamp,
}

/// A record of a single task execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRunRecord {
    /// Unique identifier for this run.
    pub id:          Uuid,
    /// The task that was executed.
    pub task_id:     SchedulerTaskId,
    /// Outcome of the run.
    pub status:      TaskRunStatus,
    /// When the run started.
    pub started_at:  Timestamp,
    /// When the run finished.
    pub finished_at: Option<Timestamp>,
    /// Duration in milliseconds.
    pub duration_ms: Option<i64>,
    /// Error message (if failed).
    pub error:       Option<String>,
    /// Structured output from the run.
    pub output:      Option<serde_json::Value>,
    /// When this record was created.
    pub created_at:  Timestamp,
}

/// Parameters for creating (registering) a new scheduler task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTaskRequest {
    /// Human-readable name for the task.
    pub name:      String,
    /// Cron expression for scheduling.
    pub cron_expr: String,
}

/// Criteria for listing/filtering scheduler tasks.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskFilter {
    /// Filter by enabled/disabled status.
    pub enabled:       Option<bool>,
    /// Filter by tasks whose name contains this string.
    pub name_contains: Option<String>,
}

// ---------------------------------------------------------------------------
// DB model conversions
// ---------------------------------------------------------------------------

use rara_domain_shared::convert::{
    chrono_opt_to_timestamp, chrono_to_timestamp, timestamp_opt_to_chrono, timestamp_to_chrono,
    u8_from_i16,
};
use rara_model::scheduler::{SchedulerTask, TaskRunHistory};

fn task_run_status_from_i16(value: i16) -> TaskRunStatus {
    let repr = u8_from_i16(value, "scheduler_task.last_status/task_run_history.status");
    TaskRunStatus::from_repr(repr).unwrap_or_else(|| panic!("invalid task run status: {value}"))
}

/// Store `SchedulerTask` -> Domain `ScheduledTask`.
impl From<SchedulerTask> for ScheduledTask {
    fn from(t: SchedulerTask) -> Self {
        Self {
            id:            SchedulerTaskId::from(t.id),
            name:          t.name,
            cron_expr:     t.cron_expr,
            enabled:       t.enabled,
            last_run_at:   chrono_opt_to_timestamp(t.last_run_at),
            last_status:   t.last_status.map(task_run_status_from_i16),
            last_error:    t.last_error,
            run_count:     t.run_count,
            failure_count: t.failure_count,
            created_at:    chrono_to_timestamp(t.created_at),
            updated_at:    chrono_to_timestamp(t.updated_at),
        }
    }
}

/// Domain `ScheduledTask` -> Store `SchedulerTask`.
impl From<ScheduledTask> for SchedulerTask {
    fn from(t: ScheduledTask) -> Self {
        Self {
            id:            t.id.into_inner(),
            name:          t.name,
            cron_expr:     t.cron_expr,
            enabled:       t.enabled,
            last_run_at:   timestamp_opt_to_chrono(t.last_run_at),
            last_status:   t.last_status.map(|s| s as u8 as i16),
            last_error:    t.last_error,
            run_count:     t.run_count,
            failure_count: t.failure_count,
            is_deleted:    false,
            deleted_at:    None,
            created_at:    timestamp_to_chrono(t.created_at),
            updated_at:    timestamp_to_chrono(t.updated_at),
        }
    }
}

/// Store `TaskRunHistory` -> Domain `TaskRunRecord`.
impl From<TaskRunHistory> for TaskRunRecord {
    fn from(r: TaskRunHistory) -> Self {
        Self {
            id:          r.id,
            task_id:     SchedulerTaskId::from(r.task_id),
            status:      task_run_status_from_i16(r.status),
            started_at:  chrono_to_timestamp(r.started_at),
            finished_at: chrono_opt_to_timestamp(r.finished_at),
            duration_ms: r.duration_ms,
            error:       r.error,
            output:      r.output,
            created_at:  chrono_to_timestamp(r.created_at),
        }
    }
}

/// Domain `TaskRunRecord` -> Store `TaskRunHistory`.
impl From<TaskRunRecord> for TaskRunHistory {
    fn from(r: TaskRunRecord) -> Self {
        Self {
            id:          r.id,
            task_id:     r.task_id.into_inner(),
            status:      r.status as u8 as i16,
            started_at:  timestamp_to_chrono(r.started_at),
            finished_at: timestamp_opt_to_chrono(r.finished_at),
            duration_ms: r.duration_ms,
            error:       r.error,
            output:      r.output,
            created_at:  timestamp_to_chrono(r.created_at),
        }
    }
}
