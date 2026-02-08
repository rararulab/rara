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

//! # job-domain-resume
//!
//! Resume version management for the Job Automation platform.
//!
//! This crate is responsible for:
//!
//! - Storing and retrieving resume versions.
//! - Tailoring a base resume to a specific job listing (with AI assistance).
//! - Tracking which resume version was used for each application.
//!
//! The crate depends on [`job_domain_core`] for shared types and traits.

use chrono::{DateTime, Utc};
use job_domain_core::ResumeId;
use serde::{Deserialize, Serialize};

/// A versioned resume document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resume {
    /// Unique identifier.
    pub id: ResumeId,
    /// Monotonically increasing version number.
    pub version: u32,
    /// Human-readable label (e.g. "Backend Engineer v3").
    pub label: String,
    /// Raw content of the resume (Markdown or plain text).
    pub content: String,
    /// When this version was created.
    pub created_at: DateTime<Utc>,
}
