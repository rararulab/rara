// Copyright 2025 Crrow
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

//! Hybrid event queue — memory fast path + WAL persistence.
//!
//! All events flow through the in-memory tiered queue for low-latency
//! processing. Persistable events (UserMessage, SpawnAgent, Timer) are
//! additionally written to the WAL so they can survive a crash.
//!
//! On [`drain`](HybridQueue::drain), each drained event is tagged with
//! its WAL ID (if any). The event loop calls
//! [`mark_completed`](HybridQueue::mark_completed) after processing each
//! event so the WAL can garbage-collect.
//!
//! ## Crash Recovery
//!
//! Call [`HybridQueue::recover`] at startup. It reads the WAL, pushes all
//! non-completed entries into the in-memory queue, and returns the count
//! of recovered events.

use std::{
    collections::VecDeque,
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;

use rara_kernel::{
    event_queue::{EventQueue, KernelEvent},
    io::types::BusError,
    unified_event::PersistableEvent,
};

use crate::wal::{WalError, WalQueue};

// ---------------------------------------------------------------------------
// HybridQueue
// ---------------------------------------------------------------------------

/// Hybrid event queue: in-memory tiered priority queue + WAL persistence.
///
/// Implements [`EventQueue`] so it can be used as a drop-in replacement
/// for `InMemoryEventQueue`.
pub struct HybridQueue {
    /// Three tiers: [Critical, Normal, Low].
    queues:   [Mutex<VecDeque<KernelEvent>>; 3],
    /// Async notification for the event loop.
    notify:   tokio::sync::Notify,
    /// Total pending event count across all tiers.
    pending:  AtomicUsize,
    /// Maximum capacity (total across all tiers).
    capacity: usize,
    /// WAL for durable persistence of selected events.
    wal:      WalQueue,
    /// Mapping from pending_count index to WAL ID.
    /// When an event is pushed *and* written to WAL, we record the
    /// WAL entry ID. On drain, we return events in priority order; the
    /// consumer calls `mark_completed(wal_id)` after processing.
    ///
    /// We store a per-tier deque of `Option<u64>` (None = not persisted,
    /// Some(id) = WAL entry ID).
    wal_ids:  [Mutex<VecDeque<Option<u64>>>; 3],
}

impl HybridQueue {
    /// Create a new hybrid queue backed by a WAL at `wal_path`.
    ///
    /// - `capacity`: maximum total events across all tiers.
    /// - `wal_path`: file system path for the WAL file.
    /// - `truncate_after`: auto-truncation threshold for completed entries.
    pub fn open(
        capacity: usize,
        wal_path: impl AsRef<Path>,
        truncate_after: usize,
    ) -> Result<Self, WalError> {
        let wal = WalQueue::open(wal_path, truncate_after)?;
        Ok(Self {
            queues: [
                Mutex::new(VecDeque::new()),
                Mutex::new(VecDeque::new()),
                Mutex::new(VecDeque::new()),
            ],
            notify: tokio::sync::Notify::new(),
            pending: AtomicUsize::new(0),
            capacity,
            wal,
            wal_ids: [
                Mutex::new(VecDeque::new()),
                Mutex::new(VecDeque::new()),
                Mutex::new(VecDeque::new()),
            ],
        })
    }

    /// Replay non-completed WAL entries into the in-memory queue.
    ///
    /// Call this once at startup, before the event loop begins draining.
    /// Returns the number of recovered events.
    pub fn recover(&self) -> Result<usize, WalError> {
        let entries = self.wal.recover()?;
        let count = entries.len();

        for (wal_id, persistable) in entries {
            let (event, _rx) = persistable.into_kernel_event();
            let tier = event.priority() as usize;

            let mut q = self.queues[tier].lock().expect("queue lock poisoned");
            q.push_back(event);
            drop(q);

            let mut ids = self.wal_ids[tier].lock().expect("wal_ids lock poisoned");
            ids.push_back(Some(wal_id));

            self.pending.fetch_add(1, Ordering::Release);
        }

        if count > 0 {
            self.notify.notify_one();
            tracing::info!(recovered = count, "WAL recovery complete");
        }

        Ok(count)
    }

    /// Access the underlying WAL (for diagnostics/testing).
    pub fn wal(&self) -> &WalQueue {
        &self.wal
    }
}

#[async_trait]
impl EventQueue for HybridQueue {
    async fn push(&self, event: KernelEvent) -> Result<(), BusError> {
        self.try_push(event)
    }

    fn try_push(&self, event: KernelEvent) -> Result<(), BusError> {
        let current = self.pending.load(Ordering::Acquire);
        if current >= self.capacity {
            return Err(BusError::Full);
        }

        // WAL persistence for durable events.
        let wal_id = PersistableEvent::from_kernel_event(&event)
            .and_then(|pe| {
                match self.wal.append(&pe) {
                    Ok(id) => Some(id),
                    Err(e) => {
                        tracing::warn!(error = %e, "WAL append failed; event is memory-only");
                        None
                    }
                }
            });

        let tier = event.priority() as usize;

        let mut q = self.queues[tier].lock().expect("queue lock poisoned");
        q.push_back(event);
        drop(q);

        let mut ids = self.wal_ids[tier].lock().expect("wal_ids lock poisoned");
        ids.push_back(wal_id);
        drop(ids);

        self.pending.fetch_add(1, Ordering::Release);
        self.notify.notify_one();
        Ok(())
    }

    async fn drain(&self, max: usize) -> Vec<(KernelEvent, Option<u64>)> {
        let mut result = Vec::with_capacity(max);
        let mut remaining = max;

        // Drain in priority order: Critical (0) -> Normal (1) -> Low (2)
        for tier in 0..3 {
            if remaining == 0 {
                break;
            }
            let mut q = self.queues[tier].lock().expect("queue lock poisoned");
            let mut ids = self.wal_ids[tier].lock().expect("wal_ids lock poisoned");
            let n = remaining.min(q.len());
            let events = q.drain(..n);
            let wal_ids = ids.drain(..n);
            result.extend(events.zip(wal_ids));
            remaining -= n;
        }

        let drained = result.len();
        if drained > 0 {
            self.pending.fetch_sub(drained, Ordering::Release);
        }

        result
    }

    async fn wait(&self) {
        if self.pending.load(Ordering::Acquire) > 0 {
            return;
        }
        self.notify.notified().await;
    }

    fn pending_count(&self) -> usize {
        self.pending.load(Ordering::Acquire)
    }

    fn mark_completed(&self, wal_id: u64) {
        if let Err(e) = self.wal.mark_completed(wal_id) {
            tracing::warn!(wal_id, error = %e, "WAL mark_completed failed");
        }
    }
}

impl std::fmt::Debug for HybridQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HybridQueue")
            .field("pending", &self.pending_count())
            .field("capacity", &self.capacity)
            .field("wal", &self.wal)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rara_kernel::{
        channel::types::{ChannelType, MessageContent},
        io::types::{ChannelSource, InboundMessage, MessageId},
        process::{AgentId, SessionId, Signal, principal::UserId},
    };
    use std::collections::HashMap;

    fn test_inbound(text: &str) -> InboundMessage {
        InboundMessage {
            id:              MessageId::new(),
            source:          ChannelSource {
                channel_type:        ChannelType::Internal,
                platform_message_id: None,
                platform_user_id:    "test".to_string(),
                platform_chat_id:    None,
            },
            user:            UserId("u1".to_string()),
            session_id:      SessionId::new("s1"),
            target_agent_id: None,
            target_agent:    None,
            content:         MessageContent::Text(text.to_string()),
            reply_context:   None,
            timestamp:       jiff::Timestamp::now(),
            metadata:        HashMap::new(),
        }
    }

    #[tokio::test]
    async fn hybrid_push_and_drain() {
        let dir = tempfile::tempdir().unwrap();
        let q = HybridQueue::open(100, dir.path().join("test.wal"), 50).unwrap();

        q.push(KernelEvent::UserMessage(test_inbound("hello")))
            .await
            .unwrap();
        q.push(KernelEvent::UserMessage(test_inbound("world")))
            .await
            .unwrap();

        assert_eq!(q.pending_count(), 2);

        let events = q.drain(10).await;
        assert_eq!(events.len(), 2);
        assert_eq!(q.pending_count(), 0);
    }

    #[tokio::test]
    async fn hybrid_priority_ordering() {
        let dir = tempfile::tempdir().unwrap();
        let q = HybridQueue::open(100, dir.path().join("test.wal"), 50).unwrap();

        // Low priority
        q.push(KernelEvent::UserMessage(test_inbound("low")))
            .await
            .unwrap();
        // Critical priority
        q.push(KernelEvent::SendSignal {
            target: AgentId::new(),
            signal: Signal::Kill,
        })
        .await
        .unwrap();

        let events = q.drain(10).await;
        assert_eq!(events.len(), 2);
        // Critical should come first
        assert!(matches!(events[0].0, KernelEvent::SendSignal { .. }));
        assert!(matches!(events[1].0, KernelEvent::UserMessage(_)));
    }

    #[tokio::test]
    async fn hybrid_capacity_full() {
        let dir = tempfile::tempdir().unwrap();
        let q = HybridQueue::open(2, dir.path().join("test.wal"), 50).unwrap();

        q.push(KernelEvent::UserMessage(test_inbound("a")))
            .await
            .unwrap();
        q.push(KernelEvent::UserMessage(test_inbound("b")))
            .await
            .unwrap();

        let result = q.push(KernelEvent::UserMessage(test_inbound("c"))).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), BusError::Full));
    }

    #[tokio::test]
    async fn hybrid_wal_persistence_and_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let wal_path = dir.path().join("test.wal");

        // Push events, simulating a crash (drop without draining).
        {
            let q = HybridQueue::open(100, &wal_path, 50).unwrap();
            q.push(KernelEvent::UserMessage(test_inbound("msg1")))
                .await
                .unwrap();
            q.push(KernelEvent::UserMessage(test_inbound("msg2")))
                .await
                .unwrap();
            q.push(KernelEvent::Timer {
                name:    "tick".to_string(),
                payload: serde_json::json!(null),
            })
            .await
            .unwrap();
        }

        // Reopen and recover.
        let q = HybridQueue::open(100, &wal_path, 50).unwrap();
        let recovered = q.recover().unwrap();
        assert_eq!(recovered, 3);
        assert_eq!(q.pending_count(), 3);

        let events = q.drain(10).await;
        assert_eq!(events.len(), 3);
    }

    #[tokio::test]
    async fn hybrid_mark_completed_prevents_re_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let wal_path = dir.path().join("test.wal");

        // Push and process one event.
        {
            let q = HybridQueue::open(100, &wal_path, 50).unwrap();
            q.push(KernelEvent::UserMessage(test_inbound("a")))
                .await
                .unwrap();
            q.push(KernelEvent::UserMessage(test_inbound("b")))
                .await
                .unwrap();

            // Mark the first event (WAL ID 1) as completed.
            q.mark_completed(1);
        }

        // Reopen and recover — only "b" should be recovered.
        let q = HybridQueue::open(100, &wal_path, 50).unwrap();
        let recovered = q.recover().unwrap();
        assert_eq!(recovered, 1);
    }

    #[tokio::test]
    async fn hybrid_transient_events_not_in_wal() {
        let dir = tempfile::tempdir().unwrap();
        let wal_path = dir.path().join("test.wal");

        {
            let q = HybridQueue::open(100, &wal_path, 50).unwrap();
            // Signal is transient — should NOT be in WAL
            q.push(KernelEvent::SendSignal {
                target: AgentId::new(),
                signal: Signal::Kill,
            })
            .await
            .unwrap();
            q.push(KernelEvent::Shutdown).await.unwrap();
        }

        // Reopen — WAL should be empty (transient events not persisted)
        let q = HybridQueue::open(100, &wal_path, 50).unwrap();
        let recovered = q.recover().unwrap();
        assert_eq!(recovered, 0);
    }

    #[tokio::test]
    async fn hybrid_wait_wakeup() {
        let q = std::sync::Arc::new(
            HybridQueue::open(100, tempfile::tempdir().unwrap().path().join("test.wal"), 50)
                .unwrap(),
        );
        let q2 = q.clone();

        let handle = tokio::spawn(async move {
            q2.wait().await;
            let events = q2.drain(10).await;
            assert_eq!(events.len(), 1);
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        q.push(KernelEvent::UserMessage(test_inbound("wake")))
            .await
            .unwrap();

        handle.await.unwrap();
    }

    #[test]
    fn hybrid_debug_format() {
        let dir = tempfile::tempdir().unwrap();
        let q = HybridQueue::open(100, dir.path().join("test.wal"), 50).unwrap();
        let debug = format!("{:?}", q);
        assert!(debug.contains("HybridQueue"));
        assert!(debug.contains("capacity: 100"));
    }
}
