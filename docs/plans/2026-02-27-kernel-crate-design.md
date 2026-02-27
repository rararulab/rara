# Kernel Crate Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Create `crates/core/kernel` — a unified orchestrator that coordinates agent lifecycle, memory (3-layer), LLM execution, and tool dispatch.

**Architecture:** The Kernel is a thin coordination layer. It does NOT own business logic — it wires together `agent-core` (LLM runner), `memory-core` (memory traits), and `ToolRegistry` (tools). Inspired by openfang's kernel but much leaner (~300 lines vs ~5000).

**Tech Stack:** agent-core, memory-core, uuid, jiff, serde, serde_json, async-trait, snafu, dashmap, tokio, tracing

---

### Task 1: Create crate skeleton

**Files:**
- Create: `crates/core/kernel/Cargo.toml`
- Create: `crates/core/kernel/src/lib.rs` (empty placeholder)
- Modify: `Cargo.toml` (workspace root)

**Step 1: Create Cargo.toml**

```toml
[package]
name = "rara-kernel"
version = "0.0.1"
edition.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
homepage.workspace = true
readme.workspace = true
keywords.workspace = true
categories.workspace = true
description = "Unified agent orchestrator: lifecycle, memory, LLM execution, tool dispatch"

[dependencies]
agent-core.workspace = true
async-trait.workspace = true
dashmap = "6"
jiff.workspace = true
memory-core.workspace = true
serde.workspace = true
serde_json.workspace = true
snafu.workspace = true
tokio.workspace = true
tracing.workspace = true
uuid.workspace = true

[lints]
workspace = true
```

**Step 2: Create empty lib.rs** (with license header)

**Step 3: Register in workspace**

Add to `Cargo.toml` workspace `members`:
```
"crates/core/kernel",
```

Add to `[workspace.dependencies]`:
```
rara-kernel = { path = "crates/core/kernel" }
```

**Step 4: Verify**

Run: `cargo check -p rara-kernel`

**Step 5: Commit**

```bash
git add crates/core/kernel/ Cargo.toml
git commit -m "chore(kernel): scaffold empty crate"
```

---

### Task 2: Define error types

**Files:**
- Create: `crates/core/kernel/src/error.rs`
- Modify: `crates/core/kernel/src/lib.rs`

**Step 1: Write error.rs**

```rust
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

use snafu::Snafu;
use uuid::Uuid;

#[derive(Debug, Snafu)]
pub enum KernelError {
    /// Agent not found in registry.
    #[snafu(display("agent not found: {id}"))]
    AgentNotFound { id: Uuid },

    /// Agent name already registered.
    #[snafu(display("agent already exists: {name}"))]
    AgentAlreadyExists { name: String },

    /// Agent runner error.
    #[snafu(display("agent runner error: {message}"))]
    Runner { message: String },

    /// Memory subsystem error.
    #[snafu(display("memory error: {message}"))]
    Memory { message: String },

    /// Kernel boot/initialization error.
    #[snafu(display("boot failed: {message}"))]
    Boot { message: String },
}

impl From<agent_core::err::Error> for KernelError {
    fn from(err: agent_core::err::Error) -> Self {
        Self::Runner {
            message: err.to_string(),
        }
    }
}

impl From<memory_core::MemoryError> for KernelError {
    fn from(err: memory_core::MemoryError) -> Self {
        Self::Memory {
            message: err.to_string(),
        }
    }
}

pub type Result<T> = std::result::Result<T, KernelError>;
```

**Step 2: Update lib.rs**

```rust
// ... (license header)

pub mod error;

pub use error::{KernelError, Result};
```

**Step 3: Verify**

Run: `cargo check -p rara-kernel`

**Step 4: Commit**

```bash
git add crates/core/kernel/src/
git commit -m "feat(kernel): add KernelError with snafu"
```

---

### Task 3: Define AgentRegistry

**Files:**
- Create: `crates/core/kernel/src/registry.rs`
- Modify: `crates/core/kernel/src/lib.rs`

**Step 1: Write registry.rs**

```rust
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
```

**Step 2: Update lib.rs — add `pub mod registry` and re-exports**

**Step 3: Verify**

Run: `cargo check -p rara-kernel`

**Step 4: Commit**

```bash
git add crates/core/kernel/src/
git commit -m "feat(kernel): add AgentRegistry with manifest-based registration"
```

---

### Task 4: Define the Kernel struct and core API

**Files:**
- Create: `crates/core/kernel/src/kernel.rs`
- Modify: `crates/core/kernel/src/lib.rs`

**Step 1: Write kernel.rs**

```rust
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
    runner::{AgentRunResponse, AgentRunner, RunnerEvent, UserContent},
    tool_registry::ToolRegistry,
};
use memory_core::{KnowledgeMemory, LearningMemory, MemoryContext, Scope, StateMemory};
use tokio::sync::mpsc;
use tracing::{info, warn};
use uuid::Uuid;

use crate::{
    error::{KernelError, Result},
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
    pub fn with_state_memory(mut self, memory: Arc<dyn StateMemory>) -> Self {
        self.state_memory = Some(memory);
        self
    }

    /// Attach the knowledge memory layer.
    pub fn with_knowledge_memory(mut self, memory: Arc<dyn KnowledgeMemory>) -> Self {
        self.knowledge_memory = Some(memory);
        self
    }

    /// Attach the learning memory layer.
    pub fn with_learning_memory(mut self, memory: Arc<dyn LearningMemory>) -> Self {
        self.learning_memory = Some(memory);
        self
    }

    // ── Agent lifecycle ─────────────────────────────────────────────

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

    // ── Message dispatch ────────────────────────────────────────────

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
        on_event: &agent_core::runner::OnEvent,
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

    // ── Memory access ───────────────────────────────────────────────

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

    // ── Internal ────────────────────────────────────────────────────

    fn build_runner(&self, entry: &AgentEntry, content: UserContent) -> AgentRunner {
        let mut builder = AgentRunner::builder()
            .llm_provider(self.llm_provider.clone())
            .model_name(entry.model.as_str())
            .system_prompt(entry.system_prompt.as_str())
            .user_content(content)
            .max_iterations(entry.max_iterations);

        if let Some(ref hint) = entry.provider_hint {
            builder = builder.provider_hint(hint.as_str());
        }

        builder.build()
    }
}
```

**Step 2: Update lib.rs — final version**

```rust
// Copyright 2025 Crrow
//
// Licensed under the Apache License, Version 2.0 (the "License");
// ...

//! # rara-kernel
//!
//! Unified agent orchestrator — coordinates agent lifecycle, 3-layer memory,
//! LLM execution, and tool dispatch.
//!
//! ## Architecture
//!
//! ```text
//!                      ┌──────────────────┐
//!                      │     Kernel        │  ← unified entry point
//!                      └──┬──┬──┬──┬──────┘
//!          registry ──────┘  │  │  └────── tools (ToolRegistry)
//!          memory (3-layer) ─┘  └── llm (AgentRunner)
//! ```
//!
//! The kernel does NOT own business logic. It wires together:
//! - [`agent_core::runner::AgentRunner`] for LLM execution
//! - [`memory_core`] traits for 3-layer memory
//! - [`agent_core::tool_registry::ToolRegistry`] for tool dispatch

pub mod error;
pub mod kernel;
pub mod registry;

pub use error::{KernelError, Result};
pub use kernel::{Kernel, KernelConfig};
pub use registry::{AgentEntry, AgentManifest, AgentRegistry, AgentState};
```

**Step 3: Verify**

Run: `cargo check -p rara-kernel`

**Step 4: Commit**

```bash
git add crates/core/kernel/src/
git commit -m "feat(kernel): add Kernel struct with agent lifecycle and message dispatch"
```

---

### Task 5: Final verification

**Step 1:** `cargo check -p rara-kernel`
**Step 2:** `cargo clippy -p rara-kernel`
**Step 3:** Squash into one commit:

```bash
git log --oneline -4  # review individual commits
# single commit for the whole crate:
git reset --soft HEAD~4
git commit -m "feat(kernel): unified agent orchestrator (#N)

Add crates/core/kernel (rara-kernel) with:
- Kernel: unified entry point for agent lifecycle + message dispatch
- AgentRegistry: in-memory agent tracking with DashMap
- AgentManifest: agent configuration (model, prompt, tools)
- 3-layer memory integration via memory-core traits (optional)
- Streaming and non-streaming message dispatch via AgentRunner

Closes #N"
```
