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

//! Queue payload types for notifications.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use strum_macros::FromRepr;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum QueueMessageState {
    Ready,
    Inflight,
    Archived,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct NotificationQueueOverview {
    pub queue_name:     String,
    pub ready_count:    i64,
    pub inflight_count: i64,
    pub archived_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct NotificationQueueMessage {
    pub state:       QueueMessageState,
    pub msg_id:      i64,
    pub read_ct:     i32,
    #[schema(value_type = String)]
    pub enqueued_at: chrono::DateTime<chrono::Utc>,
    #[schema(value_type = String)]
    pub vt:          chrono::DateTime<chrono::Utc>,
    #[schema(value_type = Option<String>)]
    pub archived_at: Option<chrono::DateTime<chrono::Utc>>,
    #[schema(value_type = Object)]
    pub payload:     serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
#[derive(FromRepr)]
pub enum NotificationPriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Urgent = 3,
}

impl Default for NotificationPriority {
    fn default() -> Self { Self::Normal }
}

/// Request payload used by producer components to enqueue one telegram
/// notification task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendTelegramNotificationRequest {
    pub chat_id:        Option<i64>,
    pub subject:        Option<String>,
    pub body:           String,
    pub priority:       NotificationPriority,
    pub max_retries:    i32,
    pub reference_type: Option<String>,
    pub reference_id:   Option<Uuid>,
    pub metadata:       Option<serde_json::Value>,
    /// Optional local file path of a photo to send instead of (or alongside)
    /// text.
    pub photo_path:     Option<String>,
}

/// Canonical queued telegram notification payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedTelegramNotification {
    pub id:             Uuid,
    pub chat_id:        Option<i64>,
    pub subject:        Option<String>,
    pub body:           String,
    pub priority:       NotificationPriority,
    pub max_retries:    i32,
    pub reference_type: Option<String>,
    pub reference_id:   Option<Uuid>,
    pub metadata:       Option<serde_json::Value>,
    pub photo_path:     Option<String>,
    pub created_at:     Timestamp,
}

/// Message envelope returned from telegram queue read.
#[derive(Debug, Clone)]
pub struct DequeuedTelegramNotification {
    pub msg_id:  i64,
    pub read_ct: i32,
    pub payload: QueuedTelegramNotification,
}
