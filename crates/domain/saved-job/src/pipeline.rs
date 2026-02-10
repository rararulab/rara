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

//! Orchestration pipeline for saved jobs.
//!
//! When a saved job is created (status = `PendingCrawl`), this pipeline:
//!
//! 1. Calls Crawl4AI to convert the URL into clean markdown.
//! 2. Uploads the markdown to MinIO / S3 via the object store.
//! 3. Runs AI analysis on the markdown to extract structured data and a match
//!    score.
//! 4. Persists the analysis result in the database.
//!
//! On failure at any step the job is moved to `Failed` with an error
//! message so it can be retried later.

use std::sync::Arc;

use bytes::Bytes;
use tracing::{info, instrument, warn};
use uuid::Uuid;

use crate::{
    crawl4ai::Crawl4AiClient, error::SavedJobError, repository::SavedJobRepository,
    service::SavedJobService, types::SavedJobStatus,
};

/// Maximum characters to store as the markdown preview.
const PREVIEW_LEN: usize = 500;

/// Orchestrates the crawl → store → analyze pipeline for saved jobs.
pub struct SavedJobPipeline<R: SavedJobRepository> {
    saved_job_service: Arc<SavedJobService<R>>,
    crawl_client:      Crawl4AiClient,
    object_store:      Arc<job_object_store::ObjectStore>,
    ai_service:        Arc<job_ai::service::AiService>,
}

impl<R: SavedJobRepository> SavedJobPipeline<R> {
    /// Create a new pipeline.
    pub fn new(
        saved_job_service: Arc<SavedJobService<R>>,
        crawl_client: Crawl4AiClient,
        object_store: Arc<job_object_store::ObjectStore>,
        ai_service: Arc<job_ai::service::AiService>,
    ) -> Self {
        Self {
            saved_job_service,
            crawl_client,
            object_store,
            ai_service,
        }
    }

    /// Process a single saved job through the full pipeline.
    ///
    /// Any step failure sets the job status to `Failed` with an error
    /// message so it can be retried later.
    #[instrument(skip(self), fields(%id))]
    pub async fn process(&self, id: Uuid) -> Result<(), SavedJobError> {
        let job = self
            .saved_job_service
            .get(id)
            .await?
            .ok_or(SavedJobError::NotFound { id })?;

        if job.status != SavedJobStatus::PendingCrawl {
            info!(status = %job.status, "skipping job not in PendingCrawl status");
            return Ok(());
        }

        // --- Step 1: Crawl ---
        self.saved_job_service
            .update_status(id, SavedJobStatus::Crawling, None)
            .await?;

        let markdown = match self.crawl_client.crawl(&job.url).await {
            Ok(md) => md,
            Err(e) => {
                self.fail_job(id, &format!("crawl failed: {e}")).await;
                return Err(e);
            }
        };

        // --- Step 2: Upload to S3 ---
        let s3_key = format!("saved-jobs/{id}.md");
        if let Err(e) = self
            .object_store
            .put(&s3_key, Bytes::from(markdown.clone()))
            .await
        {
            let msg = format!("S3 upload failed: {e}");
            self.fail_job(id, &msg).await;
            return Err(SavedJobError::ObjectStoreError { message: msg });
        }

        let preview: String = markdown.chars().take(PREVIEW_LEN).collect();
        self.saved_job_service
            .update_crawl_result(id, &s3_key, &preview)
            .await?;

        info!(s3_key = %s3_key, markdown_len = markdown.len(), "crawl + upload complete");

        // --- Step 3: AI Analysis ---
        self.saved_job_service
            .update_status(id, SavedJobStatus::Analyzing, None)
            .await?;

        let analysis_json = match self.ai_service.jd_analyzer().analyze(&markdown).await {
            Ok(json) => json,
            Err(e) => {
                let msg = format!("AI analysis failed: {e}");
                self.fail_job(id, &msg).await;
                return Err(SavedJobError::AnalysisError { message: msg });
            }
        };

        // --- Step 4: Parse and store result ---
        let analysis_value: serde_json::Value = serde_json::from_str(&analysis_json)
            .unwrap_or_else(|_| serde_json::json!({ "raw_response": analysis_json }));

        let match_score = analysis_value
            .get("match_score")
            .and_then(|v| v.as_f64())
            .map(|v| v as f32)
            .unwrap_or(0.0);

        // Also extract title and company from AI analysis
        let title = analysis_value
            .get("title")
            .and_then(|v| v.as_str())
            .map(|s| s.to_owned());
        let company = analysis_value
            .get("company")
            .and_then(|v| v.as_str())
            .map(|s| s.to_owned());

        if title.is_some() || company.is_some() {
            if let Err(e) = self
                .saved_job_service
                .update_title_company(id, title, company)
                .await
            {
                warn!(error = %e, "failed to update title/company from analysis");
            }
        }

        self.saved_job_service
            .update_analysis(id, analysis_value, match_score)
            .await?;

        info!(match_score, "pipeline complete");
        Ok(())
    }

    /// Process all saved jobs currently in `PendingCrawl` status.
    ///
    /// Returns the number of jobs processed (whether successful or not).
    #[instrument(skip(self))]
    pub async fn process_pending_batch(&self) -> Result<u32, SavedJobError> {
        let pending = self
            .saved_job_service
            .list(Some(SavedJobStatus::PendingCrawl))
            .await?;

        let total = pending.len() as u32;
        if total == 0 {
            return Ok(0);
        }

        info!(count = total, "processing pending saved jobs");

        for job in pending {
            if let Err(e) = self.process(job.id).await {
                warn!(id = %job.id, error = %e, "pipeline failed for saved job");
            }
        }

        Ok(total)
    }

    /// Mark a job as failed with the given error message.
    async fn fail_job(&self, id: Uuid, message: &str) {
        if let Err(e) = self
            .saved_job_service
            .update_status(id, SavedJobStatus::Failed, Some(message.to_owned()))
            .await
        {
            warn!(id = %id, error = %e, "failed to mark job as failed");
        }
    }
}
