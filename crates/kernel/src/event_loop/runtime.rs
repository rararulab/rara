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

//! Per-session mutable runtime state managed by the kernel event loop.

use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::{
    channel::types::ChatMessage, event::KernelEventEnvelope,
    process::{AgentRunLoopResult, principal::Principal}, session::SessionKey,
};

// ---------------------------------------------------------------------------
// SessionContext — per-session mutable state managed by the kernel
// ---------------------------------------------------------------------------

/// Mutable runtime state for each agent session, managed by the kernel's
/// event loop rather than by individual per-session tokio tasks.
///
/// Stored separately from `SessionRuntime` (which lives in SessionTable and
/// must be Clone) because it contains non-Clone types like `CancellationToken`
/// and `Vec<KernelEvent>`.
pub(crate) struct SessionContext {
    /// In-memory conversation history (ChatMessage list).
    pub conversation:       Vec<ChatMessage>,
    /// Per-turn cancellation token — cancelled by Signal::Interrupt to abort
    /// the current LLM call without killing the session.
    pub turn_cancel:        CancellationToken,
    /// Session-level cancellation token — cancelled by Signal::Kill or
    /// Signal::Terminate to shut down the entire session. Child sessions
    /// use `parent_token.child_token()` so cancelling a parent cascades.
    pub process_cancel:     CancellationToken,
    /// Whether this session is paused. When true, incoming messages are
    /// buffered in `pause_buffer` instead of being processed.
    pub paused:             bool,
    /// Buffered events received while the session was paused or busy.
    pub pause_buffer:       Vec<KernelEventEnvelope>,
    /// The session key for this session.
    pub session_key:        SessionKey,
    /// The principal (identity) under which this session runs.
    pub principal:          Principal,
    /// Per-session semaphore limiting concurrent child sessions.
    pub child_semaphore:    Arc<Semaphore>,
    /// Maximum context tokens for compaction.
    pub max_context_tokens: usize,
    /// Last successful result (for final output when session ends).
    pub last_result:        Option<AgentRunLoopResult>,
    /// Global semaphore permit — dropped when this context is removed,
    /// automatically releasing one slot for new session spawns.
    pub _global_permit:     OwnedSemaphorePermit,
}

// ---------------------------------------------------------------------------
// RuntimeTable — domain wrapper around DashMap<SessionKey, SessionContext>
// ---------------------------------------------------------------------------

/// Table of per-session runtime state, managed by the kernel event loop.
///
/// Keyed by `SessionKey`. Created when a session is spawned, removed when it
/// terminates. Wraps a `DashMap` with domain-specific methods for turn
/// control, pause management, and generic access patterns.
pub(crate) struct RuntimeTable {
    inner: DashMap<SessionKey, SessionContext>,
}

impl RuntimeTable {
    /// Create a new empty runtime table.
    pub fn new() -> Self {
        Self {
            inner: DashMap::new(),
        }
    }

    /// Insert a new session context entry.
    pub fn insert(&self, key: SessionKey, rt: SessionContext) { self.inner.insert(key, rt); }

    /// Remove a session context entry, returning the key-value pair if it
    /// existed.
    pub fn remove(&self, key: &SessionKey) -> Option<(SessionKey, SessionContext)> {
        self.inner.remove(key)
    }

    /// Check whether a runtime exists for the given session.
    pub fn contains(&self, key: &SessionKey) -> bool { self.inner.contains_key(key) }

    // -- Turn control -------------------------------------------------------

    /// Cancel the current LLM turn for the given session.
    pub fn cancel_turn(&self, id: &SessionKey) {
        if let Some(rt) = self.inner.get(id) {
            rt.turn_cancel.cancel();
        }
    }

    /// Cancel the current turn and replace the token with a fresh one.
    /// Used by Signal::Interrupt so the next turn gets a fresh cancellation
    /// token.
    pub fn cancel_and_refresh_turn(&self, id: &SessionKey) {
        if let Some(mut rt) = self.inner.get_mut(id) {
            rt.turn_cancel.cancel();
            rt.turn_cancel = CancellationToken::new();
        }
    }

    /// Cancel the session-level token (kills the entire session).
    pub fn cancel_process(&self, id: &SessionKey) {
        if let Some(rt) = self.inner.get(id) {
            rt.process_cancel.cancel();
        }
    }

    /// Clone the session-level cancellation token for the given session.
    pub fn clone_process_cancel(&self, id: &SessionKey) -> Option<CancellationToken> {
        self.inner.get(id).map(|rt| rt.process_cancel.clone())
    }

    // -- Pause management ---------------------------------------------------

    /// Set the paused flag for the given session.
    pub fn set_paused(&self, id: &SessionKey, paused: bool) {
        if let Some(mut rt) = self.inner.get_mut(id) {
            rt.paused = paused;
        }
    }

    /// Buffer an event for a paused session.
    pub fn buffer_event(&self, id: &SessionKey, event: KernelEventEnvelope) {
        if let Some(mut rt) = self.inner.get_mut(id) {
            rt.pause_buffer.push(event);
        }
    }

    /// Drain the pause buffer, returning all buffered events.
    pub fn drain_pause_buffer(&self, id: &SessionKey) -> Vec<KernelEventEnvelope> {
        if let Some(mut rt) = self.inner.get_mut(id) {
            std::mem::take(&mut rt.pause_buffer)
        } else {
            vec![]
        }
    }

    // -- Generic access for complex operations ------------------------------

    /// Read-only access to a session context via closure.
    pub fn with<F, R>(&self, id: &SessionKey, f: F) -> Option<R>
    where
        F: FnOnce(&SessionContext) -> R,
    {
        self.inner.get(id).map(|rt| f(&rt))
    }

    /// Mutable access to a session context via closure.
    pub fn with_mut<F, R>(&self, id: &SessionKey, f: F) -> Option<R>
    where
        F: FnOnce(&mut SessionContext) -> R,
    {
        self.inner.get_mut(id).map(|mut rt| f(&mut rt))
    }
}
