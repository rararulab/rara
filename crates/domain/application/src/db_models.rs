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

//! Database (sqlx) model types for the application tables.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// A job application record (DB row).
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Application {
    pub id:           Uuid,
    pub job_id:       Uuid,
    pub resume_id:    Option<Uuid>,
    pub channel:      i16,
    pub status:       i16,
    pub cover_letter: Option<String>,
    pub notes:        Option<String>,
    pub tags:         Vec<String>,
    pub priority:     i16,
    pub trace_id:     Option<String>,
    pub is_deleted:   bool,
    pub deleted_at:   Option<DateTime<Utc>>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub created_at:   DateTime<Utc>,
    pub updated_at:   DateTime<Utc>,
}

/// An immutable record of an application status transition (DB row).
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ApplicationStatusHistory {
    pub id:             Uuid,
    pub application_id: Uuid,
    pub from_status:    Option<i16>,
    pub to_status:      i16,
    pub changed_by:     Option<String>,
    pub note:           Option<String>,
    pub trace_id:       Option<String>,
    pub created_at:     DateTime<Utc>,
}
