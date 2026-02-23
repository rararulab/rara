//! Chat agent -- interactive conversation agent with memory and MCP tools.

use std::sync::Arc;

use crate::{
    orchestrator::{
        context::to_chat_message,
        AgentOrchestrator,
        error::OrchestratorError,
    },
    runner::UserContent,
    tool_registry::ToolRegistry,
};
use rara_sessions::types::ChatMessage;

use super::AgentOutput;

/// Interactive chat agent with memory injection, MCP tools, and context compaction.
#[derive(Clone)]
pub struct ChatAgent {
    orchestrator: AgentOrchestrator,
}

impl ChatAgent {
    pub fn new(orchestrator: AgentOrchestrator) -> Self {
        Self { orchestrator }
    }

    /// Execute a single chat interaction.
    ///
    /// The caller is responsible for session I/O (reading history, persisting
    /// messages). This method handles: compaction check, system prompt assembly
    /// (memory + skills injection), effective tool building (MCP), and agent
    /// execution.
    pub async fn run(
        &self,
        base_system_prompt: &str,
        user_content: UserContent,
        history: &[ChatMessage],
        model: &str,
        context_length: usize,
    ) -> Result<AgentOutput, OrchestratorError> {
        // 1. Check if compaction is needed
        let effective_history = if self.orchestrator.needs_compaction(history, context_length) {
            let summary = self.orchestrator.summarize_history(history, model).await?;
            vec![summary]
        } else {
            history.to_vec()
        };

        // 2. Build system prompt (soul + memory profile + memory prefetch + skills)
        let user_text = match &user_content {
            UserContent::Text(t) => t.clone(),
            UserContent::Multimodal { text, .. } => text.clone(),
        };
        let system_prompt = self
            .orchestrator
            .build_chat_system_prompt(base_system_prompt, &user_text, effective_history.len())
            .await;

        // 3. Build effective tools (static + MCP)
        let effective_tools = self.orchestrator.build_effective_tools().await;

        // 4. Convert history and build runner
        let chat_history = effective_history.iter().map(to_chat_message).collect();
        let runner = self.orchestrator.build_runner(
            model.to_owned(),
            system_prompt,
            user_content,
            chat_history,
        );

        // 5. Execute
        let result = runner
            .run(&effective_tools, None)
            .await
            .map_err(|e| OrchestratorError::AgentError {
                message: e.to_string(),
            })?;

        let response_text = result
            .provider_response
            .choices
            .first()
            .and_then(|c| c.message.content.as_deref())
            .unwrap_or_default()
            .to_owned();

        // 6. Post-process: memory reflection (fire-and-forget)
        self.orchestrator
            .spawn_memory_reflection(&user_text, &response_text);

        Ok(AgentOutput {
            response_text,
            iterations: result.iterations,
            tool_calls_made: result.tool_calls_made,
        })
    }

    /// Streaming variant -- returns the runner and tools for the caller to drive.
    ///
    /// The caller (ChatService) owns the streaming loop and persistence.
    /// This method returns a prepared runner + tools so the caller can call
    /// `runner.run_streaming(tools)`.
    pub async fn prepare_streaming(
        &self,
        base_system_prompt: &str,
        user_content: UserContent,
        history: &[ChatMessage],
        model: &str,
        context_length: usize,
    ) -> Result<ChatAgentStreamSetup, OrchestratorError> {
        let effective_history = if self.orchestrator.needs_compaction(history, context_length) {
            let summary = self.orchestrator.summarize_history(history, model).await?;
            vec![summary]
        } else {
            history.to_vec()
        };

        let user_text = match &user_content {
            UserContent::Text(t) => t.clone(),
            UserContent::Multimodal { text, .. } => text.clone(),
        };
        let system_prompt = self
            .orchestrator
            .build_chat_system_prompt(base_system_prompt, &user_text, effective_history.len())
            .await;
        let effective_tools = self.orchestrator.build_effective_tools().await;
        let chat_history = effective_history.iter().map(to_chat_message).collect();
        let runner = self.orchestrator.build_runner(
            model.to_owned(),
            system_prompt,
            user_content,
            chat_history,
        );

        Ok(ChatAgentStreamSetup {
            runner,
            effective_tools,
            orchestrator: self.orchestrator.clone(),
        })
    }

    /// Return a reference to the orchestrator (for accessors like tools(), settings()).
    pub fn orchestrator(&self) -> &AgentOrchestrator {
        &self.orchestrator
    }
}

/// Setup bundle for streaming chat -- caller drives the stream loop.
pub struct ChatAgentStreamSetup {
    pub runner: crate::runner::AgentRunner,
    pub effective_tools: Arc<ToolRegistry>,
    pub orchestrator: AgentOrchestrator,
}
