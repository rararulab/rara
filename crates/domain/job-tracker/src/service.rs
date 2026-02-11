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

//! Application-level service for saved job management.

use std::sync::{Arc, RwLock};

use jiff::Timestamp;
use job_common_worker::{Notifiable, NotifyHandle};
use tracing::{instrument, warn};
use uuid::Uuid;

use crate::{
    error::SavedJobError,
    repository::SavedJobRepository,
    types::{PipelineEvent, PipelineEventKind, PipelineStage, SavedJob, SavedJobStatus},
};

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

/// High-level service for saved job CRUD and pipeline status management.
pub struct SavedJobService {
    repo:           Arc<dyn SavedJobRepository>,
    notify_trigger: RwLock<Option<NotifyHandle>>,
}

impl SavedJobService {
    /// Create a new service backed by the given repository.
    #[must_use]
    pub fn new(repo: Arc<dyn SavedJobRepository>) -> Self {
        Self {
            repo,
            notify_trigger: RwLock::new(None),
        }
    }

    /// Registers the runtime notify handle used to trigger immediate
    /// pipeline processing when new jobs are saved.
    pub fn set_notify_trigger(&self, handle: NotifyHandle) {
        if let Ok(mut guard) = self.notify_trigger.write() {
            *guard = Some(handle);
        } else {
            warn!("failed to acquire saved-job notify trigger write lock");
        }
    }

    fn trigger_pipeline(&self) {
        if let Ok(guard) = self.notify_trigger.read() {
            if let Some(handle) = guard.as_ref() {
                handle.notify();
            }
        }
    }

    /// Save a new job by URL.
    #[instrument(skip(self))]
    pub async fn create(&self, url: &str) -> Result<SavedJob, SavedJobError> {
        let url = url.trim();
        if url.is_empty() {
            return Err(SavedJobError::ValidationError {
                message: "url must not be empty".to_owned(),
            });
        }
        let job = self.repo.create(url).await?;
        let _ = self
            .log_event(
                job.id,
                PipelineStage::Crawl,
                PipelineEventKind::Info,
                "job saved, pending crawl",
                None,
            )
            .await;
        self.trigger_pipeline();
        Ok(job)
    }

    /// Get a saved job by id.
    #[instrument(skip(self))]
    pub async fn get(&self, id: Uuid) -> Result<Option<SavedJob>, SavedJobError> {
        self.repo.get_by_id(id).await
    }

    /// List saved jobs, optionally filtered by status.
    #[instrument(skip(self))]
    pub async fn list(
        &self,
        status: Option<SavedJobStatus>,
    ) -> Result<Vec<SavedJob>, SavedJobError> {
        self.repo.list(status).await
    }

    /// Delete a saved job.
    #[instrument(skip(self))]
    pub async fn delete(&self, id: Uuid) -> Result<(), SavedJobError> { self.repo.delete(id).await }

    /// Update the pipeline status (and optionally record an error).
    #[instrument(skip(self))]
    pub async fn update_status(
        &self,
        id: Uuid,
        status: SavedJobStatus,
        error_message: Option<String>,
    ) -> Result<(), SavedJobError> {
        self.repo.update_status(id, status, error_message).await
    }

    /// Store the crawl result.
    #[instrument(skip(self, preview))]
    pub async fn update_crawl_result(
        &self,
        id: Uuid,
        s3_key: &str,
        preview: &str,
    ) -> Result<(), SavedJobError> {
        self.repo.update_crawl_result(id, s3_key, preview).await
    }

    /// Store the analysis result.
    #[instrument(skip(self, result))]
    pub async fn update_analysis(
        &self,
        id: Uuid,
        result: serde_json::Value,
        score: f32,
    ) -> Result<(), SavedJobError> {
        self.repo.update_analysis(id, result, score).await
    }

    /// Retry a failed or expired saved job by resetting its status to
    /// `PendingCrawl` and clearing the error.
    #[instrument(skip(self))]
    pub async fn retry(&self, id: Uuid) -> Result<(), SavedJobError> {
        self.repo
            .update_status(id, SavedJobStatus::PendingCrawl, None)
            .await?;
        let _ = self
            .log_event(
                id,
                PipelineStage::Crawl,
                PipelineEventKind::Info,
                "retry initiated",
                None,
            )
            .await;
        self.trigger_pipeline();
        Ok(())
    }

    /// List saved jobs older than the given timestamp that are not in a
    /// terminal status (Failed or Expired).
    #[instrument(skip(self))]
    pub async fn list_stale(&self, older_than: Timestamp) -> Result<Vec<SavedJob>, SavedJobError> {
        self.repo.list_stale(older_than).await
    }

    /// List saved jobs matching the given statuses that have S3 keys set.
    #[instrument(skip(self))]
    pub async fn list_with_s3_keys_by_status(
        &self,
        statuses: &[SavedJobStatus],
    ) -> Result<Vec<SavedJob>, SavedJobError> {
        self.repo.list_with_s3_keys_by_status(statuses).await
    }

    /// Clear the S3 key for a saved job after object cleanup.
    #[instrument(skip(self))]
    pub async fn clear_s3_key(&self, id: Uuid) -> Result<(), SavedJobError> {
        self.repo.clear_s3_key(id).await
    }

    /// Update the title and/or company extracted from AI analysis.
    #[instrument(skip(self))]
    pub async fn update_title_company(
        &self,
        id: Uuid,
        title: Option<String>,
        company: Option<String>,
    ) -> Result<(), SavedJobError> {
        self.repo.update_title_company(id, title, company).await
    }

    /// Record a pipeline event for a saved job.
    #[instrument(skip(self, metadata))]
    pub async fn log_event(
        &self,
        saved_job_id: Uuid,
        stage: PipelineStage,
        event_kind: PipelineEventKind,
        message: &str,
        metadata: Option<serde_json::Value>,
    ) -> Result<PipelineEvent, SavedJobError> {
        self.repo
            .create_event(saved_job_id, stage, event_kind, message, metadata)
            .await
    }

    /// List all pipeline events for a saved job.
    #[instrument(skip(self))]
    pub async fn list_events(
        &self,
        saved_job_id: Uuid,
    ) -> Result<Vec<PipelineEvent>, SavedJobError> {
        self.repo.list_events(saved_job_id).await
    }
}
