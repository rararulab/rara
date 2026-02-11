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

//! Repository trait for saved job persistence.

use jiff::Timestamp;
use uuid::Uuid;

use crate::{
    error::SavedJobError,
    types::{PipelineEvent, PipelineEventKind, PipelineStage, SavedJob, SavedJobStatus},
};

/// Persistence contract for saved jobs.
#[async_trait::async_trait]
pub trait SavedJobRepository: Send + Sync {
    /// Insert a new saved job with `status = PendingCrawl`.
    async fn create(&self, url: &str) -> Result<SavedJob, SavedJobError>;

    /// Retrieve a single saved job by its primary key.
    async fn get_by_id(&self, id: Uuid) -> Result<Option<SavedJob>, SavedJobError>;

    /// List saved jobs, optionally filtered by status.
    async fn list(&self, status: Option<SavedJobStatus>) -> Result<Vec<SavedJob>, SavedJobError>;

    /// Delete a saved job by id.
    async fn delete(&self, id: Uuid) -> Result<(), SavedJobError>;

    /// Update the status (and optionally the error message) of a saved job.
    async fn update_status(
        &self,
        id: Uuid,
        status: SavedJobStatus,
        error_message: Option<String>,
    ) -> Result<(), SavedJobError>;

    /// Store the crawl result (S3 key + preview text) and set status to
    /// Crawled.
    async fn update_crawl_result(
        &self,
        id: Uuid,
        s3_key: &str,
        preview: &str,
    ) -> Result<(), SavedJobError>;

    /// Store the analysis result (JSON + match score) and set status to
    /// Analyzed.
    async fn update_analysis(
        &self,
        id: Uuid,
        result: serde_json::Value,
        score: f32,
    ) -> Result<(), SavedJobError>;

    /// List saved jobs created before the given timestamp that are not in a
    /// terminal status (Failed or Expired). Used by the GC worker to find
    /// stale URLs that may need to be checked for expiry.
    async fn list_stale(&self, older_than: Timestamp) -> Result<Vec<SavedJob>, SavedJobError>;

    /// List saved jobs that match one of the given statuses **and** have an
    /// S3 key set. Used by the GC worker to find objects that need cleanup
    /// after a job has been marked expired.
    async fn list_with_s3_keys_by_status(
        &self,
        statuses: &[SavedJobStatus],
    ) -> Result<Vec<SavedJob>, SavedJobError>;

    /// Clear the S3 key for a saved job (after the object has been deleted).
    async fn clear_s3_key(&self, id: Uuid) -> Result<(), SavedJobError>;

    /// Update the title and/or company extracted from AI analysis.
    async fn update_title_company(
        &self,
        id: Uuid,
        title: Option<String>,
        company: Option<String>,
    ) -> Result<(), SavedJobError>;

    /// Insert a pipeline event for a saved job.
    async fn create_event(
        &self,
        saved_job_id: Uuid,
        stage: PipelineStage,
        event_kind: PipelineEventKind,
        message: &str,
        metadata: Option<serde_json::Value>,
    ) -> Result<PipelineEvent, SavedJobError>;

    /// List all pipeline events for a saved job, ordered by created_at ASC.
    async fn list_events(
        &self,
        saved_job_id: Uuid,
    ) -> Result<Vec<PipelineEvent>, SavedJobError>;
}
