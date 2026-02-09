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

//! Conversion layer between DB (store) models and domain types for
//! notifications.

use chrono::{DateTime, TimeZone as _, Utc};
use jiff::Timestamp;

use crate::{db_models, types};

fn chrono_to_timestamp(dt: DateTime<Utc>) -> Timestamp {
    Timestamp::new(dt.timestamp(), dt.timestamp_subsec_nanos() as i32)
        .expect("chrono DateTime<Utc> fits in jiff Timestamp")
}

fn chrono_opt_to_timestamp(dt: Option<DateTime<Utc>>) -> Option<Timestamp> {
    dt.map(chrono_to_timestamp)
}

fn timestamp_to_chrono(ts: Timestamp) -> DateTime<Utc> {
    let mut second = ts.as_second();
    let mut nanosecond = ts.subsec_nanosecond();
    if nanosecond < 0 {
        second = second.saturating_sub(1);
        nanosecond = nanosecond.saturating_add(1_000_000_000);
    }

    Utc.timestamp_opt(second, nanosecond as u32)
        .single()
        .expect("jiff Timestamp fits in chrono DateTime<Utc>")
}

fn timestamp_opt_to_chrono(ts: Option<Timestamp>) -> Option<DateTime<Utc>> {
    ts.map(timestamp_to_chrono)
}

fn u8_from_i16(value: i16, field: &'static str) -> u8 {
    u8::try_from(value).unwrap_or_else(|_| panic!("invalid {field}: {value}"))
}

fn notification_channel_from_i16(value: i16) -> types::NotificationChannel {
    let repr = u8_from_i16(value, "notification.channel");
    types::NotificationChannel::from_repr(repr)
        .unwrap_or_else(|| panic!("invalid notification.channel: {value}"))
}

fn notification_status_from_i16(value: i16) -> types::NotificationStatus {
    let repr = u8_from_i16(value, "notification.status");
    types::NotificationStatus::from_repr(repr)
        .unwrap_or_else(|| panic!("invalid notification.status: {value}"))
}

fn notification_priority_from_i16(value: i16) -> types::NotificationPriority {
    let repr = u8_from_i16(value, "notification.priority");
    types::NotificationPriority::from_repr(repr)
        .unwrap_or_else(|| panic!("invalid notification.priority: {value}"))
}

// ===========================================================================
// NotificationLog <-> Notification conversions
// ===========================================================================

/// Store `NotificationLog` -> Domain `Notification`.
impl From<db_models::NotificationLog> for types::Notification {
    fn from(n: db_models::NotificationLog) -> Self {
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
impl From<types::Notification> for db_models::NotificationLog {
    fn from(n: types::Notification) -> Self {
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

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    #[test]
    fn notification_channel_from_i16_works() {
        use types::NotificationChannel as D;

        assert_eq!(notification_channel_from_i16(0), D::Telegram);
        assert_eq!(notification_channel_from_i16(1), D::Email);
        assert_eq!(notification_channel_from_i16(2), D::Webhook);
    }

    #[test]
    fn notification_status_from_i16_works() {
        use types::NotificationStatus as D;

        assert_eq!(notification_status_from_i16(0), D::Pending);
        assert_eq!(notification_status_from_i16(1), D::Sent);
        assert_eq!(notification_status_from_i16(2), D::Failed);
        assert_eq!(notification_status_from_i16(3), D::Retrying);
    }

    #[test]
    fn notification_priority_from_i16_works() {
        use types::NotificationPriority as D;

        assert_eq!(notification_priority_from_i16(0), D::Low);
        assert_eq!(notification_priority_from_i16(1), D::Normal);
        assert_eq!(notification_priority_from_i16(2), D::High);
        assert_eq!(notification_priority_from_i16(3), D::Urgent);
    }

    #[test]
    fn notification_log_store_to_domain_roundtrip() {
        let now = chrono::Utc::now();
        let id = Uuid::new_v4();
        let ref_id = Uuid::new_v4();
        let store_log = db_models::NotificationLog {
            id,
            channel: 0,
            recipient: "user123".into(),
            subject: Some("Test subject".into()),
            body: "Test body".into(),
            status: 0,
            priority: 2,
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
        assert_eq!(back.channel, 0);
        assert_eq!(back.recipient, "user123");
    }
}
