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

//! Unified event queue trait — tiered priority queue for all kernel
//! interactions.
//!
//! Defines the [`EventQueue`] trait that all queue implementations must
//! satisfy. The default in-memory implementation ([`InMemoryEventQueue`]) is
//! a thin wrapper around [`ShardQueue`] for backward compatibility and testing.
//!
//! Events are auto-classified into three priority tiers:
//! - **Critical**: signals (Interrupt, Kill, Pause, Resume), shutdown
//! - **Normal**: turn completions, child completions, deliver
//! - **Low**: user messages, spawn requests, timers

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::broadcast;

use super::shard::ShardQueue;
use crate::io::types::BusError;

/// Shared reference to an [`EventQueue`] implementation.
pub type EventQueueRef = Arc<dyn EventQueue>;
// Re-export the kernel event from the sibling module.
pub use crate::event::{EventKind, EventPriority, KernelEvent};

// ---------------------------------------------------------------------------
// EventQueue trait
// ---------------------------------------------------------------------------

/// Trait for tiered priority event queues.
///
/// Implementations must be `Send + Sync + 'static` to be shared across async
/// tasks via `Arc<dyn EventQueue>`.
///
/// `push`, `try_push`, `drain`, and `pending_count` are synchronous — all
/// current implementations use `std::sync::Mutex` with trivial critical
/// sections. Only `wait` is async (it awaits a `tokio::sync::Notify`).
#[async_trait]
pub trait EventQueue: Send + Sync + 'static {
    /// Push an event into the queue. Returns `BusError::Full` if at capacity.
    fn push(&self, event: KernelEvent) -> Result<(), BusError>;

    /// Non-async push (identical to `push` for in-memory queues).
    fn try_push(&self, event: KernelEvent) -> Result<(), BusError>;

    /// Drain up to `max` events from the queue, in priority order.
    fn drain(&self, max: usize) -> Vec<KernelEvent>;

    /// Wait until events are available.
    async fn wait(&self);

    /// Current total pending count across all tiers.
    fn pending_count(&self) -> usize;

    /// Subscribe to enqueued events if this queue supports observation.
    fn subscribe(&self) -> Option<broadcast::Receiver<super::observable::ObservableKernelEvent>> {
        None
    }

    /// Whether this queue is a sharded event queue.
    ///
    /// Returns `true` for
    /// [`ShardedEventQueue`](crate::queue::ShardedEventQueue),
    /// `false` for all other implementations.
    fn is_sharded(&self) -> bool { false }
}

// ---------------------------------------------------------------------------
// InMemoryEventQueue
// ---------------------------------------------------------------------------

/// Tiered priority queue for all kernel interactions (pure in-memory).
///
/// Delegates to [`ShardQueue`] for the actual 3-tier priority queue
/// implementation.
pub struct InMemoryEventQueue {
    inner: ShardQueue,
}

impl InMemoryEventQueue {
    /// Create a new event queue with the given maximum capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: ShardQueue::new(capacity),
        }
    }
}

#[async_trait]
impl EventQueue for InMemoryEventQueue {
    fn push(&self, event: KernelEvent) -> Result<(), BusError> { self.inner.push(event) }

    fn try_push(&self, event: KernelEvent) -> Result<(), BusError> { self.inner.try_push(event) }

    fn drain(&self, max: usize) -> Vec<KernelEvent> { self.inner.drain(max) }

    async fn wait(&self) { self.inner.wait().await }

    fn pending_count(&self) -> usize { self.inner.pending_count() }
}

impl std::fmt::Debug for InMemoryEventQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryEventQueue")
            .field("pending", &self.pending_count())
            .field("inner", &self.inner)
            .finish()
    }
}
