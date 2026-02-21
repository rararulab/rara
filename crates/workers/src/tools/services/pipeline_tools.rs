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
// PrepareResumeWorktreeTool
// ---------------------------------------------------------------------------

/// Creates a git worktree in the resume project for a specific job application.
///
/// The worktree is created at `{resume_project_path}/.worktrees/apply-{company}-{role}`
/// on branch `apply/{company}-{role}`. Returns the worktree path and a recursive
/// file listing so the agent knows what files are available.
pub struct PrepareResumeWorktreeTool {
    settings_svc: rara_domain_shared::settings::SettingsSvc,
}

impl PrepareResumeWorktreeTool {
    pub fn new(settings_svc: rara_domain_shared::settings::SettingsSvc) -> Self {
        Self { settings_svc }
    }
}

#[async_trait]
impl AgentTool for PrepareResumeWorktreeTool {
    fn name(&self) -> &str { "prepare_resume_worktree" }

    fn description(&self) -> &str {
        "Create a git worktree in the resume project for a specific job application. \
         Returns the worktree path and a recursive file listing of all files in the \
         worktree so you know what data files are available to modify."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "company": {
                    "type": "string",
                    "description": "Company name (used in branch name, will be slugified)"
                },
                "role": {
                    "type": "string",
                    "description": "Role/job title (used in branch name, will be slugified)"
                }
            },
            "required": ["company", "role"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let company = params
            .get("company")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: company"))?;

        let role = params
            .get("role")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: role"))?;

        let settings = self.settings_svc.current();
        let resume_path = match settings.job_pipeline.resume_project_path.as_deref() {
            Some(p) => p.to_owned(),
            None => {
                return Ok(json!({
                    "error": "resume_project_path is not configured in job_pipeline settings. \
                              Please set it via the Settings page first."
                }));
            }
        };

        // Validate the resume project path exists and is a git repo.
        let resume_dir = std::path::Path::new(&resume_path);
        if !resume_dir.is_dir() {
            return Ok(json!({
                "error": format!("resume_project_path does not exist or is not a directory: {resume_path}")
            }));
        }

        let slug_company = slugify(company);
        let slug_role = slugify(role);
        let branch_name = format!("apply/{slug_company}-{slug_role}");
        let worktree_dir_name = format!("apply-{slug_company}-{slug_role}");
        let worktree_path = resume_dir
            .join(".worktrees")
            .join(&worktree_dir_name);

        // Create .worktrees directory if it doesn't exist.
        if let Err(e) = std::fs::create_dir_all(resume_dir.join(".worktrees")) {
            return Ok(json!({
                "error": format!("failed to create .worktrees directory: {e}")
            }));
        }

        // If worktree already exists, just return the file listing.
        if worktree_path.is_dir() {
            tracing::info!(
                worktree = %worktree_path.display(),
                "resume worktree already exists, reusing"
            );
            let files = list_files_recursive(&worktree_path);
            return Ok(json!({
                "worktree_path": worktree_path.display().to_string(),
                "branch": branch_name,
                "files": files,
                "status": "reused_existing",
            }));
        }

        // Create the git worktree.
        let output = tokio::process::Command::new("git")
            .args(["worktree", "add", worktree_path.to_str().unwrap_or_default(), "-b", &branch_name])
            .current_dir(&resume_path)
            .output()
            .await;

        match output {
            Ok(out) if out.status.success() => {
                tracing::info!(
                    worktree = %worktree_path.display(),
                    branch = %branch_name,
                    "resume worktree created"
                );
                let files = list_files_recursive(&worktree_path);
                Ok(json!({
                    "worktree_path": worktree_path.display().to_string(),
                    "branch": branch_name,
                    "files": files,
                    "status": "created",
                }))
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                // If branch already exists, try without -b (just add worktree on existing branch).
                if stderr.contains("already exists") {
                    let retry = tokio::process::Command::new("git")
                        .args(["worktree", "add", worktree_path.to_str().unwrap_or_default(), &branch_name])
                        .current_dir(&resume_path)
                        .output()
                        .await;

                    match retry {
                        Ok(r) if r.status.success() => {
                            let files = list_files_recursive(&worktree_path);
                            Ok(json!({
                                "worktree_path": worktree_path.display().to_string(),
                                "branch": branch_name,
                                "files": files,
                                "status": "created_from_existing_branch",
                            }))
                        }
                        Ok(r) => Ok(json!({
                            "error": format!(
                                "git worktree add failed (retry): {}",
                                String::from_utf8_lossy(&r.stderr)
                            )
                        })),
                        Err(e) => Ok(json!({
                            "error": format!("failed to run git: {e}")
                        })),
                    }
                } else {
                    Ok(json!({
                        "error": format!("git worktree add failed: {stderr}")
                    }))
                }
            }
            Err(e) => Ok(json!({
                "error": format!("failed to run git: {e}")
            })),
        }
    }
}

// ---------------------------------------------------------------------------
// ReadResumeFileTool
// ---------------------------------------------------------------------------

/// Reads a file from a resume worktree.
pub struct ReadResumeFileTool;

impl ReadResumeFileTool {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl AgentTool for ReadResumeFileTool {
    fn name(&self) -> &str { "read_resume_file" }

    fn description(&self) -> &str {
        "Read the content of a file from a resume worktree. Use this to inspect \
         data files before modifying them."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "worktree_path": {
                    "type": "string",
                    "description": "Absolute path to the worktree (returned by prepare_resume_worktree)"
                },
                "file_path": {
                    "type": "string",
                    "description": "Relative file path within the worktree (e.g. \"data/experience.typ\")"
                }
            },
            "required": ["worktree_path", "file_path"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let worktree_path = params
            .get("worktree_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: worktree_path"))?;

        let file_path = params
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: file_path"))?;

        let worktree = std::path::Path::new(worktree_path);
        let full_path = worktree.join(file_path);

        // Security: validate the resolved path is within the worktree.
        if let Err(e) = validate_path_within(worktree, &full_path) {
            return Ok(json!({ "error": e }));
        }

        match std::fs::read_to_string(&full_path) {
            Ok(content) => Ok(json!({
                "file_path": file_path,
                "content": content,
            })),
            Err(e) => Ok(json!({
                "error": format!("failed to read file '{}': {e}", full_path.display())
            })),
        }
    }
}

// ---------------------------------------------------------------------------
// WriteResumeFileTool
// ---------------------------------------------------------------------------

/// Writes or modifies a file in a resume worktree.
pub struct WriteResumeFileTool;

impl WriteResumeFileTool {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl AgentTool for WriteResumeFileTool {
    fn name(&self) -> &str { "write_resume_file" }

    fn description(&self) -> &str {
        "Write content to a file in the resume worktree. Use this to modify \
         data files (e.g. experience, skills, summary) to tailor the resume \
         for a specific job."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "worktree_path": {
                    "type": "string",
                    "description": "Absolute path to the worktree (returned by prepare_resume_worktree)"
                },
                "file_path": {
                    "type": "string",
                    "description": "Relative file path within the worktree (e.g. \"data/experience.typ\")"
                },
                "content": {
                    "type": "string",
                    "description": "The new content to write to the file"
                }
            },
            "required": ["worktree_path", "file_path", "content"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let worktree_path = params
            .get("worktree_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: worktree_path"))?;

        let file_path = params
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: file_path"))?;

        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: content"))?;

        let worktree = std::path::Path::new(worktree_path);
        let full_path = worktree.join(file_path);

        // Security: validate the resolved path is within the worktree.
        if let Err(e) = validate_path_within(worktree, &full_path) {
            return Ok(json!({ "error": e }));
        }

        // Ensure parent directory exists.
        if let Some(parent) = full_path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return Ok(json!({
                    "error": format!("failed to create parent directory: {e}")
                }));
            }
        }

        match std::fs::write(&full_path, content) {
            Ok(()) => {
                tracing::debug!(
                    file = %full_path.display(),
                    bytes = content.len(),
                    "resume file written"
                );
                Ok(json!({
                    "file_path": file_path,
                    "bytes_written": content.len(),
                    "message": "file written successfully",
                }))
            }
            Err(e) => Ok(json!({
                "error": format!("failed to write file '{}': {e}", full_path.display())
            })),
        }
    }
}

// ---------------------------------------------------------------------------
// RenderResumeTool
// ---------------------------------------------------------------------------

/// Runs `make` in the resume worktree to produce a PDF.
pub struct RenderResumeTool;

impl RenderResumeTool {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl AgentTool for RenderResumeTool {
    fn name(&self) -> &str { "render_resume" }

    fn description(&self) -> &str {
        "Run `make` in the resume worktree to compile the resume into a PDF. \
         Returns the path to the generated PDF file."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "worktree_path": {
                    "type": "string",
                    "description": "Absolute path to the worktree (returned by prepare_resume_worktree)"
                }
            },
            "required": ["worktree_path"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let worktree_path = params
            .get("worktree_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: worktree_path"))?;

        let worktree = std::path::Path::new(worktree_path);
        if !worktree.is_dir() {
            return Ok(json!({
                "error": format!("worktree directory does not exist: {worktree_path}")
            }));
        }

        // Check that a Makefile exists.
        if !worktree.join("Makefile").is_file() && !worktree.join("makefile").is_file() {
            return Ok(json!({
                "error": "no Makefile found in the worktree directory"
            }));
        }

        // Run `make`.
        tracing::info!(worktree = %worktree_path, "running make in resume worktree");
        let output = tokio::process::Command::new("make")
            .current_dir(worktree)
            .output()
            .await;

        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);

                if !out.status.success() {
                    return Ok(json!({
                        "error": "make failed",
                        "exit_code": out.status.code(),
                        "stdout": stdout.to_string(),
                        "stderr": stderr.to_string(),
                    }));
                }

                // Find PDF files in the worktree.
                let pdf_files = find_pdf_files(worktree);

                if pdf_files.is_empty() {
                    Ok(json!({
                        "status": "make_succeeded_but_no_pdf",
                        "message": "make completed successfully but no PDF files were found in the worktree",
                        "stdout": stdout.to_string(),
                        "stderr": stderr.to_string(),
                    }))
                } else {
                    tracing::info!(
                        pdf_count = pdf_files.len(),
                        pdf_path = %pdf_files[0],
                        "resume PDF generated"
                    );
                    Ok(json!({
                        "status": "success",
                        "pdf_path": pdf_files[0],
                        "all_pdfs": pdf_files,
                        "stdout": stdout.to_string(),
                    }))
                }
            }
            Err(e) => Ok(json!({
                "error": format!("failed to run make: {e}")
            })),
        }
    }
}

// ---------------------------------------------------------------------------
// FinalizeResumeTool
// ---------------------------------------------------------------------------

/// Commits all changes in the resume worktree and returns the PDF path.
pub struct FinalizeResumeTool;

impl FinalizeResumeTool {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl AgentTool for FinalizeResumeTool {
    fn name(&self) -> &str { "finalize_resume" }

    fn description(&self) -> &str {
        "Commit all changes in the resume worktree and finalize the optimized resume. \
         Returns the PDF path for use as an email attachment."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "worktree_path": {
                    "type": "string",
                    "description": "Absolute path to the worktree"
                },
                "company": {
                    "type": "string",
                    "description": "Company name (for commit message)"
                },
                "role": {
                    "type": "string",
                    "description": "Role/job title (for commit message)"
                },
                "pdf_path": {
                    "type": "string",
                    "description": "Path to the generated PDF (returned by render_resume)"
                }
            },
            "required": ["worktree_path", "company", "role", "pdf_path"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let worktree_path = params
            .get("worktree_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: worktree_path"))?;

        let company = params
            .get("company")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: company"))?;

        let role = params
            .get("role")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: role"))?;

        let pdf_path = params
            .get("pdf_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: pdf_path"))?;

        let worktree = std::path::Path::new(worktree_path);
        if !worktree.is_dir() {
            return Ok(json!({
                "error": format!("worktree directory does not exist: {worktree_path}")
            }));
        }

        // git add .
        let add_output = tokio::process::Command::new("git")
            .args(["add", "."])
            .current_dir(worktree)
            .output()
            .await;

        if let Err(e) = &add_output {
            return Ok(json!({
                "error": format!("failed to run git add: {e}")
            }));
        }
        let add_out = add_output.unwrap();
        if !add_out.status.success() {
            return Ok(json!({
                "error": format!(
                    "git add failed: {}",
                    String::from_utf8_lossy(&add_out.stderr)
                )
            }));
        }

        // git commit
        let commit_msg = format!("apply: {company} - {role}");
        let commit_output = tokio::process::Command::new("git")
            .args(["commit", "-m", &commit_msg])
            .current_dir(worktree)
            .output()
            .await;

        match commit_output {
            Ok(out) if out.status.success() => {
                tracing::info!(
                    worktree = %worktree_path,
                    message = %commit_msg,
                    "resume changes committed"
                );
                Ok(json!({
                    "status": "success",
                    "commit_message": commit_msg,
                    "pdf_path": pdf_path,
                    "message": format!(
                        "Resume for {company} ({role}) finalized. PDF at: {pdf_path}"
                    ),
                }))
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                // "nothing to commit" is not really an error.
                if stderr.contains("nothing to commit") {
                    Ok(json!({
                        "status": "success",
                        "commit_message": commit_msg,
                        "pdf_path": pdf_path,
                        "message": "No changes to commit (files may already be committed).",
                    }))
                } else {
                    Ok(json!({
                        "error": format!("git commit failed: {stderr}")
                    }))
                }
            }
            Err(e) => Ok(json!({
                "error": format!("failed to run git commit: {e}")
            })),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert a string to a URL/branch-safe slug.
fn slugify(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|seg| !seg.is_empty())
        .collect::<Vec<&str>>()
        .join("-")
}

/// Validate that `target` resolves to a path within `base_dir`.
///
/// Returns `Err(message)` if the path escapes the base directory (e.g. via
/// `..` components).
fn validate_path_within(
    base_dir: &std::path::Path,
    target: &std::path::Path,
) -> Result<(), String> {
    // Canonicalize base_dir (must exist).
    let canon_base = base_dir
        .canonicalize()
        .map_err(|e| format!("cannot resolve base directory '{}': {e}", base_dir.display()))?;

    // For the target, canonicalize if it exists; otherwise canonicalize the
    // parent and append the filename.
    let canon_target = if target.exists() {
        target
            .canonicalize()
            .map_err(|e| format!("cannot resolve target path '{}': {e}", target.display()))?
    } else {
        // Target doesn't exist yet (write case). Canonicalize parent.
        let parent = target.parent().ok_or_else(|| {
            format!("target path has no parent: {}", target.display())
        })?;
        let parent_canon = parent.canonicalize().map_err(|e| {
            format!(
                "cannot resolve parent directory '{}': {e}",
                parent.display()
            )
        })?;
        let file_name = target.file_name().ok_or_else(|| {
            format!("target path has no file name: {}", target.display())
        })?;
        parent_canon.join(file_name)
    };

    if !canon_target.starts_with(&canon_base) {
        return Err(format!(
            "path traversal detected: '{}' is outside the worktree '{}'",
            target.display(),
            base_dir.display()
        ));
    }

    Ok(())
}

/// Recursively list all files under `dir`, skipping `.git` directories.
/// Returns relative paths as strings.
fn list_files_recursive(dir: &std::path::Path) -> Vec<String> {
    let mut files = Vec::new();
    list_files_recursive_inner(dir, dir, &mut files);
    files.sort();
    files
}

fn list_files_recursive_inner(
    base: &std::path::Path,
    current: &std::path::Path,
    out: &mut Vec<String>,
) {
    let entries = match std::fs::read_dir(current) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // Skip .git directories.
        if path
            .file_name()
            .map_or(false, |n| n == ".git" || n == ".worktrees")
        {
            continue;
        }
        if path.is_dir() {
            list_files_recursive_inner(base, &path, out);
        } else if let Ok(rel) = path.strip_prefix(base) {
            out.push(rel.display().to_string());
        }
    }
}

/// Find all `.pdf` files in a directory (non-recursive top-level + one level
/// deep for common output directories like `build/` or `out/`).
fn find_pdf_files(dir: &std::path::Path) -> Vec<String> {
    let mut pdfs = Vec::new();
    find_pdf_files_recursive(dir, dir, &mut pdfs, 0, 2);
    // Sort so the most recently modified PDF comes first.
    pdfs.sort_by(|a, b| {
        let ma = std::fs::metadata(a)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let mb = std::fs::metadata(b)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        mb.cmp(&ma) // newest first
    });
    pdfs
}

fn find_pdf_files_recursive(
    _base: &std::path::Path,
    current: &std::path::Path,
    out: &mut Vec<String>,
    depth: usize,
    max_depth: usize,
) {
    if depth > max_depth {
        return;
    }
    let entries = match std::fs::read_dir(current) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .file_name()
            .map_or(false, |n| n == ".git" || n == ".worktrees")
        {
            continue;
        }
        if path.is_dir() {
            find_pdf_files_recursive(_base, &path, out, depth + 1, max_depth);
        } else if path.extension().map_or(false, |ext| ext == "pdf") {
            out.push(path.display().to_string());
        }
    }
}
