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
    runner::{AgentRunner, RunnerEvent, UserContent},
    tool_registry::ToolRegistry,
};
use rara_domain_shared::{
    notify::types::{NotificationPriority, SendTelegramNotificationRequest},
    settings::{SettingsSvc, model::ModelScenario},
};
use snafu::Snafu;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use crate::pg_repository::PgPipelineRepository;
use crate::repository::PipelineRepository;
use crate::tools::pipeline_tools;
use crate::types::{PipelineRunStatus, PipelineStreamEvent};

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
    mcp_manager:   rara_mcp::manager::mgr::McpManager,

    /// Whether a pipeline run is currently in progress.
    running:       Arc<AtomicBool>,
    /// Cancel flag: set to true to signal the running pipeline to stop.
    cancel_flag:   Arc<AtomicBool>,
    /// Mutex to serialize concurrent `run()` attempts.
    run_lock:      Arc<Mutex<()>>,
    /// Broadcast channel for streaming pipeline events to SSE clients.
    broadcast_tx:  Arc<tokio::sync::broadcast::Sender<PipelineStreamEvent>>,
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
        mcp_manager: rara_mcp::manager::mgr::McpManager,
    ) -> Self {
        let (broadcast_tx, _) = tokio::sync::broadcast::channel(256);
        Self {
            settings_svc,
            llm_provider,
            ai_service,
            job_service,
            pool,
            notify_client,
            composio_auth,
            mcp_manager,
            running: Arc::new(AtomicBool::new(false)),
            cancel_flag: Arc::new(AtomicBool::new(false)),
            run_lock: Arc::new(Mutex::new(())),
            broadcast_tx: Arc::new(broadcast_tx),
        }
    }

    /// Subscribe to pipeline stream events (for SSE clients).
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<PipelineStreamEvent> {
        self.broadcast_tx.subscribe()
    }

    /// Get a reference to the database pool.
    pub fn pool(&self) -> sqlx::PgPool {
        self.pool.clone()
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

        // 1. Create a persistent pipeline run record.
        let repo = PgPipelineRepository::new(self.pool.clone());
        let mut pipeline_run = repo.create_run().await.map_err(|e| PipelineError::RunFailed {
            message: format!("failed to create pipeline run: {e}"),
        })?;
        let run_id = pipeline_run.id;

        // 2. Broadcast Started event.
        let _ = self.broadcast_tx.send(PipelineStreamEvent::Started { run_id });

        let settings = self.settings_svc.current();
        let model = settings.ai.model_for(ModelScenario::Job).to_owned();

        // Build pipeline-specific tool registry.
        let tools = Arc::new(self.build_pipeline_tools().await);

        let system_prompt = DEFAULT_PIPELINE_PROMPT.to_owned();

        // Build the agent runner and start streaming.
        let runner = AgentRunner::builder()
            .llm_provider(self.llm_provider.clone())
            .model_name(model)
            .system_prompt(system_prompt)
            .user_content(UserContent::Text(PIPELINE_KICK_MESSAGE.to_owned()))
            .max_iterations(PIPELINE_MAX_ITERATIONS)
            .build();

        let mut rx = runner.run_streaming(tools);
        let mut seq: i32 = 0;
        let mut completed = false;

        // 3. Consume streaming events from the agent runner.
        while let Some(runner_event) = rx.recv().await {
            let stream_event = match &runner_event {
                RunnerEvent::Thinking => PipelineStreamEvent::Thinking,
                RunnerEvent::ThinkingDone => PipelineStreamEvent::ThinkingDone,
                RunnerEvent::Iteration(index) => PipelineStreamEvent::Iteration { index: *index },
                RunnerEvent::ToolCallStart { id, name, .. } => {
                    // Strip arguments for SSE clients (may contain sensitive data).
                    PipelineStreamEvent::ToolCallStart {
                        id: id.clone(),
                        name: name.clone(),
                    }
                }
                RunnerEvent::ToolCallEnd {
                    id,
                    name,
                    success,
                    error,
                    ..
                } => {
                    // Strip result for SSE clients.
                    PipelineStreamEvent::ToolCallEnd {
                        id: id.clone(),
                        name: name.clone(),
                        success: *success,
                        error: error.clone(),
                    }
                }
                RunnerEvent::TextDelta(text) => {
                    PipelineStreamEvent::TextDelta { text: text.clone() }
                }
                RunnerEvent::ReasoningDelta(_) => {
                    // Skip reasoning deltas for pipeline events.
                    continue;
                }
                RunnerEvent::Done {
                    text,
                    iterations,
                    tool_calls_made,
                } => {
                    completed = true;

                    // Update pipeline run as completed.
                    pipeline_run.status = PipelineRunStatus::Completed;
                    pipeline_run.summary = Some(text.clone());
                    pipeline_run.finished_at = Some(jiff::Timestamp::now());
                    if let Err(e) = repo.update_run(&pipeline_run).await {
                        error!(error = %e, "failed to update pipeline run as completed");
                    }

                    PipelineStreamEvent::Done {
                        summary: text.clone(),
                        iterations: *iterations,
                        tool_calls: *tool_calls_made,
                    }
                }
                RunnerEvent::Error(msg) => {
                    completed = true;

                    // Update pipeline run as failed.
                    pipeline_run.status = PipelineRunStatus::Failed;
                    pipeline_run.error = Some(msg.clone());
                    pipeline_run.finished_at = Some(jiff::Timestamp::now());
                    if let Err(e) = repo.update_run(&pipeline_run).await {
                        error!(error = %e, "failed to update pipeline run as failed");
                    }

                    PipelineStreamEvent::Error {
                        message: msg.clone(),
                    }
                }
            };

            // Broadcast event to SSE subscribers.
            let _ = self.broadcast_tx.send(stream_event.clone());

            // Persist event to the database.
            let event_type = stream_event.event_type_name();
            let payload = serde_json::to_value(&stream_event).unwrap_or_default();
            if let Err(e) = repo.insert_event(run_id, seq, event_type, payload).await {
                warn!(error = %e, seq, event_type, "failed to persist pipeline event");
            }
            seq += 1;
        }

        // 4. If the channel closed without Done/Error, mark as failed.
        if !completed {
            warn!("pipeline runner channel closed without terminal event");
            pipeline_run.status = PipelineRunStatus::Failed;
            pipeline_run.error = Some("runner channel closed unexpectedly".to_owned());
            pipeline_run.finished_at = Some(jiff::Timestamp::now());
            if let Err(e) = repo.update_run(&pipeline_run).await {
                error!(error = %e, "failed to update pipeline run as failed (channel closed)");
            }

            let err_event = PipelineStreamEvent::Error {
                message: "runner channel closed unexpectedly".to_owned(),
            };
            let _ = self.broadcast_tx.send(err_event.clone());

            let payload = serde_json::to_value(&err_event).unwrap_or_default();
            let _ = repo
                .insert_event(run_id, seq, err_event.event_type_name(), payload)
                .await;
        }

        // 5. Send a completion notification via Telegram.
        self.send_completion_notification(&pipeline_run).await;

        info!(run_id = %run_id, "pipeline run finished");
        Ok(())
    }

    /// Send a Telegram notification with the pipeline run result.
    ///
    /// When `notification_channel_id` is configured, sends directly via the
    /// Telegram Bot API (fire-and-forget, no PGMQ persistence). Otherwise
    /// falls back to the PGMQ-based `notify_client`.
    async fn send_completion_notification(&self, run: &crate::types::PipelineRun) {
        let settings = self.settings_svc.current();

        let (emoji, status_label) = match run.status {
            PipelineRunStatus::Completed => ("\u{2705}", "completed"),
            PipelineRunStatus::Failed => ("\u{274c}", "failed"),
            PipelineRunStatus::Cancelled => ("\u{1f6d1}", "cancelled"),
            PipelineRunStatus::Running => return, // shouldn't happen
        };

        let mut body = format!("{emoji} Pipeline run {status_label}");

        if let Some(summary) = &run.summary {
            // Truncate summary for Telegram (max ~400 chars).
            let truncated = if summary.len() > 400 {
                format!("{}…", &summary[..400])
            } else {
                summary.clone()
            };
            body.push_str(&format!("\n\n{truncated}"));
        }
        if let Some(err) = &run.error {
            let truncated = if err.len() > 400 {
                format!("{}…", &err[..400])
            } else {
                err.clone()
            };
            body.push_str(&format!("\n\nError: {truncated}"));
        }

        // Fast path: send directly to the dedicated notification channel via
        // Bot API, bypassing PGMQ entirely.
        if let (Some(token), Some(channel_id)) = (
            settings.telegram.bot_token.as_deref(),
            settings.telegram.notification_channel_id,
        ) {
            let url = format!("https://api.telegram.org/bot{token}/sendMessage");
            let payload = serde_json::json!({
                "chat_id": channel_id,
                "text": body,
                "parse_mode": "Markdown",
            });
            match reqwest::Client::new().post(&url).json(&payload).send().await {
                Ok(resp) if !resp.status().is_success() => {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    warn!(
                        %status, body = %text,
                        "telegram channel notification failed"
                    );
                }
                Err(e) => {
                    warn!(error = %e, "failed to send telegram channel notification");
                }
                Ok(_) => {
                    info!("pipeline notification sent to channel {channel_id}");
                }
            }
            return;
        }

        // Fallback: enqueue via PGMQ-based notify client.
        let request = SendTelegramNotificationRequest {
            chat_id: settings.telegram.chat_id,
            subject: Some(format!("Pipeline {status_label}")),
            body,
            priority: NotificationPriority::Normal,
            max_retries: 3,
            reference_type: Some("pipeline_run".to_owned()),
            reference_id: Some(run.id),
            metadata: None,
            photo_path: None,
        };
        if let Err(e) = self.notify_client.send_telegram(request).await {
            warn!(error = %e, "failed to send pipeline completion notification");
        }
    }

    /// Build the tool registry for the pipeline agent.
    ///
    /// Includes:
    /// - Standard primitive tools (db_query, db_mutate, notify, send_email, etc.)
    /// - Pipeline-specific tools (get_job_preferences, score_job, optimize_resume)
    /// - The existing job_pipeline tool (save job URL)
    async fn build_pipeline_tools(&self) -> ToolRegistry {
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

        // Layer 3: MCP tools (e.g. LinkedIn job search)
        match rara_mcp::tool_bridge::McpToolBridge::from_manager(self.mcp_manager.clone()).await {
            Ok(bridges) => {
                for bridge in bridges {
                    let server = bridge.server_name().to_string();
                    registry.register_mcp(Arc::new(bridge), server);
                }
            }
            Err(e) => {
                warn!(error = %e, "failed to load MCP tools for pipeline agent");
            }
        }

        registry
    }
}
