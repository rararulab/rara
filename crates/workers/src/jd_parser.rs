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

//! JD parser worker — drains the parse channel and processes each request.

use async_trait::async_trait;
use common_worker::{FallibleWorker, WorkResult, WorkerContext};
use tokio::sync::mpsc;
use tracing::{error, info};

use crate::{types::JdParseRequest, worker_state::AppState};

/// Worker that drains the JD parse channel on each tick.
///
/// For every [`JdParseRequest`]:
/// 1. Calls [`JobService::parse_jd`] which runs the AI agent and saves the job.
/// 2. Logs the outcome.
pub struct JdParserWorker {
    rx: mpsc::Receiver<JdParseRequest>,
}

impl JdParserWorker {
    pub fn new(rx: mpsc::Receiver<JdParseRequest>) -> Self { Self { rx } }
}

#[async_trait]
impl FallibleWorker<AppState> for JdParserWorker {
    async fn work(&mut self, ctx: WorkerContext<AppState>) -> WorkResult {
        // Drain all pending requests from the channel.
        while let Ok(req) = self.rx.try_recv() {
            let state = ctx.state();

            match state.job_service.parse_jd(&req.text).await {
                Ok(job) => {
                    info!(
                        title = %job.title,
                        company = %job.company,
                        "JD parsed and saved"
                    );
                }
                Err(e) => {
                    error!(error = %e, "JD parse failed");
                }
            }
        }

        Ok(())
    }
}
