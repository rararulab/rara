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

//! Job Pipeline Agent service.
//!
//! [`PipelineService`] wraps a specialized agent that automates the job
//! discovery and application pipeline: search -> score -> optimize resume
//! -> create application -> email -> notify.
//!
//! The service provides `run()` / `cancel()` / `is_running()` methods and
//! is wired into HTTP routes and a rara chat tool.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use axum::http::StatusCode;
use rara_agents::{
    provider::LlmProviderLoaderRef,
    runner::{AgentRunner, UserContent},
    tool_registry::ToolRegistry,
};
use rara_domain_shared::settings::{SettingsSvc, model::ModelScenario};
use snafu::Snafu;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use crate::tools::pipeline_tools;

/// Default pipeline system prompt embedded into the binary.
const DEFAULT_PIPELINE_PROMPT: &str = include_str!("prompt.md");

/// Maximum agent loop iterations per pipeline run.
const PIPELINE_MAX_ITERATIONS: usize = 25;

/// The user message sent to the pipeline agent to kick off a run.
const PIPELINE_KICK_MESSAGE: &str =
    "Execute the job pipeline: read preferences, search for matching jobs, \
     score them against my preferences, and process high-scoring ones \
     (optimize resume, create applications, send notifications). \
     Provide a summary at the end.";

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Snafu)]
pub enum PipelineError {
    #[snafu(display("pipeline is already running"))]
    AlreadyRunning,

    #[snafu(display("pipeline is not running"))]
    NotRunning,

    #[snafu(display("AI is not configured"))]
    AiNotConfigured,

    #[snafu(display("pipeline run failed: {message}"))]
    RunFailed { message: String },
}

impl axum::response::IntoResponse for PipelineError {
    fn into_response(self) -> axum::response::Response {
        let status = match &self {
            Self::AlreadyRunning | Self::NotRunning => StatusCode::CONFLICT,
            Self::AiNotConfigured => StatusCode::PRECONDITION_FAILED,
            Self::RunFailed { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, self.to_string()).into_response()
    }
}

// ---------------------------------------------------------------------------
// PipelineService
// ---------------------------------------------------------------------------

/// Service that manages the job pipeline agent lifecycle.
///
/// The pipeline agent is a specialized `AgentRunner` with its own system
/// prompt and tool set, focused entirely on the job discovery + application
/// automation workflow.
#[derive(Clone)]
pub struct PipelineService {
    settings_svc:  SettingsSvc,
    llm_provider:  LlmProviderLoaderRef,
    ai_service:    rara_ai::service::AiService,
    job_service:   rara_domain_job::service::JobService,
    pool:          sqlx::PgPool,
    notify_client: rara_domain_shared::notify::client::NotifyClient,
    composio_auth: Arc<dyn rara_composio::ComposioAuthProvider>,

    /// Whether a pipeline run is currently in progress.
    running:       Arc<AtomicBool>,
    /// Cancel flag: set to true to signal the running pipeline to stop.
    cancel_flag:   Arc<AtomicBool>,
    /// Mutex to serialize concurrent `run()` attempts.
    run_lock:      Arc<Mutex<()>>,
}

impl PipelineService {
    /// Create a new pipeline service from application dependencies.
    pub fn new(
        settings_svc: SettingsSvc,
        llm_provider: LlmProviderLoaderRef,
        ai_service: rara_ai::service::AiService,
        job_service: rara_domain_job::service::JobService,
        pool: sqlx::PgPool,
        notify_client: rara_domain_shared::notify::client::NotifyClient,
        composio_auth: Arc<dyn rara_composio::ComposioAuthProvider>,
    ) -> Self {
        Self {
            settings_svc,
            llm_provider,
            ai_service,
            job_service,
            pool,
            notify_client,
            composio_auth,
            running: Arc::new(AtomicBool::new(false)),
            cancel_flag: Arc::new(AtomicBool::new(false)),
            run_lock: Arc::new(Mutex::new(())),
        }
    }

    /// Trigger a pipeline run. Returns immediately after spawning the
    /// background task (fire-and-forget).
    ///
    /// Returns `Err(PipelineError::AlreadyRunning)` if a run is in progress.
    pub async fn run(&self) -> Result<(), PipelineError> {
        // Quick atomic check (non-blocking).
        if self.running.load(Ordering::SeqCst) {
            return Err(PipelineError::AlreadyRunning);
        }

        // Acquire the mutex to serialize concurrent callers.
        let _guard = self.run_lock.lock().await;
        if self.running.load(Ordering::SeqCst) {
            return Err(PipelineError::AlreadyRunning);
        }

        // Guard: AI must be configured.
        let settings = self.settings_svc.current();
        if settings.ai.openrouter_api_key.is_none() {
            return Err(PipelineError::AiNotConfigured);
        }

        self.running.store(true, Ordering::SeqCst);
        self.cancel_flag.store(false, Ordering::SeqCst);

        let svc = self.clone();
        tokio::spawn(async move {
            if let Err(e) = svc.run_inner().await {
                error!(error = %e, "pipeline run failed");
            }
            svc.running.store(false, Ordering::SeqCst);
        });

        Ok(())
    }

    /// Cancel a running pipeline.
    pub fn cancel(&self) -> Result<(), PipelineError> {
        if !self.running.load(Ordering::SeqCst) {
            return Err(PipelineError::NotRunning);
        }
        self.cancel_flag.store(true, Ordering::SeqCst);
        info!("pipeline cancellation requested");
        Ok(())
    }

    /// Check if the pipeline is currently running.
    pub fn is_running(&self) -> bool { self.running.load(Ordering::SeqCst) }

    // -----------------------------------------------------------------------
    // Internal
    // -----------------------------------------------------------------------

    /// The core pipeline execution logic.
    async fn run_inner(&self) -> Result<(), PipelineError> {
        info!("pipeline run started");

        let settings = self.settings_svc.current();
        let model = settings.ai.model_for(ModelScenario::Job).to_owned();

        // Build pipeline-specific tool registry.
        let tools = self.build_pipeline_tools();

        let system_prompt = DEFAULT_PIPELINE_PROMPT.to_owned();

        // Build and run the agent.
        let runner = AgentRunner::builder()
            .llm_provider(self.llm_provider.clone())
            .model_name(model)
            .system_prompt(system_prompt)
            .user_content(UserContent::Text(PIPELINE_KICK_MESSAGE.to_owned()))
            .max_iterations(PIPELINE_MAX_ITERATIONS)
            .build();

        match runner.run(&tools, None).await {
            Ok(result) => {
                let response_text = result
                    .provider_response
                    .choices
                    .first()
                    .and_then(|c| c.message.content.as_deref())
                    .unwrap_or_default();

                info!(
                    iterations = result.iterations,
                    tool_calls = result.tool_calls_made,
                    response_len = response_text.len(),
                    "pipeline run completed"
                );

                Ok(())
            }
            Err(e) => {
                warn!(error = %e, "pipeline agent run failed");
                Err(PipelineError::RunFailed {
                    message: e.to_string(),
                })
            }
        }
    }

    /// Build the tool registry for the pipeline agent.
    ///
    /// Includes:
    /// - Standard primitive tools (db_query, db_mutate, notify, send_email, etc.)
    /// - Pipeline-specific tools (get_job_preferences, score_job, optimize_resume)
    /// - The existing job_pipeline tool (save job URL)
    fn build_pipeline_tools(&self) -> ToolRegistry {
        let mut registry = ToolRegistry::new();

        // Layer 1: Primitive tools (db, notify, email, storage, etc.)
        let deps = tool_core::PrimitiveDeps {
            pool:                   self.pool.clone(),
            notify_client:          self.notify_client.clone(),
            settings_svc:           self.settings_svc.clone(),
            object_store:           opendal::Operator::new(opendal::services::Memory::default())
                .expect("memory operator")
                .finish(),
            composio_auth_provider: self.composio_auth.clone(),
        };
        for tool in tool_core::default_primitives(deps) {
            registry.register_primitive(tool);
        }

        // Layer 2: Pipeline-specific tools
        registry.register_service(Arc::new(pipeline_tools::GetJobPreferencesTool::new(
            self.settings_svc.clone(),
        )));
        registry.register_service(Arc::new(pipeline_tools::ScoreJobTool::new(
            self.ai_service.clone(),
            self.settings_svc.clone(),
        )));
        // Resume optimization sub-tools (worktree-based workflow)
        registry.register_service(Arc::new(pipeline_tools::PrepareResumeWorktreeTool::new(
            self.settings_svc.clone(),
        )));
        registry.register_service(Arc::new(pipeline_tools::ReadResumeFileTool::new()));
        registry.register_service(Arc::new(pipeline_tools::WriteResumeFileTool::new()));
        registry.register_service(Arc::new(pipeline_tools::RenderResumeTool::new()));
        registry.register_service(Arc::new(pipeline_tools::FinalizeResumeTool::new()));

        // Re-use the existing job_pipeline tool (save job URL).
        registry.register_service(Arc::new(
            crate::tools::job_pipeline_tool::JobPipelineTool::new(self.job_service.clone()),
        ));

        registry
    }
}
