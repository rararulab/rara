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

//! # job-domain-job
//!
//! Unified job domain: discovery, tracking, and pipeline management.

use std::sync::Arc;

use sqlx::PgPool;

pub mod bot_internal_routes;
pub mod crawl4ai;
pub mod dedup;
pub mod discovery_service;
pub mod error;
pub mod jobspy;
pub mod pg_repository;
pub mod repository;
pub mod routes;
pub mod tracker_service;
pub mod types;

#[must_use]
pub fn wire_job_repository(pool: PgPool) -> Arc<dyn repository::JobRepository> {
    Arc::new(pg_repository::PgJobRepository::new(pool))
}

pub fn wire_job_source_service() -> Result<discovery_service::JobSourceService, error::SourceError> {
    let driver = jobspy::JobSpyDriver::new()?;
    Ok(discovery_service::JobSourceService::new(driver))
}

#[must_use]
pub fn wire_saved_job_service(pool: PgPool) -> tracker_service::SavedJobService {
    let repo: Arc<dyn repository::SavedJobRepository> =
        Arc::new(pg_repository::PgSavedJobRepository::new(pool));
    tracker_service::SavedJobService::new(repo)
}
