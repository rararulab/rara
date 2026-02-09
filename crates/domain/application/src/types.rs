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
use job_domain_core::{
    id::{ApplicationId, JobSourceId, ResumeId},
    status::ApplicationStatus,
};
use serde::{Deserialize, Serialize};
use strum_macros::{Display, FromRepr};
use uuid::Uuid;

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
