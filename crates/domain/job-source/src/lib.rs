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

//! # job-domain-job-source
//!
//! Job source driver abstraction layer for the Job Automation
//! platform.
//!
//! This crate owns the concept of a *job source* -- an external
//! platform (LinkedIn, Indeed, Glassdoor, ...) or a manual entry
//! point that provides job listings.  It defines:
//!
//! - The [`JobSourceDriver`] trait that every concrete driver must
//!   implement.
//! - Data types for discovery criteria, raw and normalized jobs, and
//!   source errors.
//! - Deduplication logic (exact and fuzzy).
//! - A [`JobSourceService`] that orchestrates multiple drivers.
//!
//! Concrete driver implementations live in sub-modules:
//! - [`manual`] -- jobs entered via the API.
//! - [`linkedin`] -- stub for LinkedIn scraping (WIP).

pub mod dedup;
pub mod driver;
pub mod linkedin;
pub mod manual;
pub mod service;
pub mod types;

// Re-export key types for ergonomic imports.
pub use dedup::{FuzzyKey, SourceKey};
pub use driver::JobSourceDriver;
pub use linkedin::LinkedInSource;
pub use manual::ManualSource;
pub use service::{DiscoveryResult, JobSourceService};
pub use types::{DiscoveryCriteria, NormalizedJob, RawJob, SourceError};
