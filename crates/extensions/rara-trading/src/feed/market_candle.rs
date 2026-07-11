// Copyright 2026 Rararulab
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

//! Config-driven latest market candle data feed.
//!
//! This transport fetches an operator-managed normalized candle endpoint and
//! emits one `market_candle_closed` event per closed bar.

use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use jiff::Timestamp;
use rara_kernel::data_feed::{
    AuthConfig, DataFeed, DataFeedConfig, FeedEvent, FeedEventId, FeedStatus, StatusReporterRef,
    polling::apply_request_auth,
};
use reqwest::{
    Url,
    header::{HeaderName, HeaderValue},
};
use rust_decimal::Decimal;
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, instrument, warn};

const MAX_CANDLES_PER_POLL: usize = 10_000;
const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
pub struct MarketCandleTransport {
    pub url:                  String,
    pub interval_secs:        u64,
    #[serde(default)]
    pub headers:              HashMap<String, String>,
    pub venue:                String,
    pub symbols:              Vec<String>,
    pub timeframes:           Vec<String>,
    pub max_candles_per_poll: usize,
}

pub struct MarketCandleSource {
    name:       String,
    tags:       Vec<String>,
    transport:  MarketCandleTransport,
    auth:       Option<AuthConfig>,
    client:     reqwest::Client,
    reporter:   Option<StatusReporterRef>,
    in_error:   Arc<AtomicBool>,
    symbols:    HashSet<String>,
    timeframes: HashSet<String>,
}

#[derive(Debug, Deserialize)]
struct CandleBatch {
    candles: Vec<RawCandle>,
}

#[derive(Debug, Deserialize)]
struct RawCandle {
    venue:      String,
    symbol:     String,
    timeframe:  String,
    open_time:  String,
    close_time: String,
    open:       String,
    high:       String,
    low:        String,
    close:      String,
    volume:     String,
    closed:     bool,
}

struct ParsedCandle {
    venue:      String,
    symbol:     String,
    timeframe:  String,
    open_time:  String,
    close_time: String,
    open:       Decimal,
    high:       Decimal,
    low:        Decimal,
    close:      Decimal,
    volume:     Decimal,
}

impl MarketCandleSource {
    pub fn from_config(config: &DataFeedConfig) -> anyhow::Result<Self> {
        let transport: MarketCandleTransport = serde_json::from_value(config.transport.clone())?;
        validate_transport(&transport)?;
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()?;
        let symbols = transport.symbols.iter().cloned().collect();
        let timeframes = transport.timeframes.iter().cloned().collect();
        Ok(Self {
            name: config.name.clone(),
            tags: config.tags.clone(),
            transport,
            auth: config.auth.clone(),
            client,
            reporter: None,
            in_error: Arc::new(AtomicBool::new(false)),
            symbols,
            timeframes,
        })
    }

    #[must_use]
    pub fn with_reporter(mut self, reporter: StatusReporterRef) -> Self {
        self.reporter = Some(reporter);
        self
    }

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

    fn record_error(&self, message: String) {
        let was_in_error = self.in_error.swap(true, Ordering::SeqCst);
        if was_in_error {
            debug!(feed = %self.name, error = %message, "market candle fetch failed");
        } else {
            warn!(feed = %self.name, error = %message, "market candle fetch failed");
            if let Some(reporter) = self.reporter.clone() {
                let name = self.name.clone();
                tokio::spawn(async move {
                    reporter
                        .report(&name, FeedStatus::Error, Some(message))
                        .await;
                });
            }
        }
    }

    fn record_success(&self) {
        if self.in_error.swap(false, Ordering::SeqCst) {
            info!(feed = %self.name, "market candle feed recovered");
            if let Some(reporter) = self.reporter.clone() {
                let name = self.name.clone();
                tokio::spawn(async move {
                    reporter.report(&name, FeedStatus::Running, None).await;
                });
            }
        }
    }

    fn parse_candle_events(&self, body: &[u8]) -> anyhow::Result<Vec<FeedEvent>> {
        let batch: CandleBatch = serde_json::from_slice(body)?;
        let received_at = Timestamp::now();
        let mut events = Vec::new();

        for raw in batch
            .candles
            .into_iter()
            .take(self.transport.max_candles_per_poll)
        {
            let Some(candle) = self.parse_raw_candle(raw) else {
                continue;
            };
            let mut tags = self.tags.clone();
            tags.extend([
                "finance".to_owned(),
                "market-data".to_owned(),
                format!("source:{}", self.name),
                format!("venue:{}", candle.venue),
                format!("symbol:{}", candle.symbol),
                format!("timeframe:{}", candle.timeframe),
            ]);
            dedupe_sort(&mut tags);

            let payload = serde_json::json!({
                "venue": candle.venue,
                "symbol": candle.symbol,
                "timeframe": candle.timeframe,
                "open_time": candle.open_time,
                "close_time": candle.close_time,
                "open": candle.open.to_string(),
                "high": candle.high.to_string(),
                "low": candle.low.to_string(),
                "close": candle.close.to_string(),
                "volume": candle.volume.to_string(),
            });

            let identity = format!(
                "{}:{}:{}:{}:{}",
                self.name,
                payload["venue"].as_str().unwrap_or_default(),
                payload["symbol"].as_str().unwrap_or_default(),
                payload["timeframe"].as_str().unwrap_or_default(),
                payload["open_time"].as_str().unwrap_or_default()
            );

            events.push(
                FeedEvent::builder()
                    .id(FeedEventId::deterministic(&identity))
                    .source_name(self.name.clone())
                    .event_type("market_candle_closed".to_owned())
                    .tags(tags)
                    .payload(payload)
                    .received_at(received_at)
                    .build(),
            );
        }

        Ok(events)
    }

    fn parse_raw_candle(&self, raw: RawCandle) -> Option<ParsedCandle> {
        if !raw.closed {
            return None;
        }
        if raw.venue != self.transport.venue {
            return None;
        }
        if !self.symbols.contains(&raw.symbol) || !self.timeframes.contains(&raw.timeframe) {
            return None;
        }
        Some(ParsedCandle {
            venue:      raw.venue,
            symbol:     raw.symbol,
            timeframe:  raw.timeframe,
            open_time:  raw.open_time,
            close_time: raw.close_time,
            open:       Decimal::from_str(&raw.open).ok()?,
            high:       Decimal::from_str(&raw.high).ok()?,
            low:        Decimal::from_str(&raw.low).ok()?,
            close:      Decimal::from_str(&raw.close).ok()?,
            volume:     Decimal::from_str(&raw.volume).ok()?,
        })
    }

    async fn poll_once(&self, tx: &mpsc::Sender<FeedEvent>) -> bool {
        let url = match self.build_url() {
            Ok(url) => url,
            Err(err) => {
                self.record_error(format!("failed to build market candle URL: {err}"));
                return true;
            }
        };

        let mut request = self.client.get(url);
        for (key, value) in &self.transport.headers {
            request = request.header(key.as_str(), value.as_str());
        }
        request = apply_request_auth(request, &self.auth);

        let response = match request.send().await {
            Ok(response) => response,
            Err(err) => {
                self.record_error(format!("market candle fetch failed: {err}"));
                return true;
            }
        };

        let status = response.status();
        if !status.is_success() {
            self.record_error(format!(
                "market candle fetch received non-success status: {status}"
            ));
            return true;
        }

        let body = match response.bytes().await {
            Ok(body) => body,
            Err(err) => {
                self.record_error(format!("failed to read market candle body: {err}"));
                return true;
            }
        };
        if body.len() > MAX_BODY_BYTES {
            self.record_error(format!(
                "market candle response exceeded {MAX_BODY_BYTES} bytes"
            ));
            return true;
        }

        let events = match self.parse_candle_events(&body) {
            Ok(events) => events,
            Err(err) => {
                self.record_error(format!("failed to parse market candle response: {err}"));
                return true;
            }
        };
        for event in events {
            if tx.send(event).await.is_err() {
                tracing::info!("event channel closed, stopping market candle loop");
                return false;
            }
        }

        self.record_success();
        true
    }
}

#[async_trait]
impl DataFeed for MarketCandleSource {
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
            "market candle feed started"
        );

        let mut interval_timer = tokio::time::interval(interval);
        interval_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                () = cancel.cancelled() => {
                    tracing::info!("market candle feed cancelled, shutting down");
                    break;
                }
                _ = interval_timer.tick() => {
                    if !self.poll_once(&tx).await {
                        break;
                    }
                }
            }
        }

        Ok(())
    }
}

fn validate_transport(transport: &MarketCandleTransport) -> anyhow::Result<()> {
    let url = Url::parse(&transport.url)?;
    anyhow::ensure!(url.scheme() == "https", "market candle URL must use HTTPS");
    anyhow::ensure!(
        transport.interval_secs > 0,
        "market candle interval_secs must be greater than zero"
    );
    anyhow::ensure!(
        !transport.venue.is_empty(),
        "market candle venue is required"
    );
    anyhow::ensure!(
        !transport.symbols.is_empty(),
        "market candle symbols must not be empty"
    );
    anyhow::ensure!(
        !transport.timeframes.is_empty(),
        "market candle timeframes must not be empty"
    );
    anyhow::ensure!(
        transport.max_candles_per_poll > 0,
        "market candle max_candles_per_poll must be greater than zero"
    );
    anyhow::ensure!(
        transport.max_candles_per_poll <= MAX_CANDLES_PER_POLL,
        "market candle max_candles_per_poll must be <= {MAX_CANDLES_PER_POLL}"
    );
    for (name, value) in &transport.headers {
        HeaderName::from_bytes(name.as_bytes())?;
        HeaderValue::from_str(value)?;
    }
    Ok(())
}

fn dedupe_sort(values: &mut Vec<String>) {
    let mut seen = HashSet::new();
    values.retain(|value| seen.insert(value.clone()));
    values.sort();
}

#[cfg(test)]
mod tests {
    use rara_kernel::data_feed::{DataFeedConfig, FeedStatus, FeedType};

    use super::MarketCandleSource;

    fn candle_source() -> MarketCandleSource {
        let config = DataFeedConfig::builder()
            .id("candles-test".to_owned())
            .name("binance-spot".to_owned())
            .feed_type(FeedType::MarketCandle)
            .tags(vec!["finance".to_owned(), "crypto".to_owned()])
            .transport(serde_json::json!({
                "url": "https://market-data.example/candles/latest",
                "interval_secs": 60,
                "venue": "binance",
                "symbols": ["BTCUSDT", "ETHUSDT"],
                "timeframes": ["15m", "1h"],
                "max_candles_per_poll": 1000
            }))
            .enabled(true)
            .status(FeedStatus::Idle)
            .created_at(jiff::Timestamp::UNIX_EPOCH)
            .updated_at(jiff::Timestamp::UNIX_EPOCH)
            .build();

        MarketCandleSource::from_config(&config).expect("market candle config should parse")
    }

    #[test]
    fn closed_candles_for_many_symbols_become_batched_events() {
        let source = candle_source();
        let events = source
            .parse_candle_events(
                br#"{
  "candles": [
    {
      "venue": "binance",
      "symbol": "BTCUSDT",
      "timeframe": "15m",
      "open_time": "2026-07-10T08:15:00Z",
      "close_time": "2026-07-10T08:30:00Z",
      "open": "61500.12",
      "high": "61640.00",
      "low": "61480.50",
      "close": "61610.30",
      "volume": "124.551",
      "closed": true
    },
    {
      "venue": "binance",
      "symbol": "ETHUSDT",
      "timeframe": "15m",
      "open_time": "2026-07-10T08:15:00Z",
      "close_time": "2026-07-10T08:30:00Z",
      "open": "3500.00",
      "high": "3510.00",
      "low": "3490.00",
      "close": "3505.25",
      "volume": "991.5",
      "closed": true
    }
  ]
}"#,
            )
            .expect("candles should parse");

        assert_eq!(events.len(), 2);
        let event = &events[0];
        assert_eq!(event.source_name, "binance-spot");
        assert_eq!(event.event_type, "market_candle_closed");
        assert!(event.tags.contains(&"finance".to_owned()));
        assert!(event.tags.contains(&"market-data".to_owned()));
        assert!(event.tags.contains(&"source:binance-spot".to_owned()));
        assert!(event.tags.contains(&"venue:binance".to_owned()));
        assert!(event.tags.contains(&"symbol:BTCUSDT".to_owned()));
        assert!(event.tags.contains(&"timeframe:15m".to_owned()));
        assert_eq!(event.payload["open"], "61500.12");
        assert_eq!(event.payload["close"], "61610.30");
        assert_eq!(event.payload["volume"], "124.551");
    }

    #[test]
    fn same_candle_has_same_event_id_across_polls() {
        let source = candle_source();
        let body = br#"{
  "candles": [
    {
      "venue": "binance",
      "symbol": "BTCUSDT",
      "timeframe": "15m",
      "open_time": "2026-07-10T08:15:00Z",
      "close_time": "2026-07-10T08:30:00Z",
      "open": "61500.12",
      "high": "61640.00",
      "low": "61480.50",
      "close": "61610.30",
      "volume": "124.551",
      "closed": true
    }
  ]
}"#;

        let first = source.parse_candle_events(body).expect("first parse");
        let second = source.parse_candle_events(body).expect("second parse");

        assert_eq!(first[0].id, second[0].id);
    }

    #[test]
    fn open_or_invalid_decimal_candles_are_not_emitted() {
        let source = candle_source();
        let events = source
            .parse_candle_events(
                br#"{
  "candles": [
    {
      "venue": "binance",
      "symbol": "BTCUSDT",
      "timeframe": "15m",
      "open_time": "2026-07-10T08:15:00Z",
      "close_time": "2026-07-10T08:30:00Z",
      "open": "61500.12",
      "high": "61640.00",
      "low": "61480.50",
      "close": "61610.30",
      "volume": "124.551",
      "closed": false
    },
    {
      "venue": "binance",
      "symbol": "ETHUSDT",
      "timeframe": "15m",
      "open_time": "2026-07-10T08:15:00Z",
      "close_time": "2026-07-10T08:30:00Z",
      "open": "not-a-decimal",
      "high": "3510.00",
      "low": "3490.00",
      "close": "3505.25",
      "volume": "991.5",
      "closed": true
    }
  ]
}"#,
            )
            .expect("payload should parse while invalid candles are skipped");

        assert!(events.is_empty());
    }
}
