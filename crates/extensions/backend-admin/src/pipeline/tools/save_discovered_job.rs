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

//! Tool for the pipeline agent to persist discovered jobs to the database.

use async_trait::async_trait;
use serde_json::json;
use sqlx::PgPool;
use tool_core::AgentTool;
use tracing::warn;

use super::super::pg_repository::PgPipelineRepository;
use super::super::repository::PipelineRepository;
use super::super::types::DiscoveredJobAction;

/// Pipeline agent tool that saves a discovered job to the
/// `pipeline_discovered_jobs` table.
pub struct SaveDiscoveredJobTool {
    pool: PgPool,
}

impl SaveDiscoveredJobTool {
    pub fn new(pool: PgPool) -> Self { Self { pool } }
}

#[async_trait]
impl AgentTool for SaveDiscoveredJobTool {
    fn name(&self) -> &str { "save_discovered_job" }

    fn description(&self) -> &str {
        "Save a discovered job to the database for tracking. Call this for each \
         job after it has been persisted to the job table, so the frontend can \
         display all discovered jobs for a pipeline run. Requires the job_id \
         (UUID) from the job table."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "run_id": {
                    "type": "string",
                    "description": "The pipeline run ID (UUID from the kick message)"
                },
                "job_id": {
                    "type": "string",
                    "description": "The job UUID from the job table"
                },
                "score": {
                    "type": "integer",
                    "description": "Match score (0-100)"
                },
                "action": {
                    "type": "string",
                    "enum": ["discovered", "notified", "applied", "skipped"],
                    "description": "What action was taken for this job"
                }
            },
            "required": ["run_id", "job_id"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let run_id_str = params
            .get("run_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: run_id"))?;

        let run_id: uuid::Uuid = run_id_str
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid run_id UUID: {e}"))?;

        let job_id_str = params
            .get("job_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: job_id"))?;

        let job_id: uuid::Uuid = job_id_str
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid job_id UUID: {e}"))?;

        let score = params.get("score").and_then(|v| v.as_i64()).map(|v| v as i32);

        let action = match params.get("action").and_then(|v| v.as_str()) {
            Some("notified") => DiscoveredJobAction::Notified,
            Some("applied") => DiscoveredJobAction::Applied,
            Some("skipped") => DiscoveredJobAction::Skipped,
            _ => DiscoveredJobAction::Discovered,
        };

        let repo = PgPipelineRepository::new(self.pool.clone());
        match repo
            .insert_discovered_job(run_id, job_id, score, action)
            .await
        {
            Ok(job) => Ok(json!({
                "status": "saved",
                "id": job.id.to_string(),
                "job_id": job.job_id.to_string(),
                "score": job.score,
                "action": format!("{}", job.action),
            })),
            Err(e) => {
                warn!(error = %e, "save_discovered_job: DB insert failed");
                Ok(json!({ "error": format!("{e}") }))
            }
        }
    }
}
