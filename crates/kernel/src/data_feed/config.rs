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
//!
//! [`DataFeedConfig`] is the persisted representation stored in the
//! `data_feeds` table. Transport-specific settings live in the
//! [`transport`](DataFeedConfig::transport) JSON blob, and authentication
//! is handled uniformly via [`AuthConfig`].

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// DataFeedConfig
// ---------------------------------------------------------------------------

/// Persisted configuration for a registered data feed.
///
/// Each config represents a single external data source (webhook endpoint,
/// WebSocket connection, or polling target). Configs are stored in the
/// `data_feeds` table for startup recovery.
#[derive(Debug, Clone, Serialize, Deserialize, bon::Builder)]
pub struct DataFeedConfig {
    /// Unique identifier (UUID).
    pub id: String,

    /// Unique human-readable name (e.g. `"github-rara"`, `"crypto-binance"`).
    pub name: String,

    /// Transport type that determines how events are ingested.
    pub feed_type: FeedType,

    /// Tags for subscription matching. Subscribers filter events by tag.
    pub tags: Vec<String>,

    /// Transport-specific configuration (URL, interval, headers, etc.).
    ///
    /// The schema depends on `feed_type` — each transport implementation
    /// deserialises this into its own strongly-typed config struct
    /// (e.g. [`PollingTransport`](super::polling::PollingTransport)).
    pub transport: serde_json::Value,

    /// Authentication configuration. `None` means no auth required.
    pub auth: Option<AuthConfig>,

    /// Whether this feed is enabled. Disabled feeds are not started.
    pub enabled: bool,

    /// Runtime status of the feed.
    pub status: FeedStatus,

    /// Last error message if the feed is in error state.
    pub last_error: Option<String>,

    /// When this feed was registered.
    pub created_at: Timestamp,

    /// When this feed was last updated.
    pub updated_at: Timestamp,
}

// ---------------------------------------------------------------------------
// FeedType
// ---------------------------------------------------------------------------

/// Transport type for a data feed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, strum::Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum FeedType {
    /// HTTP POST receiver — external services push events to rara.
    Webhook,
    /// Outbound WebSocket client — rara connects to an external WS endpoint.
    WebSocket,
    /// Periodic HTTP GET — rara polls an external API at fixed intervals.
    Polling,
}

// ---------------------------------------------------------------------------
// FeedStatus
// ---------------------------------------------------------------------------

/// Runtime status of a data feed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, strum::Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum FeedStatus {
    /// Registered but not currently running.
    Idle,
    /// Actively ingesting events.
    Running,
    /// Stopped due to an error.
    Error,
}

// ---------------------------------------------------------------------------
// AuthConfig
// ---------------------------------------------------------------------------

/// Unified authentication configuration for data feeds.
///
/// Serialised with `#[serde(tag = "type")]` so the JSON representation is
/// a tagged union:
///
/// ```json
/// { "type": "bearer", "token": "eyJ..." }
/// { "type": "header", "name": "X-API-Key", "value": "sk-xxx" }
/// { "type": "hmac", "secret": "whsec_xxx", "header": "X-Hub-Signature-256" }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthConfig {
    /// API key in an HTTP header.
    ///
    /// ```json
    /// { "type": "header", "name": "X-API-Key", "value": "sk-xxx" }
    /// ```
    Header {
        /// Header name (e.g. `"X-API-Key"`).
        name:  String,
        /// Header value (the API key).
        value: String,
    },

    /// API key in a query parameter.
    ///
    /// ```json
    /// { "type": "query", "name": "apikey", "value": "sk-xxx" }
    /// ```
    Query {
        /// Query parameter name.
        name:  String,
        /// Query parameter value (the API key).
        value: String,
    },

    /// Bearer token authentication.
    ///
    /// ```json
    /// { "type": "bearer", "token": "eyJ..." }
    /// ```
    Bearer {
        /// The bearer token.
        token: String,
    },

    /// HTTP Basic authentication.
    ///
    /// ```json
    /// { "type": "basic", "username": "user", "password": "pass" }
    /// ```
    Basic {
        /// Username.
        username: String,
        /// Password.
        password: String,
    },

    /// HMAC signature verification (primarily for webhooks).
    ///
    /// ```json
    /// { "type": "hmac", "secret": "whsec_xxx", "header": "X-Hub-Signature-256" }
    /// ```
    Hmac {
        /// Shared secret for HMAC-SHA256 computation.
        secret: String,
        /// HTTP header containing the signature (e.g. `"X-Hub-Signature-256"`).
        header: String,
    },
}
