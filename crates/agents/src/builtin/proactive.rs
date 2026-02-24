//! Proactive agent -- reviews recent activity and takes autonomous actions.

use agent_core::runner::UserContent;
use crate::orchestrator::{context::to_chat_message, AgentOrchestrator, error::OrchestratorError};
use rara_sessions::types::ChatMessage;

use super::AgentOutput;

/// Maximum number of agent loop iterations per proactive run.
const MAX_ITERATIONS: usize = 15;

/// Proactive agent that reviews recent activity and takes autonomous action.
#[derive(Clone)]
pub struct ProactiveAgent {
    orchestrator: AgentOrchestrator,
}

impl ProactiveAgent {
    pub fn new(orchestrator: AgentOrchestrator) -> Self {
        Self { orchestrator }
    }

    /// Build the user prompt for a proactive review cycle.
    ///
    /// This is also used by the worker to persist the user turn.
    pub fn build_user_prompt(activity_summary: &str) -> String {
        format!(
            "以下是最近24小时的用户活动摘要：\n\n{}\n\n根据你的行为策略，\
             决定是否需要主动联系用户。\n你可以使用工具查询更多信息、发送通知、或安排后续任务。\
             \n如果没有值得做的事情，直接回复 DONE。",
            activity_summary
        )
    }

    /// Execute a proactive review cycle.
    ///
    /// `activity_summary` is pre-collected by the caller (worker).
    /// `history` is the proactive session's recent messages.
    pub async fn run(
        &self,
        activity_summary: &str,
        history: &[ChatMessage],
    ) -> Result<AgentOutput, OrchestratorError> {
        let user_prompt = Self::build_user_prompt(activity_summary);

        let policy = self.orchestrator.build_worker_policy().await;
        let model = self.orchestrator.model_for_key("proactive");
        let provider_hint = self.orchestrator.settings().ai.provider;
        let tools = self.orchestrator.tools().clone();
        let chat_history = history.iter().map(to_chat_message).collect();

        let runner = agent_core::runner::AgentRunner::builder()
            .llm_provider(self.orchestrator.llm_provider().clone())
            .provider_hint(provider_hint.unwrap_or_default())
            .model_name(model)
            .system_prompt(policy)
            .user_content(UserContent::Text(user_prompt))
            .history(chat_history)
            .max_iterations(MAX_ITERATIONS)
            .build();

        let result = runner
            .run(&tools, None)
            .await
            .map_err(|e| OrchestratorError::AgentError {
                message: format!("proactive agent run failed: {e}"),
            })?;

        Ok(AgentOutput::from_run_response(&result))
    }
}
