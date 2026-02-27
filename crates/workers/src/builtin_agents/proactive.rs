//! Proactive agent -- reviews recent activity and takes autonomous actions.

use std::sync::Arc;

use rara_kernel::{
    agent_context::AgentContext,
    agent_output::AgentOutput,
    error::KernelError,
    runner::UserContent,
};
use rara_sessions::types::ChatMessage;

use rara_domain_chat::message_utils::to_chat_message;

/// Proactive agent that reviews recent activity and takes autonomous action.
#[derive(Clone)]
pub struct ProactiveAgent {
    ctx: Arc<dyn AgentContext>,
}

impl ProactiveAgent {
    pub fn new(ctx: Arc<dyn AgentContext>) -> Self { Self { ctx } }

    /// Build the user prompt for a proactive review cycle.
    ///
    /// This is also used by the worker to persist the user turn.
    pub fn build_user_prompt(activity_summary: &str) -> String {
        format!(
            "以下是最近24小时的用户活动摘要：\n\n{}\n\n根据你的行为策略，\
             决定是否需要主动联系用户。\n你可以使用工具查询更多信息、发送通知、或安排后续任务。\\
             n如果没有值得做的事情，直接回复 DONE。",
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
    ) -> Result<AgentOutput, KernelError> {
        let user_prompt = Self::build_user_prompt(activity_summary);

        let policy = self.ctx.build_worker_policy().await;
        let model = self.ctx.model_for_key("proactive");
        let provider_hint = self.ctx.provider_hint();
        let tools = self.ctx.tools().clone();
        let chat_history = history.iter().map(to_chat_message).collect();

        let runner = rara_kernel::runner::AgentRunner::builder()
            .llm_provider(self.ctx.llm_provider().clone())
            .provider_hint(provider_hint.unwrap_or_default())
            .model_name(model)
            .system_prompt(policy)
            .user_content(UserContent::Text(user_prompt))
            .history(chat_history)
            .max_iterations(self.ctx.max_iterations("proactive"))
            .build();

        let result = runner.run(&tools, None).await?;

        Ok(AgentOutput::from_run_response(&result))
    }
}
