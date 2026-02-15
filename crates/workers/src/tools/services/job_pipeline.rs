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

//! Layer 2 service: job pipeline tool.
//!
//! High-level business workflow that saves a job URL for automated crawling
//! and AI analysis. Migrated from the former `save_job_url` flat tool.

use async_trait::async_trait;
use rara_agents::tool_registry::AgentTool;
use serde_json::json;

/// Layer 2 service tool: save a job URL into the automated pipeline.
pub struct JobPipelineTool {
    job_service: rara_domain_job::service::JobService,
}

impl JobPipelineTool {
    pub fn new(job_service: rara_domain_job::service::JobService) -> Self { Self { job_service } }
}

#[async_trait]
impl AgentTool for JobPipelineTool {
    fn name(&self) -> &str { "job_pipeline" }

    fn description(&self) -> &str {
        "Save a job URL for automated crawling and AI analysis pipeline. The system will \
         automatically fetch the page content, extract the job description, analyze it for \
         relevance, and compute a match score. Returns the saved job record with its ID and \
         current pipeline status."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL of the job posting to save for pipeline processing"
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
    ) -> rara_agents::err::Result<serde_json::Value> {
        let url = params.get("url").and_then(|v| v.as_str()).ok_or_else(|| {
            rara_agents::err::Error::Other {
                message: "missing required parameter: url".into(),
            }
        })?;

        match self.job_service.create(url).await {
            Ok(job) => Ok(json!({
                "id": job.id.to_string(),
                "url": job.url,
                "status": job.status.to_string(),
                "created_at": job.created_at.to_string(),
            })),
            Err(e) => Ok(json!({
                "error": format!("{e}"),
            })),
        }
    }
}
