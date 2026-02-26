//! Chat agent -- interactive conversation agent with memory and MCP tools.

use std::sync::Arc;

use agent_core::{
    runner::{AgentRunner, UserContent},
    tool_registry::ToolRegistry,
};
use rara_memory::recall_engine::{EventKind, RecallContext};
use rara_sessions::types::ChatMessage;

use super::AgentOutput;
use crate::orchestrator::{AgentOrchestrator, context::to_chat_message, error::OrchestratorError};

/// Effect returned when compaction occurs, so the caller can persist.
pub struct CompactionEffect {
    /// The summary message that replaced the full history.
    pub summary: ChatMessage,
}

/// Interactive chat agent with memory injection, MCP tools, and context
/// compaction.
#[derive(Clone)]
pub struct ChatAgent {
    orchestrator: AgentOrchestrator,
}

impl ChatAgent {
    pub fn new(orchestrator: AgentOrchestrator) -> Self { Self { orchestrator } }

    /// Execute a single chat interaction.
    ///
    /// The caller is responsible for session I/O (reading history, persisting
    /// messages). This method handles: compaction check, system prompt assembly
    /// (memory + skills injection), effective tool building (MCP), and agent
    /// execution.
    ///
    /// Returns `(AgentOutput, Option<CompactionEffect>)`. When compaction
    /// occurred the caller **must** persist the effect (clear old messages,
    /// store the summary) so subsequent requests see the compacted history.
    pub async fn run(
        &self,
        base_system_prompt: &str,
        user_content: UserContent,
        history: &[ChatMessage],
        model: &str,
        context_length: usize,
    ) -> Result<(AgentOutput, Option<CompactionEffect>), OrchestratorError> {
        let PrepareResult {
            runner,
            effective_tools,
            compaction,
        } = self
            .prepare(
                base_system_prompt,
                user_content,
                history,
                model,
                context_length,
            )
            .await?;

        let result = runner.run(&effective_tools, None).await.map_err(|e| {
            OrchestratorError::AgentError {
                message: e.to_string(),
            }
        })?;

        let output = AgentOutput::from_run_response(&result);

        Ok((output, compaction))
    }

    /// Streaming variant -- returns the runner and tools for the caller to
    /// drive.
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
        let PrepareResult {
            runner,
            effective_tools,
            compaction,
        } = self
            .prepare(
                base_system_prompt,
                user_content,
                history,
                model,
                context_length,
            )
            .await?;

        Ok(ChatAgentStreamSetup {
            runner,
            effective_tools,
            orchestrator: self.orchestrator.clone(),
            compaction,
        })
    }

    /// Return a reference to the orchestrator (for accessors like tools(),
    /// settings()).
    pub fn orchestrator(&self) -> &AgentOrchestrator { &self.orchestrator }

    /// Common preparation logic shared by [`run`] and [`prepare_streaming`].
    ///
    /// Handles: compaction check, recall engine execution, system prompt
    /// assembly, effective tool building, history conversion, and runner
    /// construction.
    async fn prepare(
        &self,
        base_system_prompt: &str,
        user_content: UserContent,
        history: &[ChatMessage],
        model: &str,
        context_length: usize,
    ) -> Result<PrepareResult, OrchestratorError> {
        // 1. Check if compaction is needed
        let (effective_history, compaction) =
            if self.orchestrator.needs_compaction(history, context_length) {
                let summary = self.orchestrator.summarize_history(history, model).await?;
                let effect = CompactionEffect {
                    summary: summary.clone(),
                };
                (vec![summary], Some(effect))
            } else {
                (history.to_vec(), None)
            };

        // 2. Build RecallContext with appropriate events
        let user_text = user_content.text();

        let mut events = Vec::new();
        if compaction.is_some() {
            events.push(EventKind::Compaction);
        }
        if effective_history.len() < 3 {
            events.push(EventKind::NewSession);
        }

        let summary_text = compaction.as_ref().map(|e| e.summary.content.as_text());

        let recall_ctx = RecallContext {
            user_text: user_text.to_owned(),
            turn_count: effective_history.len(),
            events,
            elapsed_since_last_secs: 0,
            summary: summary_text,
            session_topic: None,
        };

        // 3. Build system prompt using the recall engine
        let system_prompt = self
            .orchestrator
            .build_chat_system_prompt(
                base_system_prompt,
                user_text,
                effective_history.len(),
                Some(&recall_ctx),
            )
            .await;

        // 4. Build effective tools (static + MCP)
        let effective_tools = self.orchestrator.build_effective_tools().await;

        // 5. Convert history and build runner
        let chat_history = effective_history.iter().map(to_chat_message).collect();
        let runner = self.orchestrator.build_runner(
            model.to_owned(),
            system_prompt,
            user_content,
            chat_history,
        );

        Ok(PrepareResult {
            runner,
            effective_tools,
            compaction,
        })
    }
}

/// Internal result from [`ChatAgent::prepare`].
struct PrepareResult {
    runner:          AgentRunner,
    effective_tools: Arc<ToolRegistry>,
    compaction:      Option<CompactionEffect>,
}

/// Setup bundle for streaming chat -- caller drives the stream loop.
pub struct ChatAgentStreamSetup {
    pub runner:          AgentRunner,
    pub effective_tools: Arc<ToolRegistry>,
    pub orchestrator:    AgentOrchestrator,
    /// If compaction occurred, the caller must persist this.
    pub compaction:      Option<CompactionEffect>,
}
