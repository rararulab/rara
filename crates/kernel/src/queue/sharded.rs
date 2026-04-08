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
//! `EventProcessor`s.
//!
//! Event classification:
//! - **Global**: `UserMessage`, `SpawnAgent`, `Shutdown`
//! - **Sharded by session_key**: `Syscall`, `TurnCompleted`, `ChildCompleted`,
//!   `SendSignal`, `Deliver`
//!
//! Shard index is computed as `session_key.0.as_u128() as usize % num_shards`.

use std::sync::Arc;

use crossbeam_queue::SegQueue;
use tokio::sync;

use crate::{event::KernelEventEnvelope, io::IOError};

/// A single lock-free FIFO shard with async notification.
pub(crate) struct ShardQueue {
    queue:    SegQueue<KernelEventEnvelope>,
    notify:   sync::Notify,
    capacity: usize,
}

impl ShardQueue {
    pub fn new(capacity: usize) -> Self {
        Self {
            queue: SegQueue::new(),
            notify: sync::Notify::new(),
            capacity,
        }
    }

    /// Push an event. Returns `IOError::Full` if at capacity.
    pub fn push(&self, event: KernelEventEnvelope) -> Result<(), IOError> {
        if self.queue.len() >= self.capacity {
            return Err(IOError::Full);
        }
        self.queue.push(event);
        self.notify.notify_one();
        Ok(())
    }

    /// Push an event, returning the rejected event back to the caller on
    /// `Full`. Used by retry helpers that need to keep ownership of the
    /// envelope across attempts (events are not `Clone` because they may
    /// carry oneshot reply channels).
    pub fn push_returning(
        &self,
        event: KernelEventEnvelope,
    ) -> Result<(), (IOError, KernelEventEnvelope)> {
        if self.queue.len() >= self.capacity {
            return Err((IOError::Full, event));
        }
        self.queue.push(event);
        self.notify.notify_one();
        Ok(())
    }

    /// Lazy iterator that pops up to `max` events. Zero allocation.
    pub fn drain(&self, max: usize) -> impl Iterator<Item = KernelEventEnvelope> + '_ {
        let mut remaining = max;
        std::iter::from_fn(move || {
            if remaining == 0 {
                return None;
            }
            let event = self.queue.pop()?;
            remaining -= 1;
            Some(event)
        })
    }

    /// Wait until events are available.
    pub async fn wait(&self) {
        // Register the notification future BEFORE checking emptiness to avoid
        // a race where push() + notify_one() lands between the check and .await.
        let notified = self.notify.notified();
        if !self.queue.is_empty() {
            return;
        }
        notified.await;
    }

    pub fn pending_count(&self) -> usize { self.queue.len() }
}

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
/// Provides the push ingress path and internal access to individual shards
/// for the multi-processor event loop.
///
/// All shard queues are stored as `Arc<ShardQueue>` so that
/// `EventProcessor` tasks can
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
                let shard_idx = session_key.as_uuid().as_u128() as usize % self.shards.len();
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

    /// Push an event into the queue, routing it to the correct shard or the
    /// global queue.
    ///
    /// Returns `Err(IOError::Full)` if the target queue is at capacity.
    /// Callers in the ingress path SHOULD use
    /// [`push_with_retry`](crate::queue::push_with_retry) to handle
    /// backpressure — silent drops are a reliability bug (see issue #1148).
    pub fn push(&self, event: KernelEventEnvelope) -> Result<(), IOError> {
        match self.classify(&event) {
            ShardTarget::Global => self.global.push(event),
            ShardTarget::Shard(idx) => self.shards[idx].push(event),
        }
    }

    /// Non-blocking push — identical to [`push`](Self::push) for this
    /// in-memory queue.
    pub fn try_push(&self, event: KernelEventEnvelope) -> Result<(), IOError> { self.push(event) }

    /// Push an event, returning the rejected envelope back to the caller on
    /// `Full` so it can be reused across retry attempts.
    ///
    /// This is the entry point used by
    /// [`push_with_retry`](crate::queue::push_with_retry); direct callers
    /// should keep using [`push`](Self::push)/[`try_push`](Self::try_push).
    pub fn push_returning(
        &self,
        event: KernelEventEnvelope,
    ) -> Result<(), (IOError, KernelEventEnvelope)> {
        match self.classify(&event) {
            ShardTarget::Global => self.global.push_returning(event),
            ShardTarget::Shard(idx) => self.shards[idx].push_returning(event),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::event::KernelEventEnvelope;

    #[tokio::test]
    async fn wait_returns_after_push() {
        let shared = Arc::new(ShardQueue::new(16));
        let shared2 = Arc::clone(&shared);

        let pusher = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            shared2
                .push(KernelEventEnvelope::shutdown())
                .expect("push should succeed");
        });

        // wait() should return once the push + notify lands — not hang forever.
        tokio::time::timeout(Duration::from_secs(2), shared.wait())
            .await
            .expect("wait() should not hang — notification must not be lost");

        pusher.await.expect("pusher task should complete");

        // Verify the event can be drained.
        let events: Vec<_> = shared.drain(10).collect();
        assert_eq!(events.len(), 1, "expected exactly one event after push");
    }

    #[tokio::test]
    async fn wait_returns_immediately_when_non_empty() {
        let queue = Arc::new(ShardQueue::new(16));
        queue
            .push(KernelEventEnvelope::shutdown())
            .expect("push should succeed");

        // wait() should return immediately since queue is non-empty.
        tokio::time::timeout(Duration::from_millis(100), queue.wait())
            .await
            .expect("wait() should return immediately for non-empty queue");
    }
}
