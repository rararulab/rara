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

//! Configuration types for data feed registration.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// Persisted configuration for a registered data feed.
///
/// Each config represents a single external data source (webhook endpoint,
/// WebSocket connection, or polling target). Configs are serialised to JSON
/// and stored in the settings KV store for startup recovery.
#[derive(Debug, Clone, Serialize, Deserialize, bon::Builder)]
pub struct DataFeedConfig {
    /// Unique name for this feed (e.g. `"github-rara"`, `"crypto-binance"`).
    pub name: String,

    /// Transport type that determines how events are ingested.
    pub feed_type: FeedType,

    /// Tags for subscription matching. Subscribers filter events by tag.
    pub tags: Vec<String>,

    /// Type-specific configuration (URL, auth, intervals, etc.).
    ///
    /// The schema depends on `feed_type` — each transport implementation
    /// deserialises this into its own strongly-typed config struct.
    pub config: serde_json::Value,

    /// When this feed was registered.
    pub created_at: Timestamp,
}

/// Transport type for a data feed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedType {
    /// HTTP POST receiver — external services push events to rara.
    Webhook,
    /// Outbound WebSocket client — rara connects to an external WS endpoint.
    WebSocket,
    /// Periodic HTTP GET — rara polls an external API at fixed intervals.
    Polling,
}
