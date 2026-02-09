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

//! Conversion layer between DB models and domain types for application.

use chrono::{DateTime, TimeZone as _, Utc};
use jiff::Timestamp;
use uuid::Uuid;

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

fn application_status_from_i16(value: i16) -> crate::types::ApplicationStatus {
    let repr = u8_from_i16(value, "application.status");
    crate::types::ApplicationStatus::from_repr(repr)
        .unwrap_or_else(|| panic!("invalid application.status: {value}"))
}

fn application_channel_from_i16(value: i16) -> types::ApplicationChannel {
    let repr = u8_from_i16(value, "application.channel");
    types::ApplicationChannel::from_repr(repr)
        .unwrap_or_else(|| panic!("invalid application.channel: {value}"))
}

fn application_priority_from_i16(value: i16) -> types::Priority {
    let repr = u8_from_i16(value, "application.priority");
    types::Priority::from_repr(repr)
        .unwrap_or_else(|| panic!("invalid application.priority: {value}"))
}

// ---------------------------------------------------------------------------
// Application aggregate conversions
// ---------------------------------------------------------------------------

/// Store `Application` -> Domain `Application`.
///
/// `resume_id` in the store is `Option<Uuid>`, but in the domain it is
/// a `ResumeId` (mandatory). We fall back to `Uuid::nil()` when the
/// store row has no resume linked.
impl From<db_models::Application> for types::Application {
    fn from(a: db_models::Application) -> Self {
        Self {
            id:           job_domain_shared::id::ApplicationId::from(a.id),
            job_id:       job_domain_shared::id::JobSourceId::from(a.job_id),
            resume_id:    job_domain_shared::id::ResumeId::from(a.resume_id.unwrap_or(Uuid::nil())),
            channel:      application_channel_from_i16(a.channel),
            status:       application_status_from_i16(a.status),
            cover_letter: a.cover_letter,
            notes:        a.notes,
            tags:         a.tags,
            priority:     application_priority_from_i16(a.priority),
            trace_id:     a.trace_id,
            is_deleted:   a.is_deleted,
            submitted_at: chrono_opt_to_timestamp(a.submitted_at),
            created_at:   chrono_to_timestamp(a.created_at),
            updated_at:   chrono_to_timestamp(a.updated_at),
        }
    }
}

/// Domain `Application` -> Store `Application`.
///
/// `resume_id` is stored as `Option<Uuid>`; if the domain id is nil we
/// store `None`.
impl From<types::Application> for db_models::Application {
    fn from(a: types::Application) -> Self {
        let resume_uuid = a.resume_id.into_inner();
        Self {
            id:           a.id.into_inner(),
            job_id:       a.job_id.into_inner(),
            resume_id:    if resume_uuid.is_nil() {
                None
            } else {
                Some(resume_uuid)
            },
            channel:      a.channel as u8 as i16,
            status:       a.status as u8 as i16,
            cover_letter: a.cover_letter,
            notes:        a.notes,
            tags:         a.tags,
            priority:     a.priority as u8 as i16,
            trace_id:     a.trace_id,
            is_deleted:   a.is_deleted,
            deleted_at:   None,
            submitted_at: timestamp_opt_to_chrono(a.submitted_at),
            created_at:   timestamp_to_chrono(a.created_at),
            updated_at:   timestamp_to_chrono(a.updated_at),
        }
    }
}

// ---------------------------------------------------------------------------
// ApplicationStatusHistory / StatusChangeRecord conversions
// ---------------------------------------------------------------------------

/// Parse a `changed_by` string into a domain `ChangeSource`.
///
/// Known values: `"manual"`, `"system"`, `"email_parse"`.
/// Anything else (or `None`) defaults to `System`.
fn parse_change_source(s: Option<&str>) -> types::ChangeSource {
    match s {
        Some("manual") => types::ChangeSource::Manual,
        Some("system") => types::ChangeSource::System,
        Some("email_parse") => types::ChangeSource::EmailParse,
        _ => types::ChangeSource::System,
    }
}

/// Store `ApplicationStatusHistory` -> Domain `StatusChangeRecord`.
///
/// `from_status` in the store is `Option`; if absent we default to `Draft`.
impl From<db_models::ApplicationStatusHistory> for types::StatusChangeRecord {
    fn from(h: db_models::ApplicationStatusHistory) -> Self {
        Self {
            id:             h.id,
            application_id: job_domain_shared::id::ApplicationId::from(h.application_id),
            from_status:    h
                .from_status
                .map(application_status_from_i16)
                .unwrap_or(crate::types::ApplicationStatus::Draft),
            to_status:      application_status_from_i16(h.to_status),
            changed_by:     parse_change_source(h.changed_by.as_deref()),
            note:           h.note,
            created_at:     chrono_to_timestamp(h.created_at),
        }
    }
}

/// Domain `StatusChangeRecord` -> Store `ApplicationStatusHistory`.
impl From<types::StatusChangeRecord> for db_models::ApplicationStatusHistory {
    fn from(r: types::StatusChangeRecord) -> Self {
        Self {
            id:             r.id,
            application_id: r.application_id.into_inner(),
            from_status:    Some(r.from_status as u8 as i16),
            to_status:      r.to_status as u8 as i16,
            changed_by:     Some(r.changed_by.to_string()),
            note:           r.note,
            trace_id:       None,
            created_at:     timestamp_to_chrono(r.created_at),
        }
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    #[test]
    fn application_status_from_i16_works() {
        use crate::types::ApplicationStatus as D;

        assert_eq!(application_status_from_i16(0), D::Draft);
        assert_eq!(application_status_from_i16(1), D::Submitted);
        assert_eq!(application_status_from_i16(2), D::UnderReview);
        assert_eq!(application_status_from_i16(3), D::Interview);
        assert_eq!(application_status_from_i16(4), D::Offered);
    }

    #[test]
    fn application_channel_from_i16_works() {
        use types::ApplicationChannel as D;

        assert_eq!(application_channel_from_i16(0), D::Direct);
        assert_eq!(application_channel_from_i16(1), D::Referral);
        assert_eq!(application_channel_from_i16(2), D::LinkedIn);
        assert_eq!(application_channel_from_i16(3), D::Email);
        assert_eq!(application_channel_from_i16(4), D::Other);
    }

    #[test]
    fn application_priority_from_i16_works() {
        use types::Priority as D;

        assert_eq!(application_priority_from_i16(0), D::Low);
        assert_eq!(application_priority_from_i16(1), D::Medium);
        assert_eq!(application_priority_from_i16(2), D::High);
        assert_eq!(application_priority_from_i16(3), D::Critical);
    }

    #[test]
    fn application_store_to_domain_nil_resume() {
        let now = chrono::Utc::now();
        let store = db_models::Application {
            id:           Uuid::new_v4(),
            job_id:       Uuid::new_v4(),
            resume_id:    None,
            channel:      0,
            status:       0,
            cover_letter: None,
            notes:        None,
            tags:         vec![],
            priority:     1,
            trace_id:     None,
            is_deleted:   false,
            deleted_at:   None,
            submitted_at: None,
            created_at:   now,
            updated_at:   now,
        };

        let domain: types::Application = store.into();
        assert!(domain.resume_id.into_inner().is_nil());
    }

    #[test]
    fn change_source_parsing() {
        assert_eq!(
            parse_change_source(Some("manual")),
            types::ChangeSource::Manual
        );
        assert_eq!(
            parse_change_source(Some("system")),
            types::ChangeSource::System
        );
        assert_eq!(
            parse_change_source(Some("email_parse")),
            types::ChangeSource::EmailParse
        );
        assert_eq!(
            parse_change_source(Some("unknown")),
            types::ChangeSource::System
        );
        assert_eq!(parse_change_source(None), types::ChangeSource::System);
    }
}
