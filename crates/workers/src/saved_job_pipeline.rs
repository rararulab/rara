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

//! Background worker that processes saved jobs through the crawl + AI
//! analysis pipeline.
//!
//! Triggered via notify when a new saved job is created. Drains all
//! `PendingCrawl` jobs from the database each cycle.

use async_trait::async_trait;
use job_common_worker::{FallibleWorker, WorkError, WorkResult, WorkerContext};
use tracing::{info, warn};

use crate::notification_processor::WorkerState;

/// Worker that processes pending saved jobs through the pipeline.
pub struct SavedJobPipelineWorker;

#[async_trait]
impl FallibleWorker<WorkerState> for SavedJobPipelineWorker {
    async fn work(&mut self, ctx: WorkerContext<WorkerState>) -> WorkResult {
        let state = ctx.state();

        let pipeline = match &state.saved_job_pipeline {
            Some(p) => p,
            None => return Ok(()), // Pipeline not configured
        };

        match pipeline.process_pending_batch().await {
            Ok(count) => {
                if count > 0 {
                    info!(processed = count, "saved job pipeline batch complete");
                }
                Ok(())
            }
            Err(e) => {
                warn!(error = %e, "saved job pipeline batch failed");
                Err(WorkError::transient(format!(
                    "saved job pipeline failed: {e}"
                )))
            }
        }
    }
}
