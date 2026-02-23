//! Scheduled agent -- executes due jobs from the agent scheduler.

use agent_core::runner::UserContent;
use crate::orchestrator::{context::to_chat_message, AgentOrchestrator, error::OrchestratorError};
use rara_sessions::types::ChatMessage;

use super::AgentOutput;

/// Maximum iterations for scheduled job execution.
const MAX_ITERATIONS: usize = 15;

/// Agent that executes scheduled jobs with full tool access.
#[derive(Clone)]
pub struct ScheduledAgent {
    orchestrator: AgentOrchestrator,
}

impl ScheduledAgent {
    pub fn new(orchestrator: AgentOrchestrator) -> Self {
        Self { orchestrator }
    }

    /// Execute a single scheduled job.
    ///
    /// `message` is the job's task description.
    /// `history` is optional session context.
    pub async fn run(
        &self,
        message: &str,
        history: Option<&[ChatMessage]>,
    ) -> Result<AgentOutput, OrchestratorError> {
        let policy = self.orchestrator.build_worker_policy().await;
        let model = self.orchestrator.current_default_model();
        let tools = self.orchestrator.tools().clone();

        let chat_history = history
            .map(|h| h.iter().map(to_chat_message).collect())
            .unwrap_or_default();

        let runner = agent_core::runner::AgentRunner::builder()
            .llm_provider(self.orchestrator.llm_provider().clone())
            .model_name(model)
            .system_prompt(policy)
            .user_content(UserContent::Text(message.to_owned()))
            .history(chat_history)
            .max_iterations(MAX_ITERATIONS)
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
