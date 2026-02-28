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

//! Background worker that polls the `AgentScheduler` and spawns due jobs
//! as Kernel agent processes.

use std::sync::Arc;

use async_trait::async_trait;
use common_worker::{FallibleWorker, WorkResult, WorkerContext};
use rara_kernel::process::{AgentManifest, SessionId, principal::Principal};
use tracing::{info, warn};

use crate::{agent_scheduler::AgentScheduler, worker_state::AppState};

/// Worker that periodically checks the agent scheduler for due jobs and
/// spawns each one via the Kernel.
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
            "agent-scheduler: spawning due jobs via kernel"
        );

        let policy = state.agent_ctx.build_worker_policy().await;
        let model = state.agent_ctx.model_for_key("scheduled");

        for job in &due_jobs {
            let manifest = AgentManifest {
                name:           format!("scheduled:{}", job.id),
                description:    format!("Scheduled job: {}", job.id),
                model:          model.clone(),
                system_prompt:  policy.clone(),
                provider_hint:  state.agent_ctx.provider_hint(),
                max_iterations: Some(state.agent_ctx.max_iterations("scheduled")),
                tools:          vec![], // inherit all tools
                max_children:   None,
                metadata:       serde_json::json!({ "job_id": job.id }),
            };

            let session_id = SessionId::new(job.session_key.clone());
            let principal = Principal::admin("system");

            match state
                .kernel
                .spawn_with_input(manifest, job.message.clone(), principal, session_id, None)
                .await
            {
                Ok(handle) => {
                    info!(
                        job_id = %job.id,
                        agent_id = %handle.agent_id,
                        "agent-scheduler: job spawned via kernel"
                    );
                    // Mark the job as executed after successful spawn.
                    if let Err(e) = self.scheduler.mark_executed(&job.id).await {
                        warn!(
                            job_id = %job.id,
                            error = %e,
                            "agent-scheduler: failed to mark job executed"
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        job_id = %job.id,
                        error = %e,
                        "agent-scheduler: failed to spawn job via kernel"
                    );
                }
            }
        }

        Ok(())
    }
}
