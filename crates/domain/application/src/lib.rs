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
//! - The [`Application`] aggregate with rich metadata (tags, priority,
//!   channel).
//! - A configurable [`StateMachine`] for validating status transitions.
//! - [`StatusChangeRecord`] for auditing every transition with its source
//!   (manual, system, email parse).
//! - An [`ApplicationRepository`] trait for persistence.
//! - An [`ApplicationService`] that orchestrates transitions, CRUD, and
//!   statistics.
//!
//! The crate depends on [`job_domain_core`] for shared types and traits.

pub mod error;
pub mod repository;
pub mod service;
pub mod state_machine;
pub mod types;

// Re-exports for convenience.
pub use error::ApplicationError;
pub use repository::ApplicationRepository;
pub use service::ApplicationService;
pub use state_machine::{StateMachine, TransitionRule};
pub use types::{
    Application, ApplicationChannel, ApplicationFilter, ApplicationStatistics, ChangeSource,
    CreateApplicationRequest, Priority, StatusChangeRecord, UpdateApplicationRequest,
};
