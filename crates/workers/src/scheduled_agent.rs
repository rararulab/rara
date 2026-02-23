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
use rara_agents::builtin::scheduled::ScheduledAgent;
use rara_sessions::types::SessionKey;
use tracing::{info, warn};

use crate::{agent_scheduler::AgentScheduler, worker_state::AppState};

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

        let agent = ScheduledAgent::new(state.orchestrator.clone());

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
                Ok(msgs) => Some(msgs),
                Err(e) => {
                    warn!(
                        session = %session_key,
                        error = %e,
                        "agent-scheduler: failed to load session history"
                    );
                    None
                }
            };

            // Delegate to ScheduledAgent.
            match agent.run(&job.message, history.as_deref()).await {
                Ok(output) => {
                    info!(
                        job_id = %job.id,
                        iterations = output.iterations,
                        tool_calls = output.tool_calls_made,
                        response_len = output.response_text.len(),
                        "agent-scheduler: job completed"
                    );

                    // Append user + assistant messages to session.
                    if let Err(e) = state
                        .chat_service
                        .append_messages(&session_key, &job.message, &output.response_text)
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

