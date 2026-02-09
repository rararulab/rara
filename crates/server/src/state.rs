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

//! Shared application state for HTTP API handlers.

use std::sync::Arc;

use job_domain_application::service::ApplicationService;
use job_domain_interview::service::InterviewService;
use job_domain_notify::service::NotificationService;
use job_domain_resume::{repository::ResumeRepository, service::ResumeService};
use job_domain_scheduler::service::SchedulerService;

/// Shared state passed to all API route handlers via axum's
/// `State` extractor.
pub struct AppState<R: ResumeRepository> {
    /// Application lifecycle service.
    pub application_service: Arc<ApplicationService>,
    /// Interview plan management service.
    pub interview_service: Arc<InterviewService>,
    /// Resume version management service.
    pub resume_service: Arc<ResumeService<R>>,
    /// Notification dispatch service.
    pub notification_service: Arc<NotificationService>,
    /// Scheduler task management service.
    pub scheduler_service: Arc<SchedulerService>,
}
