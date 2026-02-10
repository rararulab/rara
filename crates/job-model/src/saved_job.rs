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

//! Store models for the saved-job domain.

use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// A saved job posting (DB row).
#[derive(Debug, Clone, FromRow)]
pub struct SavedJob {
    pub id:               Uuid,
    pub url:              String,
    pub title:            Option<String>,
    pub company:          Option<String>,
    pub status:           i16,
    pub markdown_s3_key:  Option<String>,
    pub markdown_preview: Option<String>,
    pub analysis_result:  Option<serde_json::Value>,
    pub match_score:      Option<f32>,
    pub error_message:    Option<String>,
    pub crawled_at:       Option<DateTime<Utc>>,
    pub analyzed_at:      Option<DateTime<Utc>>,
    pub expires_at:       Option<DateTime<Utc>>,
    pub created_at:       DateTime<Utc>,
    pub updated_at:       DateTime<Utc>,
}
