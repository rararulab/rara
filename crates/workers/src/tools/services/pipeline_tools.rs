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

//! Pipeline-specific tools for the Job Pipeline Agent.
//!
//! These tools are registered on the pipeline agent's tool registry.
//! The pipeline agent also has access to standard primitive tools
//! (db_query, db_mutate, notify, send_email, etc.).

use async_trait::async_trait;
use serde_json::json;
use tool_core::AgentTool;

// ---------------------------------------------------------------------------
// GetJobPreferencesTool
// ---------------------------------------------------------------------------

/// Reads job pipeline preferences from runtime settings.
pub struct GetJobPreferencesTool {
    settings_svc: rara_domain_shared::settings::SettingsSvc,
}

impl GetJobPreferencesTool {
    pub fn new(settings_svc: rara_domain_shared::settings::SettingsSvc) -> Self {
        Self { settings_svc }
    }
}

#[async_trait]
impl AgentTool for GetJobPreferencesTool {
    fn name(&self) -> &str { "get_job_preferences" }

    fn description(&self) -> &str {
        "Read the user's job pipeline preferences including target roles, tech stack, score \
         thresholds, and resume project path. Always call this first before any pipeline work."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(&self, _params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let settings = self.settings_svc.current();
        let jp = &settings.job_pipeline;

        Ok(json!({
            "job_preferences": jp.job_preferences,
            "score_threshold_auto": jp.score_threshold_auto,
            "score_threshold_notify": jp.score_threshold_notify,
            "resume_project_path": jp.resume_project_path,
            "gmail_configured": settings.agent.gmail.address.is_some()
                && settings.agent.gmail.app_password.is_some(),
            "auto_send_enabled": settings.agent.gmail.auto_send_enabled,
            "telegram_configured": settings.telegram.chat_id.is_some(),
        }))
    }
}

// ---------------------------------------------------------------------------
// ScoreJobTool
// ---------------------------------------------------------------------------

/// Scores a job against the user's preferences using AI evaluation.
pub struct ScoreJobTool {
    ai_service:   rara_ai::service::AiService,
    settings_svc: rara_domain_shared::settings::SettingsSvc,
}

impl ScoreJobTool {
    pub fn new(
        ai_service: rara_ai::service::AiService,
        settings_svc: rara_domain_shared::settings::SettingsSvc,
    ) -> Self {
        Self {
            ai_service,
            settings_svc,
        }
    }
}

#[async_trait]
impl AgentTool for ScoreJobTool {
    fn name(&self) -> &str { "score_job" }

    fn description(&self) -> &str {
        "Score a job posting (0-100) against the user's preferences. Returns a score and brief \
         assessment of fit including strengths and gaps."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "job_title": {
                    "type": "string",
                    "description": "Job title"
                },
                "company": {
                    "type": "string",
                    "description": "Company name"
                },
                "job_description": {
                    "type": "string",
                    "description": "Full job description text"
                }
            },
            "required": ["job_title", "job_description"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let job_title = params
            .get("job_title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: job_title"))?;

        let job_description = params
            .get("job_description")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: job_description"))?;

        let company = params
            .get("company")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown");

        let settings = self.settings_svc.current();
        let preferences = settings
            .job_pipeline
            .job_preferences
            .as_deref()
            .unwrap_or("No specific preferences configured.");

        // Use the candidate's preferences as a pseudo-resume for the fit
        // scoring agent. The preferences text typically contains target roles,
        // tech stack, and experience level -- close enough for scoring.
        let jd = format!(
            "**Title**: {job_title}\n**Company**: {company}\n\n{job_description}"
        );
        let resume_proxy = format!(
            "## Candidate Profile / Preferences\n\n{preferences}\n\n\
             (Note: score the JOB against these PREFERENCES. Respond with ONLY a \
             JSON object containing: score (0-100), summary, strengths[], gaps[].)"
        );

        let agent = match self.ai_service.job_fit() {
            Ok(agent) => agent,
            Err(e) => return Ok(json!({ "error": format!("AI not configured: {e}") })),
        };

        match agent.analyze(&jd, &resume_proxy).await {
            Ok(response) => {
                // Try to parse as JSON; if it fails, wrap the raw text.
                match serde_json::from_str::<serde_json::Value>(&response) {
                    Ok(parsed) => Ok(parsed),
                    Err(_) => Ok(json!({
                        "raw_response": response,
                        "error": "Failed to parse AI response as JSON"
                    })),
                }
            }
            Err(e) => Ok(json!({ "error": format!("AI scoring failed: {e}") })),
        }
    }
}

// ---------------------------------------------------------------------------
// OptimizeResumeTool (skeleton)
// ---------------------------------------------------------------------------

/// Skeleton tool for resume optimization.
///
/// TODO: Full implementation involves:
/// 1. Creating a git worktree from the resume project
/// 2. Using AI to tailor the resume content for the specific job
/// 3. Compiling the Typst resume to PDF
/// 4. Returning the path to the optimized PDF
///
/// For now, this returns a placeholder indicating the tool is not yet
/// fully implemented.
pub struct OptimizeResumeTool {
    settings_svc: rara_domain_shared::settings::SettingsSvc,
    _ai_service:  rara_ai::service::AiService,
}

impl OptimizeResumeTool {
    pub fn new(
        settings_svc: rara_domain_shared::settings::SettingsSvc,
        ai_service: rara_ai::service::AiService,
    ) -> Self {
        Self {
            settings_svc,
            _ai_service: ai_service,
        }
    }
}

#[async_trait]
impl AgentTool for OptimizeResumeTool {
    fn name(&self) -> &str { "optimize_resume" }

    fn description(&self) -> &str {
        "Optimize the user's resume for a specific job posting. Creates a tailored version \
         of the resume emphasizing relevant skills and experience. Returns the path to the \
         optimized resume PDF. NOTE: This tool is a skeleton; full implementation pending."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "job_title": {
                    "type": "string",
                    "description": "The job title to optimize for"
                },
                "company": {
                    "type": "string",
                    "description": "The company name"
                },
                "job_description": {
                    "type": "string",
                    "description": "The full job description to tailor the resume against"
                }
            },
            "required": ["job_title", "job_description"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let job_title = params
            .get("job_title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: job_title"))?;

        let company = params
            .get("company")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown");

        let settings = self.settings_svc.current();
        let resume_path = settings.job_pipeline.resume_project_path.as_deref();

        match resume_path {
            Some(path) => {
                // TODO: Implement full resume optimization pipeline:
                // 1. git worktree add .worktrees/{company}-{timestamp} -b temp/resume-{company}
                // 2. Read current resume content from the worktree
                // 3. Use AI to generate tailored content
                // 4. Write optimized content to the worktree
                // 5. Run `typst compile` or `make` to produce PDF
                // 6. Return the PDF path
                // 7. Clean up worktree after PDF is used

                tracing::info!(
                    job_title = job_title,
                    company = company,
                    resume_path = path,
                    "resume optimization requested (skeleton -- not yet implemented)"
                );

                Ok(json!({
                    "status": "skeleton",
                    "message": format!(
                        "Resume optimization for '{job_title}' at {company} is not yet \
                         fully implemented. Resume project path: {path}"
                    ),
                    "resume_project_path": path,
                }))
            }
            None => Ok(json!({
                "error": "resume_project_path is not configured in job_pipeline settings"
            })),
        }
    }
}
