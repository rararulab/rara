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

//! Feed event type — the atomic unit of external data ingestion.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

base::define_id!(
    /// Unique identifier for a feed event (UUID v4).
    ///
    /// Used for deduplication and read-cursor tracking across subscribers.
    FeedEventId
);

/// An event received from an external data source.
///
/// Each `FeedEvent` represents a single datum (webhook delivery, WebSocket
/// message, polled record) that has been normalised into a common envelope.
/// Events are persisted to the [`FeedStore`](super::FeedStore) and dispatched
/// to subscribing agent sessions.
#[derive(Debug, Clone, Serialize, Deserialize, bon::Builder)]
pub struct FeedEvent {
    /// Unique event identifier for deduplication and cursor tracking.
    pub id: FeedEventId,

    /// Name of the source that produced this event (e.g. `"github-rara"`).
    pub source_name: String,

    /// Discriminator within a source (e.g. `"push"`, `"price_update"`).
    pub event_type: String,

    /// Tags for subscription matching — inherited from the source plus any
    /// event-specific tags added by the transport layer.
    pub tags: Vec<String>,

    /// Raw event payload as a JSON value.
    pub payload: serde_json::Value,

    /// Wall-clock time when the event was received by rara.
    pub received_at: Timestamp,
}
