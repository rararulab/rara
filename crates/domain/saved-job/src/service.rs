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

use std::sync::Arc;

use tracing::instrument;
use uuid::Uuid;

use crate::error::SavedJobError;
use crate::repository::SavedJobRepository;
use crate::types::{SavedJob, SavedJobStatus};

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

/// High-level service for saved job CRUD and pipeline status management.
pub struct SavedJobService<R: SavedJobRepository> {
    repo: Arc<R>,
}

impl<R: SavedJobRepository> SavedJobService<R> {
    /// Create a new service backed by the given repository.
    #[must_use]
    pub const fn new(repo: Arc<R>) -> Self { Self { repo } }

    /// Save a new job by URL.
    #[instrument(skip(self))]
    pub async fn create(&self, url: &str) -> Result<SavedJob, SavedJobError> {
        let url = url.trim();
        if url.is_empty() {
            return Err(SavedJobError::ValidationError {
                message: "url must not be empty".to_owned(),
            });
        }
        self.repo.create(url).await
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
    pub async fn delete(&self, id: Uuid) -> Result<(), SavedJobError> {
        self.repo.delete(id).await
    }

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
            .await
    }
}
