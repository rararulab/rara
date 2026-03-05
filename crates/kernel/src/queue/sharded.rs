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

//! Sharded event queue — routes events to N session-sharded queues + 1 global
//! queue for parallel processing by
//! [`EventProcessor`](crate::processor::EventProcessor)s.
//!
//! Event classification:
//! - **Global**: `UserMessage`, `SpawnAgent`, `Shutdown`, `Deliver`
//! - **Sharded by session_key**: `Syscall`, `TurnCompleted`, `ChildCompleted`,
//!   `SendSignal`
//!
//! Shard index is computed as `session_key.0.as_u128() as usize % num_shards`.

use std::sync::Arc;

use async_trait::async_trait;
use smart_default::SmartDefault;

use super::{in_memory::EventQueue, shard::ShardQueue};
use crate::{event::KernelEventEnvelope, io::IOError};

/// Shared reference to the [`ShardedEventQueue`].
pub type ShardedQueueRef = Arc<ShardedEventQueue>;

// ---------------------------------------------------------------------------
// ShardTarget — classification result
// ---------------------------------------------------------------------------

/// Where an event should be routed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShardTarget {
    /// Route to the global queue (processed by the global EventProcessor).
    Global,
    /// Route to a specific shard (identified by shard index).
    Shard(usize),
}

// ---------------------------------------------------------------------------
// ShardedEventQueue
// ---------------------------------------------------------------------------

/// Configuration for the sharded event queue.
#[derive(Debug, Clone, smart_default::SmartDefault)]
pub struct ShardedEventQueueConfig {
    /// Number of agent shards. Each shard gets its own `EventProcessor`.
    #[default = 2]
    pub num_shards:      usize,
    /// Per-shard capacity (total across all tiers within one shard).
    #[default = 2048]
    pub shard_capacity:  usize,
    /// Global queue capacity.
    #[default = 2048]
    pub global_capacity: usize,
}

/// Returns the number of available CPUs (logical cores).
fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
}

/// Sharded event queue with N agent shards + 1 global queue.
///
/// Implements [`EventQueue`] and provides internal access to individual
/// shards for the multi-processor event loop.
///
/// All shard queues are stored as `Arc<ShardQueue>` so that
/// [`EventProcessor`](crate::processor::EventProcessor) tasks can
/// hold references to them independently.
pub struct ShardedEventQueue {
    /// Per-session shards. Events are routed by `session_key % num_shards`.
    shards: Vec<Arc<ShardQueue>>,
    /// Global queue for non-session-scoped events.
    global: Arc<ShardQueue>,
}

impl ShardedEventQueue {
    /// Create a new sharded event queue with the given configuration.
    pub fn new(config: ShardedEventQueueConfig) -> Self {
        let shards = (0..config.num_shards)
            .map(|_| Arc::new(ShardQueue::new(config.shard_capacity)))
            .collect();
        Self {
            shards,
            global: Arc::new(ShardQueue::new(config.global_capacity)),
        }
    }

    /// Classify a kernel event into its routing target.
    ///
    /// When `num_shards == 0` (single-queue mode), all events are routed to
    /// the global queue regardless of `session_key`.
    pub(crate) fn classify(&self, event: &KernelEventEnvelope) -> ShardTarget {
        if self.shards.is_empty() {
            return ShardTarget::Global;
        }
        match event.shard_key() {
            Some(session_key) => {
                let shard_idx = session_key.0.as_u128() as usize % self.shards.len();
                ShardTarget::Shard(shard_idx)
            }
            None => ShardTarget::Global,
        }
    }

    /// Access a specific shard by index (Arc-wrapped for task sharing).
    pub(crate) fn shard(&self, idx: usize) -> &Arc<ShardQueue> { &self.shards[idx] }

    /// Access the global queue (Arc-wrapped for task sharing).
    pub(crate) fn global(&self) -> &Arc<ShardQueue> { &self.global }

    /// Number of agent shards.
    pub(crate) fn num_shards(&self) -> usize { self.shards.len() }

    fn total_pending(&self) -> usize {
        self.global.pending_count()
            + self
                .shards
                .iter()
                .map(|shard| shard.pending_count())
                .sum::<usize>()
    }
}

#[async_trait]
impl EventQueue for ShardedEventQueue {
    fn push(&self, event: KernelEventEnvelope) -> Result<(), IOError> { self.try_push(event) }

    fn try_push(&self, event: KernelEventEnvelope) -> Result<(), IOError> {
        match self.classify(&event) {
            ShardTarget::Global => self.global.push(event),
            ShardTarget::Shard(idx) => self.shards[idx].push(event),
        }
    }

    fn drain(&self, max: usize) -> Vec<KernelEventEnvelope> {
        // Drain from global first, then round-robin across shards.
        let mut result = Vec::with_capacity(max);
        let mut remaining = max;

        // Global first
        let global_events = self.global.drain(remaining);
        remaining -= global_events.len();
        result.extend(global_events);

        // Then shards
        for shard in &self.shards {
            if remaining == 0 {
                break;
            }
            let shard_events = shard.drain(remaining);
            remaining -= shard_events.len();
            result.extend(shard_events);
        }

        result
    }

    async fn wait(&self) {
        // Fast path: if anything is pending, return immediately.
        if self.pending_count() > 0 {
            return;
        }

        // Wait on the global queue's notify.
        // Each EventProcessor also waits on its own shard independently.
        self.global.wait().await;
    }

    fn pending_count(&self) -> usize { self.total_pending() }

    fn is_sharded(&self) -> bool { !self.shards.is_empty() }
}

impl std::fmt::Debug for ShardedEventQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShardedEventQueue")
            .field("num_shards", &self.shards.len())
            .field("total_pending", &self.total_pending())
            .finish()
    }
}
