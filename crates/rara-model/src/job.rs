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

//! Store models for the job domain.

use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// PostgreSQL enum mapping for `job_status`.
#[derive(Debug, Clone, sqlx::Type)]
#[sqlx(type_name = "job_status", rename_all = "snake_case")]
pub enum JobStatus {
    Active,
    Archived,
    Closed,
}

/// A job posting row from the `job` table.
#[derive(Debug, Clone, FromRow)]
pub struct Job {
    pub id:              Uuid,
    pub source_job_id:   String,
    pub source_name:     String,
    pub title:           String,
    pub company:         String,
    pub location:        Option<String>,
    pub description:     Option<String>,
    pub url:             Option<String>,
    pub salary_min:      Option<i32>,
    pub salary_max:      Option<i32>,
    pub salary_currency: Option<String>,
    pub tags:            Vec<String>,
    pub status:          JobStatus,
    pub raw_data:        Option<serde_json::Value>,
    pub trace_id:        Option<String>,
    pub is_deleted:      bool,
    pub deleted_at:      Option<DateTime<Utc>>,
    pub posted_at:       Option<DateTime<Utc>>,
    pub created_at:      DateTime<Utc>,
    pub updated_at:      DateTime<Utc>,
}
