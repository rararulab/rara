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

//! Kernel — the unified orchestrator for agent lifecycle, memory, and LLM
//! execution.
//!
//! The [`Kernel`] struct is the single entry point for all agent operations.
//! It coordinates:
//!
//! - **Agent registry** — spawn, list, kill agents
//! - **LLM execution** — delegates to [`AgentRunner`] from `agent-core`
//! - **Memory** — 3-layer memory (State/Knowledge/Learning) via `memory-core` traits
//! - **Tools** — shared [`ToolRegistry`] with per-agent filtering

use std::sync::Arc;

use agent_core::{
    model::LlmProviderLoaderRef,
    runner::{AgentRunResponse, AgentRunner, OnEvent, RunnerEvent, UserContent},
    tool_registry::ToolRegistry,
};
use memory_core::{KnowledgeMemory, LearningMemory, MemoryContext, StateMemory};
use tokio::sync::mpsc;
use tracing::info;
use uuid::Uuid;

use crate::{
    error::Result,
    registry::{AgentEntry, AgentManifest, AgentRegistry, AgentState},
};

/// Kernel configuration.
#[derive(Debug, Clone)]
pub struct KernelConfig {
    /// Default user ID for memory context.
    pub user_id: Uuid,
}

/// The unified agent orchestrator.
pub struct Kernel {
    config: KernelConfig,
    registry: AgentRegistry,
    llm_provider: LlmProviderLoaderRef,
    tools: Arc<ToolRegistry>,

    // 3-layer memory (all optional — kernel works without memory)
    state_memory: Option<Arc<dyn StateMemory>>,
    knowledge_memory: Option<Arc<dyn KnowledgeMemory>>,
    learning_memory: Option<Arc<dyn LearningMemory>>,
}

impl Kernel {
    /// Create a new kernel with the given configuration.
    pub fn boot(
        config: KernelConfig,
        llm_provider: LlmProviderLoaderRef,
        tools: Arc<ToolRegistry>,
    ) -> Self {
        info!("Booting kernel");
        Self {
            config,
            registry: AgentRegistry::new(),
            llm_provider,
            tools,
            state_memory: None,
            knowledge_memory: None,
            learning_memory: None,
        }
    }

    /// Attach the state memory layer.
    #[must_use]
    pub fn with_state_memory(mut self, memory: Arc<dyn StateMemory>) -> Self {
        self.state_memory = Some(memory);
        self
    }

    /// Attach the knowledge memory layer.
    #[must_use]
    pub fn with_knowledge_memory(mut self, memory: Arc<dyn KnowledgeMemory>) -> Self {
        self.knowledge_memory = Some(memory);
        self
    }

    /// Attach the learning memory layer.
    #[must_use]
    pub fn with_learning_memory(mut self, memory: Arc<dyn LearningMemory>) -> Self {
        self.learning_memory = Some(memory);
        self
    }

    // -- Agent lifecycle -------------------------------------------------------

    /// Register a new agent and return its ID.
    pub fn register_agent(&self, manifest: AgentManifest) -> Result<Uuid> {
        let id = self.registry.register(manifest)?;
        info!(agent_id = %id, "Agent registered");
        Ok(id)
    }

    /// Get an agent entry by ID.
    pub fn get_agent(&self, id: Uuid) -> Result<AgentEntry> {
        self.registry.get(id)
    }

    /// Find an agent by name.
    pub fn find_agent(&self, name: &str) -> Option<AgentEntry> {
        self.registry.find_by_name(name)
    }

    /// List all registered agents.
    pub fn list_agents(&self) -> Vec<AgentEntry> {
        self.registry.list()
    }

    /// Remove an agent from the registry.
    pub fn kill_agent(&self, id: Uuid) -> Result<AgentEntry> {
        let entry = self.registry.remove(id)?;
        info!(agent_id = %id, name = %entry.name, "Agent killed");
        Ok(entry)
    }

    // -- Message dispatch ------------------------------------------------------

    /// Send a message to an agent and wait for the full response.
    pub async fn send_message(
        &self,
        agent_id: Uuid,
        message: impl Into<UserContent>,
    ) -> Result<AgentRunResponse> {
        let entry = self.registry.get(agent_id)?;
        self.registry.set_state(agent_id, AgentState::Running)?;

        let tools = self.tools.filtered(&entry.tools);
        let runner = self.build_runner(&entry, message.into());

        let result = runner.run(&tools, None).await;

        self.registry.set_state(agent_id, AgentState::Idle)?;
        Ok(result?)
    }

    /// Send a message with an event callback.
    pub async fn send_message_with_events(
        &self,
        agent_id: Uuid,
        message: impl Into<UserContent>,
        on_event: &OnEvent,
    ) -> Result<AgentRunResponse> {
        let entry = self.registry.get(agent_id)?;
        self.registry.set_state(agent_id, AgentState::Running)?;

        let tools = self.tools.filtered(&entry.tools);
        let runner = self.build_runner(&entry, message.into());

        let result = runner.run(&tools, Some(on_event)).await;

        self.registry.set_state(agent_id, AgentState::Idle)?;
        Ok(result?)
    }

    /// Send a message with streaming responses.
    pub fn send_message_streaming(
        &self,
        agent_id: Uuid,
        message: impl Into<UserContent>,
    ) -> Result<mpsc::Receiver<RunnerEvent>> {
        let entry = self.registry.get(agent_id)?;
        self.registry.set_state(agent_id, AgentState::Running)?;

        let tools = Arc::new(self.tools.filtered(&entry.tools));
        let runner = self.build_runner(&entry, message.into());

        Ok(runner.run_streaming(tools))
    }

    // -- Memory access ---------------------------------------------------------

    /// Build a [`MemoryContext`] for the given agent.
    pub fn memory_context(&self, agent_id: Uuid, session_id: Option<Uuid>) -> MemoryContext {
        MemoryContext {
            user_id: self.config.user_id,
            agent_id,
            session_id,
        }
    }

    /// Access the state memory layer (if attached).
    pub fn state_memory(&self) -> Option<&Arc<dyn StateMemory>> {
        self.state_memory.as_ref()
    }

    /// Access the knowledge memory layer (if attached).
    pub fn knowledge_memory(&self) -> Option<&Arc<dyn KnowledgeMemory>> {
        self.knowledge_memory.as_ref()
    }

    /// Access the learning memory layer (if attached).
    pub fn learning_memory(&self) -> Option<&Arc<dyn LearningMemory>> {
        self.learning_memory.as_ref()
    }

    /// Access the agent registry.
    pub fn registry(&self) -> &AgentRegistry {
        &self.registry
    }

    /// Access the tool registry.
    pub fn tools(&self) -> &Arc<ToolRegistry> {
        &self.tools
    }

    // -- Internal --------------------------------------------------------------

    fn build_runner(&self, entry: &AgentEntry, content: UserContent) -> AgentRunner {
        let hint = entry
            .provider_hint
            .clone()
            .unwrap_or_default();
        AgentRunner::builder()
            .llm_provider(self.llm_provider.clone())
            .provider_hint(hint)
            .model_name(entry.model.clone())
            .system_prompt(entry.system_prompt.clone())
            .user_content(content)
            .max_iterations(entry.max_iterations)
            .build()
    }
}
