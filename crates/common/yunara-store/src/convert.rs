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

//! Conversion layer between store (DB) models and domain types.
//!
//! The store models map 1:1 to PostgreSQL columns while the domain
//! types are richer (newtypes, enums with variants not in the DB, etc.).
//! This module bridges the two via `From` implementations.

use uuid::Uuid;

use crate::models;

// ===========================================================================
// Resume conversions
// ===========================================================================

/// Store `ResumeSource` -> Domain `ResumeSource`.
impl From<models::resume::ResumeSource> for job_domain_resume::types::ResumeSource {
    fn from(s: models::resume::ResumeSource) -> Self {
        match s {
            models::resume::ResumeSource::Manual => Self::Manual,
            models::resume::ResumeSource::AiGenerated => Self::AiGenerated,
            models::resume::ResumeSource::Optimized => Self::Optimized,
        }
    }
}

/// Domain `ResumeSource` -> Store `ResumeSource`.
impl From<job_domain_resume::types::ResumeSource> for models::resume::ResumeSource {
    fn from(s: job_domain_resume::types::ResumeSource) -> Self {
        match s {
            job_domain_resume::types::ResumeSource::Manual => Self::Manual,
            job_domain_resume::types::ResumeSource::AiGenerated => Self::AiGenerated,
            job_domain_resume::types::ResumeSource::Optimized => Self::Optimized,
        }
    }
}

/// Store `Resume` -> Domain `Resume`.
impl From<models::resume::Resume> for job_domain_resume::types::Resume {
    fn from(r: models::resume::Resume) -> Self {
        Self {
            id:                  r.id,
            title:               r.title,
            version_tag:         r.version_tag,
            content_hash:        r.content_hash,
            source:              r.source.into(),
            content:             r.content,
            parent_resume_id:    r.parent_resume_id,
            target_job_id:       r.target_job_id,
            customization_notes: r.customization_notes,
            tags:                r.tags,
            metadata:            r.metadata,
            trace_id:            r.trace_id,
            is_deleted:          r.is_deleted,
            deleted_at:          r.deleted_at,
            created_at:          r.created_at,
            updated_at:          r.updated_at,
        }
    }
}

/// Domain `Resume` -> Store `Resume`.
impl From<job_domain_resume::types::Resume> for models::resume::Resume {
    fn from(r: job_domain_resume::types::Resume) -> Self {
        Self {
            id:                  r.id,
            title:               r.title,
            version_tag:         r.version_tag,
            content_hash:        r.content_hash,
            source:              r.source.into(),
            content:             r.content,
            parent_resume_id:    r.parent_resume_id,
            target_job_id:       r.target_job_id,
            customization_notes: r.customization_notes,
            tags:                r.tags,
            metadata:            r.metadata,
            trace_id:            r.trace_id,
            is_deleted:          r.is_deleted,
            deleted_at:          r.deleted_at,
            created_at:          r.created_at,
            updated_at:          r.updated_at,
        }
    }
}

// ===========================================================================
// Application status conversions
// ===========================================================================

/// Store `ApplicationStatus` -> Domain `ApplicationStatus`.
///
/// Key mappings:
/// - `InProgress` -> `UnderReview`
/// - `Interviewing` -> `Interview`
impl From<models::application::ApplicationStatus>
    for job_domain_core::status::ApplicationStatus
{
    fn from(s: models::application::ApplicationStatus) -> Self {
        match s {
            models::application::ApplicationStatus::Draft => Self::Draft,
            models::application::ApplicationStatus::Submitted => Self::Submitted,
            models::application::ApplicationStatus::InProgress => Self::UnderReview,
            models::application::ApplicationStatus::Interviewing => Self::Interview,
            models::application::ApplicationStatus::Offered => Self::Offered,
            models::application::ApplicationStatus::Rejected => Self::Rejected,
            models::application::ApplicationStatus::Withdrawn => Self::Withdrawn,
            models::application::ApplicationStatus::Accepted => Self::Accepted,
        }
    }
}

/// Domain `ApplicationStatus` -> Store `ApplicationStatus`.
///
/// Reverse of the above:
/// - `UnderReview` -> `InProgress`
/// - `Interview` -> `Interviewing`
impl From<job_domain_core::status::ApplicationStatus>
    for models::application::ApplicationStatus
{
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

// ===========================================================================
// Application channel conversions
// ===========================================================================

/// Store `ApplicationChannel` -> Domain `ApplicationChannel`.
impl From<models::application::ApplicationChannel>
    for job_domain_application::types::ApplicationChannel
{
    fn from(c: models::application::ApplicationChannel) -> Self {
        match c {
            models::application::ApplicationChannel::Direct => Self::Direct,
            models::application::ApplicationChannel::Referral => Self::Referral,
            models::application::ApplicationChannel::Linkedin => Self::LinkedIn,
            models::application::ApplicationChannel::Email => Self::Email,
            models::application::ApplicationChannel::Other => Self::Other,
        }
    }
}

/// Domain `ApplicationChannel` -> Store `ApplicationChannel`.
impl From<job_domain_application::types::ApplicationChannel>
    for models::application::ApplicationChannel
{
    fn from(c: job_domain_application::types::ApplicationChannel) -> Self {
        match c {
            job_domain_application::types::ApplicationChannel::Direct => Self::Direct,
            job_domain_application::types::ApplicationChannel::Referral => Self::Referral,
            job_domain_application::types::ApplicationChannel::LinkedIn => Self::Linkedin,
            job_domain_application::types::ApplicationChannel::Email => Self::Email,
            job_domain_application::types::ApplicationChannel::Other => Self::Other,
        }
    }
}

// ===========================================================================
// Application priority conversions
// ===========================================================================

/// Store `ApplicationPriority` -> Domain `Priority`.
impl From<models::application::ApplicationPriority>
    for job_domain_application::types::Priority
{
    fn from(p: models::application::ApplicationPriority) -> Self {
        match p {
            models::application::ApplicationPriority::Low => Self::Low,
            models::application::ApplicationPriority::Medium => Self::Medium,
            models::application::ApplicationPriority::High => Self::High,
            models::application::ApplicationPriority::Critical => Self::Critical,
        }
    }
}

/// Domain `Priority` -> Store `ApplicationPriority`.
impl From<job_domain_application::types::Priority>
    for models::application::ApplicationPriority
{
    fn from(p: job_domain_application::types::Priority) -> Self {
        match p {
            job_domain_application::types::Priority::Low => Self::Low,
            job_domain_application::types::Priority::Medium => Self::Medium,
            job_domain_application::types::Priority::High => Self::High,
            job_domain_application::types::Priority::Critical => Self::Critical,
        }
    }
}

// ===========================================================================
// Application aggregate conversions
// ===========================================================================

/// Store `Application` -> Domain `Application`.
///
/// `resume_id` in the store is `Option<Uuid>`, but in the domain it is
/// a `ResumeId` (mandatory). We fall back to `Uuid::nil()` when the
/// store row has no resume linked.
impl From<models::application::Application> for job_domain_application::types::Application {
    fn from(a: models::application::Application) -> Self {
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
impl From<job_domain_application::types::Application> for models::application::Application {
    fn from(a: job_domain_application::types::Application) -> Self {
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

// ===========================================================================
// ApplicationStatusHistory / StatusChangeRecord conversions
// ===========================================================================

/// Parse a `changed_by` string into a domain `ChangeSource`.
///
/// Known values: `"manual"`, `"system"`, `"email_parse"`.
/// Anything else (or `None`) defaults to `System`.
fn parse_change_source(s: Option<&str>) -> job_domain_application::types::ChangeSource {
    match s {
        Some("manual") => job_domain_application::types::ChangeSource::Manual,
        Some("system") => job_domain_application::types::ChangeSource::System,
        Some("email_parse") => job_domain_application::types::ChangeSource::EmailParse,
        _ => job_domain_application::types::ChangeSource::System,
    }
}

/// Store `ApplicationStatusHistory` -> Domain `StatusChangeRecord`.
///
/// `from_status` in the store is `Option`; if absent we default to `Draft`.
impl From<models::application::ApplicationStatusHistory>
    for job_domain_application::types::StatusChangeRecord
{
    fn from(h: models::application::ApplicationStatusHistory) -> Self {
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
impl From<job_domain_application::types::StatusChangeRecord>
    for models::application::ApplicationStatusHistory
{
    fn from(r: job_domain_application::types::StatusChangeRecord) -> Self {
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

// ===========================================================================
// Interview round conversions
// ===========================================================================

/// Parse a round string from the DB into a domain `InterviewRound`.
pub fn parse_interview_round(s: &str) -> job_domain_interview::types::InterviewRound {
    match s {
        "phone_screen" => job_domain_interview::types::InterviewRound::PhoneScreen,
        "technical" => job_domain_interview::types::InterviewRound::Technical,
        "system_design" => job_domain_interview::types::InterviewRound::SystemDesign,
        "behavioral" => job_domain_interview::types::InterviewRound::Behavioral,
        "culture_fit" => job_domain_interview::types::InterviewRound::CultureFit,
        "manager_round" => job_domain_interview::types::InterviewRound::ManagerRound,
        "final_round" => job_domain_interview::types::InterviewRound::FinalRound,
        other => job_domain_interview::types::InterviewRound::Other(other.to_owned()),
    }
}

/// Serialise a domain `InterviewRound` to its DB string representation.
pub fn interview_round_to_string(r: &job_domain_interview::types::InterviewRound) -> String {
    match r {
        job_domain_interview::types::InterviewRound::PhoneScreen => "phone_screen".to_owned(),
        job_domain_interview::types::InterviewRound::Technical => "technical".to_owned(),
        job_domain_interview::types::InterviewRound::SystemDesign => "system_design".to_owned(),
        job_domain_interview::types::InterviewRound::Behavioral => "behavioral".to_owned(),
        job_domain_interview::types::InterviewRound::CultureFit => "culture_fit".to_owned(),
        job_domain_interview::types::InterviewRound::ManagerRound => "manager_round".to_owned(),
        job_domain_interview::types::InterviewRound::FinalRound => "final_round".to_owned(),
        job_domain_interview::types::InterviewRound::Other(s) => s.clone(),
    }
}

// ===========================================================================
// Interview task status conversions
// ===========================================================================

/// Store `InterviewTaskStatus` -> Domain `InterviewTaskStatus`.
impl From<models::interview::InterviewTaskStatus>
    for job_domain_interview::types::InterviewTaskStatus
{
    fn from(s: models::interview::InterviewTaskStatus) -> Self {
        match s {
            models::interview::InterviewTaskStatus::Pending => Self::Pending,
            models::interview::InterviewTaskStatus::InProgress => Self::InProgress,
            models::interview::InterviewTaskStatus::Completed => Self::Completed,
            models::interview::InterviewTaskStatus::Skipped => Self::Skipped,
        }
    }
}

/// Domain `InterviewTaskStatus` -> Store `InterviewTaskStatus`.
impl From<job_domain_interview::types::InterviewTaskStatus>
    for models::interview::InterviewTaskStatus
{
    fn from(s: job_domain_interview::types::InterviewTaskStatus) -> Self {
        match s {
            job_domain_interview::types::InterviewTaskStatus::Pending => Self::Pending,
            job_domain_interview::types::InterviewTaskStatus::InProgress => Self::InProgress,
            job_domain_interview::types::InterviewTaskStatus::Completed => Self::Completed,
            job_domain_interview::types::InterviewTaskStatus::Skipped => Self::Skipped,
        }
    }
}

// ===========================================================================
// Interview plan conversions
// ===========================================================================

/// Store `InterviewPlan` -> Domain `InterviewPlan`.
///
/// `materials` (JSONB) is deserialized into `PrepMaterials`; on failure
/// we fall back to `PrepMaterials::default()`.
impl From<models::interview::InterviewPlan> for job_domain_interview::types::InterviewPlan {
    fn from(p: models::interview::InterviewPlan) -> Self {
        let prep_materials: job_domain_interview::types::PrepMaterials = p
            .materials
            .as_ref()
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        Self {
            id:              job_domain_core::id::InterviewId::from(p.id),
            application_id:  job_domain_core::id::ApplicationId::from(p.application_id),
            title:           p.title,
            company:         p.company,
            position:        p.position,
            job_description: p.job_description,
            round:           parse_interview_round(&p.round),
            scheduled_at:    p.scheduled_at,
            task_status:     p.task_status.into(),
            prep_materials,
            notes:           p.notes,
            trace_id:        p.trace_id,
            is_deleted:      p.is_deleted,
            deleted_at:      p.deleted_at,
            created_at:      p.created_at,
            updated_at:      p.updated_at,
        }
    }
}

/// Domain `InterviewPlan` -> Store `InterviewPlan`.
///
/// `prep_materials` is serialised to JSONB.
impl From<job_domain_interview::types::InterviewPlan> for models::interview::InterviewPlan {
    fn from(p: job_domain_interview::types::InterviewPlan) -> Self {
        let materials = serde_json::to_value(&p.prep_materials).ok();

        Self {
            id:              p.id.into_inner(),
            application_id:  p.application_id.into_inner(),
            title:           p.title,
            company:         p.company,
            position:        p.position,
            job_description: p.job_description,
            round:           interview_round_to_string(&p.round),
            description:     None,
            scheduled_at:    p.scheduled_at,
            task_status:     p.task_status.into(),
            materials,
            notes:           p.notes,
            trace_id:        p.trace_id,
            is_deleted:      p.is_deleted,
            deleted_at:      p.deleted_at,
            created_at:      p.created_at,
            updated_at:      p.updated_at,
        }
    }
}

// ===========================================================================
// Notification conversions
// ===========================================================================

/// Store `NotificationChannel` -> Domain `NotificationChannel`.
impl From<models::notification::NotificationChannel>
    for job_domain_notify::types::NotificationChannel
{
    fn from(value: models::notification::NotificationChannel) -> Self {
        match value {
            models::notification::NotificationChannel::Telegram => Self::Telegram,
            models::notification::NotificationChannel::Email => Self::Email,
            models::notification::NotificationChannel::Webhook => Self::Webhook,
            models::notification::NotificationChannel::Other => Self::Webhook,
        }
    }
}

/// Domain `NotificationChannel` -> Store `NotificationChannel`.
impl From<job_domain_notify::types::NotificationChannel>
    for models::notification::NotificationChannel
{
    fn from(value: job_domain_notify::types::NotificationChannel) -> Self {
        match value {
            job_domain_notify::types::NotificationChannel::Telegram => Self::Telegram,
            job_domain_notify::types::NotificationChannel::Email => Self::Email,
            job_domain_notify::types::NotificationChannel::Webhook => Self::Webhook,
        }
    }
}

/// Store `NotificationStatus` -> Domain `NotificationStatus`.
impl From<models::notification::NotificationStatus>
    for job_domain_notify::types::NotificationStatus
{
    fn from(value: models::notification::NotificationStatus) -> Self {
        match value {
            models::notification::NotificationStatus::Pending => Self::Pending,
            models::notification::NotificationStatus::Sent => Self::Sent,
            models::notification::NotificationStatus::Failed => Self::Failed,
            models::notification::NotificationStatus::Retrying => Self::Retrying,
        }
    }
}

/// Domain `NotificationStatus` -> Store `NotificationStatus`.
impl From<job_domain_notify::types::NotificationStatus>
    for models::notification::NotificationStatus
{
    fn from(value: job_domain_notify::types::NotificationStatus) -> Self {
        match value {
            job_domain_notify::types::NotificationStatus::Pending => Self::Pending,
            job_domain_notify::types::NotificationStatus::Sent => Self::Sent,
            job_domain_notify::types::NotificationStatus::Failed => Self::Failed,
            job_domain_notify::types::NotificationStatus::Retrying => Self::Retrying,
        }
    }
}

/// Store `NotificationPriority` -> Domain `NotificationPriority`.
impl From<models::notification::NotificationPriority>
    for job_domain_notify::types::NotificationPriority
{
    fn from(value: models::notification::NotificationPriority) -> Self {
        match value {
            models::notification::NotificationPriority::Low => Self::Low,
            models::notification::NotificationPriority::Normal => Self::Normal,
            models::notification::NotificationPriority::High => Self::High,
            models::notification::NotificationPriority::Urgent => Self::Urgent,
        }
    }
}

/// Domain `NotificationPriority` -> Store `NotificationPriority`.
impl From<job_domain_notify::types::NotificationPriority>
    for models::notification::NotificationPriority
{
    fn from(value: job_domain_notify::types::NotificationPriority) -> Self {
        match value {
            job_domain_notify::types::NotificationPriority::Low => Self::Low,
            job_domain_notify::types::NotificationPriority::Normal => Self::Normal,
            job_domain_notify::types::NotificationPriority::High => Self::High,
            job_domain_notify::types::NotificationPriority::Urgent => Self::Urgent,
        }
    }
}

/// Store `NotificationLog` -> Domain `Notification`.
impl From<models::notification::NotificationLog> for job_domain_notify::types::Notification {
    fn from(n: models::notification::NotificationLog) -> Self {
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
impl From<job_domain_notify::types::Notification> for models::notification::NotificationLog {
    fn from(n: job_domain_notify::types::Notification) -> Self {
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

// ===========================================================================
// Scheduler conversions
// ===========================================================================

/// Store `TaskRunStatus` -> Domain `TaskRunStatus`.
impl From<models::scheduler::TaskRunStatus> for job_domain_scheduler::types::TaskRunStatus {
    fn from(value: models::scheduler::TaskRunStatus) -> Self {
        match value {
            models::scheduler::TaskRunStatus::Success => Self::Success,
            models::scheduler::TaskRunStatus::Failed => Self::Failed,
            models::scheduler::TaskRunStatus::Running => Self::Running,
        }
    }
}

/// Domain `TaskRunStatus` -> Store `TaskRunStatus`.
impl From<job_domain_scheduler::types::TaskRunStatus> for models::scheduler::TaskRunStatus {
    fn from(value: job_domain_scheduler::types::TaskRunStatus) -> Self {
        match value {
            job_domain_scheduler::types::TaskRunStatus::Success => Self::Success,
            job_domain_scheduler::types::TaskRunStatus::Failed => Self::Failed,
            job_domain_scheduler::types::TaskRunStatus::Running => Self::Running,
        }
    }
}

/// Store `SchedulerTask` -> Domain `ScheduledTask`.
impl From<models::scheduler::SchedulerTask> for job_domain_scheduler::types::ScheduledTask {
    fn from(t: models::scheduler::SchedulerTask) -> Self {
        Self {
            id:            job_domain_core::id::SchedulerTaskId::from(t.id),
            name:          t.name,
            cron_expr:     t.cron_expr,
            enabled:       t.enabled,
            last_run_at:   t.last_run_at,
            last_status:   t.last_status.map(Into::into),
            last_error:    t.last_error,
            run_count:     t.run_count,
            failure_count: t.failure_count,
            created_at:    t.created_at,
            updated_at:    t.updated_at,
        }
    }
}

/// Domain `ScheduledTask` -> Store `SchedulerTask`.
impl From<job_domain_scheduler::types::ScheduledTask> for models::scheduler::SchedulerTask {
    fn from(t: job_domain_scheduler::types::ScheduledTask) -> Self {
        Self {
            id:            t.id.into_inner(),
            name:          t.name,
            cron_expr:     t.cron_expr,
            enabled:       t.enabled,
            last_run_at:   t.last_run_at,
            last_status:   t.last_status.map(Into::into),
            last_error:    t.last_error,
            run_count:     t.run_count,
            failure_count: t.failure_count,
            is_deleted:    false,
            deleted_at:    None,
            created_at:    t.created_at,
            updated_at:    t.updated_at,
        }
    }
}

/// Store `TaskRunHistory` -> Domain `TaskRunRecord`.
impl From<models::scheduler::TaskRunHistory> for job_domain_scheduler::types::TaskRunRecord {
    fn from(r: models::scheduler::TaskRunHistory) -> Self {
        Self {
            id:          r.id,
            task_id:     job_domain_core::id::SchedulerTaskId::from(r.task_id),
            status:      r.status.into(),
            started_at:  r.started_at,
            finished_at: r.finished_at,
            duration_ms: r.duration_ms,
            error:       r.error,
            output:      r.output,
            created_at:  r.created_at,
        }
    }
}

/// Domain `TaskRunRecord` -> Store `TaskRunHistory`.
impl From<job_domain_scheduler::types::TaskRunRecord> for models::scheduler::TaskRunHistory {
    fn from(r: job_domain_scheduler::types::TaskRunRecord) -> Self {
        Self {
            id:          r.id,
            task_id:     r.task_id.into_inner(),
            status:      r.status.into(),
            started_at:  r.started_at,
            finished_at: r.finished_at,
            duration_ms: r.duration_ms,
            error:       r.error,
            output:      r.output,
            created_at:  r.created_at,
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use uuid::Uuid;

    use super::*;

    // -----------------------------------------------------------------------
    // Resume conversions
    // -----------------------------------------------------------------------

    #[test]
    fn resume_source_store_to_domain() {
        assert_eq!(
            job_domain_resume::types::ResumeSource::from(models::resume::ResumeSource::Manual),
            job_domain_resume::types::ResumeSource::Manual,
        );
        assert_eq!(
            job_domain_resume::types::ResumeSource::from(
                models::resume::ResumeSource::AiGenerated
            ),
            job_domain_resume::types::ResumeSource::AiGenerated,
        );
        assert_eq!(
            job_domain_resume::types::ResumeSource::from(models::resume::ResumeSource::Optimized),
            job_domain_resume::types::ResumeSource::Optimized,
        );
    }

    #[test]
    fn resume_source_domain_to_store() {
        assert_eq!(
            models::resume::ResumeSource::from(job_domain_resume::types::ResumeSource::Manual),
            models::resume::ResumeSource::Manual,
        );
        assert_eq!(
            models::resume::ResumeSource::from(
                job_domain_resume::types::ResumeSource::AiGenerated
            ),
            models::resume::ResumeSource::AiGenerated,
        );
        assert_eq!(
            models::resume::ResumeSource::from(job_domain_resume::types::ResumeSource::Optimized),
            models::resume::ResumeSource::Optimized,
        );
    }

    #[test]
    fn resume_store_to_domain_roundtrip() {
        let now = Utc::now();
        let id = Uuid::new_v4();
        let store_resume = models::resume::Resume {
            id,
            title: "Backend v1".into(),
            version_tag: "v1.0".into(),
            content_hash: "abc123".into(),
            source: models::resume::ResumeSource::Manual,
            content: Some("Resume content".into()),
            parent_resume_id: None,
            target_job_id: None,
            customization_notes: None,
            tags: vec!["rust".into()],
            metadata: None,
            trace_id: None,
            is_deleted: false,
            deleted_at: None,
            created_at: now,
            updated_at: now,
        };

        let domain: job_domain_resume::types::Resume = store_resume.into();
        assert_eq!(domain.id, id);
        assert_eq!(domain.title, "Backend v1");
        assert_eq!(domain.tags, vec!["rust".to_owned()]);

        let back: models::resume::Resume = domain.into();
        assert_eq!(back.id, id);
        assert_eq!(back.title, "Backend v1");
    }

    // -----------------------------------------------------------------------
    // Application status conversions
    // -----------------------------------------------------------------------

    #[test]
    fn application_status_store_to_domain() {
        use job_domain_core::status::ApplicationStatus as D;
        use models::application::ApplicationStatus as S;

        assert_eq!(D::from(S::Draft), D::Draft);
        assert_eq!(D::from(S::Submitted), D::Submitted);
        assert_eq!(D::from(S::InProgress), D::UnderReview);
        assert_eq!(D::from(S::Interviewing), D::Interview);
        assert_eq!(D::from(S::Offered), D::Offered);
        assert_eq!(D::from(S::Rejected), D::Rejected);
        assert_eq!(D::from(S::Withdrawn), D::Withdrawn);
        assert_eq!(D::from(S::Accepted), D::Accepted);
    }

    #[test]
    fn application_status_domain_to_store() {
        use job_domain_core::status::ApplicationStatus as D;
        use models::application::ApplicationStatus as S;

        assert_eq!(S::from(D::Draft), S::Draft);
        assert_eq!(S::from(D::Submitted), S::Submitted);
        assert_eq!(S::from(D::UnderReview), S::InProgress);
        assert_eq!(S::from(D::Interview), S::Interviewing);
        assert_eq!(S::from(D::Offered), S::Offered);
        assert_eq!(S::from(D::Rejected), S::Rejected);
        assert_eq!(S::from(D::Withdrawn), S::Withdrawn);
        assert_eq!(S::from(D::Accepted), S::Accepted);
    }

    // -----------------------------------------------------------------------
    // Application channel conversions
    // -----------------------------------------------------------------------

    #[test]
    fn application_channel_roundtrip() {
        use job_domain_application::types::ApplicationChannel as D;
        use models::application::ApplicationChannel as S;

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

    // -----------------------------------------------------------------------
    // Application priority conversions
    // -----------------------------------------------------------------------

    #[test]
    fn application_priority_roundtrip() {
        use job_domain_application::types::Priority as D;
        use models::application::ApplicationPriority as S;

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

    // -----------------------------------------------------------------------
    // Application aggregate conversions
    // -----------------------------------------------------------------------

    #[test]
    fn application_store_to_domain_with_resume() {
        let now = Utc::now();
        let resume_id = Uuid::new_v4();
        let store = models::application::Application {
            id: Uuid::new_v4(),
            job_id: Uuid::new_v4(),
            resume_id: Some(resume_id),
            channel: models::application::ApplicationChannel::Linkedin,
            status: models::application::ApplicationStatus::InProgress,
            cover_letter: Some("CL".into()),
            notes: None,
            tags: vec!["tag1".into()],
            priority: models::application::ApplicationPriority::High,
            trace_id: None,
            is_deleted: false,
            deleted_at: None,
            submitted_at: None,
            created_at: now,
            updated_at: now,
        };

        let domain: job_domain_application::types::Application = store.into();
        assert_eq!(domain.resume_id.into_inner(), resume_id);
        assert_eq!(domain.status, job_domain_core::status::ApplicationStatus::UnderReview);
        assert_eq!(domain.channel, job_domain_application::types::ApplicationChannel::LinkedIn);
        assert_eq!(domain.priority, job_domain_application::types::Priority::High);
    }

    #[test]
    fn application_store_to_domain_nil_resume() {
        let now = Utc::now();
        let store = models::application::Application {
            id: Uuid::new_v4(),
            job_id: Uuid::new_v4(),
            resume_id: None,
            channel: models::application::ApplicationChannel::Direct,
            status: models::application::ApplicationStatus::Draft,
            cover_letter: None,
            notes: None,
            tags: vec![],
            priority: models::application::ApplicationPriority::Medium,
            trace_id: None,
            is_deleted: false,
            deleted_at: None,
            submitted_at: None,
            created_at: now,
            updated_at: now,
        };

        let domain: job_domain_application::types::Application = store.into();
        assert!(domain.resume_id.into_inner().is_nil());
    }

    #[test]
    fn application_domain_to_store_nil_resume() {
        let now = Utc::now();
        let domain = job_domain_application::types::Application {
            id: job_domain_core::id::ApplicationId::from(Uuid::new_v4()),
            job_id: job_domain_core::id::JobSourceId::from(Uuid::new_v4()),
            resume_id: job_domain_core::id::ResumeId::from(Uuid::nil()),
            channel: job_domain_application::types::ApplicationChannel::Direct,
            status: job_domain_core::status::ApplicationStatus::Draft,
            cover_letter: None,
            notes: None,
            tags: vec![],
            priority: job_domain_application::types::Priority::Medium,
            trace_id: None,
            is_deleted: false,
            submitted_at: None,
            created_at: now,
            updated_at: now,
        };

        let store: models::application::Application = domain.into();
        assert!(store.resume_id.is_none());
    }

    // -----------------------------------------------------------------------
    // Status history / ChangeSource conversions
    // -----------------------------------------------------------------------

    #[test]
    fn change_source_parsing() {
        use job_domain_application::types::ChangeSource;

        assert_eq!(parse_change_source(Some("manual")), ChangeSource::Manual);
        assert_eq!(parse_change_source(Some("system")), ChangeSource::System);
        assert_eq!(
            parse_change_source(Some("email_parse")),
            ChangeSource::EmailParse
        );
        assert_eq!(parse_change_source(Some("unknown")), ChangeSource::System);
        assert_eq!(parse_change_source(None), ChangeSource::System);
    }

    #[test]
    fn status_history_store_to_domain() {
        let now = Utc::now();
        let store = models::application::ApplicationStatusHistory {
            id: Uuid::new_v4(),
            application_id: Uuid::new_v4(),
            from_status: Some(models::application::ApplicationStatus::Submitted),
            to_status: models::application::ApplicationStatus::InProgress,
            changed_by: Some("manual".into()),
            note: Some("Moved to review".into()),
            trace_id: None,
            created_at: now,
        };

        let domain: job_domain_application::types::StatusChangeRecord = store.into();
        assert_eq!(
            domain.from_status,
            job_domain_core::status::ApplicationStatus::Submitted
        );
        assert_eq!(
            domain.to_status,
            job_domain_core::status::ApplicationStatus::UnderReview
        );
        assert_eq!(
            domain.changed_by,
            job_domain_application::types::ChangeSource::Manual
        );
    }

    #[test]
    fn status_history_domain_to_store() {
        let now = Utc::now();
        let domain = job_domain_application::types::StatusChangeRecord {
            id: Uuid::new_v4(),
            application_id: job_domain_core::id::ApplicationId::from(Uuid::new_v4()),
            from_status: job_domain_core::status::ApplicationStatus::Interview,
            to_status: job_domain_core::status::ApplicationStatus::Offered,
            changed_by: job_domain_application::types::ChangeSource::EmailParse,
            note: None,
            created_at: now,
        };

        let store: models::application::ApplicationStatusHistory = domain.into();
        assert_eq!(
            store.from_status,
            Some(models::application::ApplicationStatus::Interviewing)
        );
        assert_eq!(
            store.to_status,
            models::application::ApplicationStatus::Offered
        );
        assert_eq!(store.changed_by, Some("email_parse".to_owned()));
    }

    // -----------------------------------------------------------------------
    // Interview round conversions
    // -----------------------------------------------------------------------

    #[test]
    fn interview_round_parse() {
        use job_domain_interview::types::InterviewRound;

        assert_eq!(parse_interview_round("phone_screen"), InterviewRound::PhoneScreen);
        assert_eq!(parse_interview_round("technical"), InterviewRound::Technical);
        assert_eq!(
            parse_interview_round("system_design"),
            InterviewRound::SystemDesign
        );
        assert_eq!(
            parse_interview_round("behavioral"),
            InterviewRound::Behavioral
        );
        assert_eq!(
            parse_interview_round("culture_fit"),
            InterviewRound::CultureFit
        );
        assert_eq!(
            parse_interview_round("manager_round"),
            InterviewRound::ManagerRound
        );
        assert_eq!(
            parse_interview_round("final_round"),
            InterviewRound::FinalRound
        );
        assert_eq!(
            parse_interview_round("panel"),
            InterviewRound::Other("panel".to_owned())
        );
    }

    #[test]
    fn interview_round_to_string_roundtrip() {
        use job_domain_interview::types::InterviewRound;

        let cases = vec![
            (InterviewRound::PhoneScreen, "phone_screen"),
            (InterviewRound::Technical, "technical"),
            (InterviewRound::SystemDesign, "system_design"),
            (InterviewRound::Behavioral, "behavioral"),
            (InterviewRound::CultureFit, "culture_fit"),
            (InterviewRound::ManagerRound, "manager_round"),
            (InterviewRound::FinalRound, "final_round"),
        ];
        for (round, expected) in cases {
            let s = interview_round_to_string(&round);
            assert_eq!(s, expected);
            assert_eq!(parse_interview_round(&s), round);
        }
    }

    // -----------------------------------------------------------------------
    // Interview task status conversions
    // -----------------------------------------------------------------------

    #[test]
    fn interview_task_status_roundtrip() {
        use job_domain_interview::types::InterviewTaskStatus as D;
        use models::interview::InterviewTaskStatus as S;

        let pairs = [
            (S::Pending, D::Pending),
            (S::InProgress, D::InProgress),
            (S::Completed, D::Completed),
            (S::Skipped, D::Skipped),
        ];
        for (store, domain) in &pairs {
            assert_eq!(D::from(*store), *domain);
            assert_eq!(S::from(*domain), *store);
        }
    }

    // -----------------------------------------------------------------------
    // Interview plan conversions
    // -----------------------------------------------------------------------

    #[test]
    fn interview_plan_store_to_domain() {
        let now = Utc::now();
        let materials = serde_json::json!({
            "knowledge_points": ["Rust", "async"],
            "project_review_items": [],
            "behavioral_questions": [],
            "questions_to_ask": [],
            "additional_resources": []
        });

        let store = models::interview::InterviewPlan {
            id: Uuid::new_v4(),
            application_id: Uuid::new_v4(),
            title: "Tech Screen".into(),
            company: "Acme".into(),
            position: "SWE".into(),
            job_description: Some("Build stuff".into()),
            round: "technical".into(),
            description: None,
            scheduled_at: None,
            task_status: models::interview::InterviewTaskStatus::Pending,
            materials: Some(materials),
            notes: None,
            trace_id: None,
            is_deleted: false,
            deleted_at: None,
            created_at: now,
            updated_at: now,
        };

        let domain: job_domain_interview::types::InterviewPlan = store.into();
        assert_eq!(domain.company, "Acme");
        assert_eq!(domain.position, "SWE");
        assert_eq!(
            domain.round,
            job_domain_interview::types::InterviewRound::Technical
        );
        assert_eq!(domain.prep_materials.knowledge_points.len(), 2);
    }

    #[test]
    fn interview_plan_domain_to_store() {
        let now = Utc::now();
        let domain = job_domain_interview::types::InterviewPlan {
            id: job_domain_core::id::InterviewId::from(Uuid::new_v4()),
            application_id: job_domain_core::id::ApplicationId::from(Uuid::new_v4()),
            title: "Final".into(),
            company: "BigCo".into(),
            position: "Staff".into(),
            job_description: None,
            round: job_domain_interview::types::InterviewRound::FinalRound,
            scheduled_at: None,
            task_status: job_domain_interview::types::InterviewTaskStatus::Completed,
            prep_materials: job_domain_interview::types::PrepMaterials::default(),
            notes: Some("Went well".into()),
            trace_id: None,
            is_deleted: false,
            deleted_at: None,
            created_at: now,
            updated_at: now,
        };

        let store: models::interview::InterviewPlan = domain.into();
        assert_eq!(store.company, "BigCo");
        assert_eq!(store.round, "final_round");
        assert_eq!(
            store.task_status,
            models::interview::InterviewTaskStatus::Completed
        );
        assert!(store.materials.is_some());
    }

    #[test]
    fn interview_plan_store_to_domain_null_materials() {
        let now = Utc::now();
        let store = models::interview::InterviewPlan {
            id: Uuid::new_v4(),
            application_id: Uuid::new_v4(),
            title: "Screen".into(),
            company: "".into(),
            position: "".into(),
            job_description: None,
            round: "phone_screen".into(),
            description: None,
            scheduled_at: None,
            task_status: models::interview::InterviewTaskStatus::Pending,
            materials: None,
            notes: None,
            trace_id: None,
            is_deleted: false,
            deleted_at: None,
            created_at: now,
            updated_at: now,
        };

        let domain: job_domain_interview::types::InterviewPlan = store.into();
        assert!(domain.prep_materials.knowledge_points.is_empty());
    }

    // -----------------------------------------------------------------------
    // Notification conversions
    // -----------------------------------------------------------------------

    #[test]
    fn notification_channel_roundtrip() {
        use job_domain_notify::types::NotificationChannel as D;
        use models::notification::NotificationChannel as S;

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
        use job_domain_notify::types::NotificationChannel as D;
        use models::notification::NotificationChannel as S;

        assert_eq!(D::from(S::Other), D::Webhook);
    }

    #[test]
    fn notification_status_roundtrip() {
        use job_domain_notify::types::NotificationStatus as D;
        use models::notification::NotificationStatus as S;

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
        use job_domain_notify::types::NotificationPriority as D;
        use models::notification::NotificationPriority as S;

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
        let store_log = models::notification::NotificationLog {
            id,
            channel: models::notification::NotificationChannel::Telegram,
            recipient: "user123".into(),
            subject: Some("Test subject".into()),
            body: "Test body".into(),
            status: models::notification::NotificationStatus::Pending,
            priority: models::notification::NotificationPriority::High,
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

        let domain: job_domain_notify::types::Notification = store_log.into();
        assert_eq!(domain.id, id);
        assert_eq!(domain.channel, job_domain_notify::types::NotificationChannel::Telegram);
        assert_eq!(domain.recipient, "user123");
        assert_eq!(domain.priority, job_domain_notify::types::NotificationPriority::High);
        assert_eq!(domain.max_retries, 3);
        assert_eq!(domain.reference_id, Some(ref_id));

        let back: models::notification::NotificationLog = domain.into();
        assert_eq!(back.id, id);
        assert_eq!(back.channel, models::notification::NotificationChannel::Telegram);
        assert_eq!(back.recipient, "user123");
    }

    // -----------------------------------------------------------------------
    // Scheduler conversions
    // -----------------------------------------------------------------------

    #[test]
    fn scheduler_task_run_status_roundtrip() {
        use job_domain_scheduler::types::TaskRunStatus as D;
        use models::scheduler::TaskRunStatus as S;

        let pairs = [
            (S::Success, D::Success),
            (S::Failed, D::Failed),
            (S::Running, D::Running),
        ];
        for (store, domain) in &pairs {
            assert_eq!(D::from(*store), *domain);
            assert_eq!(S::from(*domain), *store);
        }
    }

    #[test]
    fn scheduler_task_store_to_domain_roundtrip() {
        let now = Utc::now();
        let id = Uuid::new_v4();
        let store_task = models::scheduler::SchedulerTask {
            id,
            name: "job-discovery".into(),
            cron_expr: "0 */30 * * * *".into(),
            enabled: true,
            last_run_at: Some(now),
            last_status: Some(models::scheduler::TaskRunStatus::Success),
            last_error: None,
            run_count: 5,
            failure_count: 1,
            is_deleted: false,
            deleted_at: None,
            created_at: now,
            updated_at: now,
        };

        let domain: job_domain_scheduler::types::ScheduledTask = store_task.into();
        assert_eq!(domain.id.into_inner(), id);
        assert_eq!(domain.name, "job-discovery");
        assert_eq!(domain.run_count, 5);
        assert_eq!(
            domain.last_status,
            Some(job_domain_scheduler::types::TaskRunStatus::Success)
        );

        let back: models::scheduler::SchedulerTask = domain.into();
        assert_eq!(back.id, id);
        assert_eq!(back.name, "job-discovery");
        assert!(!back.is_deleted);
    }

    #[test]
    fn task_run_history_store_to_domain_roundtrip() {
        let now = Utc::now();
        let id = Uuid::new_v4();
        let task_id = Uuid::new_v4();
        let store_run = models::scheduler::TaskRunHistory {
            id,
            task_id,
            status: models::scheduler::TaskRunStatus::Failed,
            started_at: now,
            finished_at: Some(now),
            duration_ms: Some(1500),
            error: Some("connection refused".into()),
            output: None,
            created_at: now,
        };

        let domain: job_domain_scheduler::types::TaskRunRecord = store_run.into();
        assert_eq!(domain.id, id);
        assert_eq!(domain.task_id.into_inner(), task_id);
        assert_eq!(domain.status, job_domain_scheduler::types::TaskRunStatus::Failed);
        assert_eq!(domain.duration_ms, Some(1500));

        let back: models::scheduler::TaskRunHistory = domain.into();
        assert_eq!(back.id, id);
        assert_eq!(back.task_id, task_id);
        assert_eq!(back.error, Some("connection refused".to_owned()));
    }
}
