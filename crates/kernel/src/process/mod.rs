// Copyright 2025 Rararulab
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

//! Session-centric runtime model — core types for the unified agent lifecycle.
//!
//! This module implements a session-centric runtime model where:
//! - [`AgentManifest`] = the "binary" (static definition, YAML-loadable)
//! - [`SessionRuntime`] = a running session instance in the [`SessionTable`]
//! - [`AgentId`] = unique per-execution identifier (kept for audit/security compatibility)
//! - [`SessionId`] = persistent conversation identifier (the primary identifier)
//!
//! The [`SessionTable`] is a concurrent in-memory table (backed by `DashMap`)
//! that tracks all active session runtimes, supporting session tree queries
//! (parent/children) and state transitions.

pub mod agent_registry;
pub mod manifest_loader;
pub mod noop_user_store;
pub mod principal;
pub mod user;

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, strum::Display)]
#[strum(serialize_all = "snake_case")]
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

// ---------------------------------------------------------------------------
// Priority
// ---------------------------------------------------------------------------

/// Dispatch priority for agent messages.
///
/// Higher priority messages are processed before lower ones when the
/// scheduler is draining the inbound bus. Critical messages bypass rate
/// limiting entirely.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Default,
    Serialize,
    Deserialize,
    strum::Display,
)]
#[strum(serialize_all = "snake_case")]
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
/// This is a type alias for [`SessionKey`](crate::session::SessionKey).
/// Use `SessionId::new()` to generate a new random UUID, or
/// `SessionId::from_raw(s)` to wrap a trusted string.
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
    pub name:               String,
    /// Agent's functional role (chat, scout, planner, worker).
    #[serde(default)]
    pub role:               Option<AgentRole>,
    /// Human-readable description.
    pub description:        String,
    /// LLM model identifier (e.g., "deepseek/deepseek-chat", "gpt-4").
    ///
    /// `None` means "use the provider registry default". The kernel's
    /// `ProviderRegistry::resolve()` will fall through to the global
    /// default model when this is `None`.
    #[serde(default)]
    pub model:              Option<String>,
    /// System prompt defining agent behavior.
    pub system_prompt:      String,
    /// Optional personality/mood/voice prompt (prepended to system_prompt when
    /// building LLM messages).
    #[serde(default)]
    pub soul_prompt:        Option<String>,
    /// Optional hint for provider selection.
    #[serde(default)]
    pub provider_hint:      Option<String>,
    /// Maximum LLM iterations before forced completion.
    #[serde(default)]
    pub max_iterations:     Option<usize>,
    /// Tool names this agent is allowed to use (empty = inherit parent's
    /// tools).
    #[serde(default)]
    pub tools:              Vec<String>,
    /// Maximum number of concurrent child agents this agent can spawn.
    #[serde(default)]
    pub max_children:       Option<usize>,
    /// Maximum context window size in tokens.
    ///
    /// When the in-memory conversation history exceeds this budget, the
    /// kernel applies a
    /// [`CompactionStrategy`](crate::compaction::CompactionStrategy)
    /// to trim it before sending to the LLM. Defaults to
    /// [`DEFAULT_MAX_CONTEXT_TOKENS`](crate::compaction::DEFAULT_MAX_CONTEXT_TOKENS)
    /// (8192) when `None`.
    #[serde(default)]
    pub max_context_tokens: Option<usize>,
    /// Dispatch priority for scheduling.
    #[serde(default)]
    pub priority:           Priority,
    /// Arbitrary metadata for extension.
    #[serde(default)]
    pub metadata:           serde_json::Value,
    /// Optional sandbox configuration for file access control.
    #[serde(default)]
    pub sandbox:            Option<SandboxConfig>,
}

/// Runtime state of a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, strum::Display)]
#[strum(serialize_all = "snake_case")]
pub enum SessionState {
    /// Session is actively processing a message (LLM call in flight).
    Active,
    /// Session is idle, awaiting next message.
    Ready,
    /// Session has been suspended (timed out, resources released).
    Suspended,
    /// Session is manually paused, rejects incoming messages.
    Paused,
}

/// Backwards-compatible alias during migration.
pub type ProcessState = SessionState;

impl SessionState {
    /// Whether this state is terminal (session no longer accepts messages).
    ///
    /// In the session-centric model, sessions are never truly terminal —
    /// they transition to Suspended instead. This always returns false.
    pub fn is_terminal(self) -> bool { false }
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, strum::Display)]
#[strum(serialize_all = "snake_case")]
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

/// A running session instance in the session table.
///
/// This is the runtime counterpart to [`AgentManifest`]. Each session
/// is created with a unique [`AgentId`] (for audit/security compatibility),
/// the spawning principal, and the manifest that defines its behavior.
/// Sessions are long-lived — they transition between Active/Ready/Suspended/Paused
/// rather than being created and destroyed per message.
#[derive(Debug, Clone)]
pub struct SessionRuntime {
    /// Unique identifier for this session runtime (kept for audit compatibility).
    pub agent_id:           AgentId,
    /// Parent session (None for root-level sessions).
    pub parent_id:          Option<AgentId>,
    /// The session's conversation storage key.
    pub session_id:         SessionId,
    /// External channel binding (e.g., `web:chat123`). Only set for root
    /// sessions that entered via an external channel adapter. Used by
    /// `session_index` for routing inbound messages to the correct session.
    /// Child sessions have `None` — they are only reachable via `SessionHandle`.
    pub channel_session_id: Option<SessionId>,
    /// The agent definition driving this session.
    pub manifest:           AgentManifest,
    /// The identity under which this session runs.
    pub principal:          principal::Principal,
    /// Per-session environment.
    pub env:                AgentEnv,
    /// Current lifecycle state.
    pub state:              SessionState,
    /// When this session was created.
    pub created_at:         Timestamp,
    /// When this session was last active (for idle timeout).
    pub finished_at:        Option<Timestamp>,
    /// Result of last execution (set on turn completion).
    pub result:             Option<AgentResult>,
    /// Files created or modified by this agent (for resource tracking).
    pub created_files:      Vec<PathBuf>,
    /// Per-session runtime metrics (atomic counters for lock-free updates).
    pub metrics:            Arc<RuntimeMetrics>,
    /// Detailed turn traces for observability (most recent 50 turns).
    pub turn_traces:        Vec<crate::agent_turn::TurnTrace>,
}

/// Backwards-compatible alias during migration.
pub type AgentProcess = SessionRuntime;

/// Summary info for listing sessions.
///
/// A lightweight view of a [`SessionRuntime`] suitable for display in
/// session listings without exposing full internal state.
#[derive(Debug, Clone, Serialize)]
pub struct SessionInfo {
    pub agent_id:   AgentId,
    pub parent_id:  Option<AgentId>,
    pub name:       String,
    pub state:      SessionState,
    pub created_at: Timestamp,
}

/// Backwards-compatible alias during migration.
pub type ProcessInfo = SessionInfo;

impl From<&SessionRuntime> for SessionInfo {
    fn from(p: &SessionRuntime) -> Self {
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
    pub llm_calls:         AtomicU64,
    /// Number of tool calls executed.
    pub tool_calls:        AtomicU64,
    /// Approximate total tokens consumed (prompt + completion).
    pub tokens_consumed:   AtomicU64,
    /// Timestamp of the most recent activity.
    pub last_activity:     Mutex<Option<Timestamp>>,
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
    pub fn record_message(&self) { self.messages_received.fetch_add(1, Ordering::Relaxed); }

    /// Record an LLM call completion.
    pub fn record_llm_call(&self) { self.llm_calls.fetch_add(1, Ordering::Relaxed); }

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
            .field(
                "messages_received",
                &self.messages_received.load(Ordering::Relaxed),
            )
            .field("llm_calls", &self.llm_calls.load(Ordering::Relaxed))
            .field("tool_calls", &self.tool_calls.load(Ordering::Relaxed))
            .field(
                "tokens_consumed",
                &self.tokens_consumed.load(Ordering::Relaxed),
            )
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

/// Extended runtime statistics for a single session.
///
/// Combines static metadata from [`SessionRuntime`] with live counters from
/// [`RuntimeMetrics`]. This is the `/proc/<pid>/status` equivalent.
#[derive(Debug, Clone, Serialize)]
pub struct SessionStats {
    /// Unique session runtime identifier.
    pub agent_id:          AgentId,
    /// Session conversation key.
    pub session_id:        SessionId,
    /// The manifest name (agent definition name).
    pub manifest_name:     String,
    /// Current lifecycle state.
    pub state:             SessionState,
    /// Parent session, if any.
    pub parent_id:         Option<AgentId>,
    /// IDs of child sessions.
    pub children:          Vec<AgentId>,
    /// When this session was created.
    pub created_at:        Timestamp,
    /// When this session was last active.
    pub finished_at:       Option<Timestamp>,
    /// How long this session has been alive (milliseconds).
    pub uptime_ms:         u64,
    // -- Runtime metrics --
    /// Number of messages received by this session.
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

/// Backwards-compatible alias during migration.
pub type ProcessStats = SessionStats;

// ---------------------------------------------------------------------------
// SystemStats — kernel-wide aggregate metrics (like /proc/stat)
// ---------------------------------------------------------------------------

/// Kernel-wide aggregate statistics.
///
/// Provides a high-level overview of the kernel's current state, analogous
/// to `/proc/stat` or `/proc/meminfo` in Linux.
#[derive(Debug, Clone, Serialize)]
pub struct SystemStats {
    /// Number of currently active sessions.
    pub active_sessions:            usize,
    /// Total number of sessions ever created.
    pub total_spawned:              u64,
    /// Total number of sessions that completed successfully (legacy counter).
    pub total_completed:            u64,
    /// Total number of sessions that failed (legacy counter).
    pub total_failed:               u64,
    /// Number of global semaphore permits currently available.
    pub global_semaphore_available: usize,
    /// Sum of tokens consumed across all tracked sessions.
    pub total_tokens_consumed:      u64,
    /// Kernel uptime in milliseconds.
    pub uptime_ms:                  u64,
}

/// In-memory session table — the kernel's view of all active sessions.
///
/// Thread-safe via `DashMap`. Supports concurrent reads and writes from
/// multiple tokio tasks (e.g., kernel spawn + agent tool calls).
///
/// Includes a session index for fast `SessionId -> AgentId` lookups,
/// a name index for fast agent name -> `AgentId` lookups, and
/// a mailbox registry for sending messages to long-lived sessions.
pub struct SessionTable {
    processes:       DashMap<AgentId, SessionRuntime>,
    /// Maps a session to its currently active agent process.
    session_index:   DashMap<SessionId, AgentId>,
    /// Parent → Children index, O(1) child lookup.
    children_index:  DashMap<AgentId, Vec<AgentId>>,
    /// Agent manifest name → Vec<AgentId> (1:N, observability only).
    name_registry:   DashMap<String, Vec<AgentId>>,
    /// Monotonically increasing counter of total processes ever spawned.
    total_spawned:   AtomicU64,
    /// Total processes that completed successfully.
    total_completed: AtomicU64,
    /// Total processes that failed.
    total_failed:    AtomicU64,
}

/// Backwards-compatible alias during migration.
pub type ProcessTable = SessionTable;

impl SessionTable {
    /// Maximum number of turn traces retained per process.
    const MAX_TURN_TRACES: usize = 50;
    /// How long terminal processes remain visible before being reaped.
    const TERMINAL_TTL: std::time::Duration = std::time::Duration::from_secs(60);

    /// Create an empty process table.
    pub fn new() -> Self {
        Self {
            processes:       DashMap::new(),
            session_index:   DashMap::new(),
            children_index:  DashMap::new(),
            name_registry:   DashMap::new(),
            total_spawned:   AtomicU64::new(0),
            total_completed: AtomicU64::new(0),
            total_failed:    AtomicU64::new(0),
        }
    }

    /// Insert a process into the table.
    #[tracing::instrument(skip(self, process), fields(agent_id = %process.agent_id, agent_name = %process.manifest.name))]
    pub fn insert(&self, process: AgentProcess) {
        let agent_id = process.agent_id;
        if let Some(ref channel_sid) = process.channel_session_id {
            self.session_index.insert(channel_sid.clone(), agent_id);
        }
        // Children index: register under parent
        if let Some(parent_id) = process.parent_id {
            self.children_index
                .entry(parent_id)
                .or_default()
                .push(agent_id);
        }
        // Initialize empty children list for this process
        self.children_index.entry(agent_id).or_default();
        // Name registry (1:N)
        self.name_registry
            .entry(process.manifest.name.clone())
            .or_default()
            .push(agent_id);
        self.total_spawned.fetch_add(1, Ordering::Relaxed);
        self.processes.insert(agent_id, process);
    }

    /// Get a clone of a process by ID.
    pub fn get(&self, id: AgentId) -> Option<AgentProcess> {
        self.processes.get(&id).map(|p| p.value().clone())
    }

    /// Transition a session to a new state.
    ///
    /// Sessions are long-lived and never reach a terminal state. State
    /// transitions are: Active (processing), Ready (idle), Suspended
    /// (timed-out), Paused (manual hold).
    #[tracing::instrument(skip(self), fields(new_state = %state))]
    pub fn set_state(&self, id: AgentId, state: SessionState) -> Result<()> {
        let mut entry = self
            .processes
            .get_mut(&id)
            .ok_or(crate::error::KernelError::AgentNotFound { id: id.0 })?;
        entry.state = state;
        // Sessions are long-lived — no terminal states, no finished_at.
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
    #[tracing::instrument(skip(self))]
    pub fn remove(&self, id: AgentId) -> Option<AgentProcess> {
        let removed = self.processes.remove(&id).map(|(_, p)| p);
        if let Some(ref process) = removed {
            // Session index cleanup
            if let Some(ref channel_sid) = process.channel_session_id {
                self.session_index
                    .remove_if(channel_sid, |_, agent_id| *agent_id == id);
            }
            // Children index: remove from parent's children list
            if let Some(parent_id) = process.parent_id {
                if let Some(mut children) = self.children_index.get_mut(&parent_id) {
                    children.retain(|c| *c != id);
                }
            }
            // Remove own children entry
            self.children_index.remove(&id);
            // Name registry: remove from vec
            if let Some(mut ids) = self.name_registry.get_mut(&process.manifest.name) {
                ids.retain(|aid| *aid != id);
            }
        }
        removed
    }

    /// List all children of a given parent (O(1) lookup via children_index).
    pub fn children_of(&self, parent_id: AgentId) -> Vec<ProcessInfo> {
        let child_ids = self
            .children_index
            .get(&parent_id)
            .map(|ids| ids.clone())
            .unwrap_or_default();
        child_ids
            .iter()
            .filter_map(|id| self.processes.get(id).map(|p| ProcessInfo::from(p.value())))
            .collect()
    }

    /// List all processes.
    pub fn list(&self) -> Vec<ProcessInfo> {
        self.processes
            .iter()
            .map(|p| ProcessInfo::from(p.value()))
            .collect()
    }

    /// Push a turn trace onto a process, evicting the oldest if at capacity.
    pub fn push_turn_trace(&self, id: AgentId, trace: crate::agent_turn::TurnTrace) {
        if let Some(mut entry) = self.processes.get_mut(&id) {
            if entry.turn_traces.len() >= Self::MAX_TURN_TRACES {
                entry.turn_traces.remove(0);
            }
            entry.turn_traces.push(trace);
        }
    }

    /// Get the turn traces for a process.
    pub fn get_turn_traces(&self, id: AgentId) -> Vec<crate::agent_turn::TurnTrace> {
        self.processes
            .get(&id)
            .map(|p| p.turn_traces.clone())
            .unwrap_or_default()
    }

    /// Count active sessions (those currently processing a message).
    pub fn active_count(&self) -> usize {
        self.processes
            .iter()
            .filter(|p| p.state == SessionState::Active)
            .count()
    }

    /// Backwards-compatible alias.
    pub fn running_count(&self) -> usize { self.active_count() }

    // ----- Session index methods -----

    /// Find the active agent process for a session.
    pub fn find_by_session(&self, session_id: &SessionId) -> Option<AgentProcess> {
        let agent_id = self.session_index.get(session_id)?;
        self.get(*agent_id)
    }

    /// Find agent processes by manifest name (returns the most recently
    /// inserted).
    pub fn find_by_name(&self, name: &str) -> Option<AgentProcess> {
        let ids = self.name_registry.get(name)?;
        ids.last().and_then(|id| self.get(*id))
    }

    /// Find all agent processes with the given manifest name.
    pub fn find_all_by_name(&self, name: &str) -> Vec<AgentProcess> {
        self.name_registry
            .get(name)
            .map(|ids| ids.iter().filter_map(|id| self.get(*id)).collect())
            .unwrap_or_default()
    }

    /// Bind a session to a specific agent process (overwrites any existing
    /// binding).
    pub fn bind_session(&self, session_id: SessionId, agent_id: AgentId) {
        self.session_index.insert(session_id, agent_id);
    }

    /// Remove a session index entry only if it points to the given agent.
    pub fn session_index_remove(&self, session_id: &SessionId, agent_id: AgentId) {
        self.session_index
            .remove_if(session_id, |_, aid| *aid == agent_id);
    }

    // ----- Metrics methods -----

    /// Get a shared reference to the metrics for a process.
    pub fn get_metrics(&self, id: &AgentId) -> Option<Arc<RuntimeMetrics>> {
        self.processes.get(id).map(|p| Arc::clone(&p.metrics))
    }

    /// Build a [`SessionStats`] snapshot for a single session.
    pub async fn process_stats(&self, id: AgentId) -> Option<ProcessStats> {
        let process = self.get(id)?;
        let metrics_snapshot = process.metrics.snapshot().await;
        let children: Vec<AgentId> = self
            .children_index
            .get(&id)
            .map(|ids| ids.clone())
            .unwrap_or_default();
        let uptime_ms = Timestamp::now()
            .since(process.created_at)
            .ok()
            .map(|span| span.get_milliseconds().unsigned_abs())
            .unwrap_or(0);

        Some(ProcessStats {
            agent_id: process.agent_id,
            session_id: process.session_id,
            manifest_name: process.manifest.name,
            state: process.state,
            parent_id: process.parent_id,
            children,
            created_at: process.created_at,
            finished_at: process.finished_at,
            uptime_ms,
            messages_received: metrics_snapshot.messages_received,
            llm_calls: metrics_snapshot.llm_calls,
            tool_calls: metrics_snapshot.tool_calls,
            tokens_consumed: metrics_snapshot.tokens_consumed,
            last_activity: metrics_snapshot.last_activity,
        })
    }

    /// Build [`SessionStats`] for all sessions currently in the table.
    ///
    /// Also performs lazy reaping of suspended sessions older than the TTL.
    pub async fn all_process_stats(&self) -> Vec<ProcessStats> {
        // Lazy reap: remove stale terminal processes on observation.
        self.reap_terminal(Self::TERMINAL_TTL);

        let ids: Vec<AgentId> = self.processes.iter().map(|p| p.agent_id).collect();
        let mut stats = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(s) = self.process_stats(id).await {
                stats.push(s);
            }
        }
        stats
    }

    /// Remove suspended sessions whose last activity is older than `max_age`.
    ///
    /// In the session-centric model sessions are never terminal, so this
    /// reaps long-idle *suspended* sessions instead.  Returns the number of
    /// sessions reaped.
    pub fn reap_terminal(&self, max_age: std::time::Duration) -> usize {
        let now = Timestamp::now();
        let max_age_ms = max_age.as_millis() as i128;
        let to_remove: Vec<AgentId> = self
            .processes
            .iter()
            .filter(|entry| {
                let p = entry.value();
                if p.state != SessionState::Suspended {
                    return false;
                }
                match p.finished_at {
                    Some(finished) => {
                        let elapsed_ns = now.as_nanosecond() - finished.as_nanosecond();
                        let elapsed_ms = elapsed_ns / 1_000_000;
                        elapsed_ms > max_age_ms
                    }
                    None => false,
                }
            })
            .map(|entry| entry.agent_id)
            .collect();

        let count = to_remove.len();
        for id in to_remove {
            self.remove(id);
        }
        count
    }

    /// Get the total number of processes ever spawned.
    pub fn total_spawned(&self) -> u64 { self.total_spawned.load(Ordering::Relaxed) }

    /// Get the total number of processes that completed successfully.
    pub fn total_completed(&self) -> u64 { self.total_completed.load(Ordering::Relaxed) }

    /// Get the total number of processes that failed.
    pub fn total_failed(&self) -> u64 { self.total_failed.load(Ordering::Relaxed) }

    /// Sum of tokens consumed across all currently tracked processes.
    pub fn total_tokens_consumed(&self) -> u64 {
        self.processes
            .iter()
            .map(|p| p.metrics.tokens_consumed.load(Ordering::Relaxed))
            .sum()
    }
}

impl Default for SessionTable {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::principal::Principal;

    /// Helper to create a test manifest.
    fn test_manifest(name: &str) -> AgentManifest {
        AgentManifest {
            name:               name.to_string(),
            role:               None,
            description:        format!("Test agent: {name}"),
            model:              Some("test-model".to_string()),
            system_prompt:      "You are a test agent.".to_string(),
            soul_prompt:        None,
            provider_hint:      None,
            max_iterations:     Some(10),
            tools:              vec!["read_file".to_string()],
            max_children:       None,
            max_context_tokens: None,
            priority:           Priority::default(),
            metadata:           serde_json::Value::Null,
            sandbox:            None,
        }
    }

    fn test_process(name: &str, parent_id: Option<AgentId>) -> SessionRuntime {
        test_process_with_session(name, parent_id, "test-session")
    }

    /// Helper to create a test process simulating a subagent (no channel
    /// session).
    fn test_subagent(name: &str, parent_id: AgentId) -> SessionRuntime {
        let agent_id = AgentId::new();
        SessionRuntime {
            agent_id,
            parent_id: Some(parent_id),
            session_id: SessionId::new(),
            channel_session_id: None,
            manifest: test_manifest(name),
            principal: Principal::user("test-user"),
            env: AgentEnv::default(),
            state: SessionState::Active,
            created_at: Timestamp::now(),
            finished_at: None,
            result: None,
            created_files: vec![],
            metrics: Arc::new(RuntimeMetrics::new()),
            turn_traces: vec![],
        }
    }

    /// Helper to create a test process with a specific channel session ID.
    fn test_process_with_session(
        name: &str,
        parent_id: Option<AgentId>,
        _channel_session: &str,
    ) -> SessionRuntime {
        let agent_id = AgentId::new();
        SessionRuntime {
            agent_id,
            parent_id,
            session_id: SessionId::new(),
            channel_session_id: Some(SessionId::new()),
            manifest: test_manifest(name),
            principal: Principal::user("test-user"),
            env: AgentEnv::default(),
            state: SessionState::Active,
            created_at: Timestamp::now(),
            finished_at: None,
            result: None,
            created_files: vec![],
            metrics: Arc::new(RuntimeMetrics::new()),
            turn_traces: vec![],
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
    fn test_session_id_is_uuid() {
        let sid = SessionId::new();
        // Inner value is a valid UUID.
        let _ = sid.uuid();
        assert!(!sid.to_string().is_empty());
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
        assert_eq!(retrieved.state, SessionState::Active);
    }

    #[test]
    fn test_session_table_get_nonexistent() {
        let table = SessionTable::new();
        assert!(table.get(AgentId::new()).is_none());
    }

    #[test]
    fn test_session_table_set_state() {
        let table = SessionTable::new();
        let process = test_process("scout", None);
        let id = process.agent_id;
        table.insert(process);

        // Transition to Ready (idle)
        table.set_state(id, SessionState::Ready).unwrap();

        let p = table.get(id).unwrap();
        assert_eq!(p.state, SessionState::Ready);
    }

    #[test]
    fn test_session_table_set_state_nonexistent() {
        let table = SessionTable::new();
        let result = table.set_state(AgentId::new(), SessionState::Suspended);
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
    fn test_session_table_active_count() {
        let table = SessionTable::new();
        assert_eq!(table.active_count(), 0);

        let p1 = test_process("a", None);
        let p1_id = p1.agent_id;
        table.insert(p1);

        let p2 = test_process("b", None);
        table.insert(p2);

        assert_eq!(table.active_count(), 2);

        table.set_state(p1_id, SessionState::Ready).unwrap();
        assert_eq!(table.active_count(), 1);
    }

    #[test]
    fn test_session_info_from_session_runtime() {
        let process = test_process("scout", None);
        let info = SessionInfo::from(&process);

        assert_eq!(info.agent_id, process.agent_id);
        assert_eq!(info.parent_id, None);
        assert_eq!(info.name, "scout");
        assert_eq!(info.state, SessionState::Active);
    }

    #[test]
    fn test_agent_manifest_yaml_roundtrip() {
        let manifest = test_manifest("roundtrip");
        let yaml = serde_yaml::to_string(&manifest).unwrap();
        let deserialized: AgentManifest = serde_yaml::from_str(&yaml).unwrap();

        assert_eq!(deserialized.name, "roundtrip");
        assert_eq!(deserialized.model.as_deref(), Some("test-model"));
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
        assert_eq!(m.model.as_deref(), Some("gpt-4"));
        assert!(m.tools.is_empty());
        assert!(m.max_iterations.is_none());
        assert!(m.provider_hint.is_none());
        assert!(m.max_children.is_none());
    }

    #[test]
    fn test_agent_manifest_yaml_no_model() {
        let yaml = r#"
name: no-model
description: "Agent without model"
system_prompt: "Hello"
"#;
        let m: AgentManifest = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(m.name, "no-model");
        assert!(m.model.is_none());
    }

    #[test]
    fn test_agent_env_default() {
        let env = AgentEnv::default();
        assert!(env.workspace.is_none());
        assert!(env.vars.is_empty());
    }

    #[test]
    fn test_session_state_transitions() {
        let table = SessionTable::new();

        // Active -> Ready
        let p = test_process("a", None);
        let id = p.agent_id;
        table.insert(p);
        table.set_state(id, SessionState::Ready).unwrap();
        assert_eq!(table.get(id).unwrap().state, SessionState::Ready);

        // Ready -> Suspended
        table.set_state(id, SessionState::Suspended).unwrap();
        assert_eq!(table.get(id).unwrap().state, SessionState::Suspended);

        // Suspended -> Active (reactivation)
        table.set_state(id, SessionState::Active).unwrap();
        assert_eq!(table.get(id).unwrap().state, SessionState::Active);
    }

    #[test]
    fn test_session_state_ready_does_not_set_finished_at() {
        let table = SessionTable::new();
        let p = test_process("idler", None);
        let id = p.agent_id;
        table.insert(p);

        table.set_state(id, SessionState::Ready).unwrap();

        let session = table.get(id).unwrap();
        assert_eq!(session.state, SessionState::Ready);
        assert!(
            session.finished_at.is_none(),
            "Ready state should not set finished_at"
        );
    }

    #[test]
    fn test_session_state_none_is_terminal() {
        // All session states are non-terminal.
        assert!(!SessionState::Active.is_terminal());
        assert!(!SessionState::Ready.is_terminal());
        assert!(!SessionState::Suspended.is_terminal());
        assert!(!SessionState::Paused.is_terminal());
    }

    #[test]
    fn test_session_state_display() {
        assert_eq!(SessionState::Active.to_string(), "active");
        assert_eq!(SessionState::Ready.to_string(), "ready");
        assert_eq!(SessionState::Suspended.to_string(), "suspended");
        assert_eq!(SessionState::Paused.to_string(), "paused");
    }

    #[test]
    fn test_session_state_suspended_does_not_set_finished_at() {
        let table = SessionTable::new();
        let p = test_process("waiter", None);
        let id = p.agent_id;
        table.insert(p);

        table.set_state(id, SessionState::Suspended).unwrap();

        let session = table.get(id).unwrap();
        assert_eq!(session.state, SessionState::Suspended);
        assert!(
            session.finished_at.is_none(),
            "Suspended state should not set finished_at"
        );
    }

    #[test]
    fn test_session_state_paused_does_not_set_finished_at() {
        let table = SessionTable::new();
        let p = test_process("paused-agent", None);
        let id = p.agent_id;
        table.insert(p);

        table.set_state(id, SessionState::Paused).unwrap();

        let session = table.get(id).unwrap();
        assert_eq!(session.state, SessionState::Paused);
        assert!(
            session.finished_at.is_none(),
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
        let new_process = SessionRuntime {
            agent_id:           new_id,
            parent_id:          None,
            session_id:         SessionId::new(),
            channel_session_id: Some(channel_sid.clone()),
            manifest:           test_manifest("agent-b"),
            principal:          Principal::user("test-user"),
            env:                AgentEnv::default(),
            state:              SessionState::Active,
            created_at:         Timestamp::now(),
            finished_at:        None,
            result:             None,
            created_files:      vec![],
            metrics:            Arc::new(RuntimeMetrics::new()),
            turn_traces:        vec![],
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
    fn test_get_metrics_from_process() {
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
    fn test_set_state_cycles_through_session_states() {
        let table = SessionTable::new();

        let p = test_process("lifecycle", None);
        let id = p.agent_id;
        table.insert(p);

        // Active -> Ready -> Suspended -> Active -> Paused
        table.set_state(id, SessionState::Ready).unwrap();
        assert_eq!(table.get(id).unwrap().state, SessionState::Ready);

        table.set_state(id, SessionState::Suspended).unwrap();
        assert_eq!(table.get(id).unwrap().state, SessionState::Suspended);

        table.set_state(id, SessionState::Active).unwrap();
        assert_eq!(table.get(id).unwrap().state, SessionState::Active);

        table.set_state(id, SessionState::Paused).unwrap();
        assert_eq!(table.get(id).unwrap().state, SessionState::Paused);
    }

    #[test]
    fn test_remove_clears_metrics() {
        let table = ProcessTable::new();
        let p = test_process("rm-metrics", None);
        let id = p.agent_id;
        table.insert(p);

        assert!(table.get_metrics(&id).is_some());
        table.remove(id);
        // Process removed, get_metrics should return None.
        assert!(table.get_metrics(&id).is_none());
    }

    #[tokio::test]
    async fn test_process_stats_populated_correctly() {
        let table = ProcessTable::new();

        let parent = test_process("planner", None);
        let parent_id = parent.agent_id;
        let parent_metrics = Arc::clone(&parent.metrics);
        table.insert(parent);

        let child = test_process("worker", Some(parent_id));
        let child_id = child.agent_id;
        table.insert(child);

        // Record some metrics on the parent
        parent_metrics.record_message();
        parent_metrics.record_llm_call();
        parent_metrics.record_tool_calls(2);
        parent_metrics.record_tokens(500);
        parent_metrics.touch().await;

        let stats = table.process_stats(parent_id).await.unwrap();
        assert_eq!(stats.agent_id, parent_id);
        assert_eq!(stats.manifest_name, "planner");
        assert_eq!(stats.state, SessionState::Active);
        assert!(stats.parent_id.is_none());
        assert_eq!(stats.children, vec![child_id]);
        assert_eq!(stats.messages_received, 1);
        assert_eq!(stats.llm_calls, 1);
        assert_eq!(stats.tool_calls, 2);
        assert_eq!(stats.tokens_consumed, 500);
        assert!(stats.last_activity.is_some());
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
        let m1 = Arc::clone(&p1.metrics);
        table.insert(p1);

        let p2 = test_process("b", None);
        let m2 = Arc::clone(&p2.metrics);
        table.insert(p2);

        m1.record_tokens(100);
        m2.record_tokens(200);

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
    // Name registry tests
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

        assert!(table.find_by_name("no-such-agent").is_none());
    }

    #[test]
    fn test_name_registry_cleanup_on_remove() {
        let table = ProcessTable::new();
        let p = test_process("removable-agent", None);
        let id = p.agent_id;
        table.insert(p);

        assert!(table.find_by_name("removable-agent").is_some());

        table.remove(id);
        assert!(table.find_by_name("removable-agent").is_none());
    }

    #[test]
    fn test_name_registry_tracks_multiple() {
        let table = ProcessTable::new();

        let p1 = test_process("dup-agent", None);
        let id1 = p1.agent_id;
        table.insert(p1);

        assert_eq!(table.find_by_name("dup-agent").unwrap().agent_id, id1);

        let p2 = test_process("dup-agent", None);
        let id2 = p2.agent_id;
        table.insert(p2);

        // find_by_name returns the most recently inserted.
        assert_eq!(table.find_by_name("dup-agent").unwrap().agent_id, id2);
        assert_ne!(id1, id2);

        // find_all_by_name returns both.
        let all = table.find_all_by_name("dup-agent");
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_remove_does_not_clear_name_registry_for_different_agent() {
        let table = ProcessTable::new();

        let p1 = test_process("shared-name", None);
        let id1 = p1.agent_id;
        table.insert(p1);

        let p2 = test_process("shared-name", None);
        let id2 = p2.agent_id;
        table.insert(p2);

        // Remove p1 — name_registry should still have p2.
        table.remove(id1);
        assert!(
            table.find_by_name("shared-name").is_some(),
            "name_registry should still have p2"
        );
        assert_eq!(table.find_by_name("shared-name").unwrap().agent_id, id2);

        // Remove p2 — name_registry should be empty.
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

        // find_by_name returns the most recently inserted.
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
        assert_eq!(
            table.find_by_session(&channel_sid).unwrap().agent_id,
            agent_id
        );
        assert_eq!(
            table.find_by_session(&channel_sid).unwrap().agent_id,
            agent_id
        );

        // A different session returns None (no agent bound to it).
        let other_session = SessionId::new();
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

        // Suspend session-a. Session-b should be unaffected.
        table.set_state(id1, SessionState::Suspended).unwrap();
        assert_eq!(table.get(id1).unwrap().state, SessionState::Suspended);
        assert_eq!(table.get(id2).unwrap().state, SessionState::Active);

        // Remove session-a's process. Session-b still exists.
        table.remove(id1);
        assert!(table.find_by_session(&csid_a).is_none());
        assert_eq!(table.find_by_session(&csid_b).unwrap().agent_id, id2);
        assert_eq!(table.list().len(), 1);
    }

    #[test]
    fn test_multi_session_remove_preserves_name_registry() {
        let table = ProcessTable::new();

        let p1 = test_process_with_session("rara", None, "session-1");
        let id1 = p1.agent_id;
        table.insert(p1);

        let p2 = test_process_with_session("rara", None, "session-2");
        let id2 = p2.agent_id;
        table.insert(p2);

        // Remove the older process. Name registry should still have id2.
        table.remove(id1);
        assert_eq!(table.find_by_name("rara").unwrap().agent_id, id2);

        // Remove the newer process. Name registry should be empty.
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
        assert_eq!(
            table.find_by_session(&channel_sid).unwrap().agent_id,
            parent_id
        );
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
        assert_eq!(
            table.find_by_session(&channel_sid).unwrap().agent_id,
            parent_id
        );
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

        assert_ne!(p1_sid, p2_sid);
    }

    // -----------------------------------------------------------------------
    // Children index tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_children_index_insert_creates_entry() {
        let table = ProcessTable::new();
        let p = test_process("root", None);
        let id = p.agent_id;
        table.insert(p);

        let children = table.children_of(id);
        assert!(children.is_empty());
    }

    #[test]
    fn test_children_index_parent_child() {
        let table = ProcessTable::new();
        let parent = test_process("parent", None);
        let parent_id = parent.agent_id;
        table.insert(parent);

        let child = test_subagent("child", parent_id);
        let child_id = child.agent_id;
        table.insert(child);

        let children = table.children_of(parent_id);
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].agent_id, child_id);
    }

    #[test]
    fn test_children_index_remove_child() {
        let table = ProcessTable::new();
        let parent = test_process("parent", None);
        let parent_id = parent.agent_id;
        table.insert(parent);

        let child = test_subagent("child", parent_id);
        let child_id = child.agent_id;
        table.insert(child);

        assert_eq!(table.children_of(parent_id).len(), 1);

        table.remove(child_id);
        assert_eq!(table.children_of(parent_id).len(), 0);
    }

    #[test]
    fn test_children_index_multiple_children() {
        let table = ProcessTable::new();
        let parent = test_process("parent", None);
        let parent_id = parent.agent_id;
        table.insert(parent);

        let c1 = test_subagent("c1", parent_id);
        let c1_id = c1.agent_id;
        table.insert(c1);

        let c2 = test_subagent("c2", parent_id);
        let c2_id = c2.agent_id;
        table.insert(c2);

        let children = table.children_of(parent_id);
        assert_eq!(children.len(), 2);
        let ids: Vec<AgentId> = children.iter().map(|c| c.agent_id).collect();
        assert!(ids.contains(&c1_id));
        assert!(ids.contains(&c2_id));
    }

    // -----------------------------------------------------------------------
    // Active count tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_active_count_excludes_ready() {
        let table = SessionTable::new();

        let p1 = test_process("a", None);
        let p1_id = p1.agent_id;
        table.insert(p1);

        let p2 = test_process("b", None);
        table.insert(p2);

        assert_eq!(table.active_count(), 2);

        // Transition p1 to Ready — should NOT count as active.
        table.set_state(p1_id, SessionState::Ready).unwrap();
        assert_eq!(table.active_count(), 1);
    }

    // -----------------------------------------------------------------------
    // Suspended session reap tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_reap_suspended_removes_old_sessions() {
        let table = SessionTable::new();

        let p = test_process("reap-me", None);
        let id = p.agent_id;
        table.insert(p);

        // Suspend the session and backdate finished_at to simulate old suspension.
        table.set_state(id, SessionState::Suspended).unwrap();
        {
            let mut entry = table.processes.get_mut(&id).unwrap();
            let past = Timestamp::from_second(Timestamp::now().as_second() - 120).unwrap();
            entry.finished_at = Some(past);
        }

        // Verify backdating worked.
        let p = table.get(id).unwrap();
        assert!(p.finished_at.is_some(), "finished_at should be set");

        // Reap with 60s TTL — session suspended 120s ago, should be removed.
        let reaped = table.reap_terminal(std::time::Duration::from_secs(60));
        assert_eq!(reaped, 1);
        assert!(table.get(id).is_none());
    }

    #[test]
    fn test_reap_keeps_recently_suspended() {
        let table = SessionTable::new();

        let p = test_process("keep-me", None);
        let id = p.agent_id;
        table.insert(p);

        table.set_state(id, SessionState::Suspended).unwrap();
        // Set finished_at to now.
        {
            let mut entry = table.processes.get_mut(&id).unwrap();
            entry.finished_at = Some(Timestamp::now());
        }

        // Reap with 60s TTL — session just suspended, should be kept.
        let reaped = table.reap_terminal(std::time::Duration::from_secs(60));
        assert_eq!(reaped, 0);
        assert!(table.get(id).is_some());
    }

    #[test]
    fn test_reap_ignores_active_sessions() {
        let table = SessionTable::new();

        let p = test_process("active-session", None);
        let id = p.agent_id;
        table.insert(p);

        // Even with zero TTL, Active sessions should not be reaped.
        let reaped = table.reap_terminal(std::time::Duration::ZERO);
        assert_eq!(reaped, 0);
        assert!(table.get(id).is_some());
    }

    #[tokio::test]
    async fn test_session_stats_finished_at_stays_none() {
        let table = SessionTable::new();

        let p = test_process("fin-test", None);
        let id = p.agent_id;
        table.insert(p);

        // Sessions never set finished_at via set_state.
        let stats = table.process_stats(id).await.unwrap();
        assert!(stats.finished_at.is_none());

        table.set_state(id, SessionState::Ready).unwrap();
        let stats = table.process_stats(id).await.unwrap();
        assert!(stats.finished_at.is_none());

        table.set_state(id, SessionState::Suspended).unwrap();
        let stats = table.process_stats(id).await.unwrap();
        assert!(stats.finished_at.is_none());
    }
}
