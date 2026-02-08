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

//! # job-domain-interview
//!
//! Interview management for the Job Automation platform.
//!
//! This crate covers everything related to the interview stage:
//!
//! - Scheduling interviews (date, time, type).
//! - Tracking interview rounds and interviewers.
//! - Recording feedback and outcomes.
//!
//! The crate depends on [`job_domain_core`] for shared types and traits.

use chrono::{DateTime, Utc};
use job_domain_core::{ApplicationId, InterviewId, InterviewStatus};
use serde::{Deserialize, Serialize};

/// Type of interview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InterviewKind {
    /// Phone / video screening.
    Screening,
    /// Technical / coding interview.
    Technical,
    /// Behavioral / culture-fit interview.
    Behavioral,
    /// On-site / final round.
    Onsite,
    /// Other / unspecified.
    Other,
}

/// An interview record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Interview {
    /// Unique identifier.
    pub id: InterviewId,
    /// The application this interview belongs to.
    pub application_id: ApplicationId,
    /// What kind of interview this is.
    pub kind: InterviewKind,
    /// Current status.
    pub status: InterviewStatus,
    /// Scheduled start time.
    pub scheduled_at: DateTime<Utc>,
    /// Optional notes / feedback.
    pub notes: Option<String>,
}
