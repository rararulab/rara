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

//! Tool for updating score/action on a discovered pipeline job.

use async_trait::async_trait;
use serde_json::json;
use sqlx::PgPool;
use tool_core::AgentTool;
use uuid::Uuid;

use crate::{
    pg_repository::PgPipelineRepository,
    repository::PipelineRepository,
    types::DiscoveredJobAction,
};

pub struct UpdateDiscoveredJobScoreActionTool {
    pool: PgPool,
}

impl UpdateDiscoveredJobScoreActionTool {
    pub fn new(pool: PgPool) -> Self { Self { pool } }
}

#[async_trait]
impl AgentTool for UpdateDiscoveredJobScoreActionTool {
    fn name(&self) -> &str { "update_discovered_job_score_action" }

    fn description(&self) -> &str {
        "Update a discovered job's score and/or action in pipeline_discovered_jobs. \
         Use this after scoring each job and after notifying/applying/skipping."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Discovered job row ID (UUID)"
                },
                "score": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 100,
                    "description": "Optional match score to persist"
                },
                "action": {
                    "type": "string",
                    "enum": ["discovered", "notified", "applied", "skipped"],
                    "description": "Optional action to persist"
                }
            },
            "required": ["id"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let id_str = params
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: id"))?;
        let id: Uuid = id_str
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid discovered job id UUID: {e}"))?;

        let score = params.get("score").and_then(|v| v.as_i64()).map(|v| v as i32);
        let action = match params.get("action").and_then(|v| v.as_str()) {
            Some("discovered") => Some(DiscoveredJobAction::Discovered),
            Some("notified") => Some(DiscoveredJobAction::Notified),
            Some("applied") => Some(DiscoveredJobAction::Applied),
            Some("skipped") => Some(DiscoveredJobAction::Skipped),
            Some(other) => {
                return Err(anyhow::anyhow!(
                    "invalid action: {other} (expected discovered|notified|applied|skipped)"
                ));
            }
            None => None,
        };

        if score.is_none() && action.is_none() {
            return Err(anyhow::anyhow!(
                "at least one of score or action must be provided"
            ));
        }

        let repo = PgPipelineRepository::new(self.pool.clone());
        let updated = repo
            .update_discovered_job_score_action(id, score, action)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        match updated {
            Some(job) => Ok(json!({
                "status": "updated",
                "job": {
                    "id": job.id,
                    "run_id": job.run_id,
                    "job_id": job.job_id,
                    "score": job.score,
                    "action": job.action.to_string(),
                }
            })),
            None => Ok(json!({
                "status": "not_found",
                "id": id,
            })),
        }
    }
}
