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

//! Yahoo Finance stock data feed.
//!
//! Polls the Yahoo Finance v8 chart API for real-time stock prices and
//! emits [`FeedEvent`]s with `event_type = "price_update"`. Each tracked
//! symbol produces one event per poll cycle.
//!
//! # API endpoint
//!
//! ```text
//! GET https://query1.finance.yahoo.com/v8/finance/chart/{symbol}?interval=1d&range=1d
//! ```
//!
//! No API key is required for this endpoint.

use std::time::Duration;

use async_trait::async_trait;
use jiff::Timestamp;
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{instrument, warn};

use super::{DataFeed, FeedEvent, FeedEventId};

/// Base URL for Yahoo Finance v8 chart API.
const YAHOO_CHART_BASE: &str = "https://query1.finance.yahoo.com/v8/finance/chart";

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// Configuration for the Yahoo Finance stock polling feed.
pub struct YahooStockConfig {
    /// Stock symbols to track (e.g. `["AAPL", "GOOGL", "MSFT"]`).
    pub symbols:  Vec<String>,
    /// Polling interval between successive fetches.
    pub interval: Duration,
}

// ---------------------------------------------------------------------------
// YahooStockFeed
// ---------------------------------------------------------------------------

/// A data feed that polls Yahoo Finance for stock price updates.
///
/// Iterates over configured symbols each poll cycle, fetching the v8 chart
/// endpoint for each symbol and emitting a `price_update` [`FeedEvent`].
#[derive(bon::Builder)]
pub struct YahooStockFeed {
    /// Human-readable feed name.
    name:     String,
    /// Tags inherited by every event.
    tags:     Vec<String>,
    /// Stock symbols to track.
    symbols:  Vec<String>,
    /// Polling interval.
    interval: Duration,
    /// Shared HTTP client.
    client:   reqwest::Client,
}

impl YahooStockFeed {
    /// Create a feed from a [`YahooStockConfig`].
    pub fn from_config(name: impl Into<String>, config: YahooStockConfig) -> Self {
        Self {
            name:     name.into(),
            tags:     vec!["stock".to_owned(), "yahoo".to_owned()],
            symbols:  config.symbols,
            interval: config.interval,
            client:   reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl DataFeed for YahooStockFeed {
    fn name(&self) -> &str { &self.name }

    fn tags(&self) -> &[String] { &self.tags }

    #[instrument(skip_all, fields(feed = %self.name))]
    async fn run(
        &self,
        tx: mpsc::Sender<FeedEvent>,
        cancel: CancellationToken,
    ) -> anyhow::Result<()> {
        tracing::info!(
            symbols = ?self.symbols,
            interval = ?self.interval,
            "yahoo stock feed started"
        );

        loop {
            tokio::select! {
                () = cancel.cancelled() => {
                    tracing::info!("yahoo stock feed cancelled, shutting down");
                    break;
                }
                () = tokio::time::sleep(self.interval) => {
                    self.poll_all_symbols(&tx).await;
                }
            }
        }

        Ok(())
    }
}

impl YahooStockFeed {
    /// Poll all configured symbols and send events.
    async fn poll_all_symbols(&self, tx: &mpsc::Sender<FeedEvent>) {
        for symbol in &self.symbols {
            match self.fetch_symbol(symbol).await {
                Ok(events) => {
                    for event in events {
                        if tx.send(event).await.is_err() {
                            tracing::info!("event channel closed, stopping yahoo feed");
                            return;
                        }
                    }
                }
                Err(e) => {
                    warn!(symbol, error = %e, "failed to fetch yahoo stock data");
                }
            }
        }
    }

    /// Fetch chart data for a single symbol and convert to events.
    async fn fetch_symbol(&self, symbol: &str) -> anyhow::Result<Vec<FeedEvent>> {
        let url = format!("{YAHOO_CHART_BASE}/{symbol}?interval=1d&range=1d");
        let response = self
            .client
            .get(&url)
            .header("User-Agent", "Mozilla/5.0")
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            anyhow::bail!("yahoo API returned {status} for {symbol}");
        }

        let body = response.bytes().await?;
        parse_chart_response(&body, symbol, &self.name, &self.tags)
    }
}

// ---------------------------------------------------------------------------
// Yahoo v8 chart response types (subset)
// ---------------------------------------------------------------------------

/// Top-level response envelope.
#[derive(Debug, Deserialize)]
struct ChartResponse {
    chart: ChartResult,
}

/// Contains the result array.
#[derive(Debug, Deserialize)]
struct ChartResult {
    result: Option<Vec<ChartData>>,
}

/// A single chart result entry.
#[derive(Debug, Deserialize)]
struct ChartData {
    meta:       ChartMeta,
    indicators: ChartIndicators,
}

/// Metadata for the chart (contains current price info).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChartMeta {
    symbol:               String,
    regular_market_price: Option<f64>,
    previous_close:       Option<f64>,
}

/// Contains quote indicators.
#[derive(Debug, Deserialize)]
struct ChartIndicators {
    quote: Option<Vec<QuoteData>>,
}

/// Volume and price arrays from the quote indicator.
#[derive(Debug, Deserialize)]
struct QuoteData {
    volume: Option<Vec<Option<u64>>>,
}

// ---------------------------------------------------------------------------
// Response parsing
// ---------------------------------------------------------------------------

/// Parse a Yahoo Finance v8 chart API response into [`FeedEvent`]s.
///
/// Each chart result entry produces one `price_update` event. Returns an
/// empty vec if the response contains no usable data (e.g. market closed
/// with no `regularMarketPrice`).
pub fn parse_chart_response(
    body: &[u8],
    symbol: &str,
    source_name: &str,
    base_tags: &[String],
) -> anyhow::Result<Vec<FeedEvent>> {
    let resp: ChartResponse = serde_json::from_slice(body)?;

    let results = match resp.chart.result {
        Some(r) if !r.is_empty() => r,
        _ => return Ok(vec![]),
    };

    let mut events = Vec::with_capacity(results.len());

    for data in &results {
        let price = match data.meta.regular_market_price {
            Some(p) => p,
            None => continue,
        };

        let previous_close = data.meta.previous_close.unwrap_or(price);
        let change = price - previous_close;
        let change_percent = if previous_close.abs() > f64::EPSILON {
            (change / previous_close) * 100.0
        } else {
            0.0
        };

        // Extract latest volume from the quote indicator arrays.
        let volume = data
            .indicators
            .quote
            .as_ref()
            .and_then(|quotes| quotes.first())
            .and_then(|q| q.volume.as_ref())
            .and_then(|vols| vols.iter().rev().find_map(|v| *v))
            .unwrap_or(0);

        let now = Timestamp::now();
        let sym_upper = data.meta.symbol.to_uppercase();

        let mut tags = base_tags.to_vec();
        let sym_lower = sym_upper.to_lowercase();
        if !tags.contains(&sym_lower) {
            tags.push(sym_lower);
        }

        let payload = serde_json::json!({
            "symbol": sym_upper,
            "price": price,
            "change": (change * 100.0).round() / 100.0,
            "change_percent": (change_percent * 100.0).round() / 100.0,
            "volume": volume,
            "timestamp": now.to_string(),
        });

        let event = FeedEvent::builder()
            .id(FeedEventId::deterministic(&format!(
                "{source_name}:{symbol}:{}",
                now.as_millisecond()
            )))
            .source_name(source_name.to_owned())
            .event_type("price_update".to_owned())
            .tags(tags)
            .payload(payload)
            .received_at(now)
            .build();

        events.push(event);
    }

    Ok(events)
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

    /// Fixture: minimal Yahoo Finance v8 chart API response.
    const YAHOO_RESPONSE_FIXTURE: &str = r#"{
        "chart": {
            "result": [{
                "meta": {
                    "symbol": "AAPL",
                    "regularMarketPrice": 185.50,
                    "previousClose": 183.25
                },
                "indicators": {
                    "quote": [{
                        "volume": [null, 45123456, 52345678]
                    }]
                }
            }]
        }
    }"#;

    #[test]
    fn parse_chart_response_extracts_price_event() {
        let events = parse_chart_response(
            YAHOO_RESPONSE_FIXTURE.as_bytes(),
            "AAPL",
            "test-yahoo",
            &["stock".to_owned(), "yahoo".to_owned()],
        )
        .expect("parse should succeed");

        assert_eq!(events.len(), 1);

        let event = &events[0];
        assert_eq!(event.source_name, "test-yahoo");
        assert_eq!(event.event_type, "price_update");
        assert!(event.tags.contains(&"stock".to_owned()));
        assert!(event.tags.contains(&"yahoo".to_owned()));
        assert!(event.tags.contains(&"aapl".to_owned()));

        let payload = &event.payload;
        assert_eq!(payload["symbol"], "AAPL");
        assert_eq!(payload["price"], 185.50);
        assert_eq!(payload["volume"], 52345678);

        // change = 185.50 - 183.25 = 2.25
        assert_eq!(payload["change"], 2.25);

        // change_percent = (2.25 / 183.25) * 100 = ~1.23%
        let pct = payload["change_percent"].as_f64().expect("change_percent");
        assert!((pct - 1.23).abs() < 0.01, "expected ~1.23, got {pct}");
    }

    #[test]
    fn parse_chart_response_handles_empty_result() {
        let body = br#"{"chart": {"result": []}}"#;
        let events = parse_chart_response(body, "AAPL", "test-yahoo", &["stock".to_owned()])
            .expect("parse should succeed");

        assert!(events.is_empty());
    }

    #[test]
    fn parse_chart_response_handles_null_result() {
        let body = br#"{"chart": {"result": null}}"#;
        let events = parse_chart_response(body, "AAPL", "test-yahoo", &["stock".to_owned()])
            .expect("parse should succeed");

        assert!(events.is_empty());
    }

    #[test]
    fn parse_chart_response_skips_missing_price() {
        let body = br#"{
            "chart": {
                "result": [{
                    "meta": {
                        "symbol": "AAPL",
                        "previousClose": 183.25
                    },
                    "indicators": { "quote": [] }
                }]
            }
        }"#;
        let events = parse_chart_response(body, "AAPL", "test-yahoo", &["stock".to_owned()])
            .expect("parse should succeed");

        assert!(events.is_empty());
    }

    #[test]
    fn parse_chart_response_handles_missing_volume() {
        let body = br#"{
            "chart": {
                "result": [{
                    "meta": {
                        "symbol": "MSFT",
                        "regularMarketPrice": 420.10,
                        "previousClose": 418.00
                    },
                    "indicators": { "quote": [] }
                }]
            }
        }"#;
        let events = parse_chart_response(body, "MSFT", "test-yahoo", &["stock".to_owned()])
            .expect("parse should succeed");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].payload["volume"], 0);
    }

    #[test]
    fn parse_chart_response_rejects_invalid_json() {
        let body = b"not json at all";
        let result = parse_chart_response(body, "AAPL", "test-yahoo", &["stock".to_owned()]);

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn yahoo_feed_cancel_stops_immediately() {
        let cancel = CancellationToken::new();
        let (tx, _rx) = mpsc::channel(16);

        let feed = YahooStockFeed::builder()
            .name("test-yahoo".to_owned())
            .tags(vec!["stock".to_owned(), "yahoo".to_owned()])
            .symbols(vec!["AAPL".to_owned()])
            .interval(Duration::from_secs(3600))
            .client(reqwest::Client::new())
            .build();

        let cancel_clone = cancel.clone();
        let handle = tokio::spawn(async move { feed.run(tx, cancel_clone).await });

        cancel.cancel();
        let result = tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("task should complete within timeout")
            .expect("task should not panic");

        assert!(result.is_ok());
    }

    // ----- Integration tests (require network) -----

    #[tokio::test]
    #[ignore = "requires network access — run with `cargo test -- --ignored`"]
    async fn yahoo_feed_receives_real_price_event() {
        let cancel = CancellationToken::new();
        let (tx, mut rx) = mpsc::channel(16);

        let feed = YahooStockFeed::builder()
            .name("integration-test".to_owned())
            .tags(vec!["stock".to_owned(), "yahoo".to_owned()])
            .symbols(vec!["AAPL".to_owned()])
            // Poll immediately by using a very short interval; we only need
            // one cycle.
            .interval(Duration::from_millis(100))
            .client(reqwest::Client::new())
            .build();

        let cancel_clone = cancel.clone();
        tokio::spawn(async move {
            let _ = feed.run(tx, cancel_clone).await;
        });

        // Wait for at least one event (with generous timeout).
        let event = tokio::time::timeout(Duration::from_secs(30), rx.recv())
            .await
            .expect("should receive event within timeout")
            .expect("channel should not be closed");

        assert_eq!(event.source_name, "integration-test");
        assert_eq!(event.event_type, "price_update");
        assert_eq!(event.payload["symbol"], "AAPL");

        let price = event.payload["price"]
            .as_f64()
            .expect("price should be a number");
        assert!(price > 0.0, "price should be positive, got {price}");

        // Cleanup.
        cancel.cancel();
    }
}
