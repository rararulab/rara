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

//! Database (sqlx) model types for the interview_plan table.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// Progress status of an interview prep task (DB enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "interview_task_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum InterviewTaskStatus {
    Pending,
    InProgress,
    Completed,
    Skipped,
}

impl std::fmt::Display for InterviewTaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::InProgress => write!(f, "in_progress"),
            Self::Completed => write!(f, "completed"),
            Self::Skipped => write!(f, "skipped"),
        }
    }
}

/// An interview preparation plan (DB row).
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct InterviewPlan {
    pub id:              Uuid,
    pub application_id:  Uuid,
    pub title:           String,
    pub company:         String,
    pub position:        String,
    pub job_description: Option<String>,
    pub round:           String,
    pub description:     Option<String>,
    pub scheduled_at:    Option<DateTime<Utc>>,
    pub task_status:     InterviewTaskStatus,
    pub materials:       Option<serde_json::Value>,
    pub notes:           Option<String>,
    pub trace_id:        Option<String>,
    pub is_deleted:      bool,
    pub deleted_at:      Option<DateTime<Utc>>,
    pub created_at:      DateTime<Utc>,
    pub updated_at:      DateTime<Utc>,
}
