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
    pub task_status:     i16,
    pub materials:       Option<serde_json::Value>,
    pub notes:           Option<String>,
    pub trace_id:        Option<String>,
    pub is_deleted:      bool,
    pub deleted_at:      Option<DateTime<Utc>>,
    pub created_at:      DateTime<Utc>,
    pub updated_at:      DateTime<Utc>,
}
