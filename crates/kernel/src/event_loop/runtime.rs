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

/// Table of per-process runtime state, managed by the kernel event loop.
///
/// Keyed by `AgentId`. Created when a process is spawned, removed when it
/// terminates.
pub(crate) type RuntimeTable = DashMap<AgentId, ProcessRuntime>;
