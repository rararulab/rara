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

//! Pipeline tool that lets the agent trigger JobSpy discovery through the
//! domain `JobService`, with stable normalization and database persistence.

use std::collections::HashSet;
use std::str::FromStr;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use sqlx::PgPool;
use tool_core::AgentTool;
use tracing::warn;
use uuid::Uuid;

use rara_domain_job::{
    error::SourceError,
    types::{DiscoveryCriteria, NormalizedJob},
};

use crate::{
    pg_repository::PgPipelineRepository,
    repository::PipelineRepository,
    types::DiscoveredJobAction,
};

/// Agent tool wrapper for `JobService::discover()` + `JobRepository::save()`.
///
/// AI picks the search parameters, but the service performs the actual search,
/// normalization, dedup-within-result, and durable persistence.
pub struct SearchJobsWithJobServiceTool {
    job_service: rara_domain_job::service::JobService,
    pool:        PgPool,
}

impl SearchJobsWithJobServiceTool {
    pub fn new(job_service: rara_domain_job::service::JobService, pool: PgPool) -> Self {
        Self { job_service, pool }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
struct SearchJobsWithJobServiceParams {
    run_id:                           Uuid,
    keywords:                        Vec<String>,
    location:                        Option<String>,
    job_type:                        Option<String>,
    max_results:                     Option<u32>,
    posted_after:                    Option<String>,
    #[serde(default)]
    sites:                           Vec<String>,
}

#[async_trait]
impl AgentTool for SearchJobsWithJobServiceTool {
    fn name(&self) -> &str { "search_jobs_with_job_service" }

    fn description(&self) -> &str {
        "Search jobs via the backend JobService (JobSpy), normalize and deduplicate results, \
         persist new jobs to the job database, enqueue them for scoring in the current run, \
         and return summary counts. \
         Prefer this over raw MCP scraping for stable pipeline search."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "keywords": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Search keywords / roles (e.g. [\"rust engineer\", \"backend engineer\"])"
                },
                "run_id": {
                    "type": "string",
                    "description": "Pipeline run ID (UUID) used to enqueue discovered jobs for scoring"
                },
                "location": {
                    "type": "string",
                    "description": "Location filter (e.g. \"Remote\", \"San Francisco, CA\")"
                },
                "job_type": {
                    "type": "string",
                    "description": "Optional job type (e.g. full-time, contract)"
                },
                "max_results": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100,
                    "description": "Max results to fetch (capped at 100). Recommend 25-50."
                },
                "posted_after": {
                    "type": "string",
                    "description": "Optional timestamp filter (RFC3339/ISO-8601)"
                },
                "sites": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional sites list (e.g. [\"linkedin\", \"indeed\"]). Leave empty for defaults."
                }
            },
            "required": ["run_id", "keywords"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let parsed: SearchJobsWithJobServiceParams = serde_json::from_value(params)
            .map_err(|e| anyhow::anyhow!("invalid parameters for search_jobs_with_job_service: {e}"))?;

        if parsed.keywords.iter().all(|k| k.trim().is_empty()) {
            return Err(anyhow::anyhow!("keywords must contain at least one non-empty string"));
        }

        let max_results = parsed.max_results.unwrap_or(25).clamp(1, 100);
        let posted_after = parse_posted_after(parsed.posted_after.as_deref())?;

        let criteria = DiscoveryCriteria {
            keywords: parsed
                .keywords
                .into_iter()
                .map(|k| k.trim().to_owned())
                .filter(|k| !k.is_empty())
                .collect(),
            location: parsed.location.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()),
            job_type: parsed.job_type.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()),
            max_results: Some(max_results),
            posted_after,
            sites: parsed.sites,
        };

        let existing_source_keys = HashSet::new();
        let existing_fuzzy_keys = HashSet::new();
        let discovery_result = self
            .job_service
            .discover_all(&criteria, &existing_source_keys, &existing_fuzzy_keys)
            .await;

        if let Some(err) = discovery_result.error {
            return Ok(json!({
                "status": "error",
                "error": err.to_string(),
                "criteria": criteria,
            }));
        }

        let discovered_jobs = discovery_result.jobs;
        let discovered_count = discovered_jobs.len();
        let pipeline_repo = PgPipelineRepository::new(self.pool.clone());

        let mut persisted_jobs: Vec<NormalizedJob> = Vec::new();
        let mut duplicate_count = 0usize;
        let mut persist_errors = Vec::new();
        let mut queued_for_scoring_count = 0usize;
        let mut queue_errors = Vec::new();

        for job in discovered_jobs {
            match self.job_service.job_repo().save(&job).await {
                Ok(saved) => persisted_jobs.push(saved),
                Err(e) if is_job_unique_violation(&e) => {
                    duplicate_count += 1;
                }
                Err(e) => {
                    warn!(error = %e, title = %job.title, company = %job.company, "search_jobs_with_job_service: persist failed");
                    persist_errors.push(json!({
                        "title": job.title,
                        "company": job.company,
                        "error": e.to_string(),
                    }));
                }
            }
        }
        for job in &persisted_jobs {
            match pipeline_repo
                .insert_discovered_job(
                    parsed.run_id,
                    job.id,
                    None,
                    DiscoveredJobAction::Discovered,
                )
                .await
            {
                Ok(_) => queued_for_scoring_count += 1,
                Err(e) => {
                    warn!(error = %e, title = %job.title, company = %job.company, "search_jobs_with_job_service: enqueue discovered job failed");
                    queue_errors.push(json!({
                        "title": job.title,
                        "company": job.company,
                        "error": e.to_string(),
                    }));
                }
            }
        }

        Ok(json!({
            "status": "ok",
            "criteria": criteria,
            "summary": {
                "discovered_count": discovered_count,
                "persisted_new_count": persisted_jobs.len(),
                "duplicate_count": duplicate_count,
                "persist_failed_count": persist_errors.len(),
                "queued_for_scoring_count": queued_for_scoring_count,
                "queue_failed_count": queue_errors.len(),
            },
            "persist_errors": persist_errors,
            "queue_errors": queue_errors,
            "notes": [
                "Results are normalized and deduplicated within the search batch by JobService.",
                "New jobs are persisted to the primary job table.",
                "Newly persisted jobs are enqueued in pipeline_discovered_jobs for scoring."
            ]
        }))
    }
}

fn is_job_unique_violation(error: &SourceError) -> bool {
    match error {
        SourceError::NonRetryable { source_name, message } if source_name == "pg" => {
            message.contains("uq_job_source")
                || (message.contains("duplicate key value")
                    && message.contains("source_job_id")
                    && message.contains("source_name"))
        }
        _ => false,
    }
}

fn parse_posted_after(input: Option<&str>) -> anyhow::Result<Option<jiff::Timestamp>> {
    let Some(raw) = input.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };

    // First try the native parser (strict timestamp with offset).
    if let Ok(ts) = jiff::Timestamp::from_str(raw) {
        return Ok(Some(ts));
    }

    // Accept a date-only input by assuming start-of-day UTC.
    if raw.len() == 10 && raw.as_bytes().get(4) == Some(&b'-') && raw.as_bytes().get(7) == Some(&b'-')
    {
        let normalized = format!("{raw}T00:00:00Z");
        if let Ok(ts) = jiff::Timestamp::from_str(&normalized) {
            return Ok(Some(ts));
        }
    }

    // Accept datetime without offset by assuming UTC.
    if raw.contains('T') && !has_timezone_offset(raw) {
        let normalized = format!("{raw}Z");
        if let Ok(ts) = jiff::Timestamp::from_str(&normalized) {
            return Ok(Some(ts));
        }
    }

    Err(anyhow::anyhow!(
        "invalid posted_after timestamp: '{raw}'. Use ISO-8601 like 2026-02-22T00:00:00Z (date-only is also accepted)"
    ))
}

fn has_timezone_offset(value: &str) -> bool {
    value.ends_with('Z')
        || value
            .rsplit_once('T')
            .is_some_and(|(_, time_part)| time_part.contains('+') || time_part.contains('-'))
}
