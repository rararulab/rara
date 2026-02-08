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
//! Interview management and AI preparation plan generation for the Job
//! Automation platform.
//!
//! This crate covers everything related to the interview stage:
//!
//! - Creating and managing interview preparation plans.
//! - AI-powered generation of prep checklists (knowledge points, project
//!   reviews, behavioral questions, questions to ask).
//! - Tracking interview task status through its lifecycle.
//! - Filtering and querying interview plans by application, company, status, or
//!   round.
//!
//! The crate depends on [`job_domain_core`] for shared types and traits.
//! It does **not** depend on the AI crate directly; instead it defines the
//! [`PrepGenerator`] trait that AI adapters can implement.

pub mod error;
pub mod prep_generator;
pub mod repository;
pub mod service;
pub mod types;

// Re-exports for convenience.
pub use error::InterviewError;
pub use prep_generator::{MockPrepGenerator, PrepGenerator};
pub use repository::InterviewPlanRepository;
pub use service::InterviewService;
pub use types::{
    BehavioralQuestion, CreateInterviewPlanRequest, InterviewFilter, InterviewPlan, InterviewRound,
    InterviewTaskStatus, PrepGenerationRequest, PrepMaterials, ProjectReview,
    UpdateInterviewPlanRequest,
};
