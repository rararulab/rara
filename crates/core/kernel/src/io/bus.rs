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

//! Bus traits for durable message storage.
//!
//! The legacy `InboundBus` / `OutboundBus` / `OutboundSubscriber` traits have
//! been replaced by the unified [`EventQueue`](crate::event_queue::EventQueue).
//!
//! [`OutboxStore`] provides durable storage for messages that could not be
//! delivered immediately (user offline).

use async_trait::async_trait;

use crate::io::types::{BusError, MessageId, OutboundEnvelope};

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
