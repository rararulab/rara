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

//! DataFeed trait — abstraction for external data sources.

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::FeedEvent;

/// A registered external data feed.
///
/// Each implementation handles one transport protocol (webhook HTTP,
/// WebSocket client, polling). Feeds produce [`FeedEvent`]s that are
/// persisted to the feed store and dispatched to subscribing sessions.
///
/// The lifecycle is managed by [`DataFeedRegistry`](super::DataFeedRegistry):
/// - `run` is called once when the feed is started.
/// - The implementation owns its connection lifecycle (connect, reconnect with
///   backoff, heartbeat, etc.).
/// - When the `cancel` token is cancelled, the feed must shut down gracefully
///   and return.
#[async_trait]
pub trait DataFeed: Send + Sync + 'static {
    /// Human-readable name (e.g. `"crypto-binance"`, `"github-rara"`).
    fn name(&self) -> &str;

    /// Tags attached to all events from this feed.
    fn tags(&self) -> &[String];

    /// Start the feed, sending events through `tx` until `cancel` fires.
    ///
    /// The implementation owns its connection lifecycle (connect,
    /// reconnect with backoff, heartbeat, etc.). When the cancellation
    /// token is triggered, the feed must shut down gracefully and return
    /// `Ok(())`.
    async fn run(
        &self,
        tx: mpsc::Sender<FeedEvent>,
        cancel: CancellationToken,
    ) -> anyhow::Result<()>;
}
