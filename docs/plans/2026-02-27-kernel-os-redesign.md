# Kernel OS Redesign — Unified Agent Process Model

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the three parallel agent execution pipelines (ChatAgent, SubAgent, Dispatcher) with a unified OS-inspired process model where Kernel manages agent lifecycle through spawn/kill/list, tools act as syscalls with permission enforcement, and agents can spawn child agents forming a process tree.

**Architecture:** The Kernel acts as an OS kernel. `AgentManifest` = binary definition (loadable from YAML or constructed dynamically). `Kernel.spawn()` creates an `AgentProcess` (running instance) with a unique `agent_id`, registers it in the `ProcessTable`, and runs it via `AgentRunner`. A `ScopedKernelHandle` (per-process) provides syscall-like access to kernel capabilities (process mgmt, memory, events, guard). Dual semaphore limits concurrency globally and per-agent.

**Tech Stack:** Rust, tokio, dashmap, serde_yaml, uuid, jiff, snafu, async-trait, bon

---

## Overview of Changes

### What gets ADDED (new modules in `crates/core/kernel/src/`)
- `process/mod.rs` — AgentManifest, AgentProcess, ProcessTable, AgentId, ProcessState, AgentResult, AgentEnv
- `process/principal.rs` — Principal, UserId, Role
- `process/manifest_loader.rs` — ManifestLoader (YAML files)
- `handle/mod.rs` — KernelHandle = ProcessOps + MemoryOps + EventOps + GuardOps
- `handle/scoped.rs` — ScopedKernelHandle (per-process implementation)
- `handle/spawn_tool.rs` — SpawnTool (LLM-callable tool wrapping KernelHandle.spawn)
- `process/defaults/` — YAML agent definitions (scout.yaml, planner.yaml, worker.yaml)

### What gets DELETED
- `subagent/` — entire module (definition.rs, executor.rs, tool.rs, builtin.rs, defaults/*.md)
- `dispatcher/` — entire module (core.rs, error.rs, executor.rs, log_store.rs, metrics.rs, types.rs)
- `registry.rs` — old AgentManifest, AgentEntry, AgentRegistry, AgentState, GuardPolicy
- `context.rs` — old RunContext (replaced by ScopedKernelHandle)
- `agent_context.rs` — old trait hierarchy (CompletionFeatures, ToolFeatures, etc.)
- Workers: `task_executor.rs` (replaced by Kernel.spawn)

### What gets MODIFIED
- `kernel.rs` — rewritten: holds ProcessTable + global Semaphore, implements spawn()
- `lib.rs` — update module declarations and re-exports
- `error.rs` — add process-related error variants
- Workers: `worker_state.rs` — rewire composition root to use Kernel
- Workers: `proactive.rs`, `scheduled_agent.rs` — use Kernel.spawn() instead of Dispatcher

---

## Task 1: Core Process Model Types

**Files:**
- Create: `crates/core/kernel/src/process/mod.rs`
- Create: `crates/core/kernel/src/process/principal.rs`
- Create: `crates/core/kernel/src/process/manifest_loader.rs`
- Create: `crates/core/kernel/src/process/defaults/scout.yaml`
- Create: `crates/core/kernel/src/process/defaults/planner.yaml`
- Create: `crates/core/kernel/src/process/defaults/worker.yaml`
- Modify: `crates/core/kernel/src/lib.rs` — add `pub mod process;`
- Modify: `crates/core/kernel/Cargo.toml` — add `serde_yaml` dependency

**Step 1: Create process module with core types**

`crates/core/kernel/src/process/mod.rs`:
```rust
pub mod manifest_loader;
pub mod principal;

use std::collections::HashMap;
use dashmap::DashMap;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::error::Result;

/// Unique identifier for a running agent process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(pub Uuid);

impl AgentId {
    pub fn new() -> Self { Self(Uuid::new_v4()) }
}

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Persistent session identifier (survives across agent process restarts).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub String);

/// Agent "binary" — static definition, loadable from YAML or constructed dynamically.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentManifest {
    pub name: String,
    pub description: String,
    pub model: String,
    pub system_prompt: String,
    #[serde(default)]
    pub provider_hint: Option<String>,
    #[serde(default)]
    pub max_iterations: Option<usize>,
    #[serde(default)]
    pub tools: Vec<String>,              // empty = inherit parent's tools
    #[serde(default)]
    pub max_children: Option<usize>,     // max concurrent child agents
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Runtime state of an agent process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ProcessState {
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// Result of a completed agent process.
#[derive(Debug, Clone, Serialize)]
pub struct AgentResult {
    pub output: String,
    pub iterations: usize,
    pub tool_calls: usize,
}

/// Process environment — isolated per-agent context.
#[derive(Debug, Clone, Serialize)]
pub struct AgentEnv {
    pub workspace: Option<String>,
    pub vars: HashMap<String, String>,
}

impl Default for AgentEnv {
    fn default() -> Self {
        Self { workspace: None, vars: HashMap::new() }
    }
}

/// A running agent instance in the process table.
#[derive(Debug, Clone)]
pub struct AgentProcess {
    pub agent_id: AgentId,
    pub parent_id: Option<AgentId>,
    pub session_id: SessionId,
    pub manifest: AgentManifest,
    pub principal: principal::Principal,
    pub env: AgentEnv,
    pub state: ProcessState,
    pub created_at: Timestamp,
    pub finished_at: Option<Timestamp>,
    pub result: Option<AgentResult>,
}

/// Summary info for listing processes.
#[derive(Debug, Clone, Serialize)]
pub struct ProcessInfo {
    pub agent_id: AgentId,
    pub parent_id: Option<AgentId>,
    pub name: String,
    pub state: ProcessState,
    pub created_at: Timestamp,
}

impl From<&AgentProcess> for ProcessInfo {
    fn from(p: &AgentProcess) -> Self {
        Self {
            agent_id: p.agent_id,
            parent_id: p.parent_id,
            name: p.manifest.name.clone(),
            state: p.state,
            created_at: p.created_at,
        }
    }
}

/// In-memory process table — the kernel's view of all running agents.
pub struct ProcessTable {
    processes: DashMap<AgentId, AgentProcess>,
}

impl ProcessTable {
    pub fn new() -> Self {
        Self { processes: DashMap::new() }
    }

    pub fn insert(&self, process: AgentProcess) {
        self.processes.insert(process.agent_id, process);
    }

    pub fn get(&self, id: AgentId) -> Option<AgentProcess> {
        self.processes.get(&id).map(|p| p.value().clone())
    }

    pub fn set_state(&self, id: AgentId, state: ProcessState) -> Result<()> {
        let mut entry = self.processes.get_mut(&id)
            .ok_or(crate::error::KernelError::AgentNotFound { id: id.0 })?;
        entry.state = state;
        if matches!(state, ProcessState::Completed | ProcessState::Failed | ProcessState::Cancelled) {
            entry.finished_at = Some(Timestamp::now());
        }
        Ok(())
    }

    pub fn set_result(&self, id: AgentId, result: AgentResult) -> Result<()> {
        let mut entry = self.processes.get_mut(&id)
            .ok_or(crate::error::KernelError::AgentNotFound { id: id.0 })?;
        entry.result = Some(result);
        Ok(())
    }

    pub fn remove(&self, id: AgentId) -> Option<AgentProcess> {
        self.processes.remove(&id).map(|(_, p)| p)
    }

    /// List all children of a given parent.
    pub fn children_of(&self, parent_id: AgentId) -> Vec<ProcessInfo> {
        self.processes.iter()
            .filter(|p| p.parent_id == Some(parent_id))
            .map(|p| ProcessInfo::from(p.value()))
            .collect()
    }

    /// List all processes.
    pub fn list(&self) -> Vec<ProcessInfo> {
        self.processes.iter()
            .map(|p| ProcessInfo::from(p.value()))
            .collect()
    }

    /// Count running processes.
    pub fn running_count(&self) -> usize {
        self.processes.iter()
            .filter(|p| p.state == ProcessState::Running)
            .count()
    }
}

impl Default for ProcessTable {
    fn default() -> Self { Self::new() }
}
```

**Step 2: Create principal module**

`crates/core/kernel/src/process/principal.rs`:
```rust
use serde::{Deserialize, Serialize};

/// User identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UserId(pub String);

/// User role determining permission level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    Admin,
    User,
}

/// The identity under which an agent process runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Principal {
    pub user_id: UserId,
    pub role: Role,
}

impl Principal {
    pub fn admin(user_id: impl Into<String>) -> Self {
        Self { user_id: UserId(user_id.into()), role: Role::Admin }
    }

    pub fn user(user_id: impl Into<String>) -> Self {
        Self { user_id: UserId(user_id.into()), role: Role::User }
    }

    pub fn is_admin(&self) -> bool { self.role == Role::Admin }
}
```

**Step 3: Create manifest loader**

`crates/core/kernel/src/process/manifest_loader.rs`:
```rust
use std::path::Path;
use tracing::warn;
use crate::error::{KernelError, Result};
use super::AgentManifest;

/// Loads AgentManifest definitions from YAML files.
pub struct ManifestLoader {
    manifests: Vec<AgentManifest>,
}

impl ManifestLoader {
    pub fn new() -> Self { Self { manifests: Vec::new() } }

    /// Load all bundled agent manifests (compiled into the binary).
    pub fn load_bundled(&mut self) {
        let sources = [
            include_str!("defaults/scout.yaml"),
            include_str!("defaults/planner.yaml"),
            include_str!("defaults/worker.yaml"),
        ];
        for src in sources {
            match serde_yaml::from_str::<AgentManifest>(src) {
                Ok(m) => self.manifests.push(m),
                Err(e) => warn!(error = %e, "failed to parse bundled agent manifest"),
            }
        }
    }

    /// Load user-defined manifests from a directory. Later loads override earlier
    /// ones with the same name.
    pub fn load_dir(&mut self, dir: &Path) -> Result<usize> {
        if !dir.is_dir() { return Ok(0); }
        let mut count = 0;
        let entries = std::fs::read_dir(dir).map_err(|e| KernelError::IO {
            source: e,
            location: snafu::Location::new(file!(), line!(), 0),
        })?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "yaml" || ext == "yml") {
                let content = std::fs::read_to_string(&path).map_err(|e| KernelError::IO {
                    source: e,
                    location: snafu::Location::new(file!(), line!(), 0),
                })?;
                match serde_yaml::from_str::<AgentManifest>(&content) {
                    Ok(m) => {
                        // Override existing manifest with same name
                        self.manifests.retain(|existing| existing.name != m.name);
                        self.manifests.push(m);
                        count += 1;
                    }
                    Err(e) => {
                        warn!(path = %path.display(), error = %e, "skipping invalid agent manifest");
                    }
                }
            }
        }
        Ok(count)
    }

    /// Get a manifest by name.
    pub fn get(&self, name: &str) -> Option<&AgentManifest> {
        self.manifests.iter().find(|m| m.name == name)
    }

    /// List all loaded manifests.
    pub fn list(&self) -> &[AgentManifest] {
        &self.manifests
    }
}

impl Default for ManifestLoader {
    fn default() -> Self { Self::new() }
}
```

**Step 4: Create YAML agent definitions**

`crates/core/kernel/src/process/defaults/scout.yaml`:
```yaml
name: scout
description: "Fast codebase recon — returns structured findings"
model: "deepseek/deepseek-chat"
tools:
  - read_file
  - grep
  - find_files
  - list_directory
  - http_fetch
max_iterations: 15
system_prompt: |
  You are a scout agent. Your job is to quickly investigate a codebase or topic
  and return compressed, structured findings.

  ## Rules
  - Be thorough but fast — read only what you need
  - Return findings as structured markdown with clear sections
  - Always include file paths and line numbers when referencing code
  - If you cannot find what was asked, say so clearly
```

`crates/core/kernel/src/process/defaults/planner.yaml`:
```yaml
name: planner
description: "Creates implementation plans from investigation results"
model: "deepseek/deepseek-chat"
tools:
  - read_file
  - grep
  - find_files
max_iterations: 10
system_prompt: |
  You are a planner agent. Given investigation results from a scout, create
  a clear implementation plan.

  ## Rules
  - Break work into small, numbered steps
  - Each step should specify exact files to modify
  - Include code snippets where helpful
  - Consider edge cases and testing
```

`crates/core/kernel/src/process/defaults/worker.yaml`:
```yaml
name: worker
description: "Executes implementation tasks from a plan"
model: "deepseek/deepseek-chat"
tools:
  - read_file
  - write_file
  - edit_file
  - bash
  - grep
  - find_files
max_iterations: 20
system_prompt: |
  You are a worker agent. Given an implementation plan, execute it step by step.

  ## Rules
  - Follow the plan exactly
  - Make minimal, focused changes
  - Test after each significant change
  - Report what you did clearly
```

**Step 5: Add `serde_yaml` to Cargo.toml, update lib.rs**

Add to `crates/core/kernel/Cargo.toml` dependencies: `serde_yaml = "0.9"`

Add to `crates/core/kernel/src/lib.rs`: `pub mod process;`

**Step 6: Write unit tests for ProcessTable and ManifestLoader**

Tests in `process/mod.rs` `#[cfg(test)]` module:
- `test_process_table_insert_get`
- `test_process_table_set_state`
- `test_process_table_children_of`
- `test_process_table_remove`
- `test_manifest_loader_bundled`
- `test_agent_manifest_yaml_roundtrip`
- `test_principal_roles`

**Step 7: Verify**

Run: `cargo check -p rara-kernel && cargo test -p rara-kernel -- process`

**Step 8: Commit**

```
feat(kernel): add OS process model types — AgentManifest, ProcessTable, Principal (#N)
```

---

## Task 2: KernelHandle Trait Hierarchy

**Files:**
- Create: `crates/core/kernel/src/handle/mod.rs`
- Create: `crates/core/kernel/src/handle/scoped.rs`
- Modify: `crates/core/kernel/src/lib.rs` — add `pub mod handle;`

**Step 1: Create KernelHandle trait hierarchy**

`crates/core/kernel/src/handle/mod.rs`:
```rust
pub mod scoped;

use async_trait::async_trait;
use tokio::sync::oneshot;

use crate::process::{AgentId, AgentManifest, AgentResult, ProcessInfo};
use crate::error::Result;

/// Handle returned from spawn — allows waiting for agent completion.
pub struct AgentHandle {
    pub agent_id: AgentId,
    pub result_rx: oneshot::Receiver<AgentResult>,
}

// ─── Subsystem traits ───

/// Process lifecycle management.
#[async_trait]
pub trait ProcessOps: Send + Sync {
    /// Spawn a child agent. Inherits current principal. Child tools ⊆ parent tools.
    async fn spawn(&self, manifest: AgentManifest, input: String) -> Result<AgentHandle>;

    /// Send a message to another agent and wait for the response.
    async fn send(&self, agent_id: AgentId, message: String) -> Result<String>;

    /// Query process state.
    fn status(&self, agent_id: AgentId) -> Result<ProcessInfo>;

    /// Kill an agent and its entire subtree.
    fn kill(&self, agent_id: AgentId) -> Result<()>;

    /// List child processes of the current agent.
    fn children(&self) -> Vec<ProcessInfo>;
}

/// Cross-agent shared memory operations.
pub trait MemoryOps: Send + Sync {
    fn mem_store(&self, key: &str, value: serde_json::Value) -> Result<()>;
    fn mem_recall(&self, key: &str) -> Result<Option<serde_json::Value>>;
}

/// Event bus operations.
#[async_trait]
pub trait EventOps: Send + Sync {
    async fn publish(&self, event_type: &str, payload: serde_json::Value) -> Result<()>;
}

/// Guard / approval operations.
#[async_trait]
pub trait GuardOps: Send + Sync {
    fn requires_approval(&self, tool_name: &str) -> bool;
    async fn request_approval(&self, tool_name: &str, summary: &str) -> Result<bool>;
}

/// Unified kernel handle — the single "syscall" interface for agents.
pub trait KernelHandle: ProcessOps + MemoryOps + EventOps + GuardOps {}
impl<T: ProcessOps + MemoryOps + EventOps + GuardOps> KernelHandle for T {}
```

**Step 2: Create ScopedKernelHandle stub**

`crates/core/kernel/src/handle/scoped.rs`:
```rust
use std::sync::Arc;
use async_trait::async_trait;
use tokio::sync::{oneshot, Semaphore};

use crate::error::{KernelError, Result};
use crate::process::{AgentId, AgentManifest, AgentResult, ProcessInfo, ProcessTable};
use crate::process::principal::Principal;
use super::{AgentHandle, ProcessOps, MemoryOps, EventOps, GuardOps};

/// Per-process scoped handle to kernel capabilities.
///
/// Each AgentProcess receives its own ScopedKernelHandle with:
/// - Its agent_id (so spawn auto-sets parent_id)
/// - Its principal (so child agents inherit identity)
/// - Its allowed tools (children can only subset these)
/// - Per-agent child semaphore (limits concurrent children)
pub struct ScopedKernelHandle {
    pub(crate) agent_id: AgentId,
    pub(crate) principal: Principal,
    pub(crate) allowed_tools: Vec<String>,
    pub(crate) child_semaphore: Arc<Semaphore>,
    // Arc to shared kernel internals — will be filled in Task 3
    pub(crate) inner: Arc<KernelInner>,
}

/// Shared kernel state that ScopedKernelHandle delegates to.
/// This is the "real" kernel implementation, shared by all handles.
pub(crate) struct KernelInner {
    pub process_table: ProcessTable,
    pub global_semaphore: Arc<Semaphore>,
    pub default_child_limit: usize,
    // More fields added in Task 3 (llm_provider, tool_registry, etc.)
}
```

**Step 3: Update lib.rs**

Add: `pub mod handle;`

**Step 4: Verify**

Run: `cargo check -p rara-kernel`

**Step 5: Commit**

```
feat(kernel): add KernelHandle trait hierarchy — ProcessOps, MemoryOps, EventOps, GuardOps (#N)
```

---

## Task 3: Kernel.spawn() Implementation

**Files:**
- Modify: `crates/core/kernel/src/kernel.rs` — rewrite with ProcessTable + spawn()
- Modify: `crates/core/kernel/src/handle/scoped.rs` — implement all traits
- Modify: `crates/core/kernel/src/error.rs` — add process error variants
- Modify: `crates/core/kernel/src/lib.rs` — update re-exports

**Step 1: Add error variants**

Add to `error.rs` KernelError enum:
```rust
#[snafu(display("process not found: {id}"))]
ProcessNotFound { id: String },

#[snafu(display("permission denied: {reason}"))]
PermissionDenied { reason: String },

#[snafu(display("spawn limit reached: {message}"))]
SpawnLimitReached { message: String },

#[snafu(display("tool not allowed: {tool_name}"))]
ToolNotAllowed { tool_name: String },
```

**Step 2: Rewrite kernel.rs**

The new Kernel struct:
```rust
pub struct Kernel {
    inner: Arc<KernelInner>,
    manifest_loader: ManifestLoader,
    config: KernelConfig,
}

pub struct KernelConfig {
    pub max_concurrency: usize,        // global "CPU cores"
    pub default_child_limit: usize,    // per-agent child limit
    pub default_max_iterations: usize, // default agent iterations
}
```

Key method: `Kernel::spawn()` which:
1. Validates principal permissions
2. Validates tool subset (child ⊆ parent)
3. Allocates AgentId
4. Creates AgentProcess in ProcessTable
5. Constructs ScopedKernelHandle for child
6. Builds filtered ToolRegistry (manifest.tools + kernel syscall tools)
7. Acquires global semaphore permit
8. Spawns tokio task running AgentRunner
9. Returns AgentHandle with oneshot receiver

**Step 3: Implement ScopedKernelHandle traits**

ProcessOps::spawn — delegates to kernel inner, auto-sets parent_id and principal
ProcessOps::kill — cascading kill of process subtree
MemoryOps — delegates to kernel's Memory trait
EventOps — delegates to kernel's EventBus
GuardOps — delegates to kernel's Guard trait with principal context

**Step 4: Write integration tests**

- `test_kernel_spawn_basic` — spawn a manifest, verify process table
- `test_kernel_spawn_child_limit` — verify per-agent semaphore
- `test_kernel_spawn_tool_subset` — child can only use parent's tools
- `test_kernel_kill_cascading` — kill parent kills children

**Step 5: Commit**

```
feat(kernel): implement Kernel.spawn() with dual semaphore and process tree (#N)
```

---

## Task 4: SpawnTool — LLM-callable Agent Spawning

**Files:**
- Create: `crates/core/kernel/src/handle/spawn_tool.rs`
- Modify: `crates/core/kernel/src/handle/mod.rs` — add `pub mod spawn_tool;`

**Step 1: Create SpawnTool**

An `AgentTool` implementation that LLMs call to spawn child agents. Supports:
- **Single**: `{"agent": "scout", "task": "Find auth code"}` — spawn named manifest
- **Dynamic**: `{"manifest": {...}, "task": "..."}` — spawn from inline manifest
- **Parallel**: `{"parallel": [...]}` — spawn multiple concurrently

The tool wraps a `Arc<dyn KernelHandle>` and calls `spawn()` for each request.

Unlike the old SubagentTool:
- Goes through Kernel (permission checks, process table, semaphore)
- Agents CAN recursively spawn (no hardcoded "subagent" exclusion)
- Recursion is controlled by semaphore limits, not tool filtering

**Step 2: Register SpawnTool during kernel construction**

The SpawnTool is automatically added to every agent's tool registry by the Kernel during spawn (unless the manifest explicitly excludes it).

**Step 3: Write tests**

- `test_spawn_tool_single_mode`
- `test_spawn_tool_parallel_mode`
- `test_spawn_tool_unknown_agent`

**Step 4: Commit**

```
feat(kernel): add SpawnTool — LLM-callable agent spawning via KernelHandle (#N)
```

---

## Task 5: Migration — Delete Old Code + Rewire

**Files to DELETE entirely:**
- `crates/core/kernel/src/subagent/` — entire directory
- `crates/core/kernel/src/dispatcher/` — entire directory
- `crates/core/kernel/src/registry.rs`
- `crates/core/kernel/src/context.rs`
- `crates/workers/src/task_executor.rs`

**Files to MODIFY:**
- `crates/core/kernel/src/lib.rs` — remove deleted modules, update re-exports
- `crates/core/kernel/src/agent_context.rs` — evaluate: keep (used by ChatAgent) or migrate
- `crates/workers/src/worker_state.rs` — rewire composition root:
  - Replace SubagentTool construction with SpawnTool via Kernel
  - Replace AgentDispatcher construction with Kernel.spawn()
  - Replace AgentContextImpl with Kernel-provided context
- `crates/workers/src/proactive.rs` — use Kernel.spawn(proactive_manifest, ...) instead of dispatcher.submit()
- `crates/workers/src/scheduled_agent.rs` — use Kernel.spawn(scheduled_manifest, ...) instead of dispatcher.submit()
- `crates/chat/src/agent.rs` — ChatAgent uses Kernel.spawn() or gets AgentRunner from Kernel
- Channel adapters — use Kernel.spawn() to handle incoming messages

**Step 1: Delete old modules**

Remove files, update `lib.rs` module declarations.

**Step 2: Update workers to use Kernel.spawn()**

ProactiveAgentWorker and AgentSchedulerWorker call `kernel.spawn(manifest, message, principal, session_id, None)` instead of `dispatcher.submit(task)`.

**Step 3: Update composition root (worker_state.rs)**

Replace:
- SubagentTool construction → Kernel constructs SpawnTool internally
- AgentDispatcher construction → Kernel handles scheduling
- AgentContextImpl → Kernel provides context via ScopedKernelHandle

**Step 4: Update ChatAgent/ChatService**

ChatAgent receives a `KernelHandle` (or the Kernel spawns chat agent processes directly).

**Step 5: Verify everything compiles**

Run: `cargo check` (full workspace)
Run: `cargo test -p rara-kernel`
Run: `cd web && npm run build`

**Step 6: Commit**

```
refactor(kernel): delete old agent abstractions, unify on Kernel OS model (#N)
```

---

## Issue Breakdown for Dispatch

| Issue | Tasks | Dependencies | Can Parallel? |
|-------|-------|-------------|---------------|
| Issue A | Task 1 + Task 2 | None | Yes (with nothing) |
| Issue B | Task 3 + Task 4 | Issue A | No (sequential) |
| Issue C | Task 5 | Issue B | No (sequential) |

**Recommended approach:** Dispatch Issue A first, then Issue B after merge, then Issue C.
