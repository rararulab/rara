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

//! Rara chat tools for controlling the job pipeline.
//!
//! These are registered on the main rara agent's tool registry so the
//! user can trigger/cancel/check the pipeline from a chat session.

use async_trait::async_trait;
use rara_domain_shared::settings::model::{JobPipelineRuntimeSettingsPatch, UpdateRequest};
use serde_json::json;
use rara_kernel::tool::AgentTool;

use super::super::service::PipelineService;
use crate::settings::SettingsSvc;

// ---------------------------------------------------------------------------
// RunJobPipelineTool
// ---------------------------------------------------------------------------

/// Tool that lets the main rara agent trigger a pipeline run.
pub struct RunJobPipelineTool {
    service: PipelineService,
}

impl RunJobPipelineTool {
    pub fn new(service: PipelineService) -> Self { Self { service } }
}

#[async_trait]
impl AgentTool for RunJobPipelineTool {
    fn name(&self) -> &str { "run_job_pipeline" }

    fn description(&self) -> &str {
        "Trigger the automated job pipeline. The pipeline agent will search for jobs, score them \
         against the user's preferences, optimize resumes for high-scoring jobs, create \
         applications, and send notifications. The pipeline runs in the background."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(&self, _params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        match self.service.run().await {
            Ok(()) => Ok(json!({
                "status": "started",
                "message": "Job pipeline run started in the background."
            })),
            Err(e) => Ok(json!({
                "status": "error",
                "message": format!("{e}")
            })),
        }
    }
}

// ---------------------------------------------------------------------------
// CancelJobPipelineTool
// ---------------------------------------------------------------------------

/// Tool that lets the main rara agent cancel a running pipeline.
pub struct CancelJobPipelineTool {
    service: PipelineService,
}

impl CancelJobPipelineTool {
    pub fn new(service: PipelineService) -> Self { Self { service } }
}

#[async_trait]
impl AgentTool for CancelJobPipelineTool {
    fn name(&self) -> &str { "cancel_job_pipeline" }

    fn description(&self) -> &str {
        "Cancel a running job pipeline. The pipeline will stop after the current tool call \
         completes."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(&self, _params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        match self.service.cancel() {
            Ok(()) => Ok(json!({
                "status": "cancelling",
                "message": "Pipeline cancellation requested."
            })),
            Err(e) => Ok(json!({
                "status": "error",
                "message": format!("{e}")
            })),
        }
    }
}

// ---------------------------------------------------------------------------
// PipelineStatusTool
// ---------------------------------------------------------------------------

/// Tool that lets the main rara agent check pipeline status.
pub struct PipelineStatusTool {
    service: PipelineService,
}

impl PipelineStatusTool {
    pub fn new(service: PipelineService) -> Self { Self { service } }
}

#[async_trait]
impl AgentTool for PipelineStatusTool {
    fn name(&self) -> &str { "pipeline_status" }

    fn description(&self) -> &str { "Check if the job pipeline is currently running." }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(&self, _params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        Ok(json!({
            "running": self.service.is_running()
        }))
    }
}

// ---------------------------------------------------------------------------
// UpdateJobPreferencesTool
// ---------------------------------------------------------------------------

/// Tool that lets the main rara agent update job preferences.
pub struct UpdateJobPreferencesTool {
    settings_svc: SettingsSvc,
}

impl UpdateJobPreferencesTool {
    pub fn new(settings_svc: SettingsSvc) -> Self { Self { settings_svc } }
}

#[async_trait]
impl AgentTool for UpdateJobPreferencesTool {
    fn name(&self) -> &str { "update_job_preferences" }

    fn description(&self) -> &str {
        "Update the user's job search preferences. The preferences are stored as markdown \
         describing target roles, tech stack, location, salary expectations, company preferences, \
         etc. These preferences are used by the job pipeline to search and score jobs."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "job_preferences": {
                    "type": "string",
                    "description": "Markdown text describing the user's job search preferences: target roles, tech stack, location, salary range, company types, etc."
                }
            },
            "required": ["job_preferences"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let prefs = params
            .get("job_preferences")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned();

        if prefs.trim().is_empty() {
            return Ok(json!({
                "status": "error",
                "message": "job_preferences cannot be empty"
            }));
        }

        self.settings_svc
            .update(UpdateRequest {
                ai:           None,
                telegram:     None,
                agent:        None,
                job_pipeline: Some(JobPipelineRuntimeSettingsPatch {
                    job_preferences:        Some(prefs.clone()),
                    score_threshold_auto:   None,
                    score_threshold_notify: None,
                    resume_project_path:    None,
                    pipeline_cron:          None,
                }),
                workers:      None,
            })
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        Ok(json!({
            "status": "ok",
            "message": "Job preferences updated successfully.",
            "job_preferences": prefs
        }))
    }
}
