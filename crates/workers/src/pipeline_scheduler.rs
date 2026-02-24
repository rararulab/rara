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
//! The worker polls on a configurable interval (default 60s). On each tick it
//! reads the `pipeline_cron` field from [`JobPipelineSettings`]. It uses a
//! forward-looking cron check anchored on `last_run_at` to determine whether
//! the cron has fired, preventing duplicate execution regardless of poll
//! interval.

use std::str::FromStr;

use async_trait::async_trait;
use common_worker::{FallibleWorker, WorkResult, WorkerContext};
use tracing::{debug, info, warn};

use crate::worker_state::AppState;

/// Worker that checks the `pipeline_cron` setting on each tick and triggers
/// a pipeline run when the cron expression fires.
///
/// Tracks `last_run_at` internally to prevent duplicate execution even when
/// the worker poll interval is longer or shorter than the cron period.
pub struct PipelineSchedulerWorker {
    last_run_at: Option<jiff::Timestamp>,
}

impl PipelineSchedulerWorker {
    pub fn new() -> Self {
        Self { last_run_at: None }
    }
}

impl Default for PipelineSchedulerWorker {
    fn default() -> Self { Self::new() }
}

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
        if !settings.ai.is_configured() {
            debug!("pipeline-scheduler: skipped — AI not configured");
            return Ok(());
        }

        // Forward-looking cron check using last_run_at.
        if !is_cron_due(&cron_expr, self.last_run_at) {
            return Ok(());
        }

        // Trigger pipeline run.
        match state.pipeline_service.run().await {
            Ok(()) => {
                self.last_run_at = Some(jiff::Timestamp::now());
                info!("pipeline-scheduler: triggered pipeline run");
            }
            Err(e) => debug!("pipeline-scheduler: skipped — {e}"),
        }

        Ok(())
    }
}

/// Forward-looking cron check anchored on `last_run_at`.
///
/// Finds the next cron occurrence after `last_run_at` (or epoch if never run)
/// and returns `true` when that occurrence falls at or before now.
fn is_cron_due(expr: &str, last_run_at: Option<jiff::Timestamp>) -> bool {
    let Ok(cron) = croner::Cron::from_str(expr) else {
        warn!(expr, "pipeline-scheduler: invalid cron expression");
        return false;
    };

    let anchor_secs = last_run_at.map(|ts| ts.as_second()).unwrap_or(0);
    let anchor =
        chrono::DateTime::from_timestamp(anchor_secs, 0).unwrap_or_else(chrono::Utc::now);
    let now = chrono::Utc::now();

    cron.find_next_occurrence(&anchor, false)
        .is_ok_and(|next| next <= now)
}
