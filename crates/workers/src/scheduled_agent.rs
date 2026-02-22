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

//! Background worker that polls the [`AgentScheduler`] and executes due jobs
//! via the agent runner.

use std::sync::Arc;

use async_trait::async_trait;
use common_worker::{FallibleWorker, WorkResult, WorkerContext};
use rara_agents::runner::{AgentRunner, UserContent};
use rara_domain_chat::service::to_chat_message;
use rara_domain_shared::settings::model::Settings;
use rara_sessions::types::SessionKey;
use tracing::{info, warn};

use crate::{agent_scheduler::AgentScheduler, worker_state::AppState};

/// Default behavioural policy used when no custom policy is configured.
const DEFAULT_AGENT_POLICY: &str = include_str!("../../../prompts/workers/agent_policy.md");

fn compose_policy(base_policy: &str, soul_prompt: Option<&str>) -> String {
    if let Some(soul) = soul_prompt.filter(|s| !s.trim().is_empty()) {
        return format!("{soul}\n\n# Operational Policy\n{base_policy}");
    }
    base_policy.to_owned()
}

fn resolve_soul_prompt(settings: &Settings) -> Option<String> {
    if settings
        .agent
        .soul
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty())
    {
        return settings.agent.soul.clone();
    }
    let markdown_soul = rara_paths::load_agent_soul_prompt();
    if markdown_soul.trim().is_empty() {
        return None;
    }
    Some(markdown_soul)
}

/// Worker that periodically checks the agent scheduler for due jobs and
/// executes each one through the full agent runner pipeline.
pub struct AgentSchedulerWorker {
    scheduler: Arc<AgentScheduler>,
}

impl AgentSchedulerWorker {
    pub fn new(scheduler: Arc<AgentScheduler>) -> Self { Self { scheduler } }
}

#[async_trait]
impl FallibleWorker<AppState> for AgentSchedulerWorker {
    async fn work(&mut self, ctx: WorkerContext<AppState>) -> WorkResult {
        let state = ctx.state();
        let settings = state.settings_svc.current();

        // Guard: AI must be configured.
        if !settings.ai.is_configured() {
            warn!("agent-scheduler skipped: AI not configured");
            return Ok(());
        }

        let due_jobs = self.scheduler.get_due_jobs().await;
        if due_jobs.is_empty() {
            return Ok(());
        }

        info!(
            count = due_jobs.len(),
            "agent-scheduler: executing due jobs"
        );

        let policy = load_agent_policy(&settings).await;
        let model = settings
            .ai
            .model_for(rara_domain_shared::settings::model::ModelScenario::Chat)
            .to_owned();

        for job in &due_jobs {
            info!(
                job_id = %job.id,
                message = %job.message,
                "agent-scheduler: running job"
            );

            // Load recent session history for context.
            let session_key = SessionKey::from_raw(job.session_key.clone());
            let history = match state
                .chat_service
                .get_messages(&session_key, None, Some(50))
                .await
            {
                Ok(msgs) => {
                    let hist: Vec<_> = msgs.iter().map(to_chat_message).collect();
                    Some(hist)
                }
                Err(e) => {
                    warn!(
                        session = %session_key,
                        error = %e,
                        "agent-scheduler: failed to load session history"
                    );
                    None
                }
            };

            // Build and run the agent.
            let runner = AgentRunner::builder()
                .llm_provider(state.llm_provider.clone())
                .model_name(model.clone())
                .system_prompt(policy.clone())
                .user_content(UserContent::Text(job.message.clone()))
                .maybe_history(history)
                .max_iterations(15_usize)
                .build();

            let tools = &state.chat_service.tools();
            match runner.run(tools, None).await {
                Ok(result) => {
                    let response_text = result
                        .provider_response
                        .choices
                        .first()
                        .and_then(|c| c.message.content.as_deref())
                        .unwrap_or_default()
                        .to_owned();

                    info!(
                        job_id = %job.id,
                        iterations = result.iterations,
                        tool_calls = result.tool_calls_made,
                        response_len = response_text.len(),
                        "agent-scheduler: job completed"
                    );

                    // Append user + assistant messages to session.
                    if let Err(e) = state
                        .chat_service
                        .append_messages(&session_key, &job.message, &response_text)
                        .await
                    {
                        warn!(
                            job_id = %job.id,
                            error = %e,
                            "agent-scheduler: failed to persist session messages"
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        job_id = %job.id,
                        error = %e,
                        "agent-scheduler: agent run failed"
                    );
                }
            }

            // Mark job executed (updates last_run_at, removes Delay jobs).
            if let Err(e) = self.scheduler.mark_executed(&job.id).await {
                warn!(
                    job_id = %job.id,
                    error = %e,
                    "agent-scheduler: failed to mark job executed"
                );
            }
        }

        Ok(())
    }
}

/// Load the agent behavioural policy from settings or file system.
async fn load_agent_policy(settings: &Settings) -> String {
    let soul_prompt = resolve_soul_prompt(settings);
    // 1. Try reading from the policy file.
    let policy_path = rara_paths::agent_policy_file();
    if let Ok(content) = tokio::fs::read_to_string(policy_path).await {
        let trimmed = content.trim();
        if !trimmed.is_empty() {
            return compose_policy(trimmed, soul_prompt.as_deref());
        }
    }

    let prompt_content =
        rara_paths::load_prompt_markdown("workers/agent_policy.md", DEFAULT_AGENT_POLICY);
    if !prompt_content.trim().is_empty() {
        return compose_policy(&prompt_content, soul_prompt.as_deref());
    }

    // 2. Fall back to built-in default.
    compose_policy(DEFAULT_AGENT_POLICY, soul_prompt.as_deref())
}
