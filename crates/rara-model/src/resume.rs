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

//! Store models for the resume domain.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// A versioned resume document (DB row).
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Resume {
    pub id:                  Uuid,
    pub title:               String,
    pub version_tag:         String,
    pub content_hash:        String,
    pub source:              i16,
    pub content:             Option<String>,
    pub parent_resume_id:    Option<Uuid>,
    pub target_job_id:       Option<Uuid>,
    pub customization_notes: Option<String>,
    pub tags:                Vec<String>,
    pub metadata:            Option<serde_json::Value>,
    pub pdf_object_key:      Option<String>,
    pub pdf_file_size:       Option<i64>,
    pub trace_id:            Option<String>,
    pub is_deleted:          bool,
    pub deleted_at:          Option<DateTime<Utc>>,
    pub created_at:          DateTime<Utc>,
    pub updated_at:          DateTime<Utc>,
}
