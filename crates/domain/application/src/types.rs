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

//! Domain types for application lifecycle management.

use jiff::Timestamp;
use job_domain_shared::id::{ApplicationId, JobSourceId, ResumeId};
use serde::{Deserialize, Serialize};
use strum_macros::{Display, EnumString, FromRepr};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// ApplicationStatus
// ---------------------------------------------------------------------------

/// Lifecycle status of a job application.
#[repr(u8)]
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Display, EnumString, FromRepr,
)]
#[strum(serialize_all = "snake_case")]
pub enum ApplicationStatus {
    /// Application is in draft / being prepared.
    Draft = 0,
    /// Application has been submitted.
    Submitted = 1,
    /// Employer acknowledged receipt.
    UnderReview = 2,
    /// Candidate advanced to interview stage.
    Interview = 3,
    /// An offer was extended.
    Offered = 4,
    /// Application was rejected.
    Rejected = 5,
    /// Offer accepted.
    Accepted = 6,
    /// Application was withdrawn by the candidate.
    Withdrawn = 7,
}

// ---------------------------------------------------------------------------
// ApplicationChannel
// ---------------------------------------------------------------------------

/// The channel through which an application was submitted.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Display, FromRepr)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationChannel {
    /// Applied directly on the company website.
    #[strum(serialize = "direct")]
    Direct = 0,
    /// Referred by an employee or contact.
    #[strum(serialize = "referral")]
    Referral = 1,
    /// Applied via LinkedIn.
    #[strum(serialize = "linkedin")]
    LinkedIn = 2,
    /// Applied via email.
    #[strum(serialize = "email")]
    Email = 3,
    /// Any other channel.
    #[strum(serialize = "other")]
    Other = 4,
}

// ---------------------------------------------------------------------------
// Priority
// ---------------------------------------------------------------------------

/// Priority level assigned to an application.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Display, FromRepr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum Priority {
    /// Low priority -- nice to have.
    Low = 0,
    /// Medium priority -- worth pursuing.
    Medium = 1,
    /// High priority -- strong match.
    High = 2,
    /// Critical -- dream job or urgent deadline.
    Critical = 3,
}

impl Default for Priority {
    fn default() -> Self { Self::Medium }
}

// ---------------------------------------------------------------------------
// ChangeSource
// ---------------------------------------------------------------------------

/// How a status change was triggered.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Display, FromRepr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ChangeSource {
    /// Changed manually by the user.
    Manual = 0,
    /// Changed automatically by the system.
    System = 1,
    /// Inferred from a parsed email notification.
    EmailParse = 2,
}

// ---------------------------------------------------------------------------
// Application (aggregate root)
// ---------------------------------------------------------------------------

/// A job application aggregate.
///
/// Represents the full lifecycle of a single application, from draft
/// through submission, review, interview rounds, and final outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Application {
    /// Unique identifier.
    pub id:           ApplicationId,
    /// The job source this application targets.
    pub job_id:       JobSourceId,
    /// The resume version used for this application.
    pub resume_id:    ResumeId,
    /// Channel through which the application was submitted.
    pub channel:      ApplicationChannel,
    /// Current lifecycle status.
    pub status:       ApplicationStatus,
    /// Optional cover letter text.
    pub cover_letter: Option<String>,
    /// Free-form notes about the application.
    pub notes:        Option<String>,
    /// User-defined tags for categorization.
    pub tags:         Vec<String>,
    /// Priority level.
    pub priority:     Priority,
    /// External trace identifier for observability.
    pub trace_id:     Option<String>,
    /// Whether this application has been soft-deleted.
    pub is_deleted:   bool,
    /// When the application was submitted (if it has been).
    pub submitted_at: Option<Timestamp>,
    /// When the application was created.
    pub created_at:   Timestamp,
    /// When the application was last updated.
    pub updated_at:   Timestamp,
}

// ---------------------------------------------------------------------------
// StatusChangeRecord
// ---------------------------------------------------------------------------

/// A record of a single status transition in an application's history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusChangeRecord {
    /// Unique identifier for this history entry.
    pub id:             Uuid,
    /// The application that changed.
    pub application_id: ApplicationId,
    /// Status before the transition.
    pub from_status:    ApplicationStatus,
    /// Status after the transition.
    pub to_status:      ApplicationStatus,
    /// What triggered the change.
    pub changed_by:     ChangeSource,
    /// Optional note describing why the transition occurred.
    pub note:           Option<String>,
    /// When the transition happened.
    pub created_at:     Timestamp,
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

/// Parameters for creating a new application.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateApplicationRequest {
    /// The job source to apply for.
    pub job_id:       JobSourceId,
    /// The resume version to use.
    pub resume_id:    ResumeId,
    /// Channel of application.
    pub channel:      ApplicationChannel,
    /// Optional cover letter.
    pub cover_letter: Option<String>,
    /// Optional notes.
    pub notes:        Option<String>,
    /// Tags for categorization.
    pub tags:         Vec<String>,
    /// Priority level.
    pub priority:     Priority,
}

/// Parameters for a partial update of an existing application.
///
/// Only the fields set to `Some` will be applied.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateApplicationRequest {
    /// Update the cover letter.
    pub cover_letter: Option<Option<String>>,
    /// Update the notes.
    pub notes:        Option<Option<String>>,
    /// Replace the tag list.
    pub tags:         Option<Vec<String>>,
    /// Update the priority.
    pub priority:     Option<Priority>,
    /// Update the channel.
    pub channel:      Option<ApplicationChannel>,
}

// ---------------------------------------------------------------------------
// Filters
// ---------------------------------------------------------------------------

/// Criteria for listing/searching applications.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApplicationFilter {
    /// Filter by status.
    pub status:         Option<ApplicationStatus>,
    /// Filter by job source.
    pub job_id:         Option<JobSourceId>,
    /// Filter by resume.
    pub resume_id:      Option<ResumeId>,
    /// Filter by channel.
    pub channel:        Option<ApplicationChannel>,
    /// Filter by priority.
    pub priority:       Option<Priority>,
    /// Applications must contain *all* of these tags.
    pub tags:           Option<Vec<String>>,
    /// Created at or after this timestamp.
    pub created_after:  Option<Timestamp>,
    /// Created at or before this timestamp.
    pub created_before: Option<Timestamp>,
}

// ---------------------------------------------------------------------------
// Statistics
// ---------------------------------------------------------------------------

/// Aggregate statistics across all applications, broken down by status.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApplicationStatistics {
    /// Total number of applications.
    pub total:     usize,
    /// Number of applications in each status.
    pub by_status: Vec<(ApplicationStatus, usize)>,
}

// ---------------------------------------------------------------------------
// DB model conversions
// ---------------------------------------------------------------------------

use job_domain_shared::convert::{
    chrono_opt_to_timestamp, chrono_to_timestamp, timestamp_opt_to_chrono, timestamp_to_chrono,
    u8_from_i16,
};
use job_model::application::{Application as StoreApplication, ApplicationStatusHistory};

fn application_status_from_i16(value: i16) -> ApplicationStatus {
    let repr = u8_from_i16(value, "application.status");
    ApplicationStatus::from_repr(repr)
        .unwrap_or_else(|| panic!("invalid application.status: {value}"))
}

fn application_channel_from_i16(value: i16) -> ApplicationChannel {
    let repr = u8_from_i16(value, "application.channel");
    ApplicationChannel::from_repr(repr)
        .unwrap_or_else(|| panic!("invalid application.channel: {value}"))
}

fn application_priority_from_i16(value: i16) -> Priority {
    let repr = u8_from_i16(value, "application.priority");
    Priority::from_repr(repr).unwrap_or_else(|| panic!("invalid application.priority: {value}"))
}

/// Store `Application` -> Domain `Application`.
///
/// `resume_id` in the store is `Option<Uuid>`, but in the domain it is
/// a `ResumeId` (mandatory). We fall back to `Uuid::nil()` when the
/// store row has no resume linked.
impl From<StoreApplication> for Application {
    fn from(a: StoreApplication) -> Self {
        Self {
            id:           ApplicationId::from(a.id),
            job_id:       JobSourceId::from(a.job_id),
            resume_id:    ResumeId::from(a.resume_id.unwrap_or(Uuid::nil())),
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
impl From<Application> for StoreApplication {
    fn from(a: Application) -> Self {
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
fn parse_change_source(s: Option<&str>) -> ChangeSource {
    match s {
        Some("manual") => ChangeSource::Manual,
        Some("system") => ChangeSource::System,
        Some("email_parse") => ChangeSource::EmailParse,
        _ => ChangeSource::System,
    }
}

/// Store `ApplicationStatusHistory` -> Domain `StatusChangeRecord`.
///
/// `from_status` in the store is `Option`; if absent we default to `Draft`.
impl From<ApplicationStatusHistory> for StatusChangeRecord {
    fn from(h: ApplicationStatusHistory) -> Self {
        Self {
            id:             h.id,
            application_id: ApplicationId::from(h.application_id),
            from_status:    h
                .from_status
                .map(application_status_from_i16)
                .unwrap_or(ApplicationStatus::Draft),
            to_status:      application_status_from_i16(h.to_status),
            changed_by:     parse_change_source(h.changed_by.as_deref()),
            note:           h.note,
            created_at:     chrono_to_timestamp(h.created_at),
        }
    }
}

/// Domain `StatusChangeRecord` -> Store `ApplicationStatusHistory`.
impl From<StatusChangeRecord> for ApplicationStatusHistory {
    fn from(r: StatusChangeRecord) -> Self {
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
