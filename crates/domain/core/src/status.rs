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

//! Domain status enums shared across crates.

use serde::{Deserialize, Serialize};
use strum_macros::{Display, EnumString};

/// Lifecycle status of a job source driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Display, EnumString)]
#[strum(serialize_all = "snake_case")]
pub enum JobSourceStatus {
    /// Source is active and being polled.
    Active,
    /// Source is temporarily paused.
    Paused,
    /// Source has been permanently disabled.
    Disabled,
    /// Source encountered an error and requires attention.
    Error,
}

/// Lifecycle status of a job application.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Display, EnumString)]
#[strum(serialize_all = "snake_case")]
pub enum ApplicationStatus {
    /// Application is in draft / being prepared.
    Draft,
    /// Application has been submitted.
    Submitted,
    /// Employer acknowledged receipt.
    UnderReview,
    /// Application was rejected.
    Rejected,
    /// Candidate advanced to interview stage.
    Interview,
    /// An offer was extended.
    Offered,
    /// Offer accepted.
    Accepted,
    /// Application was withdrawn by the candidate.
    Withdrawn,
}

/// Lifecycle status of an interview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Display, EnumString)]
#[strum(serialize_all = "snake_case")]
pub enum InterviewStatus {
    /// Interview has been scheduled.
    Scheduled,
    /// Interview is in progress.
    InProgress,
    /// Interview completed, awaiting feedback.
    Completed,
    /// Interview was cancelled.
    Cancelled,
    /// Feedback has been received.
    FeedbackReceived,
}
