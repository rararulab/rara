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

//! HTTP API route modules for the job server.

pub mod analytics;
pub mod application;
pub mod error;
pub mod interview;
pub mod notification;
pub mod resume;
pub mod scheduler;

use std::sync::Arc;

use axum::Router;
use job_domain_resume::repository::ResumeRepository;

use crate::state::AppState;

/// Merge all domain API route groups into a single router.
pub fn api_routes<R: ResumeRepository + 'static>(state: Arc<AppState<R>>) -> Router {
    Router::new()
        .merge(resume::resume_routes(state.clone()))
        .merge(application::application_routes(state.clone()))
        .merge(interview::interview_routes(state.clone()))
        .merge(notification::notification_routes(state.clone()))
        .merge(scheduler::scheduler_routes(state.clone()))
        .merge(analytics::analytics_routes(state))
}
