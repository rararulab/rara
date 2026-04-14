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

//! Config-driven polling data feed.
//!
//! Periodically HTTP-GETs a URL and emits the raw JSON response as
//! [`FeedEvent`] payload. No response parsing — the subscribing agent
//! interprets the payload with its own intelligence.

use std::{collections::HashMap, time::Duration};

use async_trait::async_trait;
use jiff::Timestamp;
use reqwest::Url;
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{instrument, warn};

use super::{DataFeed, DataFeedConfig, FeedEvent, FeedEventId, config::AuthConfig};

// ---------------------------------------------------------------------------
// PollingTransport — transport-specific config
// ---------------------------------------------------------------------------

/// Polling-specific transport configuration.
///
/// Deserialised from the `transport` JSON blob of a [`DataFeedConfig`]
/// with `feed_type = "polling"`.
///
/// # Example
///
/// ```json
/// {
///     "url": "https://api.example.com/data",
///     "interval_secs": 60,
///     "headers": { "User-Agent": "Mozilla/5.0" },
///     "method": "GET"
/// }
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct PollingTransport {
    /// URL to poll.
    pub url: String,

    /// Seconds between successive polls.
    pub interval_secs: u64,

    /// Additional HTTP headers to include in each request.
    #[serde(default)]
    pub headers: HashMap<String, String>,

    /// HTTP method (defaults to `"GET"`).
    #[serde(default = "default_method")]
    pub method: String,
}

fn default_method() -> String { "GET".to_owned() }

// ---------------------------------------------------------------------------
// PollingSource
// ---------------------------------------------------------------------------

/// A config-driven polling data feed.
///
/// Created from a [`DataFeedConfig`] via [`from_config`](Self::from_config).
/// Periodically fetches the configured URL and emits the raw response body
/// as a [`FeedEvent`] payload (pass-through — no parsing).
pub struct PollingSource {
    /// Human-readable feed name.
    name:      String,
    /// Tags inherited by every event from this source.
    tags:      Vec<String>,
    /// Polling-specific transport configuration.
    transport: PollingTransport,
    /// Optional authentication configuration.
    auth:      Option<AuthConfig>,
    /// Shared HTTP client.
    client:    reqwest::Client,
}

impl PollingSource {
    /// Create a polling source from a [`DataFeedConfig`].
    ///
    /// Deserialises the `transport` JSON blob into [`PollingTransport`].
    /// Returns an error if the transport config is malformed.
    pub fn from_config(config: &DataFeedConfig) -> anyhow::Result<Self> {
        let transport: PollingTransport = serde_json::from_value(config.transport.clone())?;
        Ok(Self {
            name: config.name.clone(),
            tags: config.tags.clone(),
            transport,
            auth: config.auth.clone(),
            client: reqwest::Client::new(),
        })
    }
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
        let interval = Duration::from_secs(self.transport.interval_secs);
        tracing::info!(
            url = %self.transport.url,
            ?interval,
            "polling feed started"
        );

        loop {
            tokio::select! {
                () = cancel.cancelled() => {
                    tracing::info!("polling feed cancelled, shutting down");
                    break;
                }
                () = tokio::time::sleep(interval) => {
                    self.poll_once(&tx).await;
                }
            }
        }

        Ok(())
    }
}

impl PollingSource {
    /// Build the final URL, applying query-param auth if configured.
    fn build_url(&self) -> anyhow::Result<Url> {
        let mut url = Url::parse(&self.transport.url)?;
        if let Some(AuthConfig::Query {
            ref name,
            ref value,
        }) = self.auth
        {
            url.query_pairs_mut().append_pair(name, value);
        }
        Ok(url)
    }

    /// Execute a single poll cycle: fetch URL, emit raw response as event.
    async fn poll_once(&self, tx: &mpsc::Sender<FeedEvent>) {
        let url = match self.build_url() {
            Ok(u) => u,
            Err(e) => {
                warn!(error = %e, "failed to build poll URL");
                return;
            }
        };

        let method: reqwest::Method = self
            .transport
            .method
            .parse()
            .unwrap_or(reqwest::Method::GET);

        let mut request = self.client.request(method, url);

        // Inject custom headers from transport config.
        for (key, value) in &self.transport.headers {
            request = request.header(key.as_str(), value.as_str());
        }

        // Inject authentication (header/bearer/basic — query is handled
        // in build_url above).
        request = apply_request_auth(request, &self.auth);

        let response = match request.send().await {
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

        // Pass-through: attempt JSON parse, fall back to raw string wrapper.
        let payload: serde_json::Value = match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(_) => {
                serde_json::json!({ "raw": String::from_utf8_lossy(&body) })
            }
        };

        let now = Timestamp::now();
        let event = FeedEvent::builder()
            .id(FeedEventId::deterministic(&format!(
                "{}:poll:{}",
                self.name,
                now.as_millisecond()
            )))
            .source_name(self.name.clone())
            .event_type("poll_response".to_owned())
            .tags(self.tags.clone())
            .payload(payload)
            .received_at(now)
            .build();

        if tx.send(event).await.is_err() {
            tracing::info!("event channel closed, stopping poll loop");
        }
    }
}

// ---------------------------------------------------------------------------
// Auth helper
// ---------------------------------------------------------------------------

/// Apply [`AuthConfig`] to an outbound HTTP request builder.
///
/// Handles header, bearer, and basic auth. Query-param auth is applied
/// at URL construction time (see `PollingSource::build_url`), and HMAC
/// is for inbound verification only — both are no-ops here.
pub fn apply_request_auth(
    mut request: reqwest::RequestBuilder,
    auth: &Option<AuthConfig>,
) -> reqwest::RequestBuilder {
    match auth {
        Some(AuthConfig::Header { name, value }) => {
            request = request.header(name.as_str(), value.as_str());
        }
        Some(AuthConfig::Bearer { token }) => {
            request = request.bearer_auth(token);
        }
        Some(AuthConfig::Basic { username, password }) => {
            request = request.basic_auth(username, Some(password));
        }
        // Query auth is handled at URL construction time.
        Some(AuthConfig::Query { .. }) => {}
        // HMAC is for inbound signature verification (webhooks), not
        // outbound request auth.
        Some(AuthConfig::Hmac { .. }) => {}
        None => {}
    }
    request
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use super::*;

    #[tokio::test]
    async fn cancel_stops_polling() {
        let cancel = CancellationToken::new();
        let (tx, _rx) = mpsc::channel(16);

        let config = DataFeedConfig::builder()
            .id("test-id".to_owned())
            .name("test-poll".to_owned())
            .feed_type(super::super::FeedType::Polling)
            .tags(vec!["test".to_owned()])
            .transport(serde_json::json!({
                "url": "http://localhost:99999/nonexistent",
                "interval_secs": 3600
            }))
            .enabled(true)
            .status(super::super::config::FeedStatus::Idle)
            .created_at(jiff::Timestamp::UNIX_EPOCH)
            .updated_at(jiff::Timestamp::UNIX_EPOCH)
            .build();

        let source = PollingSource::from_config(&config).expect("should parse config");

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

    #[test]
    fn from_config_rejects_invalid_transport() {
        let config = DataFeedConfig::builder()
            .id("bad-id".to_owned())
            .name("bad-poll".to_owned())
            .feed_type(super::super::FeedType::Polling)
            .tags(vec![])
            .transport(serde_json::json!({ "not_a_url": true }))
            .enabled(true)
            .status(super::super::config::FeedStatus::Idle)
            .created_at(jiff::Timestamp::UNIX_EPOCH)
            .updated_at(jiff::Timestamp::UNIX_EPOCH)
            .build();

        let result = PollingSource::from_config(&config);
        assert!(result.is_err());
    }

    #[test]
    fn polling_transport_deserialises_defaults() {
        let json = serde_json::json!({
            "url": "https://example.com/api",
            "interval_secs": 30
        });
        let transport: PollingTransport = serde_json::from_value(json).expect("should deserialise");
        assert_eq!(transport.method, "GET");
        assert!(transport.headers.is_empty());
    }

    #[test]
    fn polling_transport_with_all_fields() {
        let json = serde_json::json!({
            "url": "https://example.com/api",
            "interval_secs": 60,
            "headers": { "User-Agent": "rara/1.0" },
            "method": "POST"
        });
        let transport: PollingTransport = serde_json::from_value(json).expect("should deserialise");
        assert_eq!(transport.method, "POST");
        assert_eq!(transport.headers.get("User-Agent").unwrap(), "rara/1.0");
    }

    #[test]
    fn build_url_appends_query_auth() {
        let config = DataFeedConfig::builder()
            .id("q-id".to_owned())
            .name("query-test".to_owned())
            .feed_type(super::super::FeedType::Polling)
            .tags(vec![])
            .transport(serde_json::json!({
                "url": "https://api.example.com/data",
                "interval_secs": 60
            }))
            .auth(AuthConfig::Query {
                name:  "apikey".to_owned(),
                value: "sk-xxx".to_owned(),
            })
            .enabled(true)
            .status(super::super::config::FeedStatus::Idle)
            .created_at(jiff::Timestamp::UNIX_EPOCH)
            .updated_at(jiff::Timestamp::UNIX_EPOCH)
            .build();

        let source = PollingSource::from_config(&config).expect("config ok");
        let url = source.build_url().expect("build_url ok");
        assert!(
            url.as_str().contains("apikey=sk-xxx"),
            "URL should contain query auth param: {url}"
        );
    }
}
