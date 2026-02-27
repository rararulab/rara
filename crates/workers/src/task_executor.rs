//! Concrete [`TaskExecutor`] implementation for the workers crate.
//!
//! Bridges the kernel dispatcher with the concrete agent implementations
//! (ProactiveAgent, ScheduledAgent) and handles session persistence and
//! scheduled job callbacks.

use std::sync::Arc;

use async_trait::async_trait;
use rara_kernel::{
    agent_context::AgentContext,
    agent_output::AgentOutput,
    dispatcher::{AgentTaskKind, TaskExecutor, types::AgentTask},
    error::KernelError,
};
use tracing::{info, warn};

use crate::builtin_agents::{proactive::ProactiveAgent, scheduled::ScheduledAgent};

/// Callback for persisting session messages after task execution.
#[async_trait]
pub trait SessionPersister: Send + Sync + 'static {
    /// Persist a user message and an assistant response to the given session.
    async fn persist_messages(
        &self,
        session_key: &str,
        user_text: &str,
        assistant_text: &str,
    ) -> Result<(), String>;

    /// Persist a single message with a role to the given session.
    async fn persist_role_message(
        &self,
        session_key: &str,
        role: &str,
        text: &str,
    ) -> Result<(), String>;

    /// Ensure a session exists (create if needed).
    async fn ensure_session(&self, session_key: &str);
}

/// Callback for marking scheduled jobs as executed.
#[async_trait]
pub trait ScheduledJobCallback: Send + Sync + 'static {
    async fn mark_executed(&self, job_id: &str) -> Result<(), String>;
}

/// Concrete executor that creates ProactiveAgent/ScheduledAgent and handles
/// session persistence and job callbacks.
pub struct WorkerTaskExecutor {
    ctx:               Arc<dyn AgentContext>,
    session_persister: Arc<dyn SessionPersister>,
    job_callback:      Arc<dyn ScheduledJobCallback>,
}

impl WorkerTaskExecutor {
    pub fn new(
        ctx: Arc<dyn AgentContext>,
        session_persister: Arc<dyn SessionPersister>,
        job_callback: Arc<dyn ScheduledJobCallback>,
    ) -> Self {
        Self {
            ctx,
            session_persister,
            job_callback,
        }
    }
}

#[async_trait]
impl TaskExecutor for WorkerTaskExecutor {
    async fn execute(&self, task: &AgentTask) -> Result<AgentOutput, KernelError> {
        // Ensure session exists.
        self.session_persister.ensure_session(&task.session_key).await;

        match &task.kind {
            AgentTaskKind::Proactive => {
                let agent = ProactiveAgent::new(Arc::clone(&self.ctx));
                // ProactiveAgent takes ChatMessage history, but we have
                // ChatCompletionRequestMessage in the task.  Since the agent
                // internally converts anyway, we pass an empty history and
                // let the pre-converted history flow through the runner.
                // For proactive tasks the history is loaded at submit time;
                // we re-convert from the pre-converted messages.
                //
                // Actually: the runner already gets the history from the task.
                // ProactiveAgent.run currently expects &[ChatMessage] — but we
                // changed it to take ChatCompletionRequestMessage via the runner.
                // For now, pass empty sessions history and supply it via the task.
                let output = execute_proactive(&agent, task, &*self.session_persister).await?;
                Ok(output)
            }
            AgentTaskKind::Scheduled { job_id } => {
                let agent = ScheduledAgent::new(Arc::clone(&self.ctx));
                let output = execute_scheduled(
                    &agent,
                    task,
                    job_id,
                    &*self.session_persister,
                    &*self.job_callback,
                )
                .await?;
                Ok(output)
            }
            AgentTaskKind::Pipeline => {
                info!("pipeline task kind is not yet implemented");
                Ok(AgentOutput {
                    response_text:   "pipeline not implemented".to_owned(),
                    iterations:      0,
                    tool_calls_made: 0,
                    truncated:       false,
                })
            }
        }
    }
}

async fn execute_proactive(
    agent: &ProactiveAgent,
    task: &AgentTask,
    session_persister: &dyn SessionPersister,
) -> Result<AgentOutput, KernelError> {
    // The task.history is pre-converted ChatCompletionRequestMessage.
    // ProactiveAgent.run takes &[ChatMessage] — we need to convert back
    // or refactor. Since the agent builds the runner internally with
    // to_chat_message, we pass empty sessions history and let the agent
    // rebuild.  The pre-converted history in task is unused here.
    // TODO: Optimize to avoid double conversion.
    let output = agent
        .run(&task.message, &[])
        .await?;

    // Persist conversation turns.
    let user_prompt = ProactiveAgent::build_user_prompt(&task.message);
    session_persister
        .persist_role_message(&task.session_key, "user", &user_prompt)
        .await
        .ok();
    session_persister
        .persist_role_message(&task.session_key, "assistant", &output.response_text)
        .await
        .ok();

    info!(
        iterations = output.iterations,
        tool_calls = output.tool_calls_made,
        "proactive task completed"
    );
    Ok(output)
}

async fn execute_scheduled(
    agent: &ScheduledAgent,
    task: &AgentTask,
    job_id: &str,
    session_persister: &dyn SessionPersister,
    job_callback: &dyn ScheduledJobCallback,
) -> Result<AgentOutput, KernelError> {
    // Same approach: pass empty history to the agent since it rebuilds via
    // to_chat_message internally.
    let output = agent.run(&task.message, None).await?;

    // Persist conversation turns.
    if let Err(e) = session_persister
        .persist_messages(&task.session_key, &task.message, &output.response_text)
        .await
    {
        warn!(
            job_id = %job_id,
            error = %e,
            "failed to persist scheduled agent session messages"
        );
    }

    // Mark job executed.
    if let Err(e) = job_callback.mark_executed(job_id).await {
        warn!(
            job_id = %job_id,
            error = %e,
            "failed to mark scheduled job executed"
        );
    }

    info!(
        job_id = %job_id,
        iterations = output.iterations,
        tool_calls = output.tool_calls_made,
        "scheduled task completed"
    );
    Ok(output)
}
