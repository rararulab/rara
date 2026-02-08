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

//! Application entity and status history tracking.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// Channel through which the application was submitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "application_channel", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ApplicationChannel {
    Direct,
    Referral,
    Linkedin,
    Email,
    Other,
}

impl std::fmt::Display for ApplicationChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Direct => write!(f, "direct"),
            Self::Referral => write!(f, "referral"),
            Self::Linkedin => write!(f, "linkedin"),
            Self::Email => write!(f, "email"),
            Self::Other => write!(f, "other"),
        }
    }
}

/// Application processing status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "application_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ApplicationStatus {
    Draft,
    Submitted,
    InProgress,
    Interviewing,
    Offered,
    Rejected,
    Withdrawn,
    Accepted,
}

impl std::fmt::Display for ApplicationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Draft => write!(f, "draft"),
            Self::Submitted => write!(f, "submitted"),
            Self::InProgress => write!(f, "in_progress"),
            Self::Interviewing => write!(f, "interviewing"),
            Self::Offered => write!(f, "offered"),
            Self::Rejected => write!(f, "rejected"),
            Self::Withdrawn => write!(f, "withdrawn"),
            Self::Accepted => write!(f, "accepted"),
        }
    }
}

/// A job application record linking a job to a resume submission.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Application {
    pub id: Uuid,
    pub job_id: Uuid,
    pub resume_id: Option<Uuid>,
    pub channel: ApplicationChannel,
    pub status: ApplicationStatus,
    pub cover_letter: Option<String>,
    pub notes: Option<String>,
    pub trace_id: Option<String>,
    pub is_deleted: bool,
    pub deleted_at: Option<DateTime<Utc>>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// An immutable record of an application status transition.
///
/// Forms an append-only audit trail for the full lifecycle of an application.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ApplicationStatusHistory {
    pub id: Uuid,
    pub application_id: Uuid,
    pub from_status: Option<ApplicationStatus>,
    pub to_status: ApplicationStatus,
    /// Who or what triggered the status change (e.g. "user", "system", "ai").
    pub changed_by: Option<String>,
    pub note: Option<String>,
    pub trace_id: Option<String>,
    pub created_at: DateTime<Utc>,
}
