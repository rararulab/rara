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

//! Notification log entity: records of sent Telegram/Email notifications.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// Delivery channel for notifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "notification_channel", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum NotificationChannel {
    Telegram,
    Email,
    Webhook,
    Other,
}

impl std::fmt::Display for NotificationChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Telegram => write!(f, "telegram"),
            Self::Email => write!(f, "email"),
            Self::Webhook => write!(f, "webhook"),
            Self::Other => write!(f, "other"),
        }
    }
}

/// Delivery status of a notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "notification_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum NotificationStatus {
    Pending,
    Sent,
    Failed,
    Retrying,
}

impl std::fmt::Display for NotificationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Sent => write!(f, "sent"),
            Self::Failed => write!(f, "failed"),
            Self::Retrying => write!(f, "retrying"),
        }
    }
}

/// Priority level for notifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "notification_priority", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum NotificationPriority {
    Low,
    Normal,
    High,
    Urgent,
}

impl std::fmt::Display for NotificationPriority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Low => write!(f, "low"),
            Self::Normal => write!(f, "normal"),
            Self::High => write!(f, "high"),
            Self::Urgent => write!(f, "urgent"),
        }
    }
}

/// A log entry for an outbound notification.
///
/// Tracks delivery attempts, retries, and final status for each
/// notification sent through any channel.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct NotificationLog {
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
    /// Polymorphic reference type (e.g. "application", "job").
    pub reference_type: Option<String>,
    /// ID of the referenced entity.
    pub reference_id:   Option<Uuid>,
    pub metadata:       Option<serde_json::Value>,
    pub trace_id:       Option<String>,
    pub sent_at:        Option<DateTime<Utc>>,
    pub created_at:     DateTime<Utc>,
}
