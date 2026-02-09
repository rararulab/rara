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

//! Database (store) models for scheduler persistence.
//!
//! These types map 1:1 to PostgreSQL columns and use `sqlx::FromRow` /
//! `sqlx::Type` for automatic deserialization.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// Run status for a scheduled task (DB enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "task_run_status", rename_all = "snake_case")]
pub enum TaskRunStatus {
    Success,
    Failed,
    Running,
}

impl std::fmt::Display for TaskRunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Success => write!(f, "success"),
            Self::Failed => write!(f, "failed"),
            Self::Running => write!(f, "running"),
        }
    }
}

/// A scheduled task row from `scheduler_task` table.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct SchedulerTask {
    pub id: Uuid,
    pub name: String,
    pub cron_expr: String,
    pub enabled: bool,
    pub last_run_at: Option<DateTime<Utc>>,
    pub last_status: Option<TaskRunStatus>,
    pub last_error: Option<String>,
    pub run_count: i64,
    pub failure_count: i64,
    pub is_deleted: bool,
    pub deleted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A task run history row from `task_run_history` table.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct TaskRunHistory {
    pub id: Uuid,
    pub task_id: Uuid,
    pub status: TaskRunStatus,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<i64>,
    pub error: Option<String>,
    pub output: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}
