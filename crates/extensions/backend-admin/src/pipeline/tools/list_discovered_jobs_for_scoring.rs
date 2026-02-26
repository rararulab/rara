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

//! Tool for fetching batches of discovered jobs that still need scoring.

use async_trait::async_trait;
use serde_json::json;
use sqlx::PgPool;
use tool_core::AgentTool;
use uuid::Uuid;

use super::super::{pg_repository::PgPipelineRepository, repository::PipelineRepository};

pub struct ListDiscoveredJobsForScoringTool {
    pool: PgPool,
}

impl ListDiscoveredJobsForScoringTool {
    pub fn new(pool: PgPool) -> Self { Self { pool } }
}

#[async_trait]
impl AgentTool for ListDiscoveredJobsForScoringTool {
    fn name(&self) -> &str { "list_discovered_jobs_for_scoring" }

    fn description(&self) -> &str {
        "List a batch of discovered jobs for a pipeline run that still need scoring (score is \
         null). Use this in a loop until it returns an empty list."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "run_id": {
                    "type": "string",
                    "description": "Pipeline run ID (UUID)"
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100,
                    "description": "Batch size (default 10)"
                },
                "offset": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Pagination offset (default 0)"
                }
            },
            "required": ["run_id"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let run_id_str = params
            .get("run_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: run_id"))?;
        let run_id: Uuid = run_id_str
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid run_id UUID: {e}"))?;

        let limit = params
            .get("limit")
            .and_then(|v| v.as_i64())
            .unwrap_or(10)
            .clamp(1, 100);
        let offset = params
            .get("offset")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            .max(0);

        let repo = PgPipelineRepository::new(self.pool.clone());
        let jobs = repo
            .list_unscored_discovered_jobs(run_id, limit, offset)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        let items = jobs
            .into_iter()
            .map(|j| {
                json!({
                    "id": j.id,
                    "run_id": j.run_id,
                    "job_id": j.job_id,
                    "title": j.title,
                    "company": j.company,
                    "location": j.location,
                    "url": j.url,
                    "description": j.description,
                    "score": j.score,
                    "action": j.action.to_string(),
                    "posted_at": j.posted_at,
                    "created_at": j.created_at,
                })
            })
            .collect::<Vec<_>>();

        Ok(json!({
            "run_id": run_id,
            "limit": limit,
            "offset": offset,
            "count": items.len(),
            "jobs": items,
        }))
    }
}
