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

//! OS process model — core types for the unified agent lifecycle.
//!
//! This module implements an OS-inspired process model where:
//! - [`AgentManifest`] = the "binary" (static definition, YAML-loadable)
//! - [`AgentProcess`] = a running instance in the [`ProcessTable`]
//! - [`AgentId`] = unique per-execution identifier (like a PID)
//! - [`SessionId`] = persistent conversation identifier (survives restarts)
//!
//! The [`ProcessTable`] is a concurrent in-memory table (backed by `DashMap`)
//! that tracks all running agent processes, supporting process tree queries
//! (parent/children) and state transitions.

pub mod manifest_loader;
pub mod principal;
pub mod user;

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use dashmap::DashMap;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::error::Result;

// ---------------------------------------------------------------------------
// AgentRole
// ---------------------------------------------------------------------------

/// Classification of an agent's functional role.
///
/// Roles enable callers to look up agents by function rather than by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AgentRole {
    /// User-facing conversational agent (default chat entry point).
    Chat,
    /// Codebase recon / investigation agent.
    Scout,
    /// Task planning agent.
    Planner,
    /// Execution / coding agent.
    Worker,
}

impl std::fmt::Display for AgentRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentRole::Chat => write!(f, "chat"),
            AgentRole::Scout => write!(f, "scout"),
            AgentRole::Planner => write!(f, "planner"),
            AgentRole::Worker => write!(f, "worker"),
        }
    }
}

// ---------------------------------------------------------------------------
// Priority
// ---------------------------------------------------------------------------

/// Dispatch priority for agent messages.
///
/// Higher priority messages are processed before lower ones when the
/// scheduler is draining the inbound bus. Critical messages bypass rate
/// limiting entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
pub enum Priority {
    /// Background tasks, batch jobs.
    Low = 0,
    /// Default priority for interactive messages.
    #[default]
    Normal = 1,
    /// Elevated priority (e.g., admin requests).
    High = 2,
    /// System-critical messages (bypass rate limiting).
    Critical = 3,
}

impl std::fmt::Display for Priority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Priority::Low => write!(f, "low"),
            Priority::Normal => write!(f, "normal"),
            Priority::High => write!(f, "high"),
            Priority::Critical => write!(f, "critical"),
        }
    }
}

// ---------------------------------------------------------------------------
// SandboxConfig — file access whitelisting for agent processes
// ---------------------------------------------------------------------------

/// Configuration for agent file-system sandboxing.
///
/// Controls which file paths an agent process is allowed to access,
/// with support for read/write, read-only, and deny lists.
/// Deny rules take precedence over allow rules.
///
/// # YAML example
/// ```yaml
/// sandbox:
///   allowed_paths:
///     - /tmp/agent-workspace
///     - /data/shared
///   read_only_paths:
///     - /etc/config
///   denied_paths:
///     - /etc/secrets
///   isolated_workspace: true
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SandboxConfig {
    /// Allowed file paths (read/write). Path-prefix matching.
    #[serde(default)]
    pub allowed_paths:      Vec<String>,
    /// Read-only paths (reads allowed, writes denied). Path-prefix matching.
    #[serde(default)]
    pub read_only_paths:    Vec<String>,
    /// Denied paths (takes precedence over allowed and read-only).
    #[serde(default)]
    pub denied_paths:       Vec<String>,
    /// Whether to create an isolated temp workspace for this agent.
    #[serde(default)]
    pub isolated_workspace: bool,
}

/// Unique identifier for a running agent process.
///
/// Each spawned agent receives a fresh `AgentId` (UUID v4). This is analogous
/// to a PID in operating systems — it identifies one specific execution, not
/// the agent definition itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(pub Uuid);

impl AgentId {
    /// Generate a new random agent ID.
    pub fn new() -> Self { Self(Uuid::new_v4()) }
}

impl Default for AgentId {
    fn default() -> Self { Self::new() }
}

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{}", self.0) }
}

/// Persistent session identifier (survives across agent process restarts).
///
/// A session groups related conversations and memory. Multiple agent processes
/// can share the same session (e.g., a chat session that spawns sub-agents).
///
/// This is a type alias for [`SessionKey`](crate::session::SessionKey) to
/// minimize churn — existing code using `SessionId::new("...")` still
/// compiles because `SessionKey` provides the same constructor.
pub type SessionId = crate::session::SessionKey;

/// Agent "binary" — static definition, loadable from YAML or constructed
/// dynamically.
///
/// An `AgentManifest` defines *what* an agent is (its model, tools, prompt,
/// limits) but not *who* runs it or *when*. It is analogous to an executable
/// file on disk.
///
/// # YAML example
/// ```yaml
/// name: scout
/// description: "Fast codebase recon"
/// model: "deepseek/deepseek-chat"
/// system_prompt: "You are a scout agent..."
/// tools:
///   - read_file
///   - grep
/// max_iterations: 15
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentManifest {
    /// Unique name identifying this agent definition.
    pub name:           String,
    /// Agent's functional role (chat, scout, planner, worker).
    #[serde(default)]
    pub role:           Option<AgentRole>,
    /// Human-readable description.
    pub description:    String,
    /// LLM model identifier (e.g., "deepseek/deepseek-chat", "gpt-4").
    pub model:          String,
    /// System prompt defining agent behavior.
    pub system_prompt:  String,
    /// Optional personality/mood/voice prompt (prepended to system_prompt when building LLM messages).
    #[serde(default)]
    pub soul_prompt:    Option<String>,
    /// Optional hint for provider selection.
    #[serde(default)]
    pub provider_hint:  Option<String>,
    /// Maximum LLM iterations before forced completion.
    #[serde(default)]
    pub max_iterations: Option<usize>,
    /// Tool names this agent is allowed to use (empty = inherit parent's
    /// tools).
    #[serde(default)]
    pub tools:          Vec<String>,
    /// Maximum number of concurrent child agents this agent can spawn.
    #[serde(default)]
    pub max_children:        Option<usize>,
    /// Maximum context window size in tokens.
    ///
    /// When the in-memory conversation history exceeds this budget, the
    /// kernel applies a [`CompactionStrategy`](crate::memory::compaction::CompactionStrategy)
    /// to trim it before sending to the LLM. Defaults to
    /// [`DEFAULT_MAX_CONTEXT_TOKENS`](crate::memory::compaction::DEFAULT_MAX_CONTEXT_TOKENS)
    /// (8192) when `None`.
    #[serde(default)]
    pub max_context_tokens:  Option<usize>,
    /// Dispatch priority for scheduling.
    #[serde(default)]
    pub priority:            Priority,
    /// Arbitrary metadata for extension.
    #[serde(default)]
    pub metadata:            serde_json::Value,
    /// Optional sandbox configuration for file access control.
    #[serde(default)]
    pub sandbox:             Option<SandboxConfig>,
}

/// Runtime state of an agent process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ProcessState {
    /// Agent is actively running (LLM loop in progress).
    Running,
    /// Agent is waiting for child agent results (mailbox still open).
    Waiting,
    /// Agent is suspended by a Pause signal. Messages are buffered.
    Paused,
    /// Agent completed successfully.
    Completed,
    /// Agent failed with an error.
    Failed,
    /// Agent was cancelled (killed by parent or timeout).
    Cancelled,
}

/// Result of a completed agent process.
#[derive(Debug, Clone, Serialize)]
pub struct AgentResult {
    /// The agent's final output text.
    pub output:     String,
    /// Number of LLM iterations consumed.
    pub iterations: usize,
    /// Number of tool calls made.
    pub tool_calls: usize,
}

/// Process environment — isolated per-agent context.
///
/// Provides workspace path and environment variables that are scoped to
/// a specific agent process.
#[derive(Debug, Clone, Serialize)]
pub struct AgentEnv {
    /// Optional workspace directory for file operations.
    pub workspace: Option<String>,
    /// Key-value environment variables.
    pub vars:      HashMap<String, String>,
}

impl Default for AgentEnv {
    fn default() -> Self {
        Self {
            workspace: None,
            vars:      HashMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Signal — control signals for agent processes
// ---------------------------------------------------------------------------

/// Control signals for agent processes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// Interrupt the current operation (cancel in-flight LLM call).
    /// The process stays alive and waits for the next message.
    Interrupt,
    /// Graceful shutdown: finish current operation (with timeout), then exit.
    Terminate,
    /// Immediate termination via CancellationToken (already handled).
    Kill,
    /// Suspend message processing. Incoming messages are buffered.
    Pause,
    /// Resume message processing. Buffered messages are drained.
    Resume,
}

/// A running agent instance in the process table.
///
/// This is the runtime counterpart to [`AgentManifest`]. Each time an agent
/// is spawned, a new `AgentProcess` is created with a unique [`AgentId`],
/// the spawning principal, and the manifest that defines its behavior.
#[derive(Debug, Clone)]
pub struct AgentProcess {
    /// Unique identifier for this process.
    pub agent_id:           AgentId,
    /// Parent process (None for root-level agents).
    pub parent_id:          Option<AgentId>,
    /// The process's own session (always `agent:{agent_id}`), used for
    /// conversation storage. Each process gets an isolated session so
    /// subagents never load or pollute the parent's history.
    pub session_id:         SessionId,
    /// External channel binding (e.g., `web:chat123`). Only set for root
    /// processes that entered via an external channel adapter. Used by
    /// `session_index` for routing inbound messages to the correct process.
    /// Subagents have `None` — they are only reachable via `AgentHandle`.
    pub channel_session_id: Option<SessionId>,
    /// The agent definition driving this process.
    pub manifest:           AgentManifest,
    /// The identity under which this process runs.
    pub principal:     principal::Principal,
    /// Per-process environment.
    pub env:           AgentEnv,
    /// Current lifecycle state.
    pub state:         ProcessState,
    /// When this process was created.
    pub created_at:    Timestamp,
    /// When this process finished (if terminal).
    pub finished_at:   Option<Timestamp>,
    /// Result of execution (set on completion/failure).
    pub result:        Option<AgentResult>,
    /// Files created or modified by this agent (for resource tracking).
    pub created_files: Vec<PathBuf>,
}

/// Summary info for listing processes.
///
/// A lightweight view of an [`AgentProcess`] suitable for display in
/// process listings without exposing full internal state.
#[derive(Debug, Clone, Serialize)]
pub struct ProcessInfo {
    pub agent_id:   AgentId,
    pub parent_id:  Option<AgentId>,
    pub name:       String,
    pub state:      ProcessState,
    pub created_at: Timestamp,
}

impl From<&AgentProcess> for ProcessInfo {
    fn from(p: &AgentProcess) -> Self {
        Self {
            agent_id:   p.agent_id,
            parent_id:  p.parent_id,
            name:       p.manifest.name.clone(),
            state:      p.state,
            created_at: p.created_at,
        }
    }
}

// ---------------------------------------------------------------------------
// RuntimeMetrics — per-process atomic counters
// ---------------------------------------------------------------------------

/// Per-process runtime metrics using atomic counters for lock-free updates.
///
/// These counters are incremented during process execution and read when
/// building [`ProcessStats`] snapshots. Atomics avoid locking overhead on
/// the hot path (every LLM call, every tool call, every message).
pub struct RuntimeMetrics {
    /// Number of messages received by this process.
    pub messages_received: AtomicU64,
    /// Number of LLM completion calls made.
    pub llm_calls: AtomicU64,
    /// Number of tool calls executed.
    pub tool_calls: AtomicU64,
    /// Approximate total tokens consumed (prompt + completion).
    pub tokens_consumed: AtomicU64,
    /// Timestamp of the most recent activity.
    pub last_activity: Mutex<Option<Timestamp>>,
}

impl RuntimeMetrics {
    /// Create a new zeroed metrics instance.
    pub fn new() -> Self {
        Self {
            messages_received: AtomicU64::new(0),
            llm_calls:         AtomicU64::new(0),
            tool_calls:        AtomicU64::new(0),
            tokens_consumed:   AtomicU64::new(0),
            last_activity:     Mutex::new(None),
        }
    }

    /// Record a message received event.
    pub fn record_message(&self) {
        self.messages_received.fetch_add(1, Ordering::Relaxed);
    }

    /// Record an LLM call completion.
    pub fn record_llm_call(&self) {
        self.llm_calls.fetch_add(1, Ordering::Relaxed);
    }

    /// Record tool calls made during a turn.
    pub fn record_tool_calls(&self, count: u64) {
        self.tool_calls.fetch_add(count, Ordering::Relaxed);
    }

    /// Record tokens consumed.
    pub fn record_tokens(&self, count: u64) {
        self.tokens_consumed.fetch_add(count, Ordering::Relaxed);
    }

    /// Update the last activity timestamp to now.
    pub async fn touch(&self) {
        let mut guard = self.last_activity.lock().await;
        *guard = Some(Timestamp::now());
    }

    /// Take a snapshot of the current counters.
    pub async fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            messages_received: self.messages_received.load(Ordering::Relaxed),
            llm_calls:         self.llm_calls.load(Ordering::Relaxed),
            tool_calls:        self.tool_calls.load(Ordering::Relaxed),
            tokens_consumed:   self.tokens_consumed.load(Ordering::Relaxed),
            last_activity:     *self.last_activity.lock().await,
        }
    }
}

impl Default for RuntimeMetrics {
    fn default() -> Self { Self::new() }
}

impl std::fmt::Debug for RuntimeMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeMetrics")
            .field("messages_received", &self.messages_received.load(Ordering::Relaxed))
            .field("llm_calls", &self.llm_calls.load(Ordering::Relaxed))
            .field("tool_calls", &self.tool_calls.load(Ordering::Relaxed))
            .field("tokens_consumed", &self.tokens_consumed.load(Ordering::Relaxed))
            .finish()
    }
}

/// Point-in-time snapshot of [`RuntimeMetrics`] (all plain values, no atomics).
#[derive(Debug, Clone, Serialize)]
pub struct MetricsSnapshot {
    pub messages_received: u64,
    pub llm_calls:         u64,
    pub tool_calls:        u64,
    pub tokens_consumed:   u64,
    pub last_activity:     Option<Timestamp>,
}

// ---------------------------------------------------------------------------
// ProcessStats — rich per-process introspection (like /proc/<pid>/status)
// ---------------------------------------------------------------------------

/// Extended runtime statistics for a single agent process.
///
/// Combines static metadata from [`AgentProcess`] with live counters from
/// [`RuntimeMetrics`]. This is the `/proc/<pid>/status` equivalent.
#[derive(Debug, Clone, Serialize)]
pub struct ProcessStats {
    /// Unique process identifier.
    pub agent_id:          AgentId,
    /// Session this process belongs to.
    pub session_id:        SessionId,
    /// The manifest name (agent definition name).
    pub manifest_name:     String,
    /// Current lifecycle state.
    pub state:             ProcessState,
    /// Parent process, if any.
    pub parent_id:         Option<AgentId>,
    /// IDs of child processes.
    pub children:          Vec<AgentId>,
    /// When this process was created.
    pub created_at:        Timestamp,
    /// How long this process has been alive (milliseconds).
    pub uptime_ms:         u64,
    // -- Runtime metrics --
    /// Number of messages received by this process.
    pub messages_received: u64,
    /// Number of LLM completion calls made.
    pub llm_calls:         u64,
    /// Number of tool calls executed.
    pub tool_calls:        u64,
    /// Approximate total tokens consumed.
    pub tokens_consumed:   u64,
    /// Timestamp of the most recent activity.
    pub last_activity:     Option<Timestamp>,
}

// ---------------------------------------------------------------------------
// SystemStats — kernel-wide aggregate metrics (like /proc/stat)
// ---------------------------------------------------------------------------

/// Kernel-wide aggregate statistics.
///
/// Provides a high-level overview of the kernel's current state, analogous
/// to `/proc/stat` or `/proc/meminfo` in Linux.
#[derive(Debug, Clone, Serialize)]
pub struct SystemStats {
    /// Number of currently active (Running or Waiting) processes.
    pub active_processes:          usize,
    /// Total number of processes ever spawned.
    pub total_spawned:             u64,
    /// Total number of processes that completed successfully.
    pub total_completed:           u64,
    /// Total number of processes that failed.
    pub total_failed:              u64,
    /// Number of global semaphore permits currently available.
    pub global_semaphore_available: usize,
    /// Sum of tokens consumed across all tracked processes.
    pub total_tokens_consumed:     u64,
    /// Kernel uptime in milliseconds.
    pub uptime_ms:                 u64,
}

/// In-memory process table — the kernel's view of all running agents.
///
/// Thread-safe via `DashMap`. Supports concurrent reads and writes from
/// multiple tokio tasks (e.g., kernel spawn + agent tool calls).
///
/// Includes a session index for fast `SessionId -> AgentId` lookups,
/// a name index for fast agent name -> `AgentId` lookups, and
/// a mailbox registry for sending messages to long-lived processes.
pub struct ProcessTable {
    processes:     DashMap<AgentId, AgentProcess>,
    /// Maps a session to its currently active agent process.
    session_index: DashMap<SessionId, AgentId>,
    /// Maps agent manifest name to its currently active agent process.
    /// Used for sender-addressed message routing (target_agent → process).
    name_index:    DashMap<String, AgentId>,
    /// Per-process runtime metrics (atomic counters for lock-free updates).
    metrics:       DashMap<AgentId, std::sync::Arc<RuntimeMetrics>>,
    /// Monotonically increasing counter of total processes ever spawned.
    total_spawned:    AtomicU64,
    /// Total processes that completed successfully.
    total_completed:  AtomicU64,
    /// Total processes that failed.
    total_failed:     AtomicU64,
}

impl ProcessTable {
    /// Create an empty process table.
    pub fn new() -> Self {
        Self {
            processes:           DashMap::new(),
            session_index:       DashMap::new(),
            name_index:          DashMap::new(),
            metrics:             DashMap::new(),
            total_spawned:       AtomicU64::new(0),
            total_completed:     AtomicU64::new(0),
            total_failed:        AtomicU64::new(0),
        }
    }

    /// Insert a process into the table.
    ///
    /// If the process has a `channel_session_id`, the session index is updated
    /// so `find_by_session` can route inbound channel messages to it.
    /// Subagents (`channel_session_id = None`) are **not** inserted into
    /// `session_index` — they are only reachable via `AgentHandle`.
    ///
    /// Updates the name index so `find_by_name` can locate it by manifest
    /// name, creates a [`RuntimeMetrics`] entry, and increments the total
    /// spawned counter.
    ///
    /// If a process with the same manifest name already exists in the name
    /// index, it is overwritten (the old entry may be stale from a process
    /// that ended but was not cleaned up).
    pub fn insert(&self, process: AgentProcess) {
        let agent_id = process.agent_id;
        if let Some(ref channel_sid) = process.channel_session_id {
            self.session_index.insert(channel_sid.clone(), agent_id);
        }
        self.name_index
            .insert(process.manifest.name.clone(), agent_id);
        self.metrics
            .insert(agent_id, std::sync::Arc::new(RuntimeMetrics::new()));
        self.total_spawned.fetch_add(1, Ordering::Relaxed);
        self.processes.insert(agent_id, process);
    }

    /// Get a clone of a process by ID.
    pub fn get(&self, id: AgentId) -> Option<AgentProcess> {
        self.processes.get(&id).map(|p| p.value().clone())
    }

    /// Transition a process to a new state.
    ///
    /// Automatically sets `finished_at` when transitioning to a terminal state
    /// and increments aggregate completed/failed counters.
    pub fn set_state(&self, id: AgentId, state: ProcessState) -> Result<()> {
        let mut entry = self
            .processes
            .get_mut(&id)
            .ok_or(crate::error::KernelError::AgentNotFound { id: id.0 })?;
        let prev_state = entry.state;
        entry.state = state;
        match state {
            ProcessState::Completed => {
                entry.finished_at = Some(Timestamp::now());
                // Only count transition once (guard against double-set).
                if prev_state != ProcessState::Completed {
                    self.total_completed.fetch_add(1, Ordering::Relaxed);
                }
            }
            ProcessState::Failed => {
                entry.finished_at = Some(Timestamp::now());
                if prev_state != ProcessState::Failed {
                    self.total_failed.fetch_add(1, Ordering::Relaxed);
                }
            }
            ProcessState::Cancelled => {
                entry.finished_at = Some(Timestamp::now());
            }
            ProcessState::Running | ProcessState::Waiting | ProcessState::Paused => {
                // Non-terminal states: do not set finished_at.
            }
        }
        Ok(())
    }

    /// Set the result of a process.
    pub fn set_result(&self, id: AgentId, result: AgentResult) -> Result<()> {
        let mut entry = self
            .processes
            .get_mut(&id)
            .ok_or(crate::error::KernelError::AgentNotFound { id: id.0 })?;
        entry.result = Some(result);
        Ok(())
    }

    /// Remove a process from the table, returning it if it existed.
    ///
    /// Also cleans up the session index (if the process had a channel
    /// binding), name index, and metrics entries.
    pub fn remove(&self, id: AgentId) -> Option<AgentProcess> {
        let removed = self.processes.remove(&id).map(|(_, p)| p);
        if let Some(ref process) = removed {
            if let Some(ref channel_sid) = process.channel_session_id {
                self.session_index
                    .remove_if(channel_sid, |_, agent_id| *agent_id == id);
            }
            self.name_index
                .remove_if(&process.manifest.name, |_, agent_id| *agent_id == id);
            self.metrics.remove(&id);
        }
        removed
    }

    /// List all children of a given parent.
    pub fn children_of(&self, parent_id: AgentId) -> Vec<ProcessInfo> {
        self.processes
            .iter()
            .filter(|p| p.parent_id == Some(parent_id))
            .map(|p| ProcessInfo::from(p.value()))
            .collect()
    }

    /// List all processes.
    pub fn list(&self) -> Vec<ProcessInfo> {
        self.processes
            .iter()
            .map(|p| ProcessInfo::from(p.value()))
            .collect()
    }

    /// Count running processes.
    pub fn running_count(&self) -> usize {
        self.processes
            .iter()
            .filter(|p| p.state == ProcessState::Running)
            .count()
    }

    // ----- Session index methods -----

    /// Find the active agent process for a session.
    pub fn find_by_session(&self, session_id: &SessionId) -> Option<AgentProcess> {
        let agent_id = self.session_index.get(session_id)?;
        self.get(*agent_id)
    }

    /// Find the active agent process by manifest name.
    pub fn find_by_name(&self, name: &str) -> Option<AgentProcess> {
        let agent_id = self.name_index.get(name)?;
        self.get(*agent_id)
    }

    /// Bind a session to a specific agent process (overwrites any existing
    /// binding).
    pub fn bind_session(&self, session_id: SessionId, agent_id: AgentId) {
        self.session_index.insert(session_id, agent_id);
    }

    // ----- Metrics methods -----

    /// Get a shared reference to the metrics for a process.
    pub fn get_metrics(&self, id: &AgentId) -> Option<std::sync::Arc<RuntimeMetrics>> {
        self.metrics.get(id).map(|m| std::sync::Arc::clone(m.value()))
    }

    /// Build a [`ProcessStats`] snapshot for a single process.
    ///
    /// Combines static process metadata with live runtime metrics.
    pub async fn process_stats(&self, id: AgentId) -> Option<ProcessStats> {
        let process = self.get(id)?;
        let metrics_snapshot = if let Some(m) = self.get_metrics(&id) {
            m.snapshot().await
        } else {
            MetricsSnapshot {
                messages_received: 0,
                llm_calls:         0,
                tool_calls:        0,
                tokens_consumed:   0,
                last_activity:     None,
            }
        };
        let children: Vec<AgentId> = self
            .processes
            .iter()
            .filter(|p| p.parent_id == Some(id))
            .map(|p| p.agent_id)
            .collect();
        let uptime_ms = Timestamp::now()
            .since(process.created_at)
            .ok()
            .map(|span| span.get_milliseconds().unsigned_abs())
            .unwrap_or(0);

        Some(ProcessStats {
            agent_id:          process.agent_id,
            session_id:        process.session_id,
            manifest_name:     process.manifest.name,
            state:             process.state,
            parent_id:         process.parent_id,
            children,
            created_at:        process.created_at,
            uptime_ms,
            messages_received: metrics_snapshot.messages_received,
            llm_calls:         metrics_snapshot.llm_calls,
            tool_calls:        metrics_snapshot.tool_calls,
            tokens_consumed:   metrics_snapshot.tokens_consumed,
            last_activity:     metrics_snapshot.last_activity,
        })
    }

    /// Build [`ProcessStats`] for all processes currently in the table.
    pub async fn all_process_stats(&self) -> Vec<ProcessStats> {
        let ids: Vec<AgentId> = self.processes.iter().map(|p| p.agent_id).collect();
        let mut stats = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(s) = self.process_stats(id).await {
                stats.push(s);
            }
        }
        stats
    }

    /// Get the total number of processes ever spawned.
    pub fn total_spawned(&self) -> u64 {
        self.total_spawned.load(Ordering::Relaxed)
    }

    /// Get the total number of processes that completed successfully.
    pub fn total_completed(&self) -> u64 {
        self.total_completed.load(Ordering::Relaxed)
    }

    /// Get the total number of processes that failed.
    pub fn total_failed(&self) -> u64 {
        self.total_failed.load(Ordering::Relaxed)
    }

    /// Sum of tokens consumed across all currently tracked processes.
    pub fn total_tokens_consumed(&self) -> u64 {
        self.metrics
            .iter()
            .map(|m| m.value().tokens_consumed.load(Ordering::Relaxed))
            .sum()
    }

}

impl Default for ProcessTable {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::principal::Principal;

    /// Helper to create a test manifest.
    fn test_manifest(name: &str) -> AgentManifest {
        AgentManifest {
            name:           name.to_string(),
        role:           None,
            description:    format!("Test agent: {name}"),
            model:          "test-model".to_string(),
            system_prompt:  "You are a test agent.".to_string(),
            soul_prompt:    None,
            provider_hint:  None,
            max_iterations: Some(10),
            tools:          vec!["read_file".to_string()],
            max_children:        None,
            max_context_tokens:  None,
            priority:            Priority::default(),
            metadata:            serde_json::Value::Null,
            sandbox:             None,
        }
    }

    /// Helper to create a test process (with a default channel session).
    fn test_process(name: &str, parent_id: Option<AgentId>) -> AgentProcess {
        test_process_with_session(name, parent_id, "test-session")
    }

    /// Helper to create a test process simulating a subagent (no channel session).
    fn test_subagent(name: &str, parent_id: AgentId) -> AgentProcess {
        let agent_id = AgentId::new();
        AgentProcess {
            agent_id,
            parent_id:          Some(parent_id),
            session_id:         SessionId::new(format!("agent:{}", agent_id)),
            channel_session_id: None,
            manifest:           test_manifest(name),
            principal:          Principal::user("test-user"),
            env:                AgentEnv::default(),
            state:              ProcessState::Running,
            created_at:         Timestamp::now(),
            finished_at:        None,
            result:             None,
            created_files:      vec![],
        }
    }

    /// Helper to create a test process with a specific channel session ID.
    fn test_process_with_session(
        name: &str,
        parent_id: Option<AgentId>,
        channel_session: &str,
    ) -> AgentProcess {
        let agent_id = AgentId::new();
        AgentProcess {
            agent_id,
            parent_id,
            session_id:         SessionId::new(format!("agent:{}", agent_id)),
            channel_session_id: Some(SessionId::new(channel_session)),
            manifest:           test_manifest(name),
            principal:          Principal::user("test-user"),
            env:                AgentEnv::default(),
            state:              ProcessState::Running,
            created_at:         Timestamp::now(),
            finished_at:        None,
            result:             None,
            created_files:      vec![],
        }
    }

    #[test]
    fn test_agent_id_display() {
        let id = AgentId::new();
        let display = id.to_string();
        // UUID format: 8-4-4-4-12
        assert_eq!(display.len(), 36);
        assert!(display.contains('-'));
    }

    #[test]
    fn test_session_id() {
        let sid = SessionId::new("my-session");
        assert_eq!(sid.to_string(), "my-session");
        assert_eq!(sid.as_str(), "my-session");
    }

    #[test]
    fn test_process_table_insert_get() {
        let table = ProcessTable::new();
        let process = test_process("scout", None);
        let id = process.agent_id;

        table.insert(process);

        let retrieved = table.get(id).unwrap();
        assert_eq!(retrieved.agent_id, id);
        assert_eq!(retrieved.manifest.name, "scout");
        assert_eq!(retrieved.state, ProcessState::Running);
    }

    #[test]
    fn test_process_table_get_nonexistent() {
        let table = ProcessTable::new();
        assert!(table.get(AgentId::new()).is_none());
    }

    #[test]
    fn test_process_table_set_state() {
        let table = ProcessTable::new();
        let process = test_process("scout", None);
        let id = process.agent_id;
        table.insert(process);

        // Transition to Completed
        table.set_state(id, ProcessState::Completed).unwrap();

        let p = table.get(id).unwrap();
        assert_eq!(p.state, ProcessState::Completed);
        assert!(p.finished_at.is_some());
    }

    #[test]
    fn test_process_table_set_state_nonexistent() {
        let table = ProcessTable::new();
        let result = table.set_state(AgentId::new(), ProcessState::Failed);
        assert!(result.is_err());
    }

    #[test]
    fn test_process_table_set_result() {
        let table = ProcessTable::new();
        let process = test_process("worker", None);
        let id = process.agent_id;
        table.insert(process);

        let result = AgentResult {
            output:     "done".to_string(),
            iterations: 5,
            tool_calls: 3,
        };
        table.set_result(id, result).unwrap();

        let p = table.get(id).unwrap();
        let r = p.result.unwrap();
        assert_eq!(r.output, "done");
        assert_eq!(r.iterations, 5);
        assert_eq!(r.tool_calls, 3);
    }

    #[test]
    fn test_process_table_remove() {
        let table = ProcessTable::new();
        let process = test_process("scout", None);
        let id = process.agent_id;
        table.insert(process);

        let removed = table.remove(id);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().agent_id, id);

        // Should be gone now
        assert!(table.get(id).is_none());
    }

    #[test]
    fn test_process_table_remove_nonexistent() {
        let table = ProcessTable::new();
        assert!(table.remove(AgentId::new()).is_none());
    }

    #[test]
    fn test_process_table_children_of() {
        let table = ProcessTable::new();

        let parent = test_process("planner", None);
        let parent_id = parent.agent_id;
        table.insert(parent);

        let child1 = test_process("worker-1", Some(parent_id));
        let child1_id = child1.agent_id;
        table.insert(child1);

        let child2 = test_process("worker-2", Some(parent_id));
        let child2_id = child2.agent_id;
        table.insert(child2);

        // Unrelated process (no parent)
        let unrelated = test_process("scout", None);
        table.insert(unrelated);

        let children = table.children_of(parent_id);
        assert_eq!(children.len(), 2);

        let child_ids: Vec<AgentId> = children.iter().map(|c| c.agent_id).collect();
        assert!(child_ids.contains(&child1_id));
        assert!(child_ids.contains(&child2_id));
    }

    #[test]
    fn test_process_table_list() {
        let table = ProcessTable::new();
        assert!(table.list().is_empty());

        table.insert(test_process("a", None));
        table.insert(test_process("b", None));
        table.insert(test_process("c", None));

        assert_eq!(table.list().len(), 3);
    }

    #[test]
    fn test_process_table_running_count() {
        let table = ProcessTable::new();
        assert_eq!(table.running_count(), 0);

        let p1 = test_process("a", None);
        let p1_id = p1.agent_id;
        table.insert(p1);

        let p2 = test_process("b", None);
        table.insert(p2);

        assert_eq!(table.running_count(), 2);

        table.set_state(p1_id, ProcessState::Completed).unwrap();
        assert_eq!(table.running_count(), 1);
    }

    #[test]
    fn test_process_info_from_agent_process() {
        let process = test_process("scout", None);
        let info = ProcessInfo::from(&process);

        assert_eq!(info.agent_id, process.agent_id);
        assert_eq!(info.parent_id, None);
        assert_eq!(info.name, "scout");
        assert_eq!(info.state, ProcessState::Running);
    }

    #[test]
    fn test_agent_manifest_yaml_roundtrip() {
        let manifest = test_manifest("roundtrip");
        let yaml = serde_yaml::to_string(&manifest).unwrap();
        let deserialized: AgentManifest = serde_yaml::from_str(&yaml).unwrap();

        assert_eq!(deserialized.name, "roundtrip");
        assert_eq!(deserialized.model, "test-model");
        assert_eq!(deserialized.max_iterations, Some(10));
        assert_eq!(deserialized.tools, vec!["read_file"]);
    }

    #[test]
    fn test_agent_manifest_yaml_minimal() {
        let yaml = r#"
name: minimal
description: "Minimal agent"
model: "gpt-4"
system_prompt: "Hello"
"#;
        let m: AgentManifest = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(m.name, "minimal");
        assert!(m.tools.is_empty());
        assert!(m.max_iterations.is_none());
        assert!(m.provider_hint.is_none());
        assert!(m.max_children.is_none());
    }

    #[test]
    fn test_agent_env_default() {
        let env = AgentEnv::default();
        assert!(env.workspace.is_none());
        assert!(env.vars.is_empty());
    }

    #[test]
    fn test_process_state_terminal_sets_finished_at() {
        let table = ProcessTable::new();

        // Test Failed
        let p = test_process("a", None);
        let id = p.agent_id;
        table.insert(p);
        table.set_state(id, ProcessState::Failed).unwrap();
        assert!(table.get(id).unwrap().finished_at.is_some());

        // Test Cancelled
        let p = test_process("b", None);
        let id = p.agent_id;
        table.insert(p);
        table.set_state(id, ProcessState::Cancelled).unwrap();
        assert!(table.get(id).unwrap().finished_at.is_some());
    }

    #[test]
    fn test_process_state_waiting_does_not_set_finished_at() {
        let table = ProcessTable::new();
        let p = test_process("waiter", None);
        let id = p.agent_id;
        table.insert(p);

        table.set_state(id, ProcessState::Waiting).unwrap();

        let process = table.get(id).unwrap();
        assert_eq!(process.state, ProcessState::Waiting);
        assert!(
            process.finished_at.is_none(),
            "Waiting state should not set finished_at"
        );
    }

    #[test]
    fn test_process_state_paused_does_not_set_finished_at() {
        let table = ProcessTable::new();
        let p = test_process("paused-agent", None);
        let id = p.agent_id;
        table.insert(p);

        table.set_state(id, ProcessState::Paused).unwrap();

        let process = table.get(id).unwrap();
        assert_eq!(process.state, ProcessState::Paused);
        assert!(
            process.finished_at.is_none(),
            "Paused state should not set finished_at"
        );
    }

    #[test]
    fn test_signal_variants() {
        // Verify all signal variants exist and are comparable.
        assert_eq!(Signal::Interrupt, Signal::Interrupt);
        assert_eq!(Signal::Terminate, Signal::Terminate);
        assert_eq!(Signal::Kill, Signal::Kill);
        assert_eq!(Signal::Pause, Signal::Pause);
        assert_eq!(Signal::Resume, Signal::Resume);
        assert_ne!(Signal::Interrupt, Signal::Terminate);
        assert_ne!(Signal::Pause, Signal::Resume);
    }

    #[test]
    fn test_process_table_session_index() {
        let table = ProcessTable::new();

        // Insert creates a session index entry (via channel_session_id)
        let p = test_process("agent-a", None);
        let agent_id = p.agent_id;
        let channel_sid = p.channel_session_id.clone().unwrap();
        table.insert(p);

        // find_by_session should return the process
        let found = table.find_by_session(&channel_sid);
        assert!(found.is_some());
        assert_eq!(found.unwrap().agent_id, agent_id);

        // bind_session overwrites
        let new_id = AgentId::new();
        let new_process = AgentProcess {
            agent_id:           new_id,
            parent_id:          None,
            session_id:         SessionId::new(format!("agent:{}", new_id)),
            channel_session_id: Some(channel_sid.clone()),
            manifest:           test_manifest("agent-b"),
            principal:          Principal::user("test-user"),
            env:                AgentEnv::default(),
            state:              ProcessState::Running,
            created_at:         Timestamp::now(),
            finished_at:        None,
            result:             None,
            created_files:      vec![],
        };
        table.insert(new_process);
        table.bind_session(channel_sid.clone(), new_id);

        let found = table.find_by_session(&channel_sid);
        assert!(found.is_some());
        assert_eq!(found.unwrap().agent_id, new_id);
    }

    #[test]
    fn test_process_table_remove_clears_session_index() {
        let table = ProcessTable::new();
        let p = test_process("removable", None);
        let agent_id = p.agent_id;
        let channel_sid = p.channel_session_id.clone().unwrap();
        table.insert(p);

        // Session index should have an entry (via channel_session_id)
        assert!(table.find_by_session(&channel_sid).is_some());

        // Remove should clear the session index
        table.remove(agent_id);
        assert!(table.find_by_session(&channel_sid).is_none());
    }

    // -----------------------------------------------------------------------
    // /proc API tests — RuntimeMetrics, ProcessStats, aggregate counters
    // -----------------------------------------------------------------------

    #[test]
    fn test_runtime_metrics_record_message() {
        let m = RuntimeMetrics::new();
        assert_eq!(m.messages_received.load(Ordering::Relaxed), 0);
        m.record_message();
        m.record_message();
        assert_eq!(m.messages_received.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn test_runtime_metrics_record_llm_and_tools() {
        let m = RuntimeMetrics::new();
        m.record_llm_call();
        m.record_tool_calls(3);
        m.record_tokens(100);
        assert_eq!(m.llm_calls.load(Ordering::Relaxed), 1);
        assert_eq!(m.tool_calls.load(Ordering::Relaxed), 3);
        assert_eq!(m.tokens_consumed.load(Ordering::Relaxed), 100);
    }

    #[tokio::test]
    async fn test_runtime_metrics_touch_and_snapshot() {
        let m = RuntimeMetrics::new();
        // Initially no last_activity
        let snap = m.snapshot().await;
        assert!(snap.last_activity.is_none());

        // Touch updates last_activity
        m.touch().await;
        let snap = m.snapshot().await;
        assert!(snap.last_activity.is_some());
    }

    #[test]
    fn test_insert_creates_metrics() {
        let table = ProcessTable::new();
        let p = test_process("metrics-test", None);
        let id = p.agent_id;
        table.insert(p);

        let metrics = table.get_metrics(&id);
        assert!(metrics.is_some());
        let m = metrics.unwrap();
        assert_eq!(m.messages_received.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_insert_increments_total_spawned() {
        let table = ProcessTable::new();
        assert_eq!(table.total_spawned(), 0);

        table.insert(test_process("a", None));
        assert_eq!(table.total_spawned(), 1);

        table.insert(test_process("b", None));
        assert_eq!(table.total_spawned(), 2);
    }

    #[test]
    fn test_set_state_increments_completed_and_failed() {
        let table = ProcessTable::new();

        let p1 = test_process("ok", None);
        let id1 = p1.agent_id;
        table.insert(p1);

        let p2 = test_process("fail", None);
        let id2 = p2.agent_id;
        table.insert(p2);

        table.set_state(id1, ProcessState::Completed).unwrap();
        assert_eq!(table.total_completed(), 1);
        assert_eq!(table.total_failed(), 0);

        table.set_state(id2, ProcessState::Failed).unwrap();
        assert_eq!(table.total_completed(), 1);
        assert_eq!(table.total_failed(), 1);
    }

    #[test]
    fn test_set_state_does_not_double_count() {
        let table = ProcessTable::new();
        let p = test_process("double", None);
        let id = p.agent_id;
        table.insert(p);

        table.set_state(id, ProcessState::Completed).unwrap();
        table.set_state(id, ProcessState::Completed).unwrap();
        assert_eq!(table.total_completed(), 1, "should not double-count");
    }

    #[test]
    fn test_remove_clears_metrics() {
        let table = ProcessTable::new();
        let p = test_process("rm-metrics", None);
        let id = p.agent_id;
        table.insert(p);

        assert!(table.get_metrics(&id).is_some());
        table.remove(id);
        assert!(table.get_metrics(&id).is_none());
    }

    #[tokio::test]
    async fn test_process_stats_populated_correctly() {
        let table = ProcessTable::new();

        let parent = test_process("planner", None);
        let parent_id = parent.agent_id;
        table.insert(parent);

        let child = test_process("worker", Some(parent_id));
        let child_id = child.agent_id;
        table.insert(child);

        // Record some metrics on the parent
        let metrics = table.get_metrics(&parent_id).unwrap();
        metrics.record_message();
        metrics.record_llm_call();
        metrics.record_tool_calls(2);
        metrics.record_tokens(500);
        metrics.touch().await;

        let stats = table.process_stats(parent_id).await.unwrap();
        assert_eq!(stats.agent_id, parent_id);
        assert_eq!(stats.manifest_name, "planner");
        assert_eq!(stats.state, ProcessState::Running);
        assert!(stats.parent_id.is_none());
        assert_eq!(stats.children, vec![child_id]);
        assert_eq!(stats.messages_received, 1);
        assert_eq!(stats.llm_calls, 1);
        assert_eq!(stats.tool_calls, 2);
        assert_eq!(stats.tokens_consumed, 500);
        assert!(stats.last_activity.is_some());
        // uptime should be a reasonable value (just check it's been set)
        let _ = stats.uptime_ms;
    }

    #[tokio::test]
    async fn test_process_stats_nonexistent_returns_none() {
        let table = ProcessTable::new();
        assert!(table.process_stats(AgentId::new()).await.is_none());
    }

    #[tokio::test]
    async fn test_all_process_stats_returns_all() {
        let table = ProcessTable::new();
        table.insert(test_process("a", None));
        table.insert(test_process("b", None));
        table.insert(test_process("c", None));

        let all = table.all_process_stats().await;
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_total_tokens_consumed_aggregates() {
        let table = ProcessTable::new();

        let p1 = test_process("a", None);
        let id1 = p1.agent_id;
        table.insert(p1);

        let p2 = test_process("b", None);
        let id2 = p2.agent_id;
        table.insert(p2);

        table.get_metrics(&id1).unwrap().record_tokens(100);
        table.get_metrics(&id2).unwrap().record_tokens(200);

        assert_eq!(table.total_tokens_consumed(), 300);
    }

    #[tokio::test]
    async fn test_process_stats_serializes_to_json() {
        let table = ProcessTable::new();
        let p = test_process("json-test", None);
        let id = p.agent_id;
        table.insert(p);

        let stats = table.process_stats(id).await.unwrap();
        let json = serde_json::to_string(&stats).unwrap();
        assert!(json.contains("\"manifest_name\":\"json-test\""));
        assert!(json.contains("\"messages_received\":0"));
    }

    // -----------------------------------------------------------------------
    // Name index tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_find_by_name_after_insert() {
        let table = ProcessTable::new();
        let p = test_process("my-agent", None);
        let id = p.agent_id;
        table.insert(p);

        let found = table.find_by_name("my-agent");
        assert!(found.is_some());
        assert_eq!(found.unwrap().agent_id, id);

        // Non-existent name returns None.
        assert!(table.find_by_name("no-such-agent").is_none());
    }

    #[test]
    fn test_name_index_cleanup_on_remove() {
        let table = ProcessTable::new();
        let p = test_process("removable-agent", None);
        let id = p.agent_id;
        table.insert(p);

        assert!(table.find_by_name("removable-agent").is_some());

        table.remove(id);
        assert!(table.find_by_name("removable-agent").is_none());
    }

    #[test]
    fn test_name_index_overwrite_on_duplicate() {
        let table = ProcessTable::new();

        let p1 = test_process("dup-agent", None);
        let id1 = p1.agent_id;
        table.insert(p1);

        assert_eq!(table.find_by_name("dup-agent").unwrap().agent_id, id1);

        // Insert a second process with the same manifest name.
        let p2 = test_process("dup-agent", None);
        let id2 = p2.agent_id;
        table.insert(p2);

        // Name index should point to the new process.
        assert_eq!(table.find_by_name("dup-agent").unwrap().agent_id, id2);
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_remove_does_not_clear_name_index_for_different_agent() {
        let table = ProcessTable::new();

        let p1 = test_process("shared-name", None);
        let id1 = p1.agent_id;
        table.insert(p1);

        // Overwrite name index with p2
        let p2 = test_process("shared-name", None);
        let id2 = p2.agent_id;
        table.insert(p2);

        // Remove p1 — should NOT clear name_index because it points to p2.
        table.remove(id1);
        assert!(
            table.find_by_name("shared-name").is_some(),
            "name_index should still point to p2"
        );
        assert_eq!(table.find_by_name("shared-name").unwrap().agent_id, id2);

        // Remove p2 — now name_index should be cleared.
        table.remove(id2);
        assert!(table.find_by_name("shared-name").is_none());
    }

    // -----------------------------------------------------------------------
    // Multi-session tests — same agent manifest, multiple process instances
    // -----------------------------------------------------------------------

    #[test]
    fn test_same_agent_multiple_sessions() {
        let table = ProcessTable::new();

        // Spawn "rara" for channel session-1.
        let p1 = test_process_with_session("rara", None, "session-1");
        let id1 = p1.agent_id;
        let csid1 = p1.channel_session_id.clone().unwrap();
        table.insert(p1);

        // Spawn "rara" for channel session-2.
        let p2 = test_process_with_session("rara", None, "session-2");
        let id2 = p2.agent_id;
        let csid2 = p2.channel_session_id.clone().unwrap();
        table.insert(p2);

        // Two different AgentProcess instances exist.
        assert_ne!(id1, id2);
        assert_eq!(table.list().len(), 2);

        // Session index routes by channel_session_id to the correct instance.
        assert_eq!(table.find_by_session(&csid1).unwrap().agent_id, id1);
        assert_eq!(table.find_by_session(&csid2).unwrap().agent_id, id2);

        // Name index only points to the most recent insertion.
        assert_eq!(table.find_by_name("rara").unwrap().agent_id, id2);

        // Both processes can be queried independently.
        let p1_info = table.get(id1).unwrap();
        let p2_info = table.get(id2).unwrap();
        assert_eq!(p1_info.manifest.name, "rara");
        assert_eq!(p2_info.manifest.name, "rara");
        // Each process has its own agent:{id} session.
        assert_ne!(p1_info.session_id, p2_info.session_id);
        // Each process has distinct channel sessions.
        assert_eq!(p1_info.channel_session_id.as_ref().unwrap(), &csid1);
        assert_eq!(p2_info.channel_session_id.as_ref().unwrap(), &csid2);
    }

    #[test]
    fn test_existing_session_routes_to_same_agent() {
        let table = ProcessTable::new();

        // Insert an agent bound to channel session-1.
        let p = test_process_with_session("rara", None, "session-1");
        let agent_id = p.agent_id;
        let channel_sid = p.channel_session_id.clone().unwrap();
        table.insert(p);

        // Repeated lookups by channel session always return the same agent.
        assert_eq!(table.find_by_session(&channel_sid).unwrap().agent_id, agent_id);
        assert_eq!(table.find_by_session(&channel_sid).unwrap().agent_id, agent_id);

        // A different session returns None (no agent bound to it).
        let other_session = SessionId::new("session-2");
        assert!(table.find_by_session(&other_session).is_none());
    }

    #[test]
    fn test_multi_session_independent_lifecycle() {
        let table = ProcessTable::new();

        // Spawn two instances of "rara" on different channel sessions.
        let p1 = test_process_with_session("rara", None, "session-a");
        let id1 = p1.agent_id;
        let csid_a = p1.channel_session_id.clone().unwrap();
        table.insert(p1);

        let p2 = test_process_with_session("rara", None, "session-b");
        let id2 = p2.agent_id;
        let csid_b = p2.channel_session_id.clone().unwrap();
        table.insert(p2);

        // Complete session-a's process. Session-b's process should be unaffected.
        table.set_state(id1, ProcessState::Completed).unwrap();
        assert_eq!(table.get(id1).unwrap().state, ProcessState::Completed);
        assert_eq!(table.get(id2).unwrap().state, ProcessState::Running);

        // Remove session-a's process. Session-b still exists.
        table.remove(id1);
        assert!(table.find_by_session(&csid_a).is_none());
        assert_eq!(table.find_by_session(&csid_b).unwrap().agent_id, id2);
        assert_eq!(table.list().len(), 1);
    }

    #[test]
    fn test_multi_session_remove_preserves_name_index() {
        let table = ProcessTable::new();

        // Spawn "rara" for session-1 (name index points to id1).
        let p1 = test_process_with_session("rara", None, "session-1");
        let id1 = p1.agent_id;
        table.insert(p1);

        // Spawn "rara" for session-2 (name index now points to id2).
        let p2 = test_process_with_session("rara", None, "session-2");
        let id2 = p2.agent_id;
        table.insert(p2);

        // Remove the older process (id1). Name index should still point to id2.
        table.remove(id1);
        assert_eq!(table.find_by_name("rara").unwrap().agent_id, id2);

        // Remove the newer process (id2). Name index should be cleared.
        table.remove(id2);
        assert!(table.find_by_name("rara").is_none());
    }

    // -----------------------------------------------------------------------
    // Session isolation tests — subagents get their own sessions
    // -----------------------------------------------------------------------

    #[test]
    fn test_subagent_not_in_session_index() {
        let table = ProcessTable::new();

        // Root process has channel session.
        let parent = test_process("rara", None);
        let parent_id = parent.agent_id;
        let channel_sid = parent.channel_session_id.clone().unwrap();
        table.insert(parent);
        assert!(table.find_by_session(&channel_sid).is_some());

        // Subagent has no channel session — should NOT appear in session_index.
        let child = test_subagent("scout", parent_id);
        let child_id = child.agent_id;
        let child_session = child.session_id.clone();
        table.insert(child);

        // The subagent's own session is NOT in session_index.
        assert!(table.find_by_session(&child_session).is_none());
        // The parent's channel session still routes to the parent.
        assert_eq!(table.find_by_session(&channel_sid).unwrap().agent_id, parent_id);
        // But the subagent is reachable via get().
        assert!(table.get(child_id).is_some());
        assert_eq!(table.get(child_id).unwrap().parent_id, Some(parent_id));
    }

    #[test]
    fn test_subagent_removal_does_not_affect_session_index() {
        let table = ProcessTable::new();

        let parent = test_process("rara", None);
        let parent_id = parent.agent_id;
        let channel_sid = parent.channel_session_id.clone().unwrap();
        table.insert(parent);

        let child = test_subagent("scout", parent_id);
        let child_id = child.agent_id;
        table.insert(child);

        // Remove subagent — parent's channel session binding must survive.
        table.remove(child_id);
        assert_eq!(table.find_by_session(&channel_sid).unwrap().agent_id, parent_id);
    }

    #[test]
    fn test_each_process_has_unique_session() {
        let table = ProcessTable::new();

        let p1 = test_process("rara", None);
        let p1_sid = p1.session_id.clone();
        let p1_id = p1.agent_id;
        table.insert(p1);

        let p2 = test_subagent("scout", p1_id);
        let p2_sid = p2.session_id.clone();
        table.insert(p2);

        // Each process has a unique agent-scoped session.
        assert_ne!(p1_sid, p2_sid);
        assert!(p1_sid.as_str().starts_with("agent:"));
        assert!(p2_sid.as_str().starts_with("agent:"));
    }
}
