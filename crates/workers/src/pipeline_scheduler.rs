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

//! Background worker that triggers the job pipeline on a configurable cron
//! schedule.
//!
//! The worker runs on a fixed 60-second interval. On each tick it reads the
//! `pipeline_cron` field from [`JobPipelineSettings`]. If the cron expression
//! has a firing point within the last 60 seconds, the worker calls
//! [`PipelineService::run()`].

use std::str::FromStr;

use async_trait::async_trait;
use common_worker::{FallibleWorker, WorkResult, WorkerContext};
use tracing::{debug, info, warn};

use crate::worker_state::AppState;

/// Worker that checks the `pipeline_cron` setting every 60 seconds and
/// triggers a pipeline run when the cron expression fires.
pub struct PipelineSchedulerWorker;

#[async_trait]
impl FallibleWorker<AppState> for PipelineSchedulerWorker {
    async fn work(&mut self, ctx: WorkerContext<AppState>) -> WorkResult {
        let state = ctx.state();
        let settings = state.settings_svc.current();

        // Guard: pipeline_cron must be configured.
        let cron_expr = match &settings.job_pipeline.pipeline_cron {
            Some(expr) => expr.clone(),
            None => return Ok(()),
        };

        // Guard: AI must be configured (pipeline needs it).
        if settings.ai.openrouter_api_key.is_none() {
            debug!("pipeline-scheduler: skipped — AI not configured");
            return Ok(());
        }

        // Check if the cron expression fires within the last 60-second window.
        if !is_cron_due(&cron_expr) {
            return Ok(());
        }

        // Trigger pipeline run.
        match state.pipeline_service.run().await {
            Ok(()) => info!("pipeline-scheduler: triggered pipeline run"),
            Err(e) => debug!("pipeline-scheduler: skipped — {e}"),
        }

        Ok(())
    }
}

/// Check if the cron expression has a firing point within the current
/// 60-second window (now - 60s .. now].
fn is_cron_due(expr: &str) -> bool {
    let Ok(cron) = croner::Cron::from_str(expr) else {
        warn!(expr, "pipeline-scheduler: invalid cron expression");
        return false;
    };

    let now = chrono::Utc::now();
    let window_start = now - chrono::Duration::seconds(60);

    cron.find_next_occurrence(&window_start, false)
        .is_ok_and(|next| next <= now)
}
