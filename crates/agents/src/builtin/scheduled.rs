//! Scheduled agent -- executes due jobs from the agent scheduler.

use std::sync::Arc;

use agent_core::{
    context::AgentContext,
    runner::UserContent,
};
use rara_sessions::types::ChatMessage;

use super::AgentOutput;
use crate::orchestrator::{context::to_chat_message, error::OrchestratorError};

/// Agent that executes scheduled jobs with full tool access.
#[derive(Clone)]
pub struct ScheduledAgent {
    ctx: Arc<dyn AgentContext>,
}

impl ScheduledAgent {
    pub fn new(ctx: Arc<dyn AgentContext>) -> Self { Self { ctx } }

    /// Execute a single scheduled job.
    ///
    /// `message` is the job's task description.
    /// `history` is optional session context.
    pub async fn run(
        &self,
        message: &str,
        history: Option<&[ChatMessage]>,
    ) -> Result<AgentOutput, OrchestratorError> {
        let policy = self.ctx.build_worker_policy().await;
        let model = self.ctx.model_for_key("scheduled");
        let provider_hint = self.ctx.provider_hint();
        let max_iterations = self.ctx.max_iterations("scheduled");
        let tools = self.ctx.tools().clone();

        let chat_history = history
            .map(|h| h.iter().map(to_chat_message).collect())
            .unwrap_or_default();

        let runner = agent_core::runner::AgentRunner::builder()
            .llm_provider(self.ctx.llm_provider().clone())
            .provider_hint(provider_hint.unwrap_or_default())
            .model_name(model)
            .system_prompt(policy)
            .user_content(UserContent::Text(message.to_owned()))
            .history(chat_history)
            .max_iterations(max_iterations)
            .build();

        let result = runner
            .run(&tools, None)
            .await
            .map_err(|e| OrchestratorError::AgentError {
                message: format!("scheduled agent run failed: {e}"),
            })?;

        Ok(AgentOutput::from_run_response(&result))
    }
}
