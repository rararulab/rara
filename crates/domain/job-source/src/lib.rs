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
//! Job source driver abstraction layer for the Job Automation platform.
//!
//! This crate owns the concept of a *job source* -- an external platform
//! (LinkedIn, Indeed, Glassdoor, ...) that provides job listings.  It defines:
//!
//! - The [`JobSourceDriver`] trait that every concrete driver must implement.
//! - Data types for job listings fetched from external sources.
//! - Logic for polling, deduplication, and source health tracking.
//!
//! Concrete driver implementations (HTTP scrapers, API clients) will be added
//! as feature-gated modules inside this crate.

/// The `driver` module will contain the `JobSourceDriver` trait and helpers.
pub mod driver;

pub use driver::JobSourceDriver;
