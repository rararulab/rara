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

//! Job domain module -- discovery and normalization.

pub mod dedup;
pub mod error;
pub mod japandev;
pub mod jobspy;
pub mod pg_repository;
pub mod repository;
mod router;
pub mod service;
pub mod types;

use std::sync::Arc;

pub use router::{bot_routes, discovery_routes};
pub use service::JobService;
use sqlx::PgPool;

/// Wire the unified [`JobService`] with all dependencies.
pub fn wire_job_service(
    pool: PgPool,
    ai_service: crate::ai_tasks::TaskAgentService,
) -> Result<service::JobService, error::SourceError> {
    let driver = jobspy::JobSpyDriver::new()?;
    let japandev_driver = japandev::JapanDevDriver::new(japandev::JapanDevConfig::default());
    let job_repo: Arc<dyn repository::JobRepository> =
        Arc::new(pg_repository::PgJobRepository::new(pool));
    Ok(service::JobService::new(
        driver,
        Some(japandev_driver),
        job_repo,
        ai_service,
    ))
}
