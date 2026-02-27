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
//! - **Memory** — 3-layer memory (State/Knowledge/Learning) via `agent-core::memory` traits
//! - **Tools** — shared [`ToolRegistry`] with per-agent filtering

use std::sync::Arc;

use agent_core::{
    memory::{Memory, MemoryContext},
    model::LlmProviderLoaderRef,
    runner::{AgentRunResponse, AgentRunner, OnEvent, RunnerEvent, UserContent},
    tool_registry::ToolRegistry,
};
use tokio::sync::mpsc;
use tracing::info;
use uuid::Uuid;

use crate::{
    error::Result,
    registry::{AgentEntry, AgentManifest, AgentRegistry, AgentState},
};

/// The unified agent orchestrator.
pub struct Kernel {
    registry: AgentRegistry,
    llm_provider: LlmProviderLoaderRef,
    tools: Arc<ToolRegistry>,
    memory: Arc<dyn Memory>,
}

impl Kernel {
    /// Create a new kernel with the given configuration.
    ///
    /// Memory is a required subsystem. Use a noop implementation if no
    /// persistence is needed.
    pub fn boot(
        llm_provider: LlmProviderLoaderRef,
        tools: Arc<ToolRegistry>,
        memory: Arc<dyn Memory>,
    ) -> Self {
        info!("Booting kernel");
        Self {
            registry: AgentRegistry::new(),
            llm_provider,
            tools,
            memory,
        }
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
    pub fn memory_context(
        &self,
        user_id: Uuid,
        agent_id: Uuid,
        session_id: Option<Uuid>,
    ) -> MemoryContext {
        MemoryContext {
            user_id,
            agent_id,
            session_id,
        }
    }

    /// Access the memory subsystem.
    pub fn memory(&self) -> &Arc<dyn Memory> {
        &self.memory
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
