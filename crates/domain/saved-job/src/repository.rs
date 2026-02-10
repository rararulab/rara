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

use uuid::Uuid;

use crate::error::SavedJobError;
use crate::types::{SavedJob, SavedJobStatus};

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

    /// Store the crawl result (S3 key + preview text) and set status to Crawled.
    async fn update_crawl_result(
        &self,
        id: Uuid,
        s3_key: &str,
        preview: &str,
    ) -> Result<(), SavedJobError>;

    /// Store the analysis result (JSON + match score) and set status to Analyzed.
    async fn update_analysis(
        &self,
        id: Uuid,
        result: serde_json::Value,
        score: f32,
    ) -> Result<(), SavedJobError>;
}
