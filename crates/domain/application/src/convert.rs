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

use uuid::Uuid;

use crate::db_models;
use crate::types;

// ---------------------------------------------------------------------------
// ApplicationStatus conversions
// ---------------------------------------------------------------------------

/// Store `ApplicationStatus` -> Domain `ApplicationStatus`.
///
/// Key mappings:
/// - `InProgress` -> `UnderReview`
/// - `Interviewing` -> `Interview`
impl From<db_models::ApplicationStatus> for job_domain_core::status::ApplicationStatus {
    fn from(s: db_models::ApplicationStatus) -> Self {
        match s {
            db_models::ApplicationStatus::Draft => Self::Draft,
            db_models::ApplicationStatus::Submitted => Self::Submitted,
            db_models::ApplicationStatus::InProgress => Self::UnderReview,
            db_models::ApplicationStatus::Interviewing => Self::Interview,
            db_models::ApplicationStatus::Offered => Self::Offered,
            db_models::ApplicationStatus::Rejected => Self::Rejected,
            db_models::ApplicationStatus::Withdrawn => Self::Withdrawn,
            db_models::ApplicationStatus::Accepted => Self::Accepted,
        }
    }
}

/// Domain `ApplicationStatus` -> Store `ApplicationStatus`.
///
/// Reverse of the above:
/// - `UnderReview` -> `InProgress`
/// - `Interview` -> `Interviewing`
impl From<job_domain_core::status::ApplicationStatus> for db_models::ApplicationStatus {
    fn from(s: job_domain_core::status::ApplicationStatus) -> Self {
        match s {
            job_domain_core::status::ApplicationStatus::Draft => Self::Draft,
            job_domain_core::status::ApplicationStatus::Submitted => Self::Submitted,
            job_domain_core::status::ApplicationStatus::UnderReview => Self::InProgress,
            job_domain_core::status::ApplicationStatus::Interview => Self::Interviewing,
            job_domain_core::status::ApplicationStatus::Offered => Self::Offered,
            job_domain_core::status::ApplicationStatus::Rejected => Self::Rejected,
            job_domain_core::status::ApplicationStatus::Withdrawn => Self::Withdrawn,
            job_domain_core::status::ApplicationStatus::Accepted => Self::Accepted,
        }
    }
}

// ---------------------------------------------------------------------------
// ApplicationChannel conversions
// ---------------------------------------------------------------------------

/// Store `ApplicationChannel` -> Domain `ApplicationChannel`.
impl From<db_models::ApplicationChannel> for types::ApplicationChannel {
    fn from(c: db_models::ApplicationChannel) -> Self {
        match c {
            db_models::ApplicationChannel::Direct => Self::Direct,
            db_models::ApplicationChannel::Referral => Self::Referral,
            db_models::ApplicationChannel::Linkedin => Self::LinkedIn,
            db_models::ApplicationChannel::Email => Self::Email,
            db_models::ApplicationChannel::Other => Self::Other,
        }
    }
}

/// Domain `ApplicationChannel` -> Store `ApplicationChannel`.
impl From<types::ApplicationChannel> for db_models::ApplicationChannel {
    fn from(c: types::ApplicationChannel) -> Self {
        match c {
            types::ApplicationChannel::Direct => Self::Direct,
            types::ApplicationChannel::Referral => Self::Referral,
            types::ApplicationChannel::LinkedIn => Self::Linkedin,
            types::ApplicationChannel::Email => Self::Email,
            types::ApplicationChannel::Other => Self::Other,
        }
    }
}

// ---------------------------------------------------------------------------
// ApplicationPriority conversions
// ---------------------------------------------------------------------------

/// Store `ApplicationPriority` -> Domain `Priority`.
impl From<db_models::ApplicationPriority> for types::Priority {
    fn from(p: db_models::ApplicationPriority) -> Self {
        match p {
            db_models::ApplicationPriority::Low => Self::Low,
            db_models::ApplicationPriority::Medium => Self::Medium,
            db_models::ApplicationPriority::High => Self::High,
            db_models::ApplicationPriority::Critical => Self::Critical,
        }
    }
}

/// Domain `Priority` -> Store `ApplicationPriority`.
impl From<types::Priority> for db_models::ApplicationPriority {
    fn from(p: types::Priority) -> Self {
        match p {
            types::Priority::Low => Self::Low,
            types::Priority::Medium => Self::Medium,
            types::Priority::High => Self::High,
            types::Priority::Critical => Self::Critical,
        }
    }
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
            id:           job_domain_core::id::ApplicationId::from(a.id),
            job_id:       job_domain_core::id::JobSourceId::from(a.job_id),
            resume_id:    job_domain_core::id::ResumeId::from(
                a.resume_id.unwrap_or(Uuid::nil()),
            ),
            channel:      a.channel.into(),
            status:       a.status.into(),
            cover_letter: a.cover_letter,
            notes:        a.notes,
            tags:         a.tags,
            priority:     a.priority.into(),
            trace_id:     a.trace_id,
            is_deleted:   a.is_deleted,
            submitted_at: a.submitted_at,
            created_at:   a.created_at,
            updated_at:   a.updated_at,
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
            channel:      a.channel.into(),
            status:       a.status.into(),
            cover_letter: a.cover_letter,
            notes:        a.notes,
            tags:         a.tags,
            priority:     a.priority.into(),
            trace_id:     a.trace_id,
            is_deleted:   a.is_deleted,
            deleted_at:   None,
            submitted_at: a.submitted_at,
            created_at:   a.created_at,
            updated_at:   a.updated_at,
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
            application_id: job_domain_core::id::ApplicationId::from(h.application_id),
            from_status:    h
                .from_status
                .map(Into::into)
                .unwrap_or(job_domain_core::status::ApplicationStatus::Draft),
            to_status:      h.to_status.into(),
            changed_by:     parse_change_source(h.changed_by.as_deref()),
            note:           h.note,
            created_at:     h.created_at,
        }
    }
}

/// Domain `StatusChangeRecord` -> Store `ApplicationStatusHistory`.
impl From<types::StatusChangeRecord> for db_models::ApplicationStatusHistory {
    fn from(r: types::StatusChangeRecord) -> Self {
        Self {
            id:             r.id,
            application_id: r.application_id.into_inner(),
            from_status:    Some(r.from_status.into()),
            to_status:      r.to_status.into(),
            changed_by:     Some(r.changed_by.to_string()),
            note:           r.note,
            trace_id:       None,
            created_at:     r.created_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use uuid::Uuid;

    use super::*;

    #[test]
    fn application_status_roundtrip() {
        use db_models::ApplicationStatus as S;
        use job_domain_core::status::ApplicationStatus as D;

        assert_eq!(D::from(S::Draft), D::Draft);
        assert_eq!(D::from(S::Submitted), D::Submitted);
        assert_eq!(D::from(S::InProgress), D::UnderReview);
        assert_eq!(D::from(S::Interviewing), D::Interview);
        assert_eq!(S::from(D::UnderReview), S::InProgress);
        assert_eq!(S::from(D::Interview), S::Interviewing);
    }

    #[test]
    fn application_channel_roundtrip() {
        use db_models::ApplicationChannel as S;
        use types::ApplicationChannel as D;

        let pairs = [
            (S::Direct, D::Direct),
            (S::Referral, D::Referral),
            (S::Linkedin, D::LinkedIn),
            (S::Email, D::Email),
            (S::Other, D::Other),
        ];
        for (store, domain) in &pairs {
            assert_eq!(D::from(*store), *domain);
            assert_eq!(S::from(*domain), *store);
        }
    }

    #[test]
    fn application_priority_roundtrip() {
        use db_models::ApplicationPriority as S;
        use types::Priority as D;

        let pairs = [
            (S::Low, D::Low),
            (S::Medium, D::Medium),
            (S::High, D::High),
            (S::Critical, D::Critical),
        ];
        for (store, domain) in &pairs {
            assert_eq!(D::from(*store), *domain);
            assert_eq!(S::from(*domain), *store);
        }
    }

    #[test]
    fn application_store_to_domain_nil_resume() {
        let now = Utc::now();
        let store = db_models::Application {
            id: Uuid::new_v4(),
            job_id: Uuid::new_v4(),
            resume_id: None,
            channel: db_models::ApplicationChannel::Direct,
            status: db_models::ApplicationStatus::Draft,
            cover_letter: None,
            notes: None,
            tags: vec![],
            priority: db_models::ApplicationPriority::Medium,
            trace_id: None,
            is_deleted: false,
            deleted_at: None,
            submitted_at: None,
            created_at: now,
            updated_at: now,
        };

        let domain: types::Application = store.into();
        assert!(domain.resume_id.into_inner().is_nil());
    }

    #[test]
    fn change_source_parsing() {
        assert_eq!(parse_change_source(Some("manual")), types::ChangeSource::Manual);
        assert_eq!(parse_change_source(Some("system")), types::ChangeSource::System);
        assert_eq!(
            parse_change_source(Some("email_parse")),
            types::ChangeSource::EmailParse
        );
        assert_eq!(parse_change_source(Some("unknown")), types::ChangeSource::System);
        assert_eq!(parse_change_source(None), types::ChangeSource::System);
    }
}
