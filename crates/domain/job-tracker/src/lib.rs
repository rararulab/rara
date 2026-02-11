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

//! # job-domain-job-tracker
//!
//! Saved job tracking with crawl and analysis pipeline.
use std::sync::Arc;
use sqlx::PgPool;

pub mod bot_internal_routes;
pub mod crawl4ai;
pub mod error;
pub mod pg_repository;
pub mod repository;
pub mod routes;
pub mod service;
pub mod types;

#[must_use]
pub fn wire_saved_job_service(pool: PgPool) -> service::SavedJobService {
    let repo: Arc<dyn repository::SavedJobRepository> =
        Arc::new(pg_repository::PgSavedJobRepository::new(pool));
    service::SavedJobService::new(repo)
}
