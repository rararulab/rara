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

//! Conversion layer between DB (store) models and domain types for notifications.

use crate::db_models;
use crate::types;

// ===========================================================================
// NotificationChannel conversions
// ===========================================================================

/// Store `NotificationChannel` -> Domain `NotificationChannel`.
impl From<db_models::NotificationChannel> for types::NotificationChannel {
    fn from(value: db_models::NotificationChannel) -> Self {
        match value {
            db_models::NotificationChannel::Telegram => Self::Telegram,
            db_models::NotificationChannel::Email => Self::Email,
            db_models::NotificationChannel::Webhook => Self::Webhook,
            db_models::NotificationChannel::Other => Self::Webhook,
        }
    }
}

/// Domain `NotificationChannel` -> Store `NotificationChannel`.
impl From<types::NotificationChannel> for db_models::NotificationChannel {
    fn from(value: types::NotificationChannel) -> Self {
        match value {
            types::NotificationChannel::Telegram => Self::Telegram,
            types::NotificationChannel::Email => Self::Email,
            types::NotificationChannel::Webhook => Self::Webhook,
        }
    }
}

// ===========================================================================
// NotificationStatus conversions
// ===========================================================================

/// Store `NotificationStatus` -> Domain `NotificationStatus`.
impl From<db_models::NotificationStatus> for types::NotificationStatus {
    fn from(value: db_models::NotificationStatus) -> Self {
        match value {
            db_models::NotificationStatus::Pending => Self::Pending,
            db_models::NotificationStatus::Sent => Self::Sent,
            db_models::NotificationStatus::Failed => Self::Failed,
            db_models::NotificationStatus::Retrying => Self::Retrying,
        }
    }
}

/// Domain `NotificationStatus` -> Store `NotificationStatus`.
impl From<types::NotificationStatus> for db_models::NotificationStatus {
    fn from(value: types::NotificationStatus) -> Self {
        match value {
            types::NotificationStatus::Pending => Self::Pending,
            types::NotificationStatus::Sent => Self::Sent,
            types::NotificationStatus::Failed => Self::Failed,
            types::NotificationStatus::Retrying => Self::Retrying,
        }
    }
}

// ===========================================================================
// NotificationPriority conversions
// ===========================================================================

/// Store `NotificationPriority` -> Domain `NotificationPriority`.
impl From<db_models::NotificationPriority> for types::NotificationPriority {
    fn from(value: db_models::NotificationPriority) -> Self {
        match value {
            db_models::NotificationPriority::Low => Self::Low,
            db_models::NotificationPriority::Normal => Self::Normal,
            db_models::NotificationPriority::High => Self::High,
            db_models::NotificationPriority::Urgent => Self::Urgent,
        }
    }
}

/// Domain `NotificationPriority` -> Store `NotificationPriority`.
impl From<types::NotificationPriority> for db_models::NotificationPriority {
    fn from(value: types::NotificationPriority) -> Self {
        match value {
            types::NotificationPriority::Low => Self::Low,
            types::NotificationPriority::Normal => Self::Normal,
            types::NotificationPriority::High => Self::High,
            types::NotificationPriority::Urgent => Self::Urgent,
        }
    }
}

// ===========================================================================
// NotificationLog <-> Notification conversions
// ===========================================================================

/// Store `NotificationLog` -> Domain `Notification`.
impl From<db_models::NotificationLog> for types::Notification {
    fn from(n: db_models::NotificationLog) -> Self {
        Self {
            id:             n.id,
            channel:        n.channel.into(),
            recipient:      n.recipient,
            subject:        n.subject,
            body:           n.body,
            status:         n.status.into(),
            priority:       n.priority.into(),
            retry_count:    n.retry_count,
            max_retries:    n.max_retries,
            error_message:  n.error_message,
            reference_type: n.reference_type,
            reference_id:   n.reference_id,
            metadata:       n.metadata,
            trace_id:       n.trace_id,
            sent_at:        n.sent_at,
            created_at:     n.created_at,
        }
    }
}

/// Domain `Notification` -> Store `NotificationLog`.
impl From<types::Notification> for db_models::NotificationLog {
    fn from(n: types::Notification) -> Self {
        Self {
            id:             n.id,
            channel:        n.channel.into(),
            recipient:      n.recipient,
            subject:        n.subject,
            body:           n.body,
            status:         n.status.into(),
            priority:       n.priority.into(),
            retry_count:    n.retry_count,
            max_retries:    n.max_retries,
            error_message:  n.error_message,
            reference_type: n.reference_type,
            reference_id:   n.reference_id,
            metadata:       n.metadata,
            trace_id:       n.trace_id,
            sent_at:        n.sent_at,
            created_at:     n.created_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use uuid::Uuid;

    use super::*;

    #[test]
    fn notification_channel_roundtrip() {
        use db_models::NotificationChannel as S;
        use types::NotificationChannel as D;

        let pairs = [
            (S::Telegram, D::Telegram),
            (S::Email, D::Email),
            (S::Webhook, D::Webhook),
        ];
        for (store, domain) in &pairs {
            assert_eq!(D::from(*store), *domain);
            assert_eq!(S::from(*domain), *store);
        }
    }

    #[test]
    fn notification_channel_other_maps_to_webhook() {
        use db_models::NotificationChannel as S;
        use types::NotificationChannel as D;

        assert_eq!(D::from(S::Other), D::Webhook);
    }

    #[test]
    fn notification_status_roundtrip() {
        use db_models::NotificationStatus as S;
        use types::NotificationStatus as D;

        let pairs = [
            (S::Pending, D::Pending),
            (S::Sent, D::Sent),
            (S::Failed, D::Failed),
            (S::Retrying, D::Retrying),
        ];
        for (store, domain) in &pairs {
            assert_eq!(D::from(*store), *domain);
            assert_eq!(S::from(*domain), *store);
        }
    }

    #[test]
    fn notification_priority_roundtrip() {
        use db_models::NotificationPriority as S;
        use types::NotificationPriority as D;

        let pairs = [
            (S::Low, D::Low),
            (S::Normal, D::Normal),
            (S::High, D::High),
            (S::Urgent, D::Urgent),
        ];
        for (store, domain) in &pairs {
            assert_eq!(D::from(*store), *domain);
            assert_eq!(S::from(*domain), *store);
        }
    }

    #[test]
    fn notification_log_store_to_domain_roundtrip() {
        let now = Utc::now();
        let id = Uuid::new_v4();
        let ref_id = Uuid::new_v4();
        let store_log = db_models::NotificationLog {
            id,
            channel: db_models::NotificationChannel::Telegram,
            recipient: "user123".into(),
            subject: Some("Test subject".into()),
            body: "Test body".into(),
            status: db_models::NotificationStatus::Pending,
            priority: db_models::NotificationPriority::High,
            retry_count: 0,
            max_retries: 3,
            error_message: None,
            reference_type: Some("application".into()),
            reference_id: Some(ref_id),
            metadata: None,
            trace_id: None,
            sent_at: None,
            created_at: now,
        };

        let domain: types::Notification = store_log.into();
        assert_eq!(domain.id, id);
        assert_eq!(domain.channel, types::NotificationChannel::Telegram);
        assert_eq!(domain.recipient, "user123");
        assert_eq!(domain.priority, types::NotificationPriority::High);
        assert_eq!(domain.max_retries, 3);
        assert_eq!(domain.reference_id, Some(ref_id));

        let back: db_models::NotificationLog = domain.into();
        assert_eq!(back.id, id);
        assert_eq!(back.channel, db_models::NotificationChannel::Telegram);
        assert_eq!(back.recipient, "user123");
    }
}
