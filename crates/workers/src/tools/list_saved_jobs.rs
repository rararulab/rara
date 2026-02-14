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

use std::str::FromStr;

use async_trait::async_trait;
use rara_agents::tool_registry::AgentTool;
use rara_domain_job::types::SavedJobStatus;
use serde_json::json;

/// Agent tool that lists saved job postings with optional status filtering.
pub struct ListSavedJobsTool {
    job_service: rara_domain_job::service::JobService,
}

impl ListSavedJobsTool {
    pub fn new(job_service: rara_domain_job::service::JobService) -> Self {
        Self { job_service }
    }
}

#[async_trait]
impl AgentTool for ListSavedJobsTool {
    fn name(&self) -> &str { "list_saved_jobs" }

    fn description(&self) -> &str {
        "List saved job postings, optionally filtered by pipeline status. Valid statuses: \
         pending_crawl, crawling, crawled, analyzing, analyzed, failed, expired. \
         Returns an array of saved jobs with their ID, URL, title, company, status, and \
         match score."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "description": "Filter by pipeline status (e.g. pending_crawl, crawling, crawled, analyzing, analyzed, failed, expired)"
                }
            }
        })
    }

    async fn execute(&self, params: serde_json::Value) -> rara_agents::err::Result<serde_json::Value> {
        let status = params
            .get("status")
            .and_then(|v| v.as_str())
            .map(|s| {
                SavedJobStatus::from_str(s).map_err(|_| rara_agents::err::Error::Other {
                    message: format!("invalid status: {s}").into(),
                })
            })
            .transpose()?;

        match self.job_service.list(status).await {
            Ok(jobs) => {
                let items: Vec<serde_json::Value> = jobs
                    .into_iter()
                    .map(|job| {
                        json!({
                            "id": job.id.to_string(),
                            "url": job.url,
                            "title": job.title,
                            "company": job.company,
                            "status": job.status.to_string(),
                            "match_score": job.match_score,
                            "created_at": job.created_at.to_string(),
                        })
                    })
                    .collect();
                Ok(json!(items))
            }
            Err(e) => Ok(json!({
                "error": format!("{e}"),
            })),
        }
    }
}
