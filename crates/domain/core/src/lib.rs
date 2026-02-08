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

//! # job-domain-core
//!
//! Shared domain interface crate for the Job Automation platform.
//!
//! This crate defines the canonical domain types, repository traits, and event
//! types that form the contract between domain crates. It contains **no**
//! implementation logic and has **no** dependencies on infrastructure crates.
//!
//! ## Design Rules
//! - No concrete implementations -- only traits and types.
//! - Domain crates depend on this crate, never the reverse.
//! - Infrastructure crates (store, runtime, etc.) must not appear in the
//!   dependency list.

pub mod events;
pub mod id;
pub mod repository;
pub mod status;

// Re-exports for convenience.
pub use id::{
    ApplicationId, InterviewId, JobSourceId, NotificationId, ResumeId, SchedulerTaskId,
};
pub use status::{ApplicationStatus, InterviewStatus, JobSourceStatus};
