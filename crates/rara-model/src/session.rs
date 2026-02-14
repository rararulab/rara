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

//! Store models for chat sessions and channel bindings.

use chrono::{DateTime, Utc};
use sqlx::FromRow;

/// Database row representation of a `chat_session` record.
#[derive(Debug, Clone, FromRow)]
pub struct ChatSessionRow {
    pub key:           String,
    pub title:         Option<String>,
    pub model:         Option<String>,
    pub system_prompt: Option<String>,
    pub message_count: i64,
    pub preview:       Option<String>,
    pub metadata:      Option<serde_json::Value>,
    pub created_at:    DateTime<Utc>,
    pub updated_at:    DateTime<Utc>,
}

/// Database row representation of a `channel_binding` record.
#[derive(Debug, Clone, FromRow)]
pub struct ChannelBindingRow {
    pub channel_type: String,
    pub account:      String,
    pub chat_id:      String,
    pub session_key:  String,
    pub created_at:   DateTime<Utc>,
    pub updated_at:   DateTime<Utc>,
}
