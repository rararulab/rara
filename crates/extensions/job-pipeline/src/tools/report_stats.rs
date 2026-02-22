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

//! Tool for the pipeline agent to report execution statistics back to the
//! database so the frontend can display accurate counts.

use async_trait::async_trait;
use serde_json::json;
use sqlx::PgPool;
use tool_core::AgentTool;
use tracing::warn;

/// Pipeline agent tool that writes execution statistics (jobs found, scored,
/// applied, notified) directly to the `pipeline_runs` table.
pub struct ReportPipelineStatsTool {
    pool: PgPool,
}

impl ReportPipelineStatsTool {
    pub fn new(pool: PgPool) -> Self { Self { pool } }
}

#[async_trait]
impl AgentTool for ReportPipelineStatsTool {
    fn name(&self) -> &str { "report_pipeline_stats" }

    fn description(&self) -> &str {
        "Report pipeline execution statistics. MUST be called at the end of the \
         pipeline run with accurate counts. This updates the database so the \
         frontend displays correct numbers."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "run_id": {
                    "type": "string",
                    "description": "The pipeline run ID (UUID provided in the kick message)"
                },
                "jobs_found": {
                    "type": "integer",
                    "description": "Total number of jobs returned by search (before dedup)"
                },
                "jobs_scored": {
                    "type": "integer",
                    "description": "Number of new jobs that were scored"
                },
                "jobs_applied": {
                    "type": "integer",
                    "description": "Number of jobs auto-applied to"
                },
                "jobs_notified": {
                    "type": "integer",
                    "description": "Number of jobs the user was notified about"
                }
            },
            "required": ["run_id", "jobs_found", "jobs_scored", "jobs_applied", "jobs_notified"]
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

        let jobs_found = params
            .get("jobs_found")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;
        let jobs_scored = params
            .get("jobs_scored")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;
        let jobs_applied = params
            .get("jobs_applied")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;
        let jobs_notified = params
            .get("jobs_notified")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;

        let result = sqlx::query(
            r#"UPDATE pipeline_runs
               SET jobs_found = $2,
                   jobs_scored = $3,
                   jobs_applied = $4,
                   jobs_notified = $5
               WHERE id = $1"#,
        )
        .bind(run_id)
        .bind(jobs_found)
        .bind(jobs_scored)
        .bind(jobs_applied)
        .bind(jobs_notified)
        .execute(&self.pool)
        .await;

        match result {
            Ok(r) => {
                if r.rows_affected() == 0 {
                    warn!(run_id = %run_id, "report_pipeline_stats: no matching run found");
                    Ok(json!({ "error": "run_id not found" }))
                } else {
                    Ok(json!({
                        "status": "updated",
                        "jobs_found": jobs_found,
                        "jobs_scored": jobs_scored,
                        "jobs_applied": jobs_applied,
                        "jobs_notified": jobs_notified,
                    }))
                }
            }
            Err(e) => {
                warn!(error = %e, "report_pipeline_stats: DB update failed");
                Ok(json!({ "error": format!("{e}") }))
            }
        }
    }
}
