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

use crate::pg_repository::PgPipelineRepository;
use crate::repository::PipelineRepository;
use crate::types::DiscoveredJobAction;

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
         job after scoring it, so the frontend can display all discovered jobs \
         for a pipeline run."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "run_id": {
                    "type": "string",
                    "description": "The pipeline run ID (UUID from the kick message)"
                },
                "title": {
                    "type": "string",
                    "description": "Job title"
                },
                "company": {
                    "type": "string",
                    "description": "Company name"
                },
                "location": {
                    "type": "string",
                    "description": "Job location"
                },
                "url": {
                    "type": "string",
                    "description": "Job posting URL"
                },
                "description": {
                    "type": "string",
                    "description": "Job description or summary"
                },
                "score": {
                    "type": "integer",
                    "description": "Match score (0-100)"
                },
                "action": {
                    "type": "string",
                    "enum": ["discovered", "notified", "applied", "skipped"],
                    "description": "What action was taken for this job"
                },
                "date_posted": {
                    "type": "string",
                    "description": "When the job was posted"
                }
            },
            "required": ["run_id", "title"]
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

        let title = params
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: title"))?;

        let company = params.get("company").and_then(|v| v.as_str());
        let location = params.get("location").and_then(|v| v.as_str());
        let url = params.get("url").and_then(|v| v.as_str());
        let description = params.get("description").and_then(|v| v.as_str());
        let score = params.get("score").and_then(|v| v.as_i64()).map(|v| v as i32);
        let date_posted = params.get("date_posted").and_then(|v| v.as_str());

        let action = match params.get("action").and_then(|v| v.as_str()) {
            Some("notified") => DiscoveredJobAction::Notified,
            Some("applied") => DiscoveredJobAction::Applied,
            Some("skipped") => DiscoveredJobAction::Skipped,
            _ => DiscoveredJobAction::Discovered,
        };

        let repo = PgPipelineRepository::new(self.pool.clone());
        match repo
            .insert_discovered_job(
                run_id,
                title,
                company,
                location,
                url,
                description,
                score,
                action,
                date_posted,
            )
            .await
        {
            Ok(job) => Ok(json!({
                "status": "saved",
                "id": job.id.to_string(),
                "title": job.title,
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
