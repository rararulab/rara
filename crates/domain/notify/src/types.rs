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

//! Domain types for the notification module.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use strum_macros::FromRepr;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[derive(FromRepr)]
pub enum NotificationChannel {
    Telegram = 0,
    Email = 1,
    Webhook = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
#[derive(FromRepr)]
pub enum NotificationStatus {
    Pending = 0,
    Sent = 1,
    Failed = 2,
    Retrying = 3,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub id:             Uuid,
    pub channel:        NotificationChannel,
    pub recipient:      String,
    pub subject:        Option<String>,
    pub body:           String,
    pub status:         NotificationStatus,
    pub priority:       NotificationPriority,
    pub retry_count:    i32,
    pub max_retries:    i32,
    pub error_message:  Option<String>,
    pub reference_type: Option<String>,
    pub reference_id:   Option<Uuid>,
    pub metadata:       Option<serde_json::Value>,
    pub trace_id:       Option<String>,
    pub sent_at:        Option<Timestamp>,
    pub created_at:     Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendNotificationRequest {
    pub channel:        NotificationChannel,
    pub recipient:      String,
    pub subject:        Option<String>,
    pub body:           String,
    pub priority:       NotificationPriority,
    pub reference_type: Option<String>,
    pub reference_id:   Option<Uuid>,
    pub metadata:       Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotificationFilter {
    pub channel:        Option<NotificationChannel>,
    pub status:         Option<NotificationStatus>,
    pub recipient:      Option<String>,
    pub reference_type: Option<String>,
    pub reference_id:   Option<Uuid>,
    pub created_after:  Option<Timestamp>,
    pub created_before: Option<Timestamp>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotificationStatistics {
    pub total:    i64,
    pub pending:  i64,
    pub sent:     i64,
    pub failed:   i64,
    pub retrying: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProcessResult {
    pub processed: i64,
    pub succeeded: i64,
    pub failed:    i64,
}
