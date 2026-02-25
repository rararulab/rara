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

use super::types::{
    DiscoveredJob, DiscoveredJobAction, DiscoveredJobWithDetails, DiscoveredJobsStats,
    PipelineEvent, PipelineRun,
};

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
    async fn insert_discovered_job(
        &self,
        run_id: Uuid,
        job_id: Uuid,
        score: Option<i32>,
        action: DiscoveredJobAction,
    ) -> Result<DiscoveredJob, PipelineRepoError>;

    /// List all discovered jobs for a pipeline run (with job details), ordered
    /// by score descending.
    async fn list_discovered_jobs(
        &self,
        run_id: Uuid,
    ) -> Result<Vec<DiscoveredJobWithDetails>, PipelineRepoError>;

    /// List discovered jobs that still need scoring for a pipeline run (with
    /// job details for the AI agent to evaluate).
    async fn list_unscored_discovered_jobs(
        &self,
        run_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<DiscoveredJobWithDetails>, PipelineRepoError>;

    /// Update score/action for a discovered job and return the updated row.
    async fn update_discovered_job_score_action(
        &self,
        id: Uuid,
        score: Option<i32>,
        action: Option<DiscoveredJobAction>,
    ) -> Result<Option<DiscoveredJob>, PipelineRepoError>;

    /// List discovered jobs across all runs with optional filters (with job
    /// details via JOIN).
    #[allow(clippy::too_many_arguments)]
    async fn list_all_discovered_jobs(
        &self,
        action: Option<DiscoveredJobAction>,
        min_score: Option<i32>,
        max_score: Option<i32>,
        run_id: Option<Uuid>,
        sort_by: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<DiscoveredJobWithDetails>, PipelineRepoError>;

    /// Count discovered jobs matching filters (for pagination).
    async fn count_discovered_jobs(
        &self,
        action: Option<DiscoveredJobAction>,
        min_score: Option<i32>,
        max_score: Option<i32>,
        run_id: Option<Uuid>,
    ) -> Result<i64, PipelineRepoError>;

    /// Get aggregated stats for discovered jobs.
    async fn discovered_jobs_stats(&self) -> Result<DiscoveredJobsStats, PipelineRepoError>;

    /// Cancel all runs still marked as `Running` (stale after process restart).
    /// Returns the number of rows updated.
    async fn reconcile_stale_runs(&self) -> Result<u64, PipelineRepoError>;
}
