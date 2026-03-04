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
    channel::types::ChatMessage,
    event::KernelEvent,
    handle::process_handle::ProcessHandle,
    process::{AgentId, AgentResult},
};

// ---------------------------------------------------------------------------
// ProcessRuntime — per-process mutable state managed by the kernel
// ---------------------------------------------------------------------------

/// Mutable runtime state for each agent process, managed by the kernel's
/// event loop rather than by individual per-process tokio tasks.
///
/// Stored separately from `AgentProcess` (which lives in ProcessTable and must
/// be Clone) because it contains non-Clone types like `CancellationToken` and
/// `Vec<KernelEvent>`.
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
    pub pause_buffer:       Vec<KernelEvent>,
    /// The ProcessHandle for this process (needed to run LLM turns).
    pub handle:             Arc<ProcessHandle>,
    /// Per-agent semaphore limiting concurrent child processes.
    pub child_semaphore:    Arc<Semaphore>,
    /// Maximum context tokens for compaction.
    pub max_context_tokens: usize,
    /// Last successful result (for final output when process ends).
    pub last_result:        Option<AgentResult>,
    /// Global semaphore permit — dropped when this runtime is removed,
    /// automatically releasing one slot for new process spawns.
    pub _global_permit:     OwnedSemaphorePermit,
}

// ---------------------------------------------------------------------------
// RuntimeTable — domain wrapper around DashMap<AgentId, ProcessRuntime>
// ---------------------------------------------------------------------------

/// Table of per-process runtime state, managed by the kernel event loop.
///
/// Keyed by `AgentId`. Created when a process is spawned, removed when it
/// terminates. Wraps a `DashMap` with domain-specific methods for turn
/// control, pause management, and generic access patterns.
pub(crate) struct RuntimeTable {
    inner: DashMap<AgentId, ProcessRuntime>,
}

impl RuntimeTable {
    /// Create a new empty runtime table.
    pub fn new() -> Self {
        Self {
            inner: DashMap::new(),
        }
    }

    /// Insert a new process runtime entry.
    pub fn insert(&self, id: AgentId, rt: ProcessRuntime) { self.inner.insert(id, rt); }

    /// Remove a process runtime entry, returning the key-value pair if it
    /// existed.
    pub fn remove(&self, id: &AgentId) -> Option<(AgentId, ProcessRuntime)> {
        self.inner.remove(id)
    }

    /// Check whether a runtime exists for the given agent.
    pub fn contains(&self, id: &AgentId) -> bool { self.inner.contains_key(id) }

    // -- Turn control -------------------------------------------------------

    /// Cancel the current LLM turn for the given agent.
    pub fn cancel_turn(&self, id: &AgentId) {
        if let Some(rt) = self.inner.get(id) {
            rt.turn_cancel.cancel();
        }
    }

    /// Cancel the current turn and replace the token with a fresh one,
    /// returning the old token. Used by Signal::Interrupt so the next turn
    /// gets a fresh cancellation token.
    pub fn cancel_and_refresh_turn(&self, id: &AgentId) {
        if let Some(mut rt) = self.inner.get_mut(id) {
            rt.turn_cancel.cancel();
            rt.turn_cancel = CancellationToken::new();
        }
    }

    /// Cancel the process-level token (kills the entire process).
    pub fn cancel_process(&self, id: &AgentId) {
        if let Some(rt) = self.inner.get(id) {
            rt.process_cancel.cancel();
        }
    }

    /// Clone the process-level cancellation token for the given agent.
    pub fn clone_process_cancel(&self, id: &AgentId) -> Option<CancellationToken> {
        self.inner.get(id).map(|rt| rt.process_cancel.clone())
    }

    // -- Pause management ---------------------------------------------------

    /// Set the paused flag for the given agent.
    pub fn set_paused(&self, id: &AgentId, paused: bool) {
        if let Some(mut rt) = self.inner.get_mut(id) {
            rt.paused = paused;
        }
    }

    /// Buffer an event for a paused process.
    pub fn buffer_event(&self, id: &AgentId, event: KernelEvent) {
        if let Some(mut rt) = self.inner.get_mut(id) {
            rt.pause_buffer.push(event);
        }
    }

    /// Drain the pause buffer, returning all buffered events.
    pub fn drain_pause_buffer(&self, id: &AgentId) -> Vec<KernelEvent> {
        if let Some(mut rt) = self.inner.get_mut(id) {
            std::mem::take(&mut rt.pause_buffer)
        } else {
            vec![]
        }
    }

    // -- Generic access for complex operations ------------------------------

    /// Read-only access to a process runtime via closure.
    pub fn with<F, R>(&self, id: &AgentId, f: F) -> Option<R>
    where
        F: FnOnce(&ProcessRuntime) -> R,
    {
        self.inner.get(id).map(|rt| f(&rt))
    }

    /// Mutable access to a process runtime via closure.
    pub fn with_mut<F, R>(&self, id: &AgentId, f: F) -> Option<R>
    where
        F: FnOnce(&mut ProcessRuntime) -> R,
    {
        self.inner.get_mut(id).map(|mut rt| f(&mut rt))
    }
}
