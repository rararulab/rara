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

//! Background worker that performs AI analysis on crawled saved jobs.
//!
//! Fetches all jobs in `Crawled` status, runs AI analysis on the markdown
//! content, and stores the analysis result + match score. On failure the
//! job is set to `Failed`.

use async_trait::async_trait;
use job_common_worker::{FallibleWorker, WorkError, WorkResult, WorkerContext};
use job_domain_job_tracker::types::SavedJobStatus;
use tracing::{info, warn};

use crate::worker_state::AppWorkerState;

/// Worker that analyzes crawled saved jobs using AI.
pub struct SavedJobAnalyzeWorker;

#[async_trait]
impl FallibleWorker<AppWorkerState> for SavedJobAnalyzeWorker {
    async fn work(&mut self, ctx: WorkerContext<AppWorkerState>) -> WorkResult {
        let state = ctx.state();

        let crawled = state
            .saved_job_service
            .list(Some(SavedJobStatus::Crawled))
            .await
            .map_err(|e| WorkError::transient(format!("list Crawled failed: {e}")))?;

        if crawled.is_empty() {
            return Ok(());
        }

        info!(count = crawled.len(), "analyzing crawled saved jobs");

        let mut analyzed_count = 0u32;

        for job in &crawled {
            let ai_service = match state
                .ai_service_handle
                .read()
                .ok()
                .and_then(|g| g.as_ref().cloned())
            {
                Some(svc) => svc,
                None => {
                    warn!("AI service not configured; skipping saved-job analyze tick");
                    return Ok(());
                }
            };

            // Claim the job first to prevent duplicate processing on restart.
            if let Err(e) = state
                .saved_job_service
                .update_status(job.id, SavedJobStatus::Analyzing, None)
                .await
            {
                warn!(id = %job.id, error = %e, "failed to set Analyzing status");
                continue;
            }

            // Fetch full markdown from S3, falling back to preview.
            let markdown = if let Some(s3_key) = &job.markdown_s3_key {
                match state.object_store.get(s3_key).await {
                    Ok(data) => String::from_utf8_lossy(&data).to_string(),
                    Err(e) => {
                        warn!(id = %job.id, error = %e, "failed to fetch markdown from S3, falling back to preview");
                        match &job.markdown_preview {
                            Some(preview) => preview.clone(),
                            None => {
                                warn!(id = %job.id, "no markdown available for analysis");
                                let _ = state
                                    .saved_job_service
                                    .update_status(
                                        job.id,
                                        SavedJobStatus::Failed,
                                        Some("no markdown available for analysis".to_owned()),
                                    )
                                    .await;
                                continue;
                            }
                        }
                    }
                }
            } else {
                match &job.markdown_preview {
                    Some(preview) => preview.clone(),
                    None => {
                        warn!(id = %job.id, "no markdown available for analysis");
                        let _ = state
                            .saved_job_service
                            .update_status(
                                job.id,
                                SavedJobStatus::Failed,
                                Some("no markdown available for analysis".to_owned()),
                            )
                            .await;
                        continue;
                    }
                }
            };

            // Run AI analysis
            let analysis_json = match ai_service.jd_analyzer().analyze(&markdown).await {
                Ok(json) => json,
                Err(e) => {
                    warn!(id = %job.id, error = %e, "AI analysis failed");
                    let _ = state
                        .saved_job_service
                        .update_status(
                            job.id,
                            SavedJobStatus::Failed,
                            Some(format!("AI analysis failed: {e}")),
                        )
                        .await;
                    continue;
                }
            };

            // Parse the analysis result
            let analysis_value: serde_json::Value = serde_json::from_str(&analysis_json)
                .unwrap_or_else(|_| serde_json::json!({ "raw_response": analysis_json }));

            let match_score = analysis_value
                .get("match_score")
                .and_then(|v| v.as_f64())
                .map(|v| v as f32)
                .unwrap_or(0.0);

            // Extract title and company from AI analysis
            let title = analysis_value
                .get("title")
                .and_then(|v| v.as_str())
                .map(|s| s.to_owned());
            let company = analysis_value
                .get("company")
                .and_then(|v| v.as_str())
                .map(|s| s.to_owned());

            if title.is_some() || company.is_some() {
                if let Err(e) = state
                    .saved_job_service
                    .update_title_company(job.id, title, company)
                    .await
                {
                    warn!(id = %job.id, error = %e, "failed to update title/company from analysis");
                }
            }

            // Store analysis result (sets status to Analyzed)
            if let Err(e) = state
                .saved_job_service
                .update_analysis(job.id, analysis_value, match_score)
                .await
            {
                warn!(id = %job.id, error = %e, "failed to store analysis result");
                let _ = state
                    .saved_job_service
                    .update_status(
                        job.id,
                        SavedJobStatus::Failed,
                        Some(format!("failed to store analysis result: {e}")),
                    )
                    .await;
                continue;
            }

            info!(id = %job.id, match_score, "analysis complete");
            analyzed_count += 1;
        }

        if analyzed_count > 0 {
            info!(analyzed = analyzed_count, "analyze batch complete");
        }

        Ok(())
    }
}
