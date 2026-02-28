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

//! In-memory implementations of the bus traits.
//!
//! - [`InMemoryInboundBus`]: `Mutex<VecDeque>` + `Notify` + `AtomicUsize`
//! - [`InMemoryOutboundBus`]: `tokio::sync::broadcast`

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync;

use crate::io::bus::{InboundBus, OutboundBus, OutboundSubscriber};
use crate::io::types::{BusError, InboundMessage, OutboundEnvelope};

// ---------------------------------------------------------------------------
// InMemoryInboundBus
// ---------------------------------------------------------------------------

/// In-memory single-consumer inbound bus.
///
/// Uses `std::sync::Mutex` (not tokio) since the critical section is trivial
/// (push/pop on a `VecDeque`). `tokio::sync::Notify` provides the async
/// wakeup mechanism.
pub struct InMemoryInboundBus {
    queue: Mutex<VecDeque<InboundMessage>>,
    notify: Arc<sync::Notify>,
    pending: AtomicUsize,
    capacity: usize,
}

impl InMemoryInboundBus {
    /// Create a new bus with the given maximum capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            notify: Arc::new(sync::Notify::new()),
            pending: AtomicUsize::new(0),
            capacity,
        }
    }
}

#[async_trait]
impl InboundBus for InMemoryInboundBus {
    async fn publish(&self, msg: InboundMessage) -> Result<(), BusError> {
        let mut q = self.queue.lock().expect("inbound bus lock poisoned");
        if q.len() >= self.capacity {
            return Err(BusError::Full);
        }
        q.push_back(msg);
        self.pending.store(q.len(), Ordering::Release);
        drop(q);
        self.notify.notify_one();
        Ok(())
    }

    async fn drain(&self, max: usize) -> Vec<InboundMessage> {
        let mut q = self.queue.lock().expect("inbound bus lock poisoned");
        let n = max.min(q.len());
        let drained: Vec<_> = q.drain(..n).collect();
        self.pending.store(q.len(), Ordering::Release);
        drained
    }

    async fn wait_for_messages(&self) {
        // Fast path: if messages are already pending, return immediately.
        if self.pending.load(Ordering::Acquire) > 0 {
            return;
        }
        self.notify.notified().await;
    }

    fn pending_count(&self) -> usize {
        self.pending.load(Ordering::Acquire)
    }
}

// ---------------------------------------------------------------------------
// InMemoryOutboundBus
// ---------------------------------------------------------------------------

/// In-memory pub/sub outbound bus backed by `tokio::sync::broadcast`.
pub struct InMemoryOutboundBus {
    tx: sync::broadcast::Sender<OutboundEnvelope>,
}

impl InMemoryOutboundBus {
    /// Create a new outbound bus with the given broadcast channel capacity.
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = sync::broadcast::channel(capacity);
        Self { tx }
    }
}

#[async_trait]
impl OutboundBus for InMemoryOutboundBus {
    async fn publish(&self, msg: OutboundEnvelope) -> Result<(), BusError> {
        // broadcast::send returns Err only if there are no active receivers,
        // which is not an error condition for us.
        let _ = self.tx.send(msg);
        Ok(())
    }

    fn subscribe(&self) -> Box<dyn OutboundSubscriber> {
        Box::new(BroadcastSubscriber {
            rx: self.tx.subscribe(),
        })
    }
}

// ---------------------------------------------------------------------------
// BroadcastSubscriber
// ---------------------------------------------------------------------------

/// Wraps a `broadcast::Receiver` to implement [`OutboundSubscriber`].
struct BroadcastSubscriber {
    rx: sync::broadcast::Receiver<OutboundEnvelope>,
}

#[async_trait]
impl OutboundSubscriber for BroadcastSubscriber {
    async fn recv(&mut self) -> Option<OutboundEnvelope> {
        loop {
            match self.rx.recv().await {
                Ok(msg) => return Some(msg),
                Err(sync::broadcast::error::RecvError::Lagged(_)) => {
                    // Subscriber fell behind — skip missed messages and continue.
                    continue;
                }
                Err(sync::broadcast::error::RecvError::Closed) => {
                    return None;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::channel::types::{ChannelType, MessageContent};
    use crate::io::types::{
        ChannelSource, MessageId, OutboundPayload, OutboundRouting,
    };
    use crate::process::principal::UserId;
    use crate::process::SessionId;

    use super::*;

    /// Helper: build a test InboundMessage.
    fn test_inbound(text: &str) -> InboundMessage {
        InboundMessage {
            id: MessageId::new(),
            source: ChannelSource {
                channel_type: ChannelType::Telegram,
                platform_message_id: None,
                platform_user_id: "tg-user".to_string(),
                platform_chat_id: None,
            },
            user: UserId("u1".to_string()),
            session_id: SessionId::new("s1"),
            content: MessageContent::Text(text.to_string()),
            reply_context: None,
            timestamp: jiff::Timestamp::now(),
            metadata: HashMap::new(),
        }
    }

    /// Helper: build a test OutboundEnvelope.
    fn test_outbound(text: &str) -> OutboundEnvelope {
        OutboundEnvelope {
            id: MessageId::new(),
            in_reply_to: MessageId::new(),
            user: UserId("u1".to_string()),
            session_id: SessionId::new("s1"),
            routing: OutboundRouting::BroadcastAll,
            payload: OutboundPayload::Reply {
                content: MessageContent::Text(text.to_string()),
                attachments: vec![],
            },
            timestamp: jiff::Timestamp::now(),
        }
    }

    #[tokio::test]
    async fn test_inbound_publish_drain() {
        let bus = InMemoryInboundBus::new(100);
        bus.publish(test_inbound("a")).await.unwrap();
        bus.publish(test_inbound("b")).await.unwrap();
        bus.publish(test_inbound("c")).await.unwrap();

        // Drain 2 of 3
        let batch = bus.drain(2).await;
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[0].content.as_text(), "a");
        assert_eq!(batch[1].content.as_text(), "b");

        // Drain remaining
        let batch = bus.drain(2).await;
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].content.as_text(), "c");
    }

    #[tokio::test]
    async fn test_inbound_capacity_full() {
        let bus = InMemoryInboundBus::new(2);
        bus.publish(test_inbound("a")).await.unwrap();
        bus.publish(test_inbound("b")).await.unwrap();

        let result = bus.publish(test_inbound("c")).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), BusError::Full));
    }

    #[tokio::test]
    async fn test_inbound_wait_wakeup() {
        let bus = Arc::new(InMemoryInboundBus::new(100));
        let bus2 = Arc::clone(&bus);

        let handle = tokio::spawn(async move {
            bus2.wait_for_messages().await;
            let batch = bus2.drain(10).await;
            assert_eq!(batch.len(), 1);
            assert_eq!(batch[0].content.as_text(), "wake");
        });

        // Small delay so the spawned task starts waiting.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        bus.publish(test_inbound("wake")).await.unwrap();

        handle.await.unwrap();
    }

    #[tokio::test]
    async fn test_pending_count() {
        let bus = InMemoryInboundBus::new(100);
        assert_eq!(bus.pending_count(), 0);

        bus.publish(test_inbound("a")).await.unwrap();
        bus.publish(test_inbound("b")).await.unwrap();
        assert_eq!(bus.pending_count(), 2);

        bus.drain(1).await;
        assert_eq!(bus.pending_count(), 1);

        bus.drain(10).await;
        assert_eq!(bus.pending_count(), 0);
    }

    #[tokio::test]
    async fn test_outbound_pubsub() {
        let bus = InMemoryOutboundBus::new(16);
        let mut sub1 = bus.subscribe();
        let mut sub2 = bus.subscribe();

        bus.publish(test_outbound("hello")).await.unwrap();

        let msg1 = sub1.recv().await.unwrap();
        let msg2 = sub2.recv().await.unwrap();

        // Both subscribers receive the same message.
        assert_eq!(msg1.id, msg2.id);
    }

    #[tokio::test]
    async fn test_outbound_no_subscriber() {
        let bus = InMemoryOutboundBus::new(16);
        // Publishing without any subscriber should not error.
        let result = bus.publish(test_outbound("nobody")).await;
        assert!(result.is_ok());
    }
}
