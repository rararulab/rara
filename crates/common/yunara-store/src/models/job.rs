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

//! Job posting entity: represents a discovered job listing from any source.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// Job posting lifecycle status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "job_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Active,
    Archived,
    Closed,
}

impl std::fmt::Display for JobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Active => write!(f, "active"),
            Self::Archived => write!(f, "archived"),
            Self::Closed => write!(f, "closed"),
        }
    }
}

/// A job posting discovered from an external source.
///
/// The combination of `source_job_id` and `source_name` forms the idempotent
/// key that prevents duplicate imports.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Job {
    pub id:              Uuid,
    /// External identifier from the source platform.
    pub source_job_id:   String,
    /// Name of the source platform (e.g. "linkedin", "indeed").
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
