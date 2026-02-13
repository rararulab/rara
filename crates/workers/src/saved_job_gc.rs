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

//! Background worker that periodically checks for expired saved job URLs
//! and cleans up associated S3 objects.
//!
//! **Phase 1 — URL liveness check**
//!
//! For every saved job older than `max_age_days` that is not already in a
//! terminal status (Failed / Expired), a lightweight HEAD request is sent to
//! the original URL. If the server responds with 404, 410, or the request
//! times out, the job is marked `Expired`.
//!
//! **Phase 2 — S3 object cleanup**
//!
//! For every saved job already marked `Expired` that still has an S3 key,
//! the corresponding object is deleted from the object store and the key is
//! cleared in the database.

use std::time::Duration;

use async_trait::async_trait;
use common_worker::{FallibleWorker, WorkError, WorkResult, WorkerContext};
use rara_domain_job::types::{PipelineEventKind, PipelineStage, SavedJobStatus};
use tracing::{info, instrument, warn};

use crate::worker_state::AppState;

// -------------------------------------------------------------------------
// Configuration
// -------------------------------------------------------------------------

/// Tuning knobs for the saved-job GC worker.
pub struct GcConfig {
    /// Maximum age (in days) of a saved job before it is checked for expiry.
    pub max_age_days: u32,
}

impl Default for GcConfig {
    fn default() -> Self { Self { max_age_days: 30 } }
}

// -------------------------------------------------------------------------
// Worker
// -------------------------------------------------------------------------

/// Periodic worker that garbage-collects stale saved job URLs and their S3
/// objects.
pub struct SavedJobGcWorker {
    config:      GcConfig,
    http_client: reqwest::Client,
}

impl SavedJobGcWorker {
    /// Create a new GC worker with the given configuration.
    #[must_use]
    pub fn new(config: GcConfig) -> Self {
        Self {
            config,
            http_client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::limited(5))
                .build()
                .expect("build reqwest client"),
        }
    }
}

#[async_trait]
impl FallibleWorker<AppState> for SavedJobGcWorker {
    #[instrument(skip_all, name = "saved_job_gc")]
    async fn work(&mut self, ctx: WorkerContext<AppState>) -> WorkResult {
        let state = ctx.state();

        let job_service = &state.job_service;

        // -----------------------------------------------------------------
        // Phase 1: Check stale URLs for liveness
        // -----------------------------------------------------------------

        let cutoff = jiff::Timestamp::now()
            .checked_sub(jiff::SignedDuration::from_hours(
                i64::from(self.config.max_age_days) * 24,
            ))
            .expect("timestamp subtraction");

        let stale_jobs = job_service
            .list_stale(cutoff)
            .await
            .map_err(|e| WorkError::transient(format!("list_stale failed: {e}")))?;

        let total_checked = stale_jobs.len();
        let mut expired_count: usize = 0;

        for job in &stale_jobs {
            if ctx.is_cancelled() {
                info!("GC worker cancelled during URL checks");
                break;
            }

            let req = self.http_client.head(&job.url).send();
            let is_dead = tokio::select! {
                () = ctx.cancelled() => {
                    info!("GC worker cancelled during URL checks");
                    break;
                }
                resp = req => {
                    match resp {
                        Ok(resp) => {
                            let status = resp.status();
                            status == reqwest::StatusCode::NOT_FOUND
                                || status == reqwest::StatusCode::GONE
                        }
                        Err(e) if e.is_timeout() || e.is_connect() => true,
                        Err(_) => false,
                    }
                }
            };

            if is_dead {
                if let Err(e) = job_service
                    .update_status(
                        job.id,
                        SavedJobStatus::Expired,
                        Some("URL no longer accessible".to_owned()),
                    )
                    .await
                {
                    warn!(id = %job.id, error = %e, "failed to mark saved job as expired");
                } else {
                    let _ = job_service
                        .log_event(
                            job.id,
                            PipelineStage::Gc,
                            PipelineEventKind::Completed,
                            "URL expired",
                            None,
                        )
                        .await;
                    expired_count += 1;
                }
            }

            // Small delay to avoid hammering external servers.
            tokio::select! {
                () = ctx.cancelled() => {
                    info!("GC worker cancelled during URL checks");
                    break;
                }
                () = tokio::time::sleep(Duration::from_millis(100)) => {}
            }
        }

        if total_checked > 0 {
            info!(total_checked, expired_count, "URL liveness check complete");
        }

        // -----------------------------------------------------------------
        // Phase 2: Clean up S3 objects for expired jobs
        // -----------------------------------------------------------------

        let object_store = &state.object_store;

        let expired_with_s3 = job_service
            .list_with_s3_keys_by_status(&[SavedJobStatus::Expired])
            .await
            .map_err(|e| {
                WorkError::transient(format!("list_with_s3_keys_by_status failed: {e}"))
            })?;

        let mut cleaned_count: usize = 0;

        for job in &expired_with_s3 {
            if ctx.is_cancelled() {
                info!("GC worker cancelled during S3 cleanup");
                break;
            }

            let s3_key = match &job.markdown_s3_key {
                Some(k) => k,
                None => continue,
            };

            if let Err(e) = object_store.delete(s3_key).await {
                warn!(
                    id = %job.id,
                    key = %s3_key,
                    error = %e,
                    "failed to delete S3 object"
                );
                continue;
            }

            if let Err(e) = job_service.clear_s3_key(job.id).await {
                warn!(
                    id = %job.id,
                    error = %e,
                    "failed to clear S3 key in DB"
                );
            } else {
                let _ = job_service
                    .log_event(
                        job.id,
                        PipelineStage::Gc,
                        PipelineEventKind::Info,
                        "S3 object cleaned up",
                        Some(serde_json::json!({ "s3_key": s3_key })),
                    )
                    .await;
                cleaned_count += 1;
            }
        }

        if !expired_with_s3.is_empty() {
            info!(
                total = expired_with_s3.len(),
                cleaned = cleaned_count,
                "S3 object cleanup complete"
            );
        }

        Ok(())
    }
}
