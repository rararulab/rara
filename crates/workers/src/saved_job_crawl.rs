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

//! Background worker that crawls saved job URLs and uploads the resulting
//! markdown to S3.
//!
//! Fetches all jobs in `PendingCrawl` status, crawls them via Crawl4AI,
//! uploads the markdown to the object store, and transitions the job to
//! `Crawled`. On failure the job is set to `Failed`.
//!
//! After processing, triggers the AnalyzeWorker via its `NotifyHandle`.

use async_trait::async_trait;
use job_common_worker::{FallibleWorker, Notifiable, WorkError, WorkResult, WorkerContext};
use job_domain_job_tracker::types::{PipelineEventKind, PipelineStage, SavedJobStatus};
use tracing::{info, warn};

use crate::worker_state::AppState;

/// Maximum characters to store as the markdown preview.
const PREVIEW_LEN: usize = 500;

/// Worker that crawls pending saved job URLs and uploads markdown to S3.
pub struct SavedJobCrawlWorker;

#[async_trait]
impl FallibleWorker<AppState> for SavedJobCrawlWorker {
    async fn work(&mut self, ctx: WorkerContext<AppState>) -> WorkResult {
        let state = ctx.state();

        let pending = state
            .saved_job_service
            .list(Some(SavedJobStatus::PendingCrawl))
            .await
            .map_err(|e| WorkError::transient(format!("list PendingCrawl failed: {e}")))?;

        if pending.is_empty() {
            return Ok(());
        }

        info!(count = pending.len(), "crawling pending saved jobs");

        let mut crawled_count = 0u32;

        for job in &pending {
            // Transition to Crawling
            if let Err(e) = state
                .saved_job_service
                .update_status(job.id, SavedJobStatus::Crawling, None)
                .await
            {
                warn!(id = %job.id, error = %e, "failed to set Crawling status");
                continue;
            }
            let _ = state
                .saved_job_service
                .log_event(
                    job.id,
                    PipelineStage::Crawl,
                    PipelineEventKind::Started,
                    "crawl started",
                    None,
                )
                .await;

            // Crawl the URL
            let markdown = match state.crawl_client.crawl(&job.url).await {
                Ok(md) => md,
                Err(e) => {
                    warn!(id = %job.id, error = %e, "crawl failed");
                    let _ = state
                        .saved_job_service
                        .update_status(
                            job.id,
                            SavedJobStatus::Failed,
                            Some(format!("crawl failed: {e}")),
                        )
                        .await;
                    let _ = state
                        .saved_job_service
                        .log_event(
                            job.id,
                            PipelineStage::Crawl,
                            PipelineEventKind::Failed,
                            &format!("crawl failed: {e}"),
                            None,
                        )
                        .await;
                    continue;
                }
            };

            // Upload to S3
            let s3_key = format!("saved-jobs/{}.md", job.id);
            if let Err(e) = state.object_store.write(&s3_key, markdown.clone()).await {
                warn!(id = %job.id, error = %e, "S3 upload failed");
                let _ = state
                    .saved_job_service
                    .update_status(
                        job.id,
                        SavedJobStatus::Failed,
                        Some(format!("S3 upload failed: {e}")),
                    )
                    .await;
                let _ = state
                    .saved_job_service
                    .log_event(
                        job.id,
                        PipelineStage::Crawl,
                        PipelineEventKind::Failed,
                        &format!("S3 upload failed: {e}"),
                        None,
                    )
                    .await;
                continue;
            }

            // Store crawl result (sets status to Crawled)
            let preview: String = markdown.chars().take(PREVIEW_LEN).collect();
            if let Err(e) = state
                .saved_job_service
                .update_crawl_result(job.id, &s3_key, &preview)
                .await
            {
                warn!(id = %job.id, error = %e, "failed to store crawl result");
                let _ = state
                    .saved_job_service
                    .update_status(
                        job.id,
                        SavedJobStatus::Failed,
                        Some(format!("failed to store crawl result: {e}")),
                    )
                    .await;
                continue;
            }

            let _ = state
                .saved_job_service
                .log_event(
                    job.id,
                    PipelineStage::Crawl,
                    PipelineEventKind::Completed,
                    "crawl completed",
                    Some(serde_json::json!({
                        "s3_key": s3_key,
                        "markdown_len": markdown.len()
                    })),
                )
                .await;
            info!(id = %job.id, s3_key = %s3_key, markdown_len = markdown.len(), "crawl + upload complete");
            crawled_count += 1;
        }

        if crawled_count > 0 {
            info!(crawled = crawled_count, "crawl batch complete");
            // Trigger the AnalyzeWorker to process newly crawled jobs
            if let Ok(guard) = state.analyze_notify.read() {
                if let Some(handle) = guard.as_ref() {
                    handle.notify();
                }
            }
        }

        Ok(())
    }
}
