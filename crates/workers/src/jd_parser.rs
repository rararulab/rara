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
use job_common_worker::{FallibleWorker, WorkResult, WorkerContext};
use tokio::sync::mpsc;
use tracing::{error, info};

use crate::{types::JdParseRequest, worker_state::AppWorkerState};

/// Worker that drains the JD parse channel on each tick.
///
/// For every [`JdParseRequest`]:
/// 1. Calls the AI agent to extract structured fields.
/// 2. Saves the resulting `NormalizedJob` via `JobRepository`.
/// 3. Persists results to DB for downstream workflows.
pub struct JdParserWorker {
    rx: mpsc::Receiver<JdParseRequest>,
}

impl JdParserWorker {
    pub fn new(rx: mpsc::Receiver<JdParseRequest>) -> Self { Self { rx } }
}

/// Intermediate struct for deserializing the AI response JSON.
#[derive(serde::Deserialize)]
struct ParsedJob {
    title:           String,
    company:         String,
    location:        Option<String>,
    description:     Option<String>,
    url:             Option<String>,
    salary_min:      Option<i32>,
    salary_max:      Option<i32>,
    salary_currency: Option<String>,
    tags:            Option<Vec<String>>,
}

#[async_trait]
impl FallibleWorker<AppWorkerState> for JdParserWorker {
    async fn work(&mut self, ctx: WorkerContext<AppWorkerState>) -> WorkResult {
        // Drain all pending requests from the channel.
        while let Ok(req) = self.rx.try_recv() {
            let state = ctx.state();

            let ai = match state
                .ai_service_handle
                .read()
                .ok()
                .and_then(|g| g.as_ref().cloned())
            {
                Some(ai) => ai,
                None => {
                    error!("AI service not configured; skipping JD parse request");
                    continue;
                }
            };
            let repo = &state.job_repo;

            // 1. AI parse
            let json_str = match ai.jd_parser().parse(&req.text).await {
                Ok(s) => s,
                Err(e) => {
                    error!(error = %e, "AI JD parse failed");
                    continue;
                }
            };

            // 2. Deserialize AI output
            let parsed: ParsedJob = match serde_json::from_str(&json_str) {
                Ok(p) => p,
                Err(e) => {
                    error!(error = %e, raw = %json_str, "Failed to deserialize AI response");
                    continue;
                }
            };

            // 3. Build NormalizedJob
            let job = job_domain_job_source::types::NormalizedJob {
                id:              uuid::Uuid::new_v4(),
                source_job_id:   uuid::Uuid::new_v4().to_string(),
                source_name:     "telegram".to_string(),
                title:           parsed.title.clone(),
                company:         parsed.company.clone(),
                location:        parsed.location,
                description:     parsed.description,
                url:             parsed.url,
                salary_min:      parsed.salary_min,
                salary_max:      parsed.salary_max,
                salary_currency: parsed.salary_currency,
                tags:            parsed.tags.unwrap_or_default(),
                raw_data:        serde_json::to_value(&req.text).ok(),
                posted_at:       None,
            };

            // 4. Save to DB
            match repo.save(&job).await {
                Ok(saved) => {
                    info!(
                        title = %saved.title,
                        company = %saved.company,
                        "JD parsed and saved"
                    );
                }
                Err(e) => {
                    error!(error = %e, "Failed to save job");
                }
            }
        }

        Ok(())
    }
}
