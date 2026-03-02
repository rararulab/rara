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

//! Unified event queue trait — tiered priority queue for all kernel interactions.
//!
//! Defines the [`EventQueue`] trait that all queue implementations must satisfy.
//! The default in-memory implementation ([`InMemoryEventQueue`]) is provided for
//! backward compatibility and testing.
//!
//! Events are auto-classified into three priority tiers:
//! - **Critical**: signals (Interrupt, Kill, Pause, Resume), shutdown
//! - **Normal**: turn completions, child completions, deliver
//! - **Low**: user messages, spawn requests, timers

use std::{
    collections::VecDeque,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use tokio::sync;

use crate::io::types::BusError;

// Re-export the unified event from the sibling module.
pub use crate::unified_event::{KernelEvent, EventPriority};

// ---------------------------------------------------------------------------
// EventQueue trait
// ---------------------------------------------------------------------------

/// Trait for tiered priority event queues.
///
/// Implementations must be `Send + Sync + 'static` to be shared across async
/// tasks via `Arc<dyn EventQueue>`.
#[async_trait]
pub trait EventQueue: Send + Sync + 'static {
    /// Push an event into the queue. Returns `BusError::Full` if at capacity.
    async fn push(&self, event: KernelEvent) -> Result<(), BusError>;

    /// Non-async push (for fire-and-forget signal sends).
    fn try_push(&self, event: KernelEvent) -> Result<(), BusError>;

    /// Drain up to `max` events from the queue, in priority order.
    ///
    /// Each returned pair is `(event, wal_id)`. The `wal_id` is `Some` for
    /// events that were persisted to a WAL; the event loop should call
    /// [`mark_completed`](Self::mark_completed) after processing each such
    /// event. For in-memory-only queues, `wal_id` is always `None`.
    async fn drain(&self, max: usize) -> Vec<(KernelEvent, Option<u64>)>;

    /// Wait until events are available.
    async fn wait(&self);

    /// Current total pending count across all tiers.
    fn pending_count(&self) -> usize;

    /// Mark a WAL entry as completed. Default no-op for non-persistent queues.
    fn mark_completed(&self, _wal_id: u64) {}

    /// Whether this queue is a sharded event queue.
    ///
    /// Returns `true` for [`ShardedEventQueue`](crate::sharded_event_queue::ShardedEventQueue),
    /// `false` for all other implementations.
    fn is_sharded(&self) -> bool { false }
}

// ---------------------------------------------------------------------------
// InMemoryEventQueue
// ---------------------------------------------------------------------------

/// Tiered priority queue for all kernel interactions (pure in-memory).
///
/// Uses three `VecDeque`s (one per priority tier) protected by `std::sync::Mutex`
/// (not tokio — critical sections are trivial push/pop operations).
///
/// `tokio::sync::Notify` provides async wakeup when events are pushed.
pub struct InMemoryEventQueue {
    /// Three tiers: [Critical, Normal, Low].
    queues:   [Mutex<VecDeque<KernelEvent>>; 3],
    /// Async notification for the event loop.
    notify:   sync::Notify,
    /// Total pending event count across all tiers.
    pending:  AtomicUsize,
    /// Maximum capacity (total across all tiers).
    capacity: usize,
}

impl InMemoryEventQueue {
    /// Create a new event queue with the given maximum capacity.
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
}

#[async_trait]
impl EventQueue for InMemoryEventQueue {
    async fn push(&self, event: KernelEvent) -> Result<(), BusError> {
        self.try_push(event)
    }

    fn try_push(&self, event: KernelEvent) -> Result<(), BusError> {
        let current = self.pending.load(Ordering::Acquire);
        if current >= self.capacity {
            return Err(BusError::Full);
        }

        let tier = event.priority() as usize;
        let mut q = self.queues[tier].lock().expect("event queue lock poisoned");
        q.push_back(event);
        drop(q);

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
            let mut q = self.queues[tier].lock().expect("event queue lock poisoned");
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

    async fn wait(&self) {
        if self.pending.load(Ordering::Acquire) > 0 {
            return;
        }
        self.notify.notified().await;
    }

    fn pending_count(&self) -> usize {
        self.pending.load(Ordering::Acquire)
    }
}

impl std::fmt::Debug for InMemoryEventQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryEventQueue")
            .field("pending", &self.pending_count())
            .field("capacity", &self.capacity)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unified_event::KernelEvent;
    use crate::io::types::InboundMessage;
    use crate::process::{AgentId, SessionId, Signal, principal::UserId};
    use crate::channel::types::{ChannelType, MessageContent};
    use crate::io::types::{ChannelSource, MessageId};
    use std::collections::HashMap;

    fn test_inbound(text: &str) -> InboundMessage {
        InboundMessage {
            id:            MessageId::new(),
            source:        ChannelSource {
                channel_type:        ChannelType::Internal,
                platform_message_id: None,
                platform_user_id:    "test".to_string(),
                platform_chat_id:    None,
            },
            user:          UserId("u1".to_string()),
            session_id:    SessionId::new("s1"),
            target_agent_id: None,
            target_agent:  None,
            content:       MessageContent::Text(text.to_string()),
            reply_context: None,
            timestamp:     jiff::Timestamp::now(),
            metadata:      HashMap::new(),
        }
    }

    #[tokio::test]
    async fn test_push_and_drain() {
        let q = InMemoryEventQueue::new(100);

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
    async fn test_priority_ordering() {
        let q = InMemoryEventQueue::new(100);

        // Push in reverse priority order: Low, Normal, Critical
        q.push(KernelEvent::UserMessage(test_inbound("low")))
            .await
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
        .await
        .unwrap();
        q.push(KernelEvent::SendSignal {
            target: AgentId::new(),
            signal: Signal::Interrupt,
        })
        .await
        .unwrap();

        let events = q.drain(10).await;
        assert_eq!(events.len(), 3);

        // First should be Critical (SendSignal)
        assert!(matches!(events[0].0, KernelEvent::SendSignal { .. }));
        // Second should be Normal (Deliver)
        assert!(matches!(events[1].0, KernelEvent::Deliver(_)));
        // Third should be Low (UserMessage)
        assert!(matches!(events[2].0, KernelEvent::UserMessage(_)));
    }

    #[tokio::test]
    async fn test_capacity_full() {
        let q = InMemoryEventQueue::new(2);

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
    async fn test_wait_wakeup() {
        let q = std::sync::Arc::new(InMemoryEventQueue::new(100));
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

    #[tokio::test]
    async fn test_drain_respects_limit() {
        let q = InMemoryEventQueue::new(100);

        for i in 0..5 {
            q.push(KernelEvent::UserMessage(test_inbound(&format!("msg{i}"))))
                .await
                .unwrap();
        }

        let events = q.drain(3).await;
        assert_eq!(events.len(), 3);
        assert_eq!(q.pending_count(), 2);
    }

    #[tokio::test]
    async fn test_try_push_sync() {
        let q = InMemoryEventQueue::new(100);

        q.try_push(KernelEvent::UserMessage(test_inbound("sync")))
            .unwrap();

        assert_eq!(q.pending_count(), 1);
        let events = q.drain(10).await;
        assert_eq!(events.len(), 1);
    }

    #[tokio::test]
    async fn test_try_push_sync_full() {
        let q = InMemoryEventQueue::new(1);

        q.try_push(KernelEvent::UserMessage(test_inbound("a")))
            .unwrap();

        let result = q.try_push(KernelEvent::UserMessage(test_inbound("b")));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), BusError::Full));
    }

    #[tokio::test]
    async fn test_drain_empty_queue() {
        let q = InMemoryEventQueue::new(100);
        let events = q.drain(10).await;
        assert_eq!(events.len(), 0);
        assert_eq!(q.pending_count(), 0);
    }

    #[tokio::test]
    async fn test_wait_returns_immediately_when_pending() {
        let q = InMemoryEventQueue::new(100);
        q.push(KernelEvent::UserMessage(test_inbound("already")))
            .await
            .unwrap();

        // wait() should return immediately because there's a pending event
        tokio::time::timeout(
            std::time::Duration::from_millis(100),
            q.wait(),
        )
        .await
        .expect("wait() should return immediately when events pending");
    }

    #[tokio::test]
    async fn test_pending_count_tracks_push_and_drain() {
        let q = InMemoryEventQueue::new(100);
        assert_eq!(q.pending_count(), 0);

        q.push(KernelEvent::UserMessage(test_inbound("a"))).await.unwrap();
        assert_eq!(q.pending_count(), 1);

        q.push(KernelEvent::UserMessage(test_inbound("b"))).await.unwrap();
        assert_eq!(q.pending_count(), 2);

        q.push(KernelEvent::UserMessage(test_inbound("c"))).await.unwrap();
        assert_eq!(q.pending_count(), 3);

        // Drain 2
        let events = q.drain(2).await;
        assert_eq!(events.len(), 2);
        assert_eq!(q.pending_count(), 1);

        // Drain remaining
        let events = q.drain(10).await;
        assert_eq!(events.len(), 1);
        assert_eq!(q.pending_count(), 0);
    }

    #[tokio::test]
    async fn test_shutdown_is_critical_priority() {
        let q = InMemoryEventQueue::new(100);

        // Push a Low event first, then Shutdown (Critical)
        q.push(KernelEvent::UserMessage(test_inbound("low")))
            .await
            .unwrap();
        q.push(KernelEvent::Shutdown).await.unwrap();

        let events = q.drain(10).await;
        assert_eq!(events.len(), 2);
        // Shutdown should come first (Critical priority)
        assert!(matches!(events[0].0, KernelEvent::Shutdown));
        assert!(matches!(events[1].0, KernelEvent::UserMessage(_)));
    }

    #[tokio::test]
    async fn test_all_priority_tiers_interleaved() {
        let q = InMemoryEventQueue::new(100);

        // Push events in reverse priority order across all tiers
        // Low: UserMessage, Timer
        q.push(KernelEvent::UserMessage(test_inbound("user-msg")))
            .await
            .unwrap();
        q.push(KernelEvent::Timer {
            name:    "tick".to_string(),
            payload: serde_json::Value::Null,
        })
        .await
        .unwrap();

        // Normal: Deliver, ChildCompleted
        q.push(KernelEvent::Deliver(crate::io::types::OutboundEnvelope {
            id:          MessageId::new(),
            in_reply_to: MessageId::new(),
            user:        UserId("u1".to_string()),
            session_id:  SessionId::new("s1"),
            routing:     crate::io::types::OutboundRouting::BroadcastAll,
            payload:     crate::io::types::OutboundPayload::Reply {
                content:     MessageContent::Text("reply".to_string()),
                attachments: vec![],
            },
            timestamp:   jiff::Timestamp::now(),
        }))
        .await
        .unwrap();
        q.push(KernelEvent::ChildCompleted {
            parent_id: AgentId::new(),
            child_id:  AgentId::new(),
            result:    crate::process::AgentResult {
                output:     "done".to_string(),
                iterations: 1,
                tool_calls: 0,
            },
        })
        .await
        .unwrap();

        // Critical: SendSignal, Shutdown
        q.push(KernelEvent::SendSignal {
            target: AgentId::new(),
            signal: Signal::Kill,
        })
        .await
        .unwrap();
        q.push(KernelEvent::Shutdown).await.unwrap();

        assert_eq!(q.pending_count(), 6);

        let events = q.drain(10).await;
        assert_eq!(events.len(), 6);
        assert_eq!(q.pending_count(), 0);

        // Critical tier first (index 0, 1)
        assert!(matches!(events[0].0, KernelEvent::SendSignal { .. }));
        assert!(matches!(events[1].0, KernelEvent::Shutdown));

        // Normal tier next (index 2, 3)
        assert!(matches!(events[2].0, KernelEvent::Deliver(_)));
        assert!(matches!(events[3].0, KernelEvent::ChildCompleted { .. }));

        // Low tier last (index 4, 5)
        assert!(matches!(events[4].0, KernelEvent::UserMessage(_)));
        assert!(matches!(events[5].0, KernelEvent::Timer { .. }));
    }

    #[tokio::test]
    async fn test_multiple_drains_consume_different_events() {
        let q = InMemoryEventQueue::new(100);

        for i in 0..6 {
            q.push(KernelEvent::UserMessage(test_inbound(&format!("msg{i}"))))
                .await
                .unwrap();
        }

        let batch1 = q.drain(2).await;
        let batch2 = q.drain(2).await;
        let batch3 = q.drain(2).await;
        let batch4 = q.drain(2).await;

        assert_eq!(batch1.len(), 2);
        assert_eq!(batch2.len(), 2);
        assert_eq!(batch3.len(), 2);
        assert_eq!(batch4.len(), 0); // all consumed
        assert_eq!(q.pending_count(), 0);
    }

    #[test]
    fn test_event_queue_debug_format() {
        let q = InMemoryEventQueue::new(50);
        let debug = format!("{:?}", q);
        assert!(debug.contains("InMemoryEventQueue"));
        assert!(debug.contains("pending: 0"));
        assert!(debug.contains("capacity: 50"));
    }
}
