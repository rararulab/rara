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

// ---------------------------------------------------------------------------
// DB model conversions
// ---------------------------------------------------------------------------

use job_domain_shared::convert::{
    chrono_opt_to_timestamp, chrono_to_timestamp, timestamp_opt_to_chrono, timestamp_to_chrono,
    u8_from_i16,
};
use job_model::notify::NotificationLog;

fn notification_channel_from_i16(value: i16) -> NotificationChannel {
    let repr = u8_from_i16(value, "notification.channel");
    NotificationChannel::from_repr(repr)
        .unwrap_or_else(|| panic!("invalid notification.channel: {value}"))
}

fn notification_status_from_i16(value: i16) -> NotificationStatus {
    let repr = u8_from_i16(value, "notification.status");
    NotificationStatus::from_repr(repr)
        .unwrap_or_else(|| panic!("invalid notification.status: {value}"))
}

fn notification_priority_from_i16(value: i16) -> NotificationPriority {
    let repr = u8_from_i16(value, "notification.priority");
    NotificationPriority::from_repr(repr)
        .unwrap_or_else(|| panic!("invalid notification.priority: {value}"))
}

/// Store `NotificationLog` -> Domain `Notification`.
impl From<NotificationLog> for Notification {
    fn from(n: NotificationLog) -> Self {
        Self {
            id:             n.id,
            channel:        notification_channel_from_i16(n.channel),
            recipient:      n.recipient,
            subject:        n.subject,
            body:           n.body,
            status:         notification_status_from_i16(n.status),
            priority:       notification_priority_from_i16(n.priority),
            retry_count:    n.retry_count,
            max_retries:    n.max_retries,
            error_message:  n.error_message,
            reference_type: n.reference_type,
            reference_id:   n.reference_id,
            metadata:       n.metadata,
            trace_id:       n.trace_id,
            sent_at:        chrono_opt_to_timestamp(n.sent_at),
            created_at:     chrono_to_timestamp(n.created_at),
        }
    }
}

/// Domain `Notification` -> Store `NotificationLog`.
impl From<Notification> for NotificationLog {
    fn from(n: Notification) -> Self {
        Self {
            id:             n.id,
            channel:        n.channel as u8 as i16,
            recipient:      n.recipient,
            subject:        n.subject,
            body:           n.body,
            status:         n.status as u8 as i16,
            priority:       n.priority as u8 as i16,
            retry_count:    n.retry_count,
            max_retries:    n.max_retries,
            error_message:  n.error_message,
            reference_type: n.reference_type,
            reference_id:   n.reference_id,
            metadata:       n.metadata,
            trace_id:       n.trace_id,
            sent_at:        timestamp_opt_to_chrono(n.sent_at),
            created_at:     timestamp_to_chrono(n.created_at),
        }
    }
}
