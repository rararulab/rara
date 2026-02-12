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

pub mod dedup;
pub mod error;
pub mod jobspy;
pub mod pg_repository;
pub mod repository;
pub mod routes;
pub mod service;
pub mod types;

/// Wire the unified [`service::JobService`] with all dependencies.
pub fn wire_job_service(
    pool: PgPool,
    ai_service: job_ai::service::AiService,
) -> Result<service::JobService, error::SourceError> {
    let driver = jobspy::JobSpyDriver::new()?;
    let saved_job_repo: Arc<dyn repository::SavedJobRepository> =
        Arc::new(pg_repository::PgSavedJobRepository::new(pool.clone()));
    let job_repo: Arc<dyn repository::JobRepository> =
        Arc::new(pg_repository::PgJobRepository::new(pool));
    Ok(service::JobService::new(
        driver,
        saved_job_repo,
        job_repo,
        ai_service,
    ))
}
