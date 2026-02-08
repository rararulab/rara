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
//! - Tailoring a base resume to a specific job listing (with AI
//!   assistance).
//! - Tracking which resume version was used for each application.
//! - Content hashing for deduplication.
//! - Version tree traversal and text diffing.
//!
//! The crate depends on [`job_domain_core`] for shared types and traits.

pub mod hash;
pub mod repository;
pub mod service;
pub mod types;
pub mod version;

// Re-exports for convenience.
pub use hash::content_hash;
pub use repository::ResumeRepository;
pub use service::ResumeService;
pub use types::{
    CreateResumeRequest, Resume, ResumeDiff, ResumeError, ResumeFilter, ResumeId, ResumeSource,
    UpdateResumeRequest,
};
pub use version::{ResumeVersion, ResumeVersionTree};
