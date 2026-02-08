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

//! # job-domain-application
//!
//! Application lifecycle management for the Job Automation platform.
//!
//! This crate models the full lifecycle of a job application from draft to
//! offer (or rejection).  It provides:
//!
//! - The [`Application`] aggregate with a state-machine for status transitions.
//! - Validation rules for legal status transitions.
//! - Domain events emitted on state changes.
//!
//! The crate depends on [`job_domain_core`] for shared types and traits.

use chrono::{DateTime, Utc};
use job_domain_core::{ApplicationId, ApplicationStatus, JobSourceId, ResumeId};
use serde::{Deserialize, Serialize};

/// A job application aggregate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Application {
    /// Unique identifier.
    pub id: ApplicationId,
    /// The job source this application targets.
    pub job_source_id: JobSourceId,
    /// The resume version used for this application.
    pub resume_id: ResumeId,
    /// Current lifecycle status.
    pub status: ApplicationStatus,
    /// When the application was created.
    pub created_at: DateTime<Utc>,
    /// When the application was last updated.
    pub updated_at: DateTime<Utc>,
}
