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
         pipeline run with accurate counts. Supports auto-aggregation from \
         pipeline_discovered_jobs to reduce counting errors. This updates the \
         database so the frontend displays correct numbers."
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
                    "description": "Total jobs found. Optional when auto_aggregate=true (defaults to discovered jobs count for this run)."
                },
                "jobs_scored": {
                    "type": "integer",
                    "description": "Number of jobs that were scored. Optional when auto_aggregate=true."
                },
                "jobs_applied": {
                    "type": "integer",
                    "description": "Number of jobs auto-applied to. Optional when auto_aggregate=true."
                },
                "jobs_notified": {
                    "type": "integer",
                    "description": "Number of jobs the user was notified about. Optional when auto_aggregate=true."
                },
                "auto_aggregate": {
                    "type": "boolean",
                    "description": "If true, compute counts from pipeline_discovered_jobs for this run. Manual values (if provided) override computed values."
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

        let run_id: uuid::Uuid = run_id_str
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid run_id UUID: {e}"))?;

        let auto_aggregate = params
            .get("auto_aggregate")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let manual_jobs_found = params
            .get("jobs_found")
            .and_then(|v| v.as_i64())
            .map(|v| v as i32);
        let manual_jobs_scored = params
            .get("jobs_scored")
            .and_then(|v| v.as_i64())
            .map(|v| v as i32);
        let manual_jobs_applied = params
            .get("jobs_applied")
            .and_then(|v| v.as_i64())
            .map(|v| v as i32);
        let manual_jobs_notified = params
            .get("jobs_notified")
            .and_then(|v| v.as_i64())
            .map(|v| v as i32);

        let (agg_jobs_found, agg_jobs_scored, agg_jobs_applied, agg_jobs_notified) = if auto_aggregate
        {
            let row = sqlx::query_as::<_, (i32, i32, i32, i32)>(
                r#"
                SELECT
                    COUNT(*)::INT AS jobs_found,
                    COUNT(*) FILTER (WHERE score IS NOT NULL)::INT AS jobs_scored,
                    COUNT(*) FILTER (WHERE action = 2)::INT AS jobs_applied,
                    COUNT(*) FILTER (WHERE action = 1)::INT AS jobs_notified
                FROM pipeline_discovered_jobs
                WHERE run_id = $1
                "#,
            )
            .bind(run_id)
            .fetch_one(&self.pool)
            .await;

            match row {
                Ok((jobs_found, jobs_scored, jobs_applied, jobs_notified)) => {
                    (jobs_found, jobs_scored, jobs_applied, jobs_notified)
                }
                Err(e) => {
                    warn!(error = %e, run_id = %run_id, "report_pipeline_stats: aggregation query failed");
                    return Ok(json!({ "error": format!("{e}") }));
                }
            }
        } else {
            (0, 0, 0, 0)
        };

        let jobs_found = manual_jobs_found.unwrap_or(agg_jobs_found);
        let jobs_scored = manual_jobs_scored.unwrap_or(agg_jobs_scored);
        let jobs_applied = manual_jobs_applied.unwrap_or(agg_jobs_applied);
        let jobs_notified = manual_jobs_notified.unwrap_or(agg_jobs_notified);

        if !auto_aggregate
            && (manual_jobs_found.is_none()
                || manual_jobs_scored.is_none()
                || manual_jobs_applied.is_none()
                || manual_jobs_notified.is_none())
        {
            return Err(anyhow::anyhow!(
                "when auto_aggregate is false, jobs_found/jobs_scored/jobs_applied/jobs_notified are required"
            ));
        }

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
                        "auto_aggregate": auto_aggregate,
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
