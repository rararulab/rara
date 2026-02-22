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

//! Repository trait for pipeline run and event persistence.

use snafu::Snafu;
use uuid::Uuid;

use crate::types::{DiscoveredJob, DiscoveredJobAction, PipelineEvent, PipelineRun};

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors from pipeline repository operations.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum PipelineRepoError {
    /// A storage/infrastructure error occurred.
    #[snafu(display("pipeline repository error: {source}"))]
    Database { source: sqlx::Error },
}

// ---------------------------------------------------------------------------
// Repository trait
// ---------------------------------------------------------------------------

/// Persistence contract for pipeline runs and events.
#[async_trait::async_trait]
pub trait PipelineRepository: Send + Sync {
    /// Create a new pipeline run with default values (status = Running).
    async fn create_run(&self) -> Result<PipelineRun, PipelineRepoError>;

    /// Update mutable fields of an existing pipeline run.
    async fn update_run(&self, run: &PipelineRun) -> Result<(), PipelineRepoError>;

    /// Retrieve a single pipeline run by its ID.
    async fn get_run(&self, id: Uuid) -> Result<Option<PipelineRun>, PipelineRepoError>;

    /// List pipeline runs ordered by `started_at` descending.
    async fn list_runs(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<PipelineRun>, PipelineRepoError>;

    /// Insert a new event for a pipeline run.
    async fn insert_event(
        &self,
        run_id: Uuid,
        seq: i32,
        event_type: &str,
        payload: serde_json::Value,
    ) -> Result<(), PipelineRepoError>;

    /// Retrieve all events for a pipeline run, ordered by sequence number.
    async fn get_events(&self, run_id: Uuid) -> Result<Vec<PipelineEvent>, PipelineRepoError>;

    /// Insert a discovered job for a pipeline run.
    #[allow(clippy::too_many_arguments)]
    async fn insert_discovered_job(
        &self,
        run_id: Uuid,
        title: &str,
        company: Option<&str>,
        location: Option<&str>,
        url: Option<&str>,
        description: Option<&str>,
        score: Option<i32>,
        action: DiscoveredJobAction,
        date_posted: Option<&str>,
    ) -> Result<DiscoveredJob, PipelineRepoError>;

    /// List all discovered jobs for a pipeline run, ordered by score descending.
    async fn list_discovered_jobs(
        &self,
        run_id: Uuid,
    ) -> Result<Vec<DiscoveredJob>, PipelineRepoError>;
}
