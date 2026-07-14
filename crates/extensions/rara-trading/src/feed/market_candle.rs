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
//! This transport fetches either an operator-managed normalized candle endpoint
//! or a built-in public provider such as Binance, then emits one
//! `market_candle_closed` event per closed bar.

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
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, instrument, warn};

use crate::market_data::Timeframe;

const MAX_CANDLES_PER_POLL: usize = 10_000;
const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

/// Minimum polling cadence for a market-candle feed, in seconds.
///
/// Sub-minute polling (5–10s) is the intended operating point: a flash crash
/// must be observed within seconds, not up to a minute late. But the Binance
/// public REST API enforces a per-IP request-weight budget, and each poll tick
/// fans out to `symbols × timeframes` `/api/v3/klines` requests — so the true
/// request rate is `symbols × timeframes / interval_secs` per second, not
/// `1 / interval_secs`. A cadence below this floor multiplies that fan-out into
/// a request burst that trips the venue's rate limit (HTTP 429/418) and gets
/// the deployment IP temporarily banned, flipping every symbol to `Error` at
/// once. Five seconds is the fastest cadence that keeps the intended monitoring
/// fan-out within the venue's budget while still catching a sub-minute crash in
/// time.
///
/// This encodes a fixed property of the venue API's rate limit — no deploy
/// operator has a principled reason to poll faster than the venue allows — so
/// it lives as a mechanism `const` rather than a YAML knob
/// (`docs/guides/anti-patterns.md`). `interval_secs` itself stays a per-feed
/// config value: a genuine deploy-relevant choice *within* the allowed range.
const MIN_INTERVAL_SECS: u64 = 5;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MarketCandleTransport {
    #[serde(default)]
    pub provider:             Option<String>,
    #[serde(default)]
    pub url:                  Option<String>,
    #[serde(default)]
    pub base_url:             Option<String>,
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
    pub fn normalize_config(config: &mut DataFeedConfig) -> anyhow::Result<()> {
        let mut transport: MarketCandleTransport =
            serde_json::from_value(config.transport.clone())?;
        normalize_transport(&mut transport)?;
        validate_transport(&transport)?;
        config.transport = serde_json::to_value(transport)?;
        Ok(())
    }

    pub fn from_config(config: &DataFeedConfig) -> anyhow::Result<Self> {
        let mut config = config.clone();
        Self::normalize_config(&mut config)?;
        let transport: MarketCandleTransport = serde_json::from_value(config.transport.clone())?;
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

    fn build_normalized_url(&self) -> anyhow::Result<Url> {
        let url = self
            .transport
            .url
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("normalized market candle URL is required"))?;
        let mut url = Url::parse(url)?;
        if let Some(AuthConfig::Query {
            ref name,
            ref value,
        }) = self.auth
        {
            url.query_pairs_mut().append_pair(name, value);
        }
        Ok(url)
    }

    fn build_binance_klines_url(&self, symbol: &str, timeframe: &str) -> anyhow::Result<Url> {
        let base_url = self
            .transport
            .base_url
            .as_deref()
            .unwrap_or("https://api.binance.com");
        let mut url = Url::parse(base_url)?.join("/api/v3/klines")?;
        url.query_pairs_mut()
            .append_pair("symbol", symbol)
            .append_pair("interval", timeframe)
            .append_pair("limit", "2");
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

    fn parse_normalized_candle_events(&self, body: &[u8]) -> anyhow::Result<Vec<FeedEvent>> {
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
            events.push(self.event_from_candle(candle, received_at));
        }

        Ok(events)
    }

    fn parse_binance_candle_events(
        &self,
        symbol: &str,
        timeframe: &str,
        body: &[u8],
    ) -> anyhow::Result<Vec<FeedEvent>> {
        let rows: Vec<Vec<serde_json::Value>> = serde_json::from_slice(body)?;
        let received_at = Timestamp::now();
        let now_ms = received_at.as_millisecond();
        let mut events = Vec::new();

        for row in rows.into_iter().take(self.transport.max_candles_per_poll) {
            let Some(candle) = self.parse_binance_row(symbol, timeframe, row, now_ms) else {
                continue;
            };
            events.push(self.event_from_candle(candle, received_at));
        }

        Ok(events)
    }

    fn event_from_candle(&self, candle: ParsedCandle, received_at: Timestamp) -> FeedEvent {
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

        FeedEvent::builder()
            .id(FeedEventId::deterministic(&identity))
            .source_name(self.name.clone())
            .event_type("market_candle_closed".to_owned())
            .tags(tags)
            .payload(payload)
            .received_at(received_at)
            .build()
    }

    fn parse_raw_candle(&self, raw: RawCandle) -> Option<ParsedCandle> {
        if !raw.closed {
            return None;
        }
        let venue = normalize_venue(raw.venue).ok()?;
        let symbol = normalize_symbol(raw.symbol).ok()?;
        let timeframe = normalize_timeframe(raw.timeframe).ok()?;
        if venue != self.transport.venue {
            return None;
        }
        if !self.symbols.contains(&symbol) || !self.timeframes.contains(&timeframe) {
            return None;
        }
        let open_time = raw.open_time.parse::<Timestamp>().ok()?.to_string();
        let close_time = raw.close_time.parse::<Timestamp>().ok()?.to_string();
        Some(ParsedCandle {
            venue,
            symbol,
            timeframe,
            open_time,
            close_time,
            open: Decimal::from_str(&raw.open).ok()?,
            high: Decimal::from_str(&raw.high).ok()?,
            low: Decimal::from_str(&raw.low).ok()?,
            close: Decimal::from_str(&raw.close).ok()?,
            volume: Decimal::from_str(&raw.volume).ok()?,
        })
    }

    fn parse_binance_row(
        &self,
        symbol: &str,
        timeframe: &str,
        row: Vec<serde_json::Value>,
        now_ms: i64,
    ) -> Option<ParsedCandle> {
        let open_time_ms = row.first()?.as_i64()?;
        let close_time_ms = row.get(6)?.as_i64()?;
        if close_time_ms > now_ms {
            return None;
        }

        Some(ParsedCandle {
            venue:      self.transport.venue.clone(),
            symbol:     symbol.to_owned(),
            timeframe:  timeframe.to_owned(),
            open_time:  Timestamp::from_millisecond(open_time_ms).ok()?.to_string(),
            close_time: Timestamp::from_millisecond(close_time_ms).ok()?.to_string(),
            open:       Decimal::from_str(row.get(1)?.as_str()?).ok()?,
            high:       Decimal::from_str(row.get(2)?.as_str()?).ok()?,
            low:        Decimal::from_str(row.get(3)?.as_str()?).ok()?,
            close:      Decimal::from_str(row.get(4)?.as_str()?).ok()?,
            volume:     Decimal::from_str(row.get(5)?.as_str()?).ok()?,
        })
    }

    async fn poll_normalized_once(&self, tx: &mpsc::Sender<FeedEvent>) -> bool {
        let url = match self.build_normalized_url() {
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

        let events = match self.parse_normalized_candle_events(&body) {
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

    async fn poll_binance_once(&self, tx: &mpsc::Sender<FeedEvent>) -> bool {
        let mut any_success = false;

        for symbol in &self.transport.symbols {
            for timeframe in &self.transport.timeframes {
                let url = match self.build_binance_klines_url(symbol, timeframe) {
                    Ok(url) => url,
                    Err(err) => {
                        self.record_error(format!("failed to build Binance klines URL: {err}"));
                        continue;
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
                        self.record_error(format!("Binance klines fetch failed: {err}"));
                        continue;
                    }
                };

                let status = response.status();
                if !status.is_success() {
                    self.record_error(format!(
                        "Binance klines fetch received non-success status: {status}"
                    ));
                    continue;
                }

                let body = match response.bytes().await {
                    Ok(body) => body,
                    Err(err) => {
                        self.record_error(format!("failed to read Binance klines body: {err}"));
                        continue;
                    }
                };
                if body.len() > MAX_BODY_BYTES {
                    self.record_error(format!(
                        "Binance klines response exceeded {MAX_BODY_BYTES} bytes"
                    ));
                    continue;
                }

                let events = match self.parse_binance_candle_events(symbol, timeframe, &body) {
                    Ok(events) => events,
                    Err(err) => {
                        self.record_error(format!(
                            "failed to parse Binance klines response: {err}"
                        ));
                        continue;
                    }
                };

                any_success = true;
                for event in events {
                    if tx.send(event).await.is_err() {
                        tracing::info!("event channel closed, stopping market candle loop");
                        return false;
                    }
                }
            }
        }

        if any_success {
            self.record_success();
        }
        true
    }

    async fn poll_once(&self, tx: &mpsc::Sender<FeedEvent>) -> bool {
        if self.transport.provider.as_deref() == Some("binance") {
            self.poll_binance_once(tx).await
        } else {
            self.poll_normalized_once(tx).await
        }
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
            provider = ?self.transport.provider,
            url = ?self.transport.url,
            base_url = ?self.transport.base_url,
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

fn normalize_transport(transport: &mut MarketCandleTransport) -> anyhow::Result<()> {
    transport.provider = transport
        .provider
        .take()
        .map(|provider| {
            normalize_selector("provider", provider).map(|value| value.to_ascii_lowercase())
        })
        .transpose()?;
    transport.venue = normalize_venue(std::mem::take(&mut transport.venue))?;
    transport.symbols = normalize_symbols(std::mem::take(&mut transport.symbols))?;
    transport.timeframes = normalize_timeframes(std::mem::take(&mut transport.timeframes))?;
    Ok(())
}

fn normalize_symbols(values: Vec<String>) -> anyhow::Result<Vec<String>> {
    let mut values = values
        .into_iter()
        .map(normalize_symbol)
        .collect::<anyhow::Result<Vec<_>>>()?;
    dedupe_sort(&mut values);
    Ok(values)
}

fn normalize_timeframes(values: Vec<String>) -> anyhow::Result<Vec<String>> {
    let mut values = values
        .into_iter()
        .map(normalize_timeframe)
        .collect::<anyhow::Result<Vec<_>>>()?;
    dedupe_sort(&mut values);
    Ok(values)
}

fn normalize_venue(value: String) -> anyhow::Result<String> {
    Ok(normalize_selector("venue", value)?.to_ascii_lowercase())
}

fn normalize_symbol(value: String) -> anyhow::Result<String> {
    Ok(normalize_selector("symbol", value)?.to_ascii_uppercase())
}

fn normalize_timeframe(value: String) -> anyhow::Result<String> {
    Ok(Timeframe::parse(normalize_selector("timeframe", value)?.to_ascii_lowercase())?.to_string())
}

fn normalize_selector(name: &str, value: String) -> anyhow::Result<String> {
    let value = value.trim();
    anyhow::ensure!(!value.is_empty(), "{name} must not be empty");
    Ok(value.to_owned())
}

fn validate_transport(transport: &MarketCandleTransport) -> anyhow::Result<()> {
    match transport.provider.as_deref().unwrap_or("normalized") {
        "normalized" => {
            let url = transport
                .url
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("market candle URL is required"))?;
            let url = Url::parse(url)?;
            anyhow::ensure!(url.scheme() == "https", "market candle URL must use HTTPS");
        }
        "binance" => {
            let base_url = transport
                .base_url
                .as_deref()
                .unwrap_or("https://api.binance.com");
            let url = Url::parse(base_url)?;
            anyhow::ensure!(
                url.scheme() == "https",
                "Binance market candle base_url must use HTTPS"
            );
        }
        provider => anyhow::bail!("unsupported market candle provider: {provider}"),
    }
    anyhow::ensure!(
        transport.interval_secs > 0,
        "market candle interval_secs must be greater than zero"
    );
    anyhow::ensure!(
        transport.interval_secs >= MIN_INTERVAL_SECS,
        "market candle interval_secs must be at least {MIN_INTERVAL_SECS} seconds: each poll tick \
         fans out to symbols × timeframes /api/v3/klines requests, so polling faster trips \
         Binance's per-IP request-weight rate limit and gets the deployment IP-banned"
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

    fn binance_source() -> MarketCandleSource {
        let config = DataFeedConfig::builder()
            .id("binance-candles-test".to_owned())
            .name("binance-public".to_owned())
            .feed_type(FeedType::MarketCandle)
            .tags(vec!["finance".to_owned(), "crypto".to_owned()])
            .transport(serde_json::json!({
                "provider": "binance",
                "base_url": "https://api.binance.com",
                "interval_secs": 60,
                "venue": "binance",
                "symbols": ["BTCUSDT"],
                "timeframes": ["1m"],
                "max_candles_per_poll": 1000
            }))
            .enabled(true)
            .status(FeedStatus::Idle)
            .created_at(jiff::Timestamp::UNIX_EPOCH)
            .updated_at(jiff::Timestamp::UNIX_EPOCH)
            .build();

        MarketCandleSource::from_config(&config).expect("Binance config should parse")
    }

    #[test]
    fn closed_candles_for_many_symbols_become_batched_events() {
        let source = candle_source();
        let events = source
            .parse_normalized_candle_events(
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

        let first = source
            .parse_normalized_candle_events(body)
            .expect("first parse");
        let second = source
            .parse_normalized_candle_events(body)
            .expect("second parse");

        assert_eq!(first[0].id, second[0].id);
    }

    #[test]
    fn open_or_invalid_decimal_candles_are_not_emitted() {
        let source = candle_source();
        let events = source
            .parse_normalized_candle_events(
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

    #[test]
    fn invalid_timestamp_candles_are_not_emitted() {
        let source = candle_source();
        let events = source
            .parse_normalized_candle_events(
                br#"{
  "candles": [
    {
      "venue": "binance",
      "symbol": "BTCUSDT",
      "timeframe": "15m",
      "open_time": "not-a-timestamp",
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
      "close_time": "not-a-timestamp",
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
            .expect("payload should parse while invalid candles are skipped");

        assert!(events.is_empty());
    }

    #[test]
    fn binance_provider_builds_public_klines_url() {
        let source = binance_source();
        let url = source
            .build_binance_klines_url("BTCUSDT", "1m")
            .expect("URL should build");

        assert_eq!(
            url.as_str(),
            "https://api.binance.com/api/v3/klines?symbol=BTCUSDT&interval=1m&limit=2"
        );
    }

    #[test]
    fn binance_provider_normalizes_config_selectors() {
        let config = DataFeedConfig::builder()
            .id("binance-candles-test".to_owned())
            .name("binance-public".to_owned())
            .feed_type(FeedType::MarketCandle)
            .tags(vec!["finance".to_owned(), "crypto".to_owned()])
            .transport(serde_json::json!({
                "provider": " Binance ",
                "base_url": "https://api.binance.com",
                "interval_secs": 60,
                "venue": " Binance ",
                "symbols": [" btcusdt ", "BTCUSDT"],
                "timeframes": [" 1M ", "1m"],
                "max_candles_per_poll": 1000
            }))
            .enabled(true)
            .status(FeedStatus::Idle)
            .created_at(jiff::Timestamp::UNIX_EPOCH)
            .updated_at(jiff::Timestamp::UNIX_EPOCH)
            .build();

        let source = MarketCandleSource::from_config(&config).expect("config should parse");

        assert_eq!(source.transport.provider.as_deref(), Some("binance"));
        assert_eq!(source.transport.venue, "binance");
        assert_eq!(source.transport.symbols, ["BTCUSDT"]);
        assert_eq!(source.transport.timeframes, ["1m"]);
    }

    #[test]
    fn normalized_endpoint_candles_are_matched_and_emitted_with_canonical_selectors() {
        let source = candle_source();
        let events = source
            .parse_normalized_candle_events(
                br#"{
  "candles": [
    {
      "venue": " Binance ",
      "symbol": " btcusdt ",
      "timeframe": " 15M ",
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
}"#,
            )
            .expect("candles should parse");

        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert!(event.tags.contains(&"venue:binance".to_owned()));
        assert!(event.tags.contains(&"symbol:BTCUSDT".to_owned()));
        assert!(event.tags.contains(&"timeframe:15m".to_owned()));
        assert_eq!(event.payload["venue"], "binance");
        assert_eq!(event.payload["symbol"], "BTCUSDT");
        assert_eq!(event.payload["timeframe"], "15m");
    }

    #[test]
    fn binance_klines_become_closed_candle_events() {
        let source = binance_source();
        let events = source
            .parse_binance_candle_events(
                "BTCUSDT",
                "1m",
                br#"[
  [
    1713100200000,
    "61500.12",
    "61640.00",
    "61480.50",
    "61610.30",
    "124.551",
    1713100259999,
    "7669543.21",
    42,
    "60.1",
    "3700000.00",
    "0"
  ]
]"#,
            )
            .expect("Binance klines should parse");

        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.source_name, "binance-public");
        assert_eq!(event.event_type, "market_candle_closed");
        assert!(event.tags.contains(&"venue:binance".to_owned()));
        assert!(event.tags.contains(&"symbol:BTCUSDT".to_owned()));
        assert!(event.tags.contains(&"timeframe:1m".to_owned()));
        assert_eq!(event.payload["venue"], "binance");
        assert_eq!(event.payload["symbol"], "BTCUSDT");
        assert_eq!(event.payload["timeframe"], "1m");
        assert_eq!(event.payload["open"], "61500.12");
        assert_eq!(event.payload["high"], "61640.00");
        assert_eq!(event.payload["low"], "61480.50");
        assert_eq!(event.payload["close"], "61610.30");
        assert_eq!(event.payload["volume"], "124.551");
    }

    fn candle_config_with_interval(interval_secs: u64) -> DataFeedConfig {
        DataFeedConfig::builder()
            .id("candles-cadence-test".to_owned())
            .name("binance-cadence".to_owned())
            .feed_type(FeedType::MarketCandle)
            .tags(vec!["finance".to_owned(), "crypto".to_owned()])
            .transport(serde_json::json!({
                "provider": "binance",
                "base_url": "https://api.binance.com",
                "interval_secs": interval_secs,
                "venue": "binance",
                "symbols": ["BTCUSDT"],
                "timeframes": ["1m"],
                "max_candles_per_poll": 2
            }))
            .enabled(true)
            .status(FeedStatus::Idle)
            .created_at(jiff::Timestamp::UNIX_EPOCH)
            .updated_at(jiff::Timestamp::UNIX_EPOCH)
            .build()
    }

    #[test]
    fn subminute_interval_within_floor_is_accepted() {
        let config = candle_config_with_interval(super::MIN_INTERVAL_SECS);

        MarketCandleSource::from_config(&config)
            .expect("a 5s monitoring cadence at the floor should pass transport validation");
    }

    #[test]
    fn interval_below_rate_limit_floor_is_rejected() {
        let config = candle_config_with_interval(super::MIN_INTERVAL_SECS - 1);

        // `MarketCandleSource` is not `Debug`, so match rather than `expect_err`.
        let error = match MarketCandleSource::from_config(&config) {
            Ok(_) => panic!("an interval below the rate-limit floor must be rejected at load"),
            Err(error) => error,
        };
        let message = error.to_string();

        assert!(
            message.contains(&super::MIN_INTERVAL_SECS.to_string()),
            "error must name the minimum interval, got: {message}"
        );
        assert!(
            message.contains("request-weight") && message.contains("rate limit"),
            "error must explain the rate-limit rationale, got: {message}"
        );
    }
}
