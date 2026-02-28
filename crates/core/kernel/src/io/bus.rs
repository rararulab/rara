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

//! Bus traits for inbound and outbound message passing.
//!
//! The I/O bus layer uses asymmetric designs:
//! - [`InboundBus`]: single-consumer queue (kernel pulls at its own pace)
//! - [`OutboundBus`]: pub/sub broadcast (multiple egress subscribers)
//!
//! Additionally, [`OutboxStore`] provides durable storage for messages that
//! could not be delivered immediately (user offline).

use async_trait::async_trait;

use crate::io::types::{BusError, InboundMessage, MessageId, OutboundEnvelope};

// ---------------------------------------------------------------------------
// InboundBus
// ---------------------------------------------------------------------------

/// Single-consumer inbound message queue.
///
/// Ingress writes messages via [`publish`](Self::publish); the kernel tick
/// loop drains them in batches via [`drain`](Self::drain). The bus owns
/// the wakeup mechanism — [`wait_for_messages`](Self::wait_for_messages)
/// blocks until new messages are available.
#[async_trait]
pub trait InboundBus: Send + Sync + 'static {
    /// Publish a message into the bus. Returns [`BusError::Full`] if at
    /// capacity.
    async fn publish(&self, msg: InboundMessage) -> Result<(), BusError>;

    /// Drain up to `max` messages from the bus (exclusive consume, removes on
    /// read).
    async fn drain(&self, max: usize) -> Vec<InboundMessage>;

    /// Block until new messages are available (encapsulates wakeup mechanism).
    async fn wait_for_messages(&self);

    /// Current backlog count (for monitoring).
    fn pending_count(&self) -> usize;
}

// ---------------------------------------------------------------------------
// OutboundBus
// ---------------------------------------------------------------------------

/// Pub/sub outbound message broadcast.
///
/// The kernel publishes final responses via [`publish`](Self::publish).
/// Each egress instance (e.g. Telegram egress, Web egress) gets an
/// independent subscriber via [`subscribe`](Self::subscribe).
#[async_trait]
pub trait OutboundBus: Send + Sync + 'static {
    /// Publish an outbound envelope to all subscribers.
    async fn publish(&self, msg: OutboundEnvelope) -> Result<(), BusError>;

    /// Create a new independent subscriber.
    fn subscribe(&self) -> Box<dyn OutboundSubscriber>;
}

// ---------------------------------------------------------------------------
// OutboundSubscriber
// ---------------------------------------------------------------------------

/// A subscriber receiving outbound envelopes from an [`OutboundBus`].
#[async_trait]
pub trait OutboundSubscriber: Send + 'static {
    /// Receive the next envelope. Returns `None` when the bus is closed.
    async fn recv(&mut self) -> Option<OutboundEnvelope>;
}

// ---------------------------------------------------------------------------
// OutboxStore
// ---------------------------------------------------------------------------

/// Durable outbox for messages that could not be delivered immediately.
///
/// When egress detects a user is offline, the envelope is appended here.
/// A background drainer periodically re-publishes pending envelopes.
#[async_trait]
pub trait OutboxStore: Send + Sync + 'static {
    /// Append an envelope to the durable outbox.
    async fn append(&self, envelope: OutboundEnvelope) -> Result<(), BusError>;

    /// Drain up to `max` pending envelopes for re-delivery.
    async fn drain_pending(&self, max: usize) -> Vec<OutboundEnvelope>;

    /// Mark an envelope as successfully delivered (remove from outbox).
    async fn mark_delivered(&self, id: &MessageId) -> Result<(), BusError>;
}
