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
//! - [`sessionKey`] = unique per-execution identifier (kept for audit/security
//!   compatibility)
//! - [`SessionId`] = persistent conversation identifier (the primary
//!   identifier)
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
        atomic::{AtomicI64, AtomicU64, Ordering},
    },
};

use dashmap::DashMap;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{error::Result, session::SessionKey};

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
    pub role:               AgentRole,
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

impl SessionState {
    /// Whether this state is terminal (session no longer accepts messages).
    ///
    /// In the session-centric model, sessions are never truly terminal —
    /// they transition to Suspended instead. This always returns false.
    pub fn is_terminal(self) -> bool { false }
}

/// Result of a completed agent process.
#[derive(Debug, Clone, Serialize)]
pub struct AgentRunLoopResult {
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
#[derive(Debug, Clone, Serialize, Default)]
pub struct AgentEnv {
    /// Optional workspace directory for file operations.
    pub workspace: Option<String>,
    /// Key-value environment variables.
    pub vars:      HashMap<String, String>,
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
/// is created with a unique [`sessionKey`] (for audit/security compatibility),
/// the spawning principal, and the manifest that defines its behavior.
/// Sessions are long-lived — they transition between
/// Active/Ready/Suspended/Paused rather than being created and destroyed per
/// message.
// FIXME: it should not be cloneable!
#[derive(Debug, Clone)]
pub struct SessionRuntime {
    /// The session's conversation storage key.
    pub session_key:        SessionKey,
    /// Parent session (None for root-level sessions).
    pub parent_id:          Option<SessionKey>,
    /// External channel binding (e.g., `web:chat123`). Only set for root
    /// sessions that entered via an external channel adapter. Used by
    /// `session_index` for routing inbound messages to the correct session.
    /// Child sessions have `None` — they are only reachable via
    /// `SessionHandle`.
    pub channel_session_id: Option<SessionKey>,
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
    pub result:             Option<AgentRunLoopResult>,
    /// Files created or modified by this agent (for resource tracking).
    pub created_files:      Vec<PathBuf>,
    /// Per-session runtime metrics (atomic counters for lock-free updates).
    pub metrics:            Arc<RuntimeMetrics>,
    /// Detailed turn traces for observability (most recent 50 turns).
    pub turn_traces:        Vec<crate::agent_turn::TurnTrace>,
}

/// Summary info for listing sessions.
///
/// A lightweight view of a [`SessionRuntime`] suitable for display in
/// session listings without exposing full internal state.
#[derive(Debug, Clone, Serialize)]
pub struct SessionInfo {
    pub session_key: SessionKey,
    pub parent_id:   Option<SessionKey>,
    pub name:        String,
    pub state:       SessionState,
    pub created_at:  Timestamp,
}

impl From<&SessionRuntime> for SessionInfo {
    fn from(p: &SessionRuntime) -> Self {
        Self {
            session_key: p.agent_id,
            parent_id:   p.parent_id,
            name:        p.manifest.name.clone(),
            state:       p.state,
            created_at:  p.created_at,
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
#[derive(Debug, Default)]
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
    pub last_activity:     AtomicI64,
}

impl RuntimeMetrics {
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
        self.last_activity
            .store(Timestamp::now().as_microsecond(), Ordering::Relaxed);
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
    /// Session conversation key.
    pub session_key:       SessionKey,
    /// The manifest name (agent definition name).
    pub manifest_name:     String,
    /// Current lifecycle state.
    pub state:             SessionState,
    /// Parent session, if any.
    pub parent_id:         Option<SessionKey>,
    /// IDs of child sessions.
    pub children:          Vec<SessionKey>,
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
/// Includes a session index for fast `SessionId -> sessionKey` lookups,
/// a name index for fast agent name -> `sessionKey` lookups, and
/// a mailbox registry for sending messages to long-lived sessions.
pub struct SessionTable {
    runtimes:        DashMap<SessionKey, SessionRuntime>,
    /// Parent → Children index, O(1) child lookup.
    children_index:  DashMap<SessionKey, Vec<SessionKey>>,
    /// Agent manifest name → Vec<SessionKey> (1:N, observability only).
    /// TODO: we don't need this i think. since we can have multiple sessions
    /// with the same manifest, it's not super useful to look up by name. we can
    /// remove this and just do a full scan of the processes table when we want
    /// to find all sessions with a given manifest name.
    name_registry:   DashMap<String, Vec<SessionKey>>,
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
            runtimes:        DashMap::new(),
            children_index:  DashMap::new(),
            name_registry:   DashMap::new(),
            total_spawned:   AtomicU64::new(0),
            total_completed: AtomicU64::new(0),
            total_failed:    AtomicU64::new(0),
        }
    }

    /// Insert a process into the table.
    #[tracing::instrument(skip(self, sr), fields(session_key = %sr.session_key, agent_name = %sr.manifest.name))]
    pub fn insert(&self, sr: SessionRuntime) {
        let session_key = sr.session_key;
        if let Some(ref channel_sid) = sr.channel_session_id {
            self.session_index.insert(channel_sid.clone(), session_key);
        }
        // Children index: register under parent
        if let Some(parent_id) = sr.parent_id {
            self.children_index
                .entry(parent_id)
                .or_default()
                .push(session_key);
        }
        // Initialize empty children list for this process
        self.children_index.entry(session_key).or_default();
        // Name registry (1:N)
        self.name_registry
            .entry(sr.manifest.name.clone())
            .or_default()
            .push(session_key);
        self.total_spawned.fetch_add(1, Ordering::Relaxed);
        self.runtimes.insert(session_key, sr);
    }

    /// Transition a session to a new state.
    ///
    /// Sessions are long-lived and never reach a terminal state. State
    /// transitions are: Active (processing), Ready (idle), Suspended
    /// (timed-out), Paused (manual hold).
    #[tracing::instrument(skip(self), fields(new_state = %state))]
    pub fn set_state(&self, key: SessionKey, state: SessionState) -> Result<()> {
        let mut entry = self
            .runtimes
            .get_mut(&key)
            .ok_or(crate::error::KernelError::SessionRuntimeNotFound { key })?;
        entry.state = state;
        // Sessions are long-lived — no terminal states, no finished_at.
        Ok(())
    }

    /// Set the result of a process.
    pub fn set_result(&self, key: SessionKey, result: AgentRunLoopResult) -> Result<()> {
        let mut entry = self
            .runtimes
            .get_mut(&key)
            .ok_or(crate::error::KernelError::SessionRuntimeNotFound { key })?;
        entry.result = Some(result);
        Ok(())
    }

    /// Remove a process from the table, returning it if it existed.
    #[tracing::instrument(skip(self))]
    pub fn remove(&self, id: SessionKey) -> Option<SessionRuntime> {
        let removed = self.runtimes.remove(&id).map(|(_, p)| p);
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
    pub fn children_of(&self, parent_id: SessionKey) -> Vec<SessionInfo> {
        let child_ids = self
            .children_index
            .get(&parent_id)
            .map(|ids| ids.clone())
            .unwrap_or_default();
        child_ids
            .iter()
            .filter_map(|id| self.runtimes.get(id).map(|p| SessionInfo::from(p.value())))
            .collect()
    }

    /// List all processes.
    pub fn list(&self) -> Vec<SessionInfo> {
        self.runtimes
            .iter()
            .map(|p| SessionInfo::from(p.value()))
            .collect()
    }

    /// Push a turn trace onto a process, evicting the oldest if at capacity.
    pub fn push_turn_trace(&self, id: SessionKey, trace: crate::agent_turn::TurnTrace) {
        if let Some(mut entry) = self.runtimes.get_mut(&id) {
            if entry.turn_traces.len() >= Self::MAX_TURN_TRACES {
                entry.turn_traces.remove(0);
            }
            entry.turn_traces.push(trace);
        }
    }

    /// Get the turn traces for a process.
    pub fn get_turn_traces(&self, key: SessionKey) -> Vec<crate::agent_turn::TurnTrace> {
        self.runtimes
            .get(&key)
            .map(|p| p.turn_traces.clone())
            .unwrap_or_default()
    }

    /// Count active sessions (those currently processing a message).
    pub fn active_count(&self) -> usize {
        self.runtimes
            .iter()
            .filter(|p| p.state == SessionState::Active)
            .count()
    }

    /// Backwards-compatible alias.
    pub fn running_count(&self) -> usize { self.active_count() }

    // ----- Session index methods -----

    /// Find the active agent process for a session.
    pub fn find_by_session(&self, session_id: &SessionKey) -> Option<AgentProcess> {
        let agent_id = self.session_index.get(session_id)?;
        self.get(*agent_id)
    }

    /// Find agent processes by manifest name (returns the most recently
    /// inserted).
    pub fn find_by_name(&self, name: &str) -> Option<SessionRuntime> {
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
    pub fn bind_session(&self, session_id: SessionKey, agent_id: SessionKey) {
        self.session_index.insert(session_id, agent_id);
    }

    /// Remove a session index entry only if it points to the given agent.
    pub fn session_index_remove(&self, session_id: &SessionKey, agent_id: SessionKey) {
        self.session_index
            .remove_if(session_id, |_, aid| *aid == agent_id);
    }

    // ----- Metrics methods -----

    /// Get a shared reference to the metrics for a process.
    pub fn get_metrics(&self, id: &SessionKey) -> Option<Arc<RuntimeMetrics>> {
        self.runtimes.get(id).map(|p| Arc::clone(&p.metrics))
    }

    /// Build a [`SessionStats`] snapshot for a single session.
    pub async fn stats(&self, id: SessionKey) -> Option<SessionStats> {
        let process = self.get(id)?;
        let metrics_snapshot = process.metrics.snapshot().await;
        let children: Vec<SessionKey> = self
            .children_index
            .get(&id)
            .map(|ids| ids.clone())
            .unwrap_or_default();
        let uptime_ms = Timestamp::now()
            .since(process.created_at)
            .ok()
            .map(|span| span.get_milliseconds().unsigned_abs())
            .unwrap_or(0);

        Some(SessionStats {
            session_key: process.session_id,
            manifest_name: process.manifest.name,
            state: process.state,
            parent_id: process.parent_id,
            children,
            created_at: process.created_at,
            finished_at: process.finished_at,
            uptime_ms,
            messages_received: self.messages_received.load(Ordering::Relaxed),
            llm_calls: self.llm_calls.load(Ordering::Relaxed),
            tool_calls: self.tool_calls.load(Ordering::Relaxed),
            tokens_consumed: self.tokens_consumed.load(Ordering::Relaxed),
            last_activity: self.last_activity.load(Ordering::Relaxed),
        })
    }

    /// Build [`SessionStats`] for all sessions currently in the table.
    ///
    /// Also performs lazy reaping of suspended sessions older than the TTL.
    pub async fn all_process_stats(&self) -> Vec<SessionStats> {
        // Lazy reap: remove stale terminal processes on observation.
        self.reap_terminal(Self::TERMINAL_TTL);

        let ids: Vec<SessionKey> = self.runtimes.iter().map(|p| p.agent_id).collect();
        let mut stats = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(s) = self.stats(id).await {
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
        let to_remove: Vec<SessionKey> = self
            .runtimes
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
        self.runtimes
            .iter()
            .map(|p| p.metrics.tokens_consumed.load(Ordering::Relaxed))
            .sum()
    }
}

impl Default for SessionTable {
    fn default() -> Self { Self::new() }
}
