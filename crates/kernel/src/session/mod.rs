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

//! Unified session types and traits.
//!
//! This module is the canonical source of truth for session-related types
//! used across the kernel and downstream crates (rara-sessions, rara-app,
//! etc.). Session metadata is managed via [`SessionIndex`]. Message
//! persistence is handled by the tape subsystem (`crate::memory`).

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering},
    },
};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use snafu::Snafu;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::{
    agent::{AgentEnv, AgentManifest, ExecutionMode, TurnTrace},
    error::Result as KernelResult,
    event::KernelEventEnvelope,
    identity::Principal,
    io::Endpoint,
};

// ---------------------------------------------------------------------------
// SessionError
// ---------------------------------------------------------------------------

/// Errors that can occur during session persistence operations.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum SessionError {
    /// The requested session was not found.
    #[snafu(display("session not found: {key}"))]
    NotFound { key: String },

    /// A session with this key already exists.
    #[snafu(display("session already exists: {key}"))]
    AlreadyExists { key: String },

    /// The session key is malformed.
    #[snafu(display("invalid session key: {message}"))]
    InvalidKey { message: String },

    /// The fork point is out of range.
    #[snafu(display("invalid fork point: seq {seq} is out of range for session {key}"))]
    InvalidForkPoint { key: String, seq: i64 },

    /// A file I/O error occurred while reading/writing message JSONL files.
    #[snafu(display("message file I/O error: {source}"))]
    FileIo { source: std::io::Error },

    /// A JSON serialization/deserialization error occurred.
    #[snafu(display("json error: {source}"))]
    Json { source: serde_json::Error },
}

// ---------------------------------------------------------------------------
// SessionKey
// ---------------------------------------------------------------------------

base::define_id!(
    /// Opaque UUID identifier for a chat session.
    ///
    /// Use [`try_from_raw`](Self::try_from_raw) only when reading from a
    /// trusted source (e.g. persisted session index).
    SessionKey
);

// ---------------------------------------------------------------------------
// SessionEntry
// ---------------------------------------------------------------------------

/// A persisted chat session with metadata.
///
/// Each session is uniquely identified by its [`SessionKey`] and tracks
/// message count, model configuration, and a short preview of the
/// conversation for UI display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEntry {
    /// Unique session key (serves as primary key in the database).
    pub key:           SessionKey,
    /// Human-readable title / label shown in session lists.
    pub title:         Option<String>,
    /// LLM model name used for this session (e.g. `"gpt-4o"`,
    /// `"claude-sonnet-4-5-20250929"`).
    pub model:         Option<String>,
    /// Optional system prompt override. When `None`, the service-level
    /// default system prompt is used.
    pub system_prompt: Option<String>,
    /// Running total of messages in this session.
    pub message_count: i64,
    /// Short preview text (typically the first user message, truncated)
    /// for display in session listings.
    pub preview:       Option<String>,
    /// Arbitrary JSON metadata for client-specific extensions.
    pub metadata:      Option<serde_json::Value>,
    /// When the session was first created.
    pub created_at:    DateTime<Utc>,
    /// When the session was last modified (message appended, metadata
    /// changed, etc.).
    pub updated_at:    DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// ChannelBinding
// ---------------------------------------------------------------------------

/// Maps an external channel to a [`SessionKey`].
///
/// Channel bindings allow external messaging platforms (Telegram, Slack, etc.)
/// to route incoming messages to the correct session without the caller
/// needing to know the internal session key.
///
/// The composite key `(channel_type, chat_id, thread_id)` is unique;
/// upserting a binding with the same composite key will update the
/// target session. For non-threaded channels, `thread_id` is `None`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelBinding {
    /// Channel type identifier, e.g.
    /// [`crate::channel::types::ChannelType::Telegram`],
    /// [`crate::channel::types::ChannelType::Web`].
    pub channel_type: crate::channel::types::ChannelType,
    /// External chat or conversation identifier within the channel
    /// (e.g. Telegram chat id, Slack channel id).
    pub chat_id:      String,
    /// Optional thread/topic identifier within the chat (e.g. Telegram
    /// forum topic). `None` for non-threaded conversations.
    #[serde(default)]
    pub thread_id:    Option<String>,
    /// The internal session key this binding resolves to.
    pub session_key:  SessionKey,
    /// When this binding was first created.
    pub created_at:   DateTime<Utc>,
    /// When this binding was last updated (e.g. re-pointed to a new session).
    pub updated_at:   DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// SessionIndex — lightweight metadata-only session interface
// ---------------------------------------------------------------------------

/// Shared reference to a [`SessionIndex`] implementation.
pub type SessionIndexRef = Arc<dyn SessionIndex>;

/// Lightweight session metadata index — no message storage.
///
/// `SessionIndex` only manages session metadata and channel bindings.
/// Message persistence is handled by the tape subsystem.
#[async_trait]
pub trait SessionIndex: Send + Sync + 'static {
    /// Persist a new session entry.
    async fn create_session(&self, entry: &SessionEntry) -> Result<SessionEntry, SessionError>;

    /// Retrieve a session by its key, or `None` if it does not exist.
    async fn get_session(&self, key: &SessionKey) -> Result<Option<SessionEntry>, SessionError>;

    /// List sessions, ordered by `updated_at` descending.
    async fn list_sessions(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<SessionEntry>, SessionError>;

    /// Update mutable session fields.
    async fn update_session(&self, entry: &SessionEntry) -> Result<SessionEntry, SessionError>;

    /// Delete a session.
    async fn delete_session(&self, key: &SessionKey) -> Result<(), SessionError>;

    /// Upsert a channel binding.
    async fn bind_channel(&self, binding: &ChannelBinding) -> Result<ChannelBinding, SessionError>;

    /// Resolve a channel binding by `(channel_type, chat_id, thread_id)`.
    async fn get_channel_binding(
        &self,
        channel_type: crate::channel::types::ChannelType,
        chat_id: &str,
        thread_id: Option<&str>,
    ) -> Result<Option<ChannelBinding>, SessionError>;

    /// Resolve the first channel binding that points to the given session.
    ///
    /// Returns `Ok(None)` when no binding exists or the implementation does
    /// not support reverse lookups. Channels use this to route responses
    /// (e.g. approval prompts) back to the originating chat.
    async fn get_channel_binding_by_session(
        &self,
        _key: &SessionKey,
    ) -> Result<Option<ChannelBinding>, SessionError> {
        Ok(None)
    }

    /// Remove all channel bindings that point to the given session.
    async fn unbind_session(&self, key: &SessionKey) -> Result<(), SessionError>;
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

/// Error returned when a requested [`SessionState`] transition is not
/// allowed from the current state.
///
/// Transitions are validated by the `Session::{activate, finish_turn, pause,
/// resume, suspend}` methods. Callers that need an unconditional override
/// (e.g. kernel `Kill`/`Terminate` signals) should use
/// [`SessionTable::force_state`] instead of the validated methods.
#[derive(Debug, Snafu)]
#[snafu(display("invalid session state transition: {from} -> {to}"))]
pub struct InvalidTransitionError {
    /// The current state that rejected the transition.
    pub from: SessionState,
    /// The target state that was requested.
    pub to:   SessionState,
}

/// Result of a completed agent process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRunLoopResult {
    /// The agent's final output text.
    pub output:     String,
    /// Number of LLM iterations consumed.
    pub iterations: usize,
    /// Number of tool calls made.
    pub tool_calls: usize,
    /// Whether the agent loop completed successfully (false when max
    /// iterations exhausted or an error occurred).
    #[serde(default = "default_success")]
    pub success:    bool,
}

/// Serde default for backward-compatible deserialization of persisted
/// results that lack the `success` field.
///
/// Defaults to `true` because historical results were only persisted on
/// successful completion — failed turns did not write an
/// `AgentRunLoopResult` to the session store.
fn default_success() -> bool { true }

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

/// Tracks a background child agent spawned by this session.
#[derive(Debug, Clone)]
pub struct BackgroundTaskEntry {
    /// Child session key (doubles as task_id).
    pub child_key:          SessionKey,
    /// Human-readable name from the spawned manifest.
    pub agent_name:         String,
    /// Description provided by the parent agent.
    pub description:        String,
    /// When the task was spawned.
    pub created_at:         jiff::Timestamp,
    /// The inbound message that triggered the spawn.
    pub trigger_message_id: crate::io::MessageId,
}

/// A running session instance in the session table.
#[derive(Debug)]
pub struct Session {
    // -- Identity & metadata --
    /// The session's conversation storage key.
    pub session_key: SessionKey,
    /// Parent session (None for root-level sessions).
    pub parent_id: Option<SessionKey>,
    /// The agent definition driving this session.
    pub manifest: AgentManifest,
    /// The identity under which this session runs.
    pub principal: Principal,
    /// Per-session environment.
    pub env: AgentEnv,
    /// Current lifecycle state.
    ///
    /// Crate-private on purpose: external callers must read via
    /// [`Session::state`] and transition via the validated methods
    /// (`activate`, `finish_turn`, `pause`, `resume`, `suspend`) or the
    /// unconditional [`SessionTable::force_state`] escape hatch. Direct
    /// assignment is only permitted within `rara-kernel` itself, where the
    /// `Session` is initially constructed in
    /// `Kernel::register_process_runtime`.
    pub(crate) state: SessionState,
    /// When this session was created.
    pub created_at: Timestamp,
    /// When this session was last active (for idle timeout).
    pub finished_at: Option<Timestamp>,
    /// Result of last execution (set on turn completion).
    pub result: Option<AgentRunLoopResult>,
    /// Channel sender for streaming `AgentEvent`s (milestones + final result)
    /// to the parent. Only set for child agents spawned via `spawn_child`.
    pub result_tx: Option<tokio::sync::mpsc::Sender<crate::io::AgentEvent>>,
    /// Files created or modified by this agent (for resource tracking).
    pub created_files: Vec<PathBuf>,
    /// Per-session runtime metrics (atomic counters for lock-free updates).
    pub metrics: Arc<RuntimeMetrics>,
    /// Detailed turn traces for observability (most recent 50 turns).
    pub turn_traces: Vec<TurnTrace>,
    // -- Cancellation --
    /// Per-turn cancellation token.
    pub turn_cancel: CancellationToken,
    /// Session-level cancellation token.
    pub process_cancel: CancellationToken,
    /// Execution mode override for this session. When `Some`, this takes
    /// priority over the agent manifest's `default_execution_mode`.
    /// Set via the `/msg_version` kernel command.
    pub execution_mode: Option<ExecutionMode>,
    /// Whether this session is paused.
    pub paused: bool,
    /// Buffered events received while the session was paused or busy.
    pub pause_buffer: Vec<KernelEventEnvelope>,
    /// Flag set when a new user message arrives during an active turn.
    /// The agent loop checks this between iterations and breaks if set.
    pub interrupted: Arc<AtomicBool>,
    /// Wakes the agent loop immediately when the `interrupted` flag is set,
    /// so it does not have to wait for the next iteration boundary.
    pub interrupt_notify: Arc<tokio::sync::Notify>,
    /// Active background tasks spawned by this session.
    pub background_tasks: Vec<BackgroundTaskEntry>,
    /// Pending tool call limit oneshot sender keyed by limit_id. When the
    /// agent loop pauses at the tool call limit, it registers a `(limit_id,
    /// sender)` here. Only a callback carrying the matching `limit_id` can
    /// resolve it, preventing stale buttons from resolving a newer limit.
    pub pending_tool_call_limit: Option<(
        u64,
        tokio::sync::oneshot::Sender<crate::io::ToolCallLimitDecision>,
    )>,
    /// The channel endpoint that originated this session (e.g. a specific
    /// Telegram chat). Used as a fallback for reply routing when the
    /// triggering message is synthetic (no platform origin).
    pub origin_endpoint: Option<Endpoint>,
    /// Deferred tools activated via `discover-tools` during this session.
    /// Persists across turns so the LLM does not need to re-discover tools
    /// after each user message.
    pub activated_deferred: std::collections::HashSet<crate::tool::ToolName>,
    /// Per-session semaphore limiting concurrent child sessions.
    pub child_semaphore: Arc<Semaphore>,
    /// Permit from the *parent*'s `child_semaphore`.
    /// Held for the lifetime of this child session; dropping it releases the
    /// slot so the parent can spawn another child.
    pub(crate) _parent_child_permit: Option<OwnedSemaphorePermit>,
    /// Global semaphore permit.
    pub(crate) _global_permit: OwnedSemaphorePermit,
}

impl Session {
    /// Read the current lifecycle state.
    pub fn state(&self) -> SessionState { self.state }

    /// `Ready` → `Active`. Called at the start of a turn when the session
    /// begins processing an inbound message.
    pub fn activate(&mut self) -> Result<(), InvalidTransitionError> {
        match self.state {
            SessionState::Ready => {
                self.state = SessionState::Active;
                Ok(())
            }
            other => Err(InvalidTransitionError {
                from: other,
                to:   SessionState::Active,
            }),
        }
    }

    /// `Active` → `Ready`. Called at the end of a turn when the session has
    /// finished processing an inbound message.
    pub fn finish_turn(&mut self) -> Result<(), InvalidTransitionError> {
        match self.state {
            SessionState::Active => {
                self.state = SessionState::Ready;
                Ok(())
            }
            other => Err(InvalidTransitionError {
                from: other,
                to:   SessionState::Ready,
            }),
        }
    }

    /// `Ready`/`Suspended` → `Paused`. Manual pause via signal.
    ///
    /// Forcing a pause from `Active` requires finishing the current turn
    /// first (or using [`SessionTable::force_state`] as an escape hatch).
    pub fn pause(&mut self) -> Result<(), InvalidTransitionError> {
        match self.state {
            SessionState::Ready | SessionState::Suspended => {
                self.state = SessionState::Paused;
                Ok(())
            }
            other => Err(InvalidTransitionError {
                from: other,
                to:   SessionState::Paused,
            }),
        }
    }

    /// `Paused` → `Ready`. Resume a manually paused session.
    pub fn resume(&mut self) -> Result<(), InvalidTransitionError> {
        match self.state {
            SessionState::Paused => {
                self.state = SessionState::Ready;
                Ok(())
            }
            other => Err(InvalidTransitionError {
                from: other,
                to:   SessionState::Ready,
            }),
        }
    }

    /// `Ready` → `Suspended`. Called on idle timeout when the session's
    /// ambient resources should be released.
    ///
    /// Forcing suspension from `Active` (e.g. `Kill`/`Terminate` signals)
    /// requires [`SessionTable::force_state`].
    pub fn suspend(&mut self) -> Result<(), InvalidTransitionError> {
        match self.state {
            SessionState::Ready => {
                self.state = SessionState::Suspended;
                Ok(())
            }
            other => Err(InvalidTransitionError {
                from: other,
                to:   SessionState::Suspended,
            }),
        }
    }
}

/// Per-process runtime metrics using atomic counters for lock-free updates.
#[derive(Debug)]
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
    /// Create a new `RuntimeMetrics` with all counters zeroed.
    pub fn new() -> Self {
        Self {
            messages_received: AtomicU64::new(0),
            llm_calls:         AtomicU64::new(0),
            tool_calls:        AtomicU64::new(0),
            tokens_consumed:   AtomicU64::new(0),
            last_activity:     AtomicI64::new(0),
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

/// Extended runtime statistics for a single session.
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

/// Kernel-wide aggregate statistics.
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
pub struct SessionTable {
    runtimes:        DashMap<SessionKey, Session>,
    /// Parent → Children index, O(1) child lookup.
    children_index:  DashMap<SessionKey, Vec<SessionKey>>,
    /// Monotonically increasing counter of total processes ever spawned.
    total_spawned:   AtomicU64,
    /// Total processes that completed successfully.
    total_completed: AtomicU64,
    /// Total processes that failed.
    total_failed:    AtomicU64,
}

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
            total_spawned:   AtomicU64::new(0),
            total_completed: AtomicU64::new(0),
            total_failed:    AtomicU64::new(0),
        }
    }

    /// Read-only access to a session runtime via closure.
    pub fn with<F, R>(&self, key: &SessionKey, f: F) -> Option<R>
    where
        F: FnOnce(&Session) -> R,
    {
        self.runtimes.get(key).map(|r| f(r.value()))
    }

    /// Mutable access to a session runtime via closure.
    pub fn with_mut<F, R>(&self, key: &SessionKey, f: F) -> Option<R>
    where
        F: FnOnce(&mut Session) -> R,
    {
        self.runtimes.get_mut(key).map(|mut r| f(r.value_mut()))
    }

    /// Insert a process into the table.
    #[tracing::instrument(skip(self, sr), fields(session_key = %sr.session_key, agent_name = %sr.manifest.name))]
    pub fn insert(&self, sr: Session) {
        let session_key = sr.session_key;
        if let Some(parent_id) = sr.parent_id {
            self.children_index
                .entry(parent_id)
                .or_default()
                .push(session_key);
        }
        self.children_index.entry(session_key).or_default();
        self.total_spawned.fetch_add(1, Ordering::Relaxed);
        self.runtimes.insert(session_key, sr);
    }

    /// Unconditionally overwrite a session's lifecycle state.
    ///
    /// This is the escape hatch used by signals like `Kill`/`Terminate` that
    /// must move a session to `Suspended` regardless of its current state.
    /// Normal lifecycle transitions MUST use the validated methods on
    /// [`Session`] (`activate`, `finish_turn`, `pause`, `resume`, `suspend`).
    #[tracing::instrument(skip(self), fields(new_state = %state))]
    pub fn force_state(&self, key: SessionKey, state: SessionState) -> KernelResult<()> {
        let mut entry = self
            .runtimes
            .get_mut(&key)
            .ok_or(crate::error::KernelError::SessionNotFound { key })?;
        entry.state = state;
        Ok(())
    }

    /// Set the result of a process.
    pub fn set_result(&self, key: SessionKey, result: AgentRunLoopResult) -> KernelResult<()> {
        let mut entry = self
            .runtimes
            .get_mut(&key)
            .ok_or(crate::error::KernelError::SessionNotFound { key })?;
        entry.result = Some(result);
        Ok(())
    }

    /// Remove a process from the table, returning it if it existed.
    #[tracing::instrument(skip(self))]
    pub fn remove(&self, id: SessionKey) -> Option<Session> {
        let removed = self.runtimes.remove(&id).map(|(_, p)| p);
        if let Some(ref process) = removed {
            if let Some(parent_id) = process.parent_id {
                if let Some(mut children) = self.children_index.get_mut(&parent_id) {
                    children.retain(|c| *c != id);
                }
            }
            self.children_index.remove(&id);
        }
        removed
    }

    /// List all children of a given parent (O(1) lookup via children_index).
    pub fn children_of(&self, parent_id: SessionKey) -> Vec<SessionStats> {
        let child_ids = self
            .children_index
            .get(&parent_id)
            .map(|ids| ids.clone())
            .unwrap_or_default();
        child_ids.iter().filter_map(|id| self.stats(*id)).collect()
    }

    /// List all processes.
    pub fn list(&self) -> Vec<SessionStats> {
        let ids: Vec<SessionKey> = self.runtimes.iter().map(|p| p.session_key).collect();
        ids.iter().filter_map(|id| self.stats(*id)).collect()
    }

    /// Push a turn trace onto a process, evicting the oldest if at capacity.
    pub fn push_turn_trace(&self, id: SessionKey, trace: TurnTrace) {
        if let Some(mut entry) = self.runtimes.get_mut(&id) {
            if entry.turn_traces.len() >= Self::MAX_TURN_TRACES {
                entry.turn_traces.remove(0);
            }
            entry.turn_traces.push(trace);
        }
    }

    /// Get the turn traces for a process.
    pub fn get_turn_traces(&self, key: SessionKey) -> Vec<TurnTrace> {
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

    /// Get a shared reference to the metrics for a process.
    pub fn get_metrics(&self, id: &SessionKey) -> Option<Arc<RuntimeMetrics>> {
        self.runtimes.get(id).map(|p| Arc::clone(&p.metrics))
    }

    /// Build a [`SessionStats`] snapshot for a single session.
    pub fn stats(&self, id: SessionKey) -> Option<SessionStats> {
        self.with(&id, |p| {
            let children: Vec<SessionKey> = self
                .children_index
                .get(&id)
                .map(|ids| ids.clone())
                .unwrap_or_default();
            let uptime_ms = Timestamp::now()
                .since(p.created_at)
                .ok()
                .map(|span| span.get_milliseconds().unsigned_abs())
                .unwrap_or(0);
            let m = &p.metrics;
            let last_ts = m.last_activity.load(Ordering::Relaxed);
            SessionStats {
                session_key: p.session_key,
                manifest_name: p.manifest.name.clone(),
                state: p.state,
                parent_id: p.parent_id,
                children,
                created_at: p.created_at,
                finished_at: p.finished_at,
                uptime_ms,
                messages_received: m.messages_received.load(Ordering::Relaxed),
                llm_calls: m.llm_calls.load(Ordering::Relaxed),
                tool_calls: m.tool_calls.load(Ordering::Relaxed),
                tokens_consumed: m.tokens_consumed.load(Ordering::Relaxed),
                last_activity: if last_ts == 0 {
                    None
                } else {
                    Timestamp::from_microsecond(last_ts).ok()
                },
            }
        })
    }

    /// Build [`SessionStats`] for all sessions currently in the table.
    ///
    /// Also performs lazy reaping of suspended sessions older than the TTL.
    pub fn all_process_stats(&self) -> Vec<SessionStats> {
        self.reap_terminal(Self::TERMINAL_TTL);
        let ids: Vec<SessionKey> = self.runtimes.iter().map(|p| p.session_key).collect();
        ids.iter().filter_map(|id| self.stats(*id)).collect()
    }

    /// Remove suspended sessions whose last activity is older than `max_age`.
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
            .map(|entry| entry.session_key)
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

    /// Check whether a runtime exists for the given session.
    pub fn contains(&self, key: &SessionKey) -> bool { self.runtimes.contains_key(key) }

    /// Cancel the current LLM turn for the given session.
    pub fn cancel_turn(&self, id: &SessionKey) {
        if let Some(rt) = self.runtimes.get(id) {
            rt.turn_cancel.cancel();
        }
    }

    /// Cancel the current turn and replace the token with a fresh one.
    pub fn cancel_and_refresh_turn(&self, id: &SessionKey) {
        if let Some(mut rt) = self.runtimes.get_mut(id) {
            rt.turn_cancel.cancel();
            rt.turn_cancel = CancellationToken::new();
        }
    }

    /// Cancel the session-level token (kills the entire session).
    pub fn cancel_process(&self, id: &SessionKey) {
        if let Some(rt) = self.runtimes.get(id) {
            rt.process_cancel.cancel();
        }
    }

    /// Clone the session-level cancellation token for the given session.
    pub fn clone_process_cancel(&self, id: &SessionKey) -> Option<CancellationToken> {
        self.runtimes.get(id).map(|rt| rt.process_cancel.clone())
    }

    /// Set the paused flag for the given session.
    pub fn set_paused(&self, id: &SessionKey, paused: bool) {
        if let Some(mut rt) = self.runtimes.get_mut(id) {
            rt.paused = paused;
        }
    }

    /// Buffer an event for a paused session.
    pub fn buffer_event(&self, id: &SessionKey, event: KernelEventEnvelope) {
        if let Some(mut rt) = self.runtimes.get_mut(id) {
            rt.pause_buffer.push(event);
        }
    }

    /// Drain the pause buffer, returning all buffered events.
    pub fn drain_pause_buffer(&self, id: &SessionKey) -> Vec<KernelEventEnvelope> {
        if let Some(mut rt) = self.runtimes.get_mut(id) {
            std::mem::take(&mut rt.pause_buffer)
        } else {
            vec![]
        }
    }
}

impl Default for SessionTable {
    fn default() -> Self { Self::new() }
}

pub mod test_utils;

#[cfg(test)]
impl Session {
    /// Build a minimal `Session` for unit tests that exercise the per-session
    /// child concurrency semaphore. All other fields are zeroed/empty.
    pub(crate) fn test_stub(
        session_key: SessionKey,
        parent_id: Option<SessionKey>,
        child_limit: usize,
        global_semaphore: Arc<Semaphore>,
    ) -> Self {
        use crate::{
            agent::{AgentEnv, AgentManifest, AgentRole, Priority},
            identity::{KernelUser, Permission, Principal, Role},
        };

        let global_permit = global_semaphore
            .try_acquire_owned()
            .expect("global semaphore must have capacity for test stubs");

        let principal = Principal::from_user(&KernelUser {
            name:        "test".into(),
            role:        Role::Root,
            permissions: vec![Permission::All],
            enabled:     true,
        });

        Self {
            session_key,
            parent_id,
            manifest: AgentManifest {
                name:                   "test".into(),
                role:                   AgentRole::default(),
                description:            "test stub".into(),
                model:                  None,
                system_prompt:          String::new(),
                soul_prompt:            None,
                provider_hint:          None,
                max_iterations:         None,
                tools:                  vec![],
                excluded_tools:         vec![],
                max_children:           Some(child_limit),
                max_context_tokens:     None,
                priority:               Priority::default(),
                metadata:               serde_json::Value::Null,
                sandbox:                None,
                default_execution_mode: None,
                tool_call_limit:        None,
                worker_timeout_secs:    None,
                max_continuations:      None,
            },
            principal,
            env: AgentEnv::default(),
            state: SessionState::Ready,
            created_at: Timestamp::now(),
            finished_at: None,
            result: None,
            result_tx: None,
            created_files: vec![],
            metrics: Arc::new(RuntimeMetrics::new()),
            turn_traces: vec![],
            turn_cancel: CancellationToken::new(),
            process_cancel: CancellationToken::new(),
            execution_mode: None,
            paused: false,
            pause_buffer: Vec::new(),
            interrupted: Arc::new(AtomicBool::new(false)),
            interrupt_notify: Arc::new(tokio::sync::Notify::new()),
            background_tasks: Vec::new(),
            pending_tool_call_limit: None,
            origin_endpoint: None,
            activated_deferred: std::collections::HashSet::new(),
            child_semaphore: Arc::new(Semaphore::new(child_limit)),
            _parent_child_permit: None,
            _global_permit: global_permit,
        }
    }
}

#[cfg(test)]
mod state_transition_tests {
    use std::sync::Arc;

    use tokio::sync::Semaphore;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::{
        agent::{AgentEnv, AgentManifest, AgentRole, Priority},
        identity::{KernelUser, Principal, Role},
    };

    fn make_manifest() -> AgentManifest {
        AgentManifest {
            name:                   "test".into(),
            role:                   AgentRole::default(),
            description:            "test agent".into(),
            model:                  None,
            system_prompt:          String::new(),
            soul_prompt:            None,
            provider_hint:          None,
            max_iterations:         None,
            tools:                  vec![],
            excluded_tools:         vec![],
            max_children:           None,
            max_context_tokens:     None,
            priority:               Priority::default(),
            metadata:               serde_json::Value::Null,
            sandbox:                None,
            default_execution_mode: None,
            tool_call_limit:        None,
            worker_timeout_secs:    None,
            max_continuations:      None,
        }
    }

    fn make_session(state: SessionState) -> Session {
        let global = Arc::new(Semaphore::new(1));
        let permit = global
            .try_acquire_owned()
            .expect("global semaphore must have a free permit");
        Session {
            session_key: SessionKey::new(),
            parent_id: None,
            manifest: make_manifest(),
            principal: Principal::from_user(&KernelUser {
                name:        "test-user".into(),
                role:        Role::User,
                permissions: vec![],
                enabled:     true,
            }),
            env: AgentEnv::default(),
            state,
            created_at: Timestamp::now(),
            finished_at: None,
            result: None,
            result_tx: None,
            created_files: vec![],
            metrics: Arc::new(RuntimeMetrics::new()),
            turn_traces: vec![],
            turn_cancel: CancellationToken::new(),
            process_cancel: CancellationToken::new(),
            execution_mode: None,
            paused: false,
            pause_buffer: vec![],
            interrupted: Arc::new(AtomicBool::new(false)),
            interrupt_notify: Arc::new(tokio::sync::Notify::new()),
            background_tasks: vec![],
            pending_tool_call_limit: None,
            origin_endpoint: None,
            activated_deferred: Default::default(),
            child_semaphore: Arc::new(Semaphore::new(1)),
            _parent_child_permit: None,
            _global_permit: permit,
        }
    }

    #[test]
    fn legal_transitions_succeed() {
        // Ready -> Active -> Ready
        let mut s = make_session(SessionState::Ready);
        s.activate().expect("Ready -> Active is legal");
        assert_eq!(s.state(), SessionState::Active);
        s.finish_turn().expect("Active -> Ready is legal");
        assert_eq!(s.state(), SessionState::Ready);

        // Ready -> Suspended
        s.suspend().expect("Ready -> Suspended is legal");
        assert_eq!(s.state(), SessionState::Suspended);

        // Suspended -> Paused -> Ready
        s.pause().expect("Suspended -> Paused is legal");
        assert_eq!(s.state(), SessionState::Paused);
        s.resume().expect("Paused -> Ready is legal");
        assert_eq!(s.state(), SessionState::Ready);

        // Ready -> Paused
        s.pause().expect("Ready -> Paused is legal");
        assert_eq!(s.state(), SessionState::Paused);
    }

    #[test]
    fn illegal_transitions_return_error() {
        // Active cannot activate again.
        let mut s = make_session(SessionState::Active);
        let err = s.activate().expect_err("Active -> Active is illegal");
        assert_eq!(err.from, SessionState::Active);
        assert_eq!(err.to, SessionState::Active);
        assert_eq!(s.state(), SessionState::Active, "state must be unchanged");

        // Ready cannot finish a turn it never started.
        let mut s = make_session(SessionState::Ready);
        let err = s.finish_turn().expect_err("Ready -> Ready is illegal");
        assert_eq!(err.from, SessionState::Ready);
        assert_eq!(err.to, SessionState::Ready);

        // Active cannot be paused directly.
        let mut s = make_session(SessionState::Active);
        let err = s.pause().expect_err("Active -> Paused is illegal");
        assert_eq!(err.from, SessionState::Active);
        assert_eq!(err.to, SessionState::Paused);
        assert_eq!(s.state(), SessionState::Active);

        // Suspended cannot resume directly (must pause first).
        let mut s = make_session(SessionState::Suspended);
        let err = s.resume().expect_err("Suspended -> Ready is illegal");
        assert_eq!(err.from, SessionState::Suspended);
        assert_eq!(err.to, SessionState::Ready);

        // Active cannot suspend itself (must use force_state).
        let mut s = make_session(SessionState::Active);
        let err = s.suspend().expect_err("Active -> Suspended is illegal");
        assert_eq!(err.from, SessionState::Active);
        assert_eq!(err.to, SessionState::Suspended);
    }
}
