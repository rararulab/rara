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

//! Layer 2 service tools for resume management and analysis.
//!
//! - [`ListResumesTool`]: list all resumes with optional source filter.
//! - [`GetResumeContentTool`]: retrieve full content of a resume by ID.
//! - [`AnalyzeResumeTool`]: AI-driven resume analysis with optimization
//!   suggestions.

use async_trait::async_trait;
use rara_agents::tool_registry::AgentTool;
use serde_json::json;
use uuid::Uuid;

const ANALYZE_PROMPT_FILE: &str = "workers/resume_analysis_instructions.md";
const DEFAULT_ANALYZE_PROMPT_INSTRUCTIONS: &str =
    include_str!("../../../../../prompts/workers/resume_analysis_instructions.md");

// ---------------------------------------------------------------------------
// list_resumes
// ---------------------------------------------------------------------------

/// Layer 2 service tool: list all resumes, optionally filtered by source.
pub struct ListResumesTool {
    resume_service: rara_domain_resume::ResumeAppService,
}

impl ListResumesTool {
    pub fn new(resume_service: rara_domain_resume::ResumeAppService) -> Self {
        Self { resume_service }
    }
}

#[async_trait]
impl AgentTool for ListResumesTool {
    fn name(&self) -> &str { "list_resumes" }

    fn description(&self) -> &str { "List all resumes. Optionally filter by source type." }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "source": {
                    "type": "string",
                    "description": "Filter by source: 'manual', 'ai_generated', or 'optimized'"
                }
            },
            "required": []
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let source_filter = params
            .get("source")
            .and_then(|v| v.as_str())
            .and_then(|s| match s {
                "manual" => Some(rara_domain_resume::types::ResumeSource::Manual),
                "ai_generated" => Some(rara_domain_resume::types::ResumeSource::AiGenerated),
                "optimized" => Some(rara_domain_resume::types::ResumeSource::Optimized),
                _ => None,
            });

        let filter = rara_domain_resume::types::ResumeFilter {
            source: source_filter,
            ..Default::default()
        };

        match self.resume_service.list(filter).await {
            Ok(resumes) => {
                let items: Vec<serde_json::Value> = resumes
                    .iter()
                    .map(|r| {
                        json!({
                            "id": r.id.to_string(),
                            "title": r.title,
                            "source": r.source.to_string(),
                            "tags": r.tags,
                            "target_job_id": r.target_job_id.map(|id| id.to_string()),
                            "parent_resume_id": r.parent_resume_id.map(|id| id.to_string()),
                            "created_at": r.created_at.to_string(),
                            "updated_at": r.updated_at.to_string(),
                        })
                    })
                    .collect();
                Ok(json!({ "resumes": items, "count": items.len() }))
            }
            Err(e) => Ok(json!({ "error": format!("{e}") })),
        }
    }
}

// ---------------------------------------------------------------------------
// get_resume_content
// ---------------------------------------------------------------------------

/// Layer 2 service tool: get the full content of a resume by ID.
pub struct GetResumeContentTool {
    resume_service: rara_domain_resume::ResumeAppService,
}

impl GetResumeContentTool {
    pub fn new(resume_service: rara_domain_resume::ResumeAppService) -> Self {
        Self { resume_service }
    }
}

#[async_trait]
impl AgentTool for GetResumeContentTool {
    fn name(&self) -> &str { "get_resume_content" }

    fn description(&self) -> &str { "Get the full content of a resume by its ID" }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "resume_id": {
                    "type": "string",
                    "description": "The UUID of the resume to retrieve"
                }
            },
            "required": ["resume_id"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let resume_id_str = params
            .get("resume_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: resume_id"))?;

        let resume_id =
            Uuid::parse_str(resume_id_str).map_err(|e| anyhow::anyhow!("invalid UUID: {e}"))?;

        match self.resume_service.get(resume_id).await {
            Ok(Some(resume)) => Ok(json!({
                "id": resume.id.to_string(),
                "title": resume.title,
                "content": resume.content,
                "source": resume.source.to_string(),
                "tags": resume.tags,
                "target_job_id": resume.target_job_id.map(|id| id.to_string()),
                "parent_resume_id": resume.parent_resume_id.map(|id| id.to_string()),
                "customization_notes": resume.customization_notes,
                "version_tag": resume.version_tag,
                "created_at": resume.created_at.to_string(),
                "updated_at": resume.updated_at.to_string(),
            })),
            Ok(None) => Ok(json!({ "error": format!("resume not found: {resume_id}") })),
            Err(e) => Ok(json!({ "error": format!("{e}") })),
        }
    }
}

// ---------------------------------------------------------------------------
// analyze_resume
// ---------------------------------------------------------------------------

/// Layer 2 service tool: AI-driven resume analysis with optimization
/// suggestions.
///
/// Retrieves a resume by ID, optionally fetches the target job description,
/// and returns a structured analysis report covering content completeness,
/// wording quality, structure, ATS compatibility, and job match.
pub struct AnalyzeResumeTool {
    resume_service: rara_domain_resume::ResumeAppService,
    job_service:    rara_domain_job::service::JobService,
    ai_service:     rara_ai::service::AiService,
}

impl AnalyzeResumeTool {
    pub fn new(
        resume_service: rara_domain_resume::ResumeAppService,
        job_service: rara_domain_job::service::JobService,
        ai_service: rara_ai::service::AiService,
    ) -> Self {
        Self {
            resume_service,
            job_service,
            ai_service,
        }
    }

    /// Build the analysis prompt based on resume content and optional job
    /// description.
    fn build_analysis_prompt(resume_content: &str, job_info: Option<&str>) -> String {
        let instructions = rara_paths::load_prompt_markdown(
            ANALYZE_PROMPT_FILE,
            DEFAULT_ANALYZE_PROMPT_INSTRUCTIONS,
        );
        let mut prompt = instructions.trim().to_owned();
        prompt.push('\n');

        if job_info.is_some() {
            prompt.push_str(
                "5. **Job Match**: How well do the candidate's skills and experience match the \
                 target job requirements? What gaps exist?\n",
            );
        }

        prompt.push_str(
            "\nFor each dimension, provide:\n- A score from 1-10\n- Key strengths\n- Specific \
             improvement suggestions\n\nFinally, provide an overall score and a prioritized list \
             of the top 3 improvements the candidate should make.\n\n--- RESUME ---\n",
        );
        prompt.push_str(resume_content);

        if let Some(jd) = job_info {
            prompt.push_str("\n\n--- TARGET JOB DESCRIPTION ---\n");
            prompt.push_str(jd);
        }

        prompt
    }
}

#[async_trait]
impl AgentTool for AnalyzeResumeTool {
    fn name(&self) -> &str { "analyze_resume" }

    fn description(&self) -> &str {
        "Analyze a resume and provide optimization suggestions. Evaluates content completeness, \
         wording quality, structure, ATS compatibility, and job match (if target job is specified)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "resume_id": {
                    "type": "string",
                    "description": "The UUID of the resume to analyze"
                }
            },
            "required": ["resume_id"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        // -- parse resume_id ----------------------------------------------------
        let resume_id_str = params
            .get("resume_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: resume_id"))?;

        let resume_id =
            Uuid::parse_str(resume_id_str).map_err(|e| anyhow::anyhow!("invalid UUID: {e}"))?;

        // -- fetch resume -------------------------------------------------------
        let resume = match self.resume_service.get(resume_id).await {
            Ok(Some(r)) => r,
            Ok(None) => {
                return Ok(json!({ "error": format!("resume not found: {resume_id}") }));
            }
            Err(e) => {
                return Ok(json!({ "error": format!("{e}") }));
            }
        };

        let resume_content = resume.content.as_deref().unwrap_or("");
        if resume_content.trim().is_empty() {
            return Ok(json!({
                "error": "resume has no text content (may be a PDF-only resume)"
            }));
        }

        // -- optionally fetch target job ----------------------------------------
        let job_description = if let Some(job_id) = resume.target_job_id {
            match self.job_service.get(job_id).await {
                Ok(Some(job)) => {
                    // Build a compact job summary from available fields.
                    let mut parts = Vec::new();
                    if let Some(ref title) = job.title {
                        parts.push(format!("Title: {title}"));
                    }
                    if let Some(ref company) = job.company {
                        parts.push(format!("Company: {company}"));
                    }
                    if let Some(ref preview) = job.markdown_preview {
                        parts.push(format!("Description:\n{preview}"));
                    }
                    if parts.is_empty() {
                        None
                    } else {
                        Some(parts.join("\n"))
                    }
                }
                _ => None,
            }
        } else {
            None
        };

        // -- build prompt and call AI -------------------------------------------
        let prompt = Self::build_analysis_prompt(resume_content, job_description.as_deref());

        let agent = match self.ai_service.resume_analyzer() {
            Ok(agent) => agent,
            Err(e) => {
                return Ok(json!({ "error": format!("AI not configured: {e}") }));
            }
        };

        match agent.analyze(&prompt).await {
            Ok(analysis) => Ok(json!({
                "resume_id": resume_id.to_string(),
                "resume_title": resume.title,
                "has_target_job": resume.target_job_id.is_some(),
                "analysis": analysis,
            })),
            Err(e) => Ok(json!({ "error": format!("AI analysis failed: {e}") })),
        }
    }
}
