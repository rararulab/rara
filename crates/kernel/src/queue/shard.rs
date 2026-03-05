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

//! Per-shard priority queue — building block for [`ShardedEventQueue`].
//!
//! Each `ShardQueue` is a 3-tier priority queue with its own `Notify` and
//! pending counter. The `ShardedEventQueue` holds N+1 of these (N agent
//! shards + 1 global shard).

use std::{
    collections::VecDeque,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use tokio::sync;

use crate::{event::KernelEventEnvelope, io::IOError};

/// A single priority shard — 3-tier queue with async notification.
///
/// This is the same data structure as `InMemoryEventQueue` but without
/// the `EventQueue` trait impl. It is designed to be composed inside
/// `ShardedEventQueue`.
pub(crate) struct ShardQueue {
    /// Three tiers: [Critical, Normal, Low].
    queues:   [Mutex<VecDeque<KernelEventEnvelope>>; 3],
    /// Async notification for the event processor.
    notify:   sync::Notify,
    /// Total pending event count across all tiers.
    pending:  AtomicUsize,
    /// Maximum capacity (total across all tiers).
    capacity: usize,
}

impl ShardQueue {
    /// Create a new shard queue with the given maximum capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            queues: [
                Mutex::new(VecDeque::new()),
                Mutex::new(VecDeque::new()),
                Mutex::new(VecDeque::new()),
            ],
            notify: sync::Notify::new(),
            pending: AtomicUsize::new(0),
            capacity,
        }
    }

    /// Push an event into the queue. Returns `BusError::Full` if at capacity.
    pub fn push(&self, event: KernelEventEnvelope) -> Result<(), IOError> {
        let current = self.pending.load(Ordering::Acquire);
        if current >= self.capacity {
            return Err(IOError::Full);
        }

        let tier = event.priority() as usize;
        let mut q = self.queues[tier].lock().expect("shard queue lock poisoned");
        q.push_back(event);
        drop(q);

        self.pending.fetch_add(1, Ordering::Release);
        self.notify.notify_one();
        Ok(())
    }

    /// Non-blocking push (same as `push` since we never await).
    pub fn try_push(&self, event: KernelEventEnvelope) -> Result<(), IOError> { self.push(event) }

    /// Drain up to `max` events from the queue, in priority order.
    pub fn drain(&self, max: usize) -> Vec<KernelEventEnvelope> {
        let mut result = Vec::with_capacity(max);
        let mut remaining = max;

        // Drain in priority order: Critical (0) -> Normal (1) -> Low (2)
        for tier in 0..3 {
            if remaining == 0 {
                break;
            }
            let mut q = self.queues[tier].lock().expect("shard queue lock poisoned");
            let n = remaining.min(q.len());
            result.extend(q.drain(..n));
            remaining -= n;
        }

        let drained = result.len();
        if drained > 0 {
            self.pending.fetch_sub(drained, Ordering::Release);
        }

        result
    }

    /// Wait until events are available.
    pub async fn wait(&self) {
        if self.pending.load(Ordering::Acquire) > 0 {
            return;
        }
        self.notify.notified().await;
    }

    /// Current total pending count across all tiers.
    pub fn pending_count(&self) -> usize { self.pending.load(Ordering::Acquire) }
}
