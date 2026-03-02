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

use crate::{io::types::BusError, unified_event::KernelEvent};

/// A single priority shard — 3-tier queue with async notification.
///
/// This is the same data structure as `InMemoryEventQueue` but without
/// the `EventQueue` trait impl. It is designed to be composed inside
/// `ShardedEventQueue`.
pub(crate) struct ShardQueue {
    /// Three tiers: [Critical, Normal, Low].
    queues:   [Mutex<VecDeque<KernelEvent>>; 3],
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
    pub fn push(&self, event: KernelEvent) -> Result<(), BusError> {
        let current = self.pending.load(Ordering::Acquire);
        if current >= self.capacity {
            return Err(BusError::Full);
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
    pub fn try_push(&self, event: KernelEvent) -> Result<(), BusError> { self.push(event) }

    /// Drain up to `max` events from the queue, in priority order.
    ///
    /// Returns `(event, wal_id)` pairs. The `wal_id` is always `None` for
    /// in-memory shard queues.
    pub fn drain(&self, max: usize) -> Vec<(KernelEvent, Option<u64>)> {
        let mut result = Vec::with_capacity(max);
        let mut remaining = max;

        // Drain in priority order: Critical (0) -> Normal (1) -> Low (2)
        for tier in 0..3 {
            if remaining == 0 {
                break;
            }
            let mut q = self.queues[tier].lock().expect("shard queue lock poisoned");
            let n = remaining.min(q.len());
            result.extend(q.drain(..n).map(|e| (e, None)));
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

impl std::fmt::Debug for ShardQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShardQueue")
            .field("pending", &self.pending_count())
            .field("capacity", &self.capacity)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::{
        channel::types::{ChannelType, MessageContent},
        io::types::{ChannelSource, InboundMessage, MessageId},
        process::{AgentId, SessionId, Signal, principal::UserId},
        unified_event::KernelEvent,
    };

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

    #[test]
    fn test_push_and_drain() {
        let q = ShardQueue::new(100);

        q.push(KernelEvent::UserMessage(test_inbound("hello")))
            .unwrap();
        q.push(KernelEvent::UserMessage(test_inbound("world")))
            .unwrap();

        assert_eq!(q.pending_count(), 2);

        let events = q.drain(10);
        assert_eq!(events.len(), 2);
        assert_eq!(q.pending_count(), 0);
    }

    #[test]
    fn test_priority_ordering() {
        let q = ShardQueue::new(100);

        // Push in reverse priority order: Low, Normal, Critical
        q.push(KernelEvent::UserMessage(test_inbound("low")))
            .unwrap();
        q.push(KernelEvent::Deliver(crate::io::types::OutboundEnvelope {
            id:          MessageId::new(),
            in_reply_to: MessageId::new(),
            user:        UserId("u1".to_string()),
            session_id:  SessionId::new("s1"),
            routing:     crate::io::types::OutboundRouting::BroadcastAll,
            payload:     crate::io::types::OutboundPayload::Reply {
                content:     MessageContent::Text("normal".to_string()),
                attachments: vec![],
            },
            timestamp:   jiff::Timestamp::now(),
        }))
        .unwrap();
        q.push(KernelEvent::SendSignal {
            target: AgentId::new(),
            signal: Signal::Interrupt,
        })
        .unwrap();

        let events = q.drain(10);
        assert_eq!(events.len(), 3);

        // First should be Critical (SendSignal)
        assert!(matches!(events[0].0, KernelEvent::SendSignal { .. }));
        // Second should be Normal (Deliver)
        assert!(matches!(events[1].0, KernelEvent::Deliver(_)));
        // Third should be Low (UserMessage)
        assert!(matches!(events[2].0, KernelEvent::UserMessage(_)));
    }

    #[test]
    fn test_capacity_full() {
        let q = ShardQueue::new(2);

        q.push(KernelEvent::UserMessage(test_inbound("a"))).unwrap();
        q.push(KernelEvent::UserMessage(test_inbound("b"))).unwrap();

        let result = q.push(KernelEvent::UserMessage(test_inbound("c")));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), BusError::Full));
    }

    #[tokio::test]
    async fn test_wait_wakeup() {
        let q = std::sync::Arc::new(ShardQueue::new(100));
        let q2 = q.clone();

        let handle = tokio::spawn(async move {
            q2.wait().await;
            let events = q2.drain(10);
            assert_eq!(events.len(), 1);
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        q.push(KernelEvent::UserMessage(test_inbound("wake")))
            .unwrap();

        handle.await.unwrap();
    }

    #[test]
    fn test_drain_respects_limit() {
        let q = ShardQueue::new(100);

        for i in 0..5 {
            q.push(KernelEvent::UserMessage(test_inbound(&format!("msg{i}"))))
                .unwrap();
        }

        let events = q.drain(3);
        assert_eq!(events.len(), 3);
        assert_eq!(q.pending_count(), 2);
    }

    #[test]
    fn test_try_push_sync() {
        let q = ShardQueue::new(100);

        q.try_push(KernelEvent::UserMessage(test_inbound("sync")))
            .unwrap();

        assert_eq!(q.pending_count(), 1);
        let events = q.drain(10);
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn test_try_push_sync_full() {
        let q = ShardQueue::new(1);

        q.try_push(KernelEvent::UserMessage(test_inbound("a")))
            .unwrap();

        let result = q.try_push(KernelEvent::UserMessage(test_inbound("b")));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), BusError::Full));
    }

    #[test]
    fn test_drain_empty_queue() {
        let q = ShardQueue::new(100);
        let events = q.drain(10);
        assert_eq!(events.len(), 0);
        assert_eq!(q.pending_count(), 0);
    }

    #[tokio::test]
    async fn test_wait_returns_immediately_when_pending() {
        let q = ShardQueue::new(100);
        q.push(KernelEvent::UserMessage(test_inbound("already")))
            .unwrap();

        // wait() should return immediately because there's a pending event
        tokio::time::timeout(std::time::Duration::from_millis(100), q.wait())
            .await
            .expect("wait() should return immediately when events pending");
    }

    #[test]
    fn test_pending_count_tracks_push_and_drain() {
        let q = ShardQueue::new(100);
        assert_eq!(q.pending_count(), 0);

        q.push(KernelEvent::UserMessage(test_inbound("a"))).unwrap();
        assert_eq!(q.pending_count(), 1);

        q.push(KernelEvent::UserMessage(test_inbound("b"))).unwrap();
        assert_eq!(q.pending_count(), 2);

        // Drain 1
        let events = q.drain(1);
        assert_eq!(events.len(), 1);
        assert_eq!(q.pending_count(), 1);

        // Drain remaining
        let events = q.drain(10);
        assert_eq!(events.len(), 1);
        assert_eq!(q.pending_count(), 0);
    }

    #[test]
    fn test_shutdown_is_critical_priority() {
        let q = ShardQueue::new(100);

        // Push a Low event first, then Shutdown (Critical)
        q.push(KernelEvent::UserMessage(test_inbound("low")))
            .unwrap();
        q.push(KernelEvent::Shutdown).unwrap();

        let events = q.drain(10);
        assert_eq!(events.len(), 2);
        // Shutdown should come first (Critical priority)
        assert!(matches!(events[0].0, KernelEvent::Shutdown));
        assert!(matches!(events[1].0, KernelEvent::UserMessage(_)));
    }

    #[test]
    fn test_debug_format() {
        let q = ShardQueue::new(50);
        let debug = format!("{:?}", q);
        assert!(debug.contains("ShardQueue"));
        assert!(debug.contains("pending: 0"));
        assert!(debug.contains("capacity: 50"));
    }
}
