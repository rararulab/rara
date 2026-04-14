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

//! Generic polling data feed — periodically fetches data from an HTTP endpoint.
//!
//! [`PollingSource`] pairs a URL + interval with a caller-supplied
//! [`ResponseParser`] that converts each HTTP response body into zero or more
//! [`FeedEvent`]s. The poll loop runs until the cancellation token fires,
//! logging warnings on transient errors without crashing.

use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{instrument, warn};

use super::{DataFeed, FeedEvent};

// ---------------------------------------------------------------------------
// ResponseParser — pluggable response-to-event conversion
// ---------------------------------------------------------------------------

/// Converts a raw HTTP response body into feed events.
///
/// Implementations are source-specific: a Yahoo Finance parser extracts price
/// data, a weather parser extracts forecasts, etc. The parser receives the
/// source name and base tags so it can populate the [`FeedEvent`] envelope
/// fields without duplicating that knowledge.
#[async_trait]
pub trait ResponseParser: Send + Sync + 'static {
    /// Parse `body` into zero or more [`FeedEvent`]s.
    ///
    /// Returning an empty vec is valid (e.g. no new data since last poll).
    /// Errors are logged but do not stop the poll loop.
    fn parse(
        &self,
        body: &[u8],
        source_name: &str,
        base_tags: &[String],
    ) -> anyhow::Result<Vec<FeedEvent>>;
}

// ---------------------------------------------------------------------------
// PollingSource
// ---------------------------------------------------------------------------

/// A polling data feed that periodically HTTP-GETs a URL and converts the
/// response into [`FeedEvent`]s via a [`ResponseParser`].
#[derive(bon::Builder)]
pub struct PollingSource {
    /// Human-readable feed name.
    name:     String,
    /// Tags inherited by every event from this source.
    tags:     Vec<String>,
    /// Endpoint to poll.
    url:      String,
    /// Time between successive polls.
    interval: Duration,
    /// HTTP client (shared across polls).
    client:   reqwest::Client,
    /// Pluggable response-to-event converter.
    parser:   Box<dyn ResponseParser>,
}

#[async_trait]
impl DataFeed for PollingSource {
    fn name(&self) -> &str { &self.name }

    fn tags(&self) -> &[String] { &self.tags }

    #[instrument(skip_all, fields(feed = %self.name))]
    async fn run(
        &self,
        tx: mpsc::Sender<FeedEvent>,
        cancel: CancellationToken,
    ) -> anyhow::Result<()> {
        tracing::info!(url = %self.url, interval = ?self.interval, "polling feed started");

        loop {
            tokio::select! {
                () = cancel.cancelled() => {
                    tracing::info!("polling feed cancelled, shutting down");
                    break;
                }
                () = tokio::time::sleep(self.interval) => {
                    self.poll_once(&tx).await;
                }
            }
        }

        Ok(())
    }
}

impl PollingSource {
    /// Execute a single poll cycle: fetch URL, parse response, send events.
    async fn poll_once(&self, tx: &mpsc::Sender<FeedEvent>) {
        let response = match self.client.get(&self.url).send().await {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, "poll fetch failed");
                return;
            }
        };

        let status = response.status();
        if !status.is_success() {
            warn!(%status, "poll received non-success status");
            return;
        }

        let body = match response.bytes().await {
            Ok(b) => b,
            Err(e) => {
                warn!(error = %e, "failed to read poll response body");
                return;
            }
        };

        let events = match self.parser.parse(&body, &self.name, &self.tags) {
            Ok(evts) => evts,
            Err(e) => {
                warn!(error = %e, "response parsing failed");
                return;
            }
        };

        for event in events {
            if tx.send(event).await.is_err() {
                // Channel closed — receiver dropped; stop polling.
                tracing::info!("event channel closed, stopping poll loop");
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use super::*;

    /// Trivial parser that always returns an empty vec.
    struct NoopParser;

    #[async_trait]
    impl ResponseParser for NoopParser {
        fn parse(
            &self,
            _body: &[u8],
            _source_name: &str,
            _base_tags: &[String],
        ) -> anyhow::Result<Vec<FeedEvent>> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn cancel_stops_polling() {
        let cancel = CancellationToken::new();
        let (tx, _rx) = mpsc::channel(16);

        let source = PollingSource::builder()
            .name("test-poll".to_owned())
            .tags(vec!["test".to_owned()])
            .url("http://localhost:99999/nonexistent".to_owned())
            .interval(Duration::from_secs(3600))
            .client(reqwest::Client::new())
            .parser(Box::new(NoopParser))
            .build();

        let cancel_clone = cancel.clone();
        let handle = tokio::spawn(async move { source.run(tx, cancel_clone).await });

        // Cancel immediately — should return Ok(()).
        cancel.cancel();
        let result = tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("task should complete within timeout")
            .expect("task should not panic");

        assert!(result.is_ok());
    }
}
