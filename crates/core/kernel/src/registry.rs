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

//! Agent registry — tracks all active agents, their state, and metadata.

use dashmap::DashMap;
use jiff::Timestamp;
use uuid::Uuid;

use crate::error::{KernelError, Result};

/// Runtime state of an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum AgentState {
    /// Agent is registered and ready to receive messages.
    Idle,
    /// Agent is currently processing a message.
    Running,
    /// Agent has been stopped and will not accept messages.
    Stopped,
}

/// Configuration for spawning an agent.
#[derive(Debug, Clone)]
pub struct AgentManifest {
    /// Human-readable agent name (must be unique within the registry).
    pub name: String,
    /// LLM model identifier (e.g. "google/gemini-2.0-flash").
    pub model: String,
    /// System prompt for the agent.
    pub system_prompt: String,
    /// Optional provider hint (e.g. "openrouter", "ollama").
    pub provider_hint: Option<String>,
    /// Maximum iterations for the agent loop.
    pub max_iterations: Option<usize>,
    /// Tool names this agent is allowed to use (empty = all tools).
    pub tools: Vec<String>,
    /// Arbitrary metadata.
    pub metadata: serde_json::Value,
}

/// A registered agent's full state.
#[derive(Debug, Clone)]
pub struct AgentEntry {
    pub id: Uuid,
    pub name: String,
    pub model: String,
    pub system_prompt: String,
    pub provider_hint: Option<String>,
    pub max_iterations: usize,
    pub tools: Vec<String>,
    pub state: AgentState,
    pub session_id: Uuid,
    pub metadata: serde_json::Value,
    pub created_at: Timestamp,
    pub last_active: Timestamp,
}

/// In-memory agent registry with name-based indexing.
pub struct AgentRegistry {
    agents: DashMap<Uuid, AgentEntry>,
    name_index: DashMap<String, Uuid>,
}

impl AgentRegistry {
    pub fn new() -> Self {
        Self {
            agents: DashMap::new(),
            name_index: DashMap::new(),
        }
    }

    /// Register a new agent from a manifest. Returns the assigned agent ID.
    pub fn register(&self, manifest: AgentManifest) -> Result<Uuid> {
        if self.name_index.contains_key(&manifest.name) {
            return Err(KernelError::AgentAlreadyExists {
                name: manifest.name,
            });
        }

        let id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let now = Timestamp::now();

        let entry = AgentEntry {
            id,
            name: manifest.name.clone(),
            model: manifest.model,
            system_prompt: manifest.system_prompt,
            provider_hint: manifest.provider_hint,
            max_iterations: manifest.max_iterations.unwrap_or(25),
            tools: manifest.tools,
            state: AgentState::Idle,
            session_id,
            metadata: manifest.metadata,
            created_at: now,
            last_active: now,
        };

        self.name_index.insert(manifest.name, id);
        self.agents.insert(id, entry);
        Ok(id)
    }

    /// Get an agent entry by ID.
    pub fn get(&self, id: Uuid) -> Result<AgentEntry> {
        self.agents
            .get(&id)
            .map(|e| e.value().clone())
            .ok_or(KernelError::AgentNotFound { id })
    }

    /// Find an agent by name.
    pub fn find_by_name(&self, name: &str) -> Option<AgentEntry> {
        self.name_index
            .get(name)
            .and_then(|id| self.agents.get(id.value()).map(|e| e.value().clone()))
    }

    /// Update agent state.
    pub fn set_state(&self, id: Uuid, state: AgentState) -> Result<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or(KernelError::AgentNotFound { id })?;
        entry.state = state;
        entry.last_active = Timestamp::now();
        Ok(())
    }

    /// Remove an agent from the registry.
    pub fn remove(&self, id: Uuid) -> Result<AgentEntry> {
        let (_, entry) = self
            .agents
            .remove(&id)
            .ok_or(KernelError::AgentNotFound { id })?;
        self.name_index.remove(&entry.name);
        Ok(entry)
    }

    /// List all registered agents.
    pub fn list(&self) -> Vec<AgentEntry> {
        self.agents.iter().map(|e| e.value().clone()).collect()
    }

    /// Number of registered agents.
    pub fn len(&self) -> usize {
        self.agents.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.agents.is_empty()
    }
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}
