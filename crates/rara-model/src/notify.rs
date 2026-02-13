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

//! Store models for the notify domain.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// A log entry for an outbound notification (DB row).
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct NotificationLog {
    pub id:             Uuid,
    pub channel:        i16,
    pub recipient:      String,
    pub subject:        Option<String>,
    pub body:           String,
    pub status:         i16,
    pub priority:       i16,
    pub retry_count:    i32,
    pub max_retries:    i32,
    pub error_message:  Option<String>,
    /// Polymorphic reference type (e.g. "application", "job").
    pub reference_type: Option<String>,
    /// ID of the referenced entity.
    pub reference_id:   Option<Uuid>,
    pub metadata:       Option<serde_json::Value>,
    pub trace_id:       Option<String>,
    pub sent_at:        Option<DateTime<Utc>>,
    pub created_at:     DateTime<Utc>,
}
