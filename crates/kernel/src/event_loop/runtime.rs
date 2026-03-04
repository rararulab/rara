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

//! Per-process mutable runtime state managed by the kernel event loop.

use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::{
    channel::types::ChatMessage, event::KernelEventEnvelope, handle::process_handle::ProcessHandle,
    process::AgentRunLoopResult, session::SessionKey,
};

// ---------------------------------------------------------------------------
// ProcessRuntime — per-process mutable state managed by the kernel
// ---------------------------------------------------------------------------

/// Mutable runtime state for each agent process, managed by the kernel's
/// event loop rather than by individual per-process tokio tasks.
///
/// Stored separately from `SessionRuntime` (which lives in SessionTable and
/// must be Clone) because it contains non-Clone types like `CancellationToken`
/// and `Vec<KernelEvent>`.
///
/// We migration from the per agent process arch to sessionRuntime based
/// architecture.
pub(crate) struct ProcessRuntime {
    /// In-memory conversation history (ChatMessage list).
    pub conversation:       Vec<ChatMessage>,
    /// Per-turn cancellation token — cancelled by Signal::Interrupt to abort
    /// the current LLM call without killing the process.
    pub turn_cancel:        CancellationToken,
    /// Process-level cancellation token — cancelled by Signal::Kill or
    /// Signal::Terminate to shut down the entire process. Child processes
    /// use `parent_token.child_token()` so cancelling a parent cascades.
    pub process_cancel:     CancellationToken,
    /// Whether this process is paused. When true, incoming messages are
    /// buffered in `pause_buffer` instead of being processed.
    pub paused:             bool,
    /// Buffered events received while the process was paused or busy.
    pub pause_buffer:       Vec<KernelEventEnvelope>,
    /// The ProcessHandle for this process (needed to run LLM turns).
    pub handle:             Arc<ProcessHandle>,
    /// Per-agent semaphore limiting concurrent child processes.
    pub child_semaphore:    Arc<Semaphore>,
    /// Maximum context tokens for compaction.
    pub max_context_tokens: usize,
    /// Last successful result (for final output when process ends).
    pub last_result:        Option<AgentRunLoopResult>,
    /// Global semaphore permit — dropped when this runtime is removed,
    /// automatically releasing one slot for new process spawns.
    pub _global_permit:     OwnedSemaphorePermit,
}

// ---------------------------------------------------------------------------
// RuntimeTable — domain wrapper around DashMap<SessionKey, ProcessRuntime>
// ---------------------------------------------------------------------------

/// Table of per-process runtime state, managed by the kernel event loop.
///
/// Keyed by `SessionKey`. Created when a process is spawned, removed when it
/// terminates. Wraps a `DashMap` with domain-specific methods for turn
/// control, pause management, and generic access patterns.
pub(crate) struct RuntimeTable {
    inner: DashMap<SessionKey, ProcessRuntime>,
}

impl RuntimeTable {
    /// Create a new empty runtime table.
    pub fn new() -> Self {
        Self {
            inner: DashMap::new(),
        }
    }

    /// Insert a new process runtime entry.
    pub fn insert(&self, key: SessionKey, rt: ProcessRuntime) { self.inner.insert(key, rt); }

    /// Remove a process runtime entry, returning the key-value pair if it
    /// existed.
    pub fn remove(&self, key: &SessionKey) -> Option<(SessionKey, ProcessRuntime)> {
        self.inner.remove(key)
    }

    /// Check whether a runtime exists for the given agent.
    pub fn contains(&self, key: &SessionKey) -> bool { self.inner.contains_key(key) }

    // -- Turn control -------------------------------------------------------

    /// Cancel the current LLM turn for the given agent.
    pub fn cancel_turn(&self, id: &SessionKey) {
        if let Some(rt) = self.inner.get(id) {
            rt.turn_cancel.cancel();
        }
    }

    /// Cancel the current turn and replace the token with a fresh one,
    /// returning the old token. Used by Signal::Interrupt so the next turn
    /// gets a fresh cancellation token.
    pub fn cancel_and_refresh_turn(&self, id: &SessionKey) {
        if let Some(mut rt) = self.inner.get_mut(id) {
            rt.turn_cancel.cancel();
            rt.turn_cancel = CancellationToken::new();
        }
    }

    /// Cancel the process-level token (kills the entire process).
    pub fn cancel_process(&self, id: &SessionKey) {
        if let Some(rt) = self.inner.get(id) {
            rt.process_cancel.cancel();
        }
    }

    /// Clone the process-level cancellation token for the given agent.
    pub fn clone_process_cancel(&self, id: &SessionKey) -> Option<CancellationToken> {
        self.inner.get(id).map(|rt| rt.process_cancel.clone())
    }

    // -- Pause management ---------------------------------------------------

    /// Set the paused flag for the given agent.
    pub fn set_paused(&self, id: &SessionKey, paused: bool) {
        if let Some(mut rt) = self.inner.get_mut(id) {
            rt.paused = paused;
        }
    }

    /// Buffer an event for a paused process.
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

    /// Read-only access to a process runtime via closure.
    pub fn with<F, R>(&self, id: &SessionKey, f: F) -> Option<R>
    where
        F: FnOnce(&ProcessRuntime) -> R,
    {
        self.inner.get(id).map(|rt| f(&rt))
    }

    /// Mutable access to a process runtime via closure.
    pub fn with_mut<F, R>(&self, id: &SessionKey, f: F) -> Option<R>
    where
        F: FnOnce(&mut ProcessRuntime) -> R,
    {
        self.inner.get_mut(id).map(|mut rt| f(&mut rt))
    }
}
