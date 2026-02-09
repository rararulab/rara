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

//! Domain events used for cross-crate communication.
//!
//! When one domain crate needs to notify others about something that happened
//! (e.g. "a new application was submitted"), it publishes a domain event rather
//! than calling into the other crate directly.  This keeps the dependency graph
//! acyclic.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    id::{ApplicationId, InterviewId, JobSourceId, ResumeId},
    status::{ApplicationStatus, InterviewStatus},
};

/// Envelope that wraps every domain event with common metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainEvent<T> {
    /// Unique event id (UUID v4).
    pub event_id:  uuid::Uuid,
    /// Timestamp when the event was produced.
    pub timestamp: DateTime<Utc>,
    /// The actual event payload.
    pub payload:   T,
}

impl<T> DomainEvent<T> {
    /// Create a new domain event with the given payload.
    #[must_use]
    pub fn new(payload: T) -> Self {
        Self {
            event_id: uuid::Uuid::new_v4(),
            timestamp: Utc::now(),
            payload,
        }
    }
}

// ---------------------------------------------------------------------------
// Job-source events
// ---------------------------------------------------------------------------

/// A new job listing was discovered by a source driver.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobDiscovered {
    pub source_id: JobSourceId,
    pub title:     String,
    pub url:       String,
}

// ---------------------------------------------------------------------------
// Application events
// ---------------------------------------------------------------------------

/// The status of an application changed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplicationStatusChanged {
    pub application_id: ApplicationId,
    pub old_status:     ApplicationStatus,
    pub new_status:     ApplicationStatus,
}

// ---------------------------------------------------------------------------
// Resume events
// ---------------------------------------------------------------------------

/// A new resume version was created.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeVersionCreated {
    pub resume_id: ResumeId,
    pub version:   u32,
}

// ---------------------------------------------------------------------------
// Interview events
// ---------------------------------------------------------------------------

/// An interview status changed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterviewStatusChanged {
    pub interview_id: InterviewId,
    pub old_status:   InterviewStatus,
    pub new_status:   InterviewStatus,
}
