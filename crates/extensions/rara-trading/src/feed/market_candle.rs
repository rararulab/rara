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
//! or a built-in public provider such as Binance or Yahoo, then emits one
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
use jiff::{SignedDuration, Timestamp};
use rara_kernel::data_feed::{
    AuthConfig, DataFeed, DataFeedConfig, FeedEvent, FeedEventId, FeedStatus, FeedType,
    StatusReporterRef, polling::apply_request_auth,
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
pub const DEFAULT_MARKET_CANDLE_REQUEST_BUDGET_PER_SECOND: f64 = 10.0;
const MAX_MARKET_CANDLE_REQUEST_BUDGET_PER_SECOND: f64 = 100.0;
const YAHOO_MARKET_CANDLE_REQUEST_BUDGET_PER_SECOND: f64 = 0.2;
const YAHOO_MIN_INTERVAL_SECS: u64 = 60;
const YAHOO_REQUEST_SPACING_SECS: u64 = 5;

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

#[derive(Debug, Clone)]
pub struct MarketCandleFanoutSafety {
    pub stream_count:                  usize,
    pub poll_request_count:            usize,
    pub configured_interval_secs:      u64,
    pub estimated_requests_per_second: f64,
    pub request_budget_per_second:     f64,
    pub minimum_safe_interval_secs:    u64,
    pub safe_to_start:                 bool,
}

pub fn validate_market_candle_request_budget(budget: f64) -> anyhow::Result<()> {
    anyhow::ensure!(
        budget.is_finite() && budget > 0.0,
        "max_requests_per_second must be positive"
    );
    anyhow::ensure!(
        budget <= MAX_MARKET_CANDLE_REQUEST_BUDGET_PER_SECOND,
        "max_requests_per_second must be <= {MAX_MARKET_CANDLE_REQUEST_BUDGET_PER_SECOND}"
    );
    Ok(())
}

pub fn market_candle_fanout_safety(
    provider: Option<&str>,
    transport: &serde_json::Value,
    symbols: &[String],
    timeframes: &[String],
    request_budget_per_second: f64,
) -> anyhow::Result<MarketCandleFanoutSafety> {
    validate_market_candle_request_budget(request_budget_per_second)?;
    let configured_interval_secs = transport
        .get("interval_secs")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| anyhow::anyhow!("market candle source has no interval_secs"))?;
    anyhow::ensure!(
        configured_interval_secs > 0,
        "market candle source has invalid interval_secs"
    );

    let stream_count = symbols.len().saturating_mul(timeframes.len());
    let poll_request_count = if matches!(provider, Some("binance" | "fmp" | "yahoo")) {
        stream_count
    } else {
        1
    };
    let request_budget_per_second = if provider == Some("yahoo") {
        request_budget_per_second.min(YAHOO_MARKET_CANDLE_REQUEST_BUDGET_PER_SECOND)
    } else {
        request_budget_per_second
    };
    let estimated_requests_per_second = poll_request_count as f64 / configured_interval_secs as f64;
    let provider_minimum_interval_secs = if provider == Some("yahoo") {
        YAHOO_MIN_INTERVAL_SECS
    } else {
        MIN_INTERVAL_SECS
    };
    let minimum_safe_interval_secs = provider_minimum_interval_secs
        .max((poll_request_count as f64 / request_budget_per_second).ceil() as u64);

    Ok(MarketCandleFanoutSafety {
        stream_count,
        poll_request_count,
        configured_interval_secs,
        estimated_requests_per_second,
        request_budget_per_second,
        minimum_safe_interval_secs,
        safe_to_start: configured_interval_secs >= minimum_safe_interval_secs,
    })
}

pub fn market_candle_config_fanout_safety(
    config: &DataFeedConfig,
) -> anyhow::Result<MarketCandleFanoutSafety> {
    anyhow::ensure!(
        config.feed_type == FeedType::MarketCandle,
        "feed type {} is not market_candle",
        config.feed_type
    );
    let final_symbols = transport_string_array(&config.transport, "symbols", true);
    let final_timeframes = transport_string_array(&config.transport, "timeframes", false);
    let provider = config
        .transport
        .get("provider")
        .and_then(serde_json::Value::as_str);
    market_candle_fanout_safety(
        provider,
        &config.transport,
        &final_symbols,
        &final_timeframes,
        DEFAULT_MARKET_CANDLE_REQUEST_BUDGET_PER_SECOND,
    )
}

pub fn unsafe_market_candle_fanout_message(safety: &MarketCandleFanoutSafety) -> String {
    format!(
        "market candle watchlist fans out to {} requests per poll at interval_secs={}; configured \
         interval is unsafe for the default request budget. Increase interval_secs to at least {} \
         or call finance_plan_instrument_watchlist and retry after reviewing the plan.",
        safety.poll_request_count,
        safety.configured_interval_secs,
        safety.minimum_safe_interval_secs
    )
}

fn transport_string_array(
    transport: &serde_json::Value,
    key: &str,
    uppercase: bool,
) -> Vec<String> {
    transport
        .get(key)
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            if uppercase {
                value.to_ascii_uppercase()
            } else {
                value.to_ascii_lowercase()
            }
        })
        .collect()
}

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
struct YahooChartResponse {
    chart: YahooChart,
}

#[derive(Debug, Deserialize)]
struct YahooChart {
    result: Option<Vec<YahooChartResult>>,
    error:  Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct YahooChartResult {
    timestamp:  Option<Vec<i64>>,
    indicators: YahooIndicators,
}

#[derive(Debug, Deserialize)]
struct YahooIndicators {
    quote: Vec<YahooQuote>,
}

#[derive(Debug, Deserialize)]
struct YahooQuote {
    open:   Vec<Option<serde_json::Number>>,
    high:   Vec<Option<serde_json::Number>>,
    low:    Vec<Option<serde_json::Number>>,
    close:  Vec<Option<serde_json::Number>>,
    #[serde(default)]
    volume: Vec<Option<serde_json::Number>>,
}

#[derive(Debug, Deserialize)]
struct FmpEodCandle {
    date:   String,
    open:   serde_json::Number,
    high:   serde_json::Number,
    low:    serde_json::Number,
    close:  serde_json::Number,
    volume: serde_json::Number,
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
        validate_provider_auth(&transport, config.auth.as_ref())?;
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

    fn build_yahoo_chart_url(&self, symbol: &str, timeframe: &str) -> anyhow::Result<Url> {
        let base_url = self
            .transport
            .base_url
            .as_deref()
            .unwrap_or("https://query1.finance.yahoo.com");
        let mut url = Url::parse(base_url)?.join("/v8/finance/chart")?;
        url.path_segments_mut()
            .map_err(|()| anyhow::anyhow!("Yahoo market candle base_url cannot be a base URL"))?
            .push(symbol);
        url.query_pairs_mut()
            .append_pair("interval", yahoo_interval(timeframe)?)
            .append_pair("range", "5d")
            .append_pair("events", "history")
            .append_pair("includePrePost", "false");
        Ok(url)
    }

    fn build_fmp_eod_url(
        &self,
        symbol: &str,
        timeframe: &str,
        now: Timestamp,
    ) -> anyhow::Result<Url> {
        anyhow::ensure!(
            timeframe == "1d",
            "FMP market candle timeframe {timeframe:?} is unsupported; use 1d"
        );
        let base_url = self
            .transport
            .base_url
            .as_deref()
            .unwrap_or("https://financialmodelingprep.com");
        let from = now.checked_sub(SignedDuration::from_hours(24 * 10))?;
        let mut url = Url::parse(base_url)?.join("/stable/historical-price-eod/full")?;
        url.query_pairs_mut()
            .append_pair("symbol", symbol)
            .append_pair("from", &from.strftime("%F").to_string())
            .append_pair("to", &now.strftime("%F").to_string());
        if let Some(AuthConfig::Query { name, value }) = &self.auth {
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

    fn parse_yahoo_candle_events(
        &self,
        symbol: &str,
        timeframe: &str,
        body: &[u8],
    ) -> anyhow::Result<Vec<FeedEvent>> {
        let response: YahooChartResponse = serde_json::from_slice(body)?;
        if let Some(error) = response.chart.error {
            anyhow::bail!("Yahoo chart returned an error: {error}");
        }
        let result = response
            .chart
            .result
            .and_then(|results| results.into_iter().next())
            .ok_or_else(|| anyhow::anyhow!("Yahoo chart response contained no result"))?;
        let timestamps = result
            .timestamp
            .ok_or_else(|| anyhow::anyhow!("Yahoo chart response contained no timestamps"))?;
        let quote = result
            .indicators
            .quote
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("Yahoo chart response contained no quote"))?;
        let received_at = Timestamp::now();
        let now_ms = received_at.as_millisecond();
        let step_ms = yahoo_timeframe_millis(timeframe)?;
        let mut events = Vec::new();

        for (index, open_time_seconds) in timestamps
            .into_iter()
            .enumerate()
            .take(self.transport.max_candles_per_poll)
        {
            let Some(open_time_ms) = open_time_seconds.checked_mul(1000) else {
                continue;
            };
            let Some(close_time_ms) = open_time_ms
                .checked_add(step_ms)
                .and_then(|value| value.checked_sub(1))
            else {
                continue;
            };
            if close_time_ms > now_ms {
                continue;
            }
            let Some(candle) = yahoo_candle_at(
                &quote,
                index,
                &self.transport.venue,
                symbol,
                timeframe,
                open_time_ms,
                close_time_ms,
            ) else {
                continue;
            };
            events.push(self.event_from_candle(candle, received_at));
        }

        Ok(events)
    }

    fn parse_fmp_candle_events(
        &self,
        symbol: &str,
        timeframe: &str,
        body: &[u8],
    ) -> anyhow::Result<Vec<FeedEvent>> {
        let rows: Vec<FmpEodCandle> = serde_json::from_slice(body)?;
        let received_at = Timestamp::now();
        let mut candles = rows
            .into_iter()
            .filter_map(|row| {
                fmp_candle(row, &self.transport.venue, symbol, timeframe, received_at)
            })
            .collect::<Vec<_>>();
        candles.sort_by(|left, right| left.open_time.cmp(&right.open_time));
        let keep_from = candles
            .len()
            .saturating_sub(self.transport.max_candles_per_poll);

        Ok(candles
            .into_iter()
            .skip(keep_from)
            .map(|candle| self.event_from_candle(candle, received_at))
            .collect())
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

    async fn poll_yahoo_once(&self, tx: &mpsc::Sender<FeedEvent>) -> bool {
        let mut any_success = false;
        let mut request_count = 0_usize;

        for symbol in &self.transport.symbols {
            for timeframe in &self.transport.timeframes {
                let url = match self.build_yahoo_chart_url(symbol, timeframe) {
                    Ok(url) => url,
                    Err(err) => {
                        self.record_error(format!("failed to build Yahoo chart URL: {err}"));
                        continue;
                    }
                };

                if request_count > 0 {
                    tokio::time::sleep(Duration::from_secs(YAHOO_REQUEST_SPACING_SECS)).await;
                }
                request_count += 1;

                let mut request = self.client.get(url);
                for (key, value) in &self.transport.headers {
                    request = request.header(key.as_str(), value.as_str());
                }
                request = apply_request_auth(request, &self.auth);

                let response = match request.send().await {
                    Ok(response) => response,
                    Err(err) => {
                        self.record_error(format!("Yahoo chart fetch failed: {err}"));
                        continue;
                    }
                };
                let status = response.status();
                if !status.is_success() {
                    self.record_error(format!(
                        "Yahoo chart fetch received non-success status: {status}"
                    ));
                    continue;
                }
                let body = match response.bytes().await {
                    Ok(body) => body,
                    Err(err) => {
                        self.record_error(format!("failed to read Yahoo chart body: {err}"));
                        continue;
                    }
                };
                if body.len() > MAX_BODY_BYTES {
                    self.record_error(format!(
                        "Yahoo chart response exceeded {MAX_BODY_BYTES} bytes"
                    ));
                    continue;
                }
                let events = match self.parse_yahoo_candle_events(symbol, timeframe, &body) {
                    Ok(events) => events,
                    Err(err) => {
                        self.record_error(format!("failed to parse Yahoo chart response: {err}"));
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

    async fn poll_fmp_once(&self, tx: &mpsc::Sender<FeedEvent>) -> bool {
        let mut any_success = false;

        for symbol in &self.transport.symbols {
            for timeframe in &self.transport.timeframes {
                let url = match self.build_fmp_eod_url(symbol, timeframe, Timestamp::now()) {
                    Ok(url) => url,
                    Err(err) => {
                        self.record_error(format!("failed to build FMP historical URL: {err}"));
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
                    Err(_) => {
                        self.record_error("FMP historical request failed".to_owned());
                        continue;
                    }
                };
                let status = response.status();
                if !status.is_success() {
                    self.record_error(format!(
                        "FMP historical request received non-success status: {status}"
                    ));
                    continue;
                }
                let body = match response.bytes().await {
                    Ok(body) => body,
                    Err(_) => {
                        self.record_error("failed to read FMP historical response".to_owned());
                        continue;
                    }
                };
                if body.len() > MAX_BODY_BYTES {
                    self.record_error(format!(
                        "FMP historical response exceeded {MAX_BODY_BYTES} bytes"
                    ));
                    continue;
                }
                let events = match self.parse_fmp_candle_events(symbol, timeframe, &body) {
                    Ok(events) => events,
                    Err(err) => {
                        self.record_error(format!(
                            "failed to parse FMP historical response: {err}"
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
        match self.transport.provider.as_deref() {
            Some("binance") => self.poll_binance_once(tx).await,
            Some("fmp") => self.poll_fmp_once(tx).await,
            Some("yahoo") => self.poll_yahoo_once(tx).await,
            _ => self.poll_normalized_once(tx).await,
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
        "yahoo" => {
            let base_url = transport
                .base_url
                .as_deref()
                .unwrap_or("https://query1.finance.yahoo.com");
            let url = Url::parse(base_url)?;
            let test_loopback = cfg!(test)
                && url.scheme() == "http"
                && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1"));
            anyhow::ensure!(
                url.scheme() == "https" || test_loopback,
                "Yahoo market candle base_url must use HTTPS"
            );
            for timeframe in &transport.timeframes {
                yahoo_interval(timeframe)?;
            }
        }
        "fmp" => {
            let base_url = transport
                .base_url
                .as_deref()
                .unwrap_or("https://financialmodelingprep.com");
            let url = Url::parse(base_url)?;
            let test_loopback = cfg!(test)
                && url.scheme() == "http"
                && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1"));
            anyhow::ensure!(
                url.scheme() == "https" || test_loopback,
                "FMP market candle base_url must use HTTPS"
            );
            for timeframe in &transport.timeframes {
                anyhow::ensure!(
                    timeframe == "1d",
                    "FMP market candle timeframe {timeframe:?} is unsupported; use 1d"
                );
            }
        }
        provider => anyhow::bail!("unsupported market candle provider: {provider}"),
    }
    anyhow::ensure!(
        transport.interval_secs > 0,
        "market candle interval_secs must be greater than zero"
    );
    let minimum_interval_secs = if transport.provider.as_deref() == Some("yahoo") {
        YAHOO_MIN_INTERVAL_SECS
    } else {
        MIN_INTERVAL_SECS
    };
    anyhow::ensure!(
        transport.interval_secs >= minimum_interval_secs,
        "market candle interval_secs must be at least {minimum_interval_secs} seconds for \
         provider {:?}; each poll may fan out to symbols × timeframes requests, so faster polling \
         can trip provider request-weight and rate limit policies",
        transport.provider.as_deref().unwrap_or("normalized")
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

fn validate_provider_auth(
    transport: &MarketCandleTransport,
    auth: Option<&AuthConfig>,
) -> anyhow::Result<()> {
    if transport.provider.as_deref() != Some("fmp") {
        return Ok(());
    }
    let valid = match auth {
        Some(AuthConfig::Header { name, value } | AuthConfig::Query { name, value }) => {
            name.eq_ignore_ascii_case("apikey") && !value.trim().is_empty()
        }
        _ => false,
    };
    anyhow::ensure!(
        valid,
        "FMP market candle provider requires a non-empty apikey header or query parameter"
    );
    Ok(())
}

fn yahoo_interval(timeframe: &str) -> anyhow::Result<&'static str> {
    match timeframe {
        "1m" => Ok("1m"),
        "5m" => Ok("5m"),
        "15m" => Ok("15m"),
        "30m" => Ok("30m"),
        "1h" => Ok("1h"),
        "1d" => Ok("1d"),
        _ => anyhow::bail!(
            "Yahoo market candle timeframe {timeframe:?} is unsupported; use one of 1m, 5m, 15m, \
             30m, 1h, or 1d"
        ),
    }
}

fn yahoo_timeframe_millis(timeframe: &str) -> anyhow::Result<i64> {
    let timeframe = Timeframe::parse(timeframe)?;
    let seconds = timeframe.step()?.as_secs();
    seconds
        .checked_mul(1000)
        .ok_or_else(|| anyhow::anyhow!("Yahoo timeframe is too large"))
}

fn yahoo_candle_at(
    quote: &YahooQuote,
    index: usize,
    venue: &str,
    symbol: &str,
    timeframe: &str,
    open_time_ms: i64,
    close_time_ms: i64,
) -> Option<ParsedCandle> {
    fn decimal_at(values: &[Option<serde_json::Number>], index: usize) -> Option<Decimal> {
        Decimal::from_str(&values.get(index)?.as_ref()?.to_string()).ok()
    }

    Some(ParsedCandle {
        venue:      venue.to_owned(),
        symbol:     symbol.to_owned(),
        timeframe:  timeframe.to_owned(),
        open_time:  Timestamp::from_millisecond(open_time_ms).ok()?.to_string(),
        close_time: Timestamp::from_millisecond(close_time_ms).ok()?.to_string(),
        open:       decimal_at(&quote.open, index)?,
        high:       decimal_at(&quote.high, index)?,
        low:        decimal_at(&quote.low, index)?,
        close:      decimal_at(&quote.close, index)?,
        volume:     quote
            .volume
            .get(index)
            .and_then(Option::as_ref)
            .and_then(|value| Decimal::from_str(&value.to_string()).ok())
            .unwrap_or(Decimal::ZERO),
    })
}

fn fmp_candle(
    row: FmpEodCandle,
    venue: &str,
    symbol: &str,
    timeframe: &str,
    received_at: Timestamp,
) -> Option<ParsedCandle> {
    let open_time = format!("{}T00:00:00Z", row.date)
        .parse::<Timestamp>()
        .ok()?;
    let close_time = open_time
        .checked_add(SignedDuration::from_hours(24))
        .ok()?
        .checked_sub(SignedDuration::from_millis(1))
        .ok()?;
    if close_time > received_at {
        return None;
    }
    let decimal = |value: serde_json::Number| Decimal::from_str(&value.to_string()).ok();
    Some(ParsedCandle {
        venue:      venue.to_owned(),
        symbol:     symbol.to_owned(),
        timeframe:  timeframe.to_owned(),
        open_time:  open_time.to_string(),
        close_time: close_time.to_string(),
        open:       decimal(row.open)?,
        high:       decimal(row.high)?,
        low:        decimal(row.low)?,
        close:      decimal(row.close)?,
        volume:     decimal(row.volume)?,
    })
}

fn dedupe_sort(values: &mut Vec<String>) {
    let mut seen = HashSet::new();
    values.retain(|value| seen.insert(value.clone()));
    values.sort();
}

#[cfg(test)]
mod tests {
    use rara_kernel::data_feed::{AuthConfig, DataFeedConfig, FeedStatus, FeedType};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path, query_param},
    };

    use super::{MarketCandleSource, market_candle_fanout_safety};

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

    fn yahoo_source() -> MarketCandleSource {
        yahoo_source_with_base_url("https://query1.finance.yahoo.com", "AAPL")
    }

    fn yahoo_source_with_base_url(base_url: &str, symbol: &str) -> MarketCandleSource {
        let config = DataFeedConfig::builder()
            .id("yahoo-candles-test".to_owned())
            .name("yahoo-public".to_owned())
            .feed_type(FeedType::MarketCandle)
            .tags(vec!["finance".to_owned(), "best-effort".to_owned()])
            .transport(serde_json::json!({
                "provider": "yahoo",
                "base_url": base_url,
                "interval_secs": 900,
                "venue": "yahoo",
                "symbols": [symbol],
                "timeframes": ["1d"],
                "max_candles_per_poll": 5
            }))
            .enabled(true)
            .status(FeedStatus::Idle)
            .created_at(jiff::Timestamp::UNIX_EPOCH)
            .updated_at(jiff::Timestamp::UNIX_EPOCH)
            .build();

        MarketCandleSource::from_config(&config).expect("Yahoo config should parse")
    }

    fn fmp_config(base_url: &str, auth: Option<AuthConfig>) -> DataFeedConfig {
        DataFeedConfig::builder()
            .id("fmp-candles-test".to_owned())
            .name("fmp-us-equities".to_owned())
            .feed_type(FeedType::MarketCandle)
            .tags(vec!["finance".to_owned(), "equities".to_owned()])
            .transport(serde_json::json!({
                "provider": "fmp",
                "base_url": base_url,
                "interval_secs": 900,
                "venue": "fmp",
                "symbols": ["AAPL"],
                "timeframes": ["1d"],
                "max_candles_per_poll": 5
            }))
            .maybe_auth(auth)
            .enabled(true)
            .status(FeedStatus::Idle)
            .created_at(jiff::Timestamp::UNIX_EPOCH)
            .updated_at(jiff::Timestamp::UNIX_EPOCH)
            .build()
    }

    fn fmp_source_with_base_url(base_url: &str) -> MarketCandleSource {
        MarketCandleSource::from_config(&fmp_config(
            base_url,
            Some(AuthConfig::Query {
                name:  "apikey".to_owned(),
                value: "test-secret".to_owned(),
            }),
        ))
        .expect("FMP config should parse")
    }

    #[test]
    fn fmp_provider_requires_an_api_key() {
        let error = match MarketCandleSource::from_config(&fmp_config(
            "https://financialmodelingprep.com",
            None,
        )) {
            Ok(_) => panic!("FMP must reject a config without an API key"),
            Err(error) => error,
        };

        assert!(
            error.to_string().contains("apikey"),
            "error should explain the required FMP credential: {error}"
        );
    }

    #[test]
    fn fmp_provider_rejects_non_apikey_query_auth() {
        let error = match MarketCandleSource::from_config(&fmp_config(
            "https://financialmodelingprep.com",
            Some(AuthConfig::Query {
                name:  "token".to_owned(),
                value: "test-secret".to_owned(),
            }),
        )) {
            Ok(_) => panic!("FMP must reject the wrong query credential name"),
            Err(error) => error,
        };

        assert!(
            error.to_string().contains("apikey"),
            "error should name the supported FMP credential: {error}"
        );
    }

    #[test]
    fn fmp_provider_accepts_apikey_header_auth() {
        MarketCandleSource::from_config(&fmp_config(
            "https://financialmodelingprep.com",
            Some(AuthConfig::Header {
                name:  "apikey".to_owned(),
                value: "test-secret".to_owned(),
            }),
        ))
        .expect("FMP should accept its documented header authentication");
    }

    #[test]
    fn fmp_provider_builds_a_bounded_eod_url() {
        let source = fmp_source_with_base_url("https://financialmodelingprep.com");
        let now = "2026-07-19T12:00:00Z"
            .parse::<jiff::Timestamp>()
            .expect("fixed timestamp");
        let url = source
            .build_fmp_eod_url("AAPL", "1d", now)
            .expect("FMP URL should build");

        assert_eq!(
            url.as_str(),
            "https://financialmodelingprep.com/stable/historical-price-eod/full?symbol=AAPL&from=2026-07-09&to=2026-07-19&apikey=test-secret"
        );
    }

    #[test]
    fn fmp_fanout_uses_one_request_per_symbol() {
        let safety = market_candle_fanout_safety(
            Some("fmp"),
            &serde_json::json!({"interval_secs": 900}),
            &["AAPL".to_owned(), "MSFT".to_owned(), "NVDA".to_owned()],
            &["1d".to_owned()],
            super::DEFAULT_MARKET_CANDLE_REQUEST_BUDGET_PER_SECOND,
        )
        .expect("fan-out should calculate");

        assert_eq!(safety.stream_count, 3);
        assert_eq!(safety.poll_request_count, 3);
    }

    #[tokio::test]
    async fn fmp_poll_fetches_eod_candles_with_query_auth() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/stable/historical-price-eod/full"))
            .and(query_param("symbol", "AAPL"))
            .and(query_param("apikey", "test-secret"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"[
  {
    "symbol": "AAPL",
    "date": "2024-04-15",
    "open": 172.50,
    "high": 176.63,
    "low": 172.45,
    "close": 175.04,
    "volume": 73531800
  },
  {
    "symbol": "AAPL",
    "date": "2100-01-01",
    "open": 190.00,
    "high": 192.00,
    "low": 189.50,
    "close": 191.25,
    "volume": 1000
  }
]"#,
                "application/json",
            ))
            .expect(1)
            .mount(&server)
            .await;
        let source = fmp_source_with_base_url(&server.uri());
        let (tx, mut rx) = tokio::sync::mpsc::channel(2);

        assert!(source.poll_once(&tx).await);
        drop(tx);

        let event = rx.recv().await.expect("closed FMP candle event");
        assert_eq!(event.source_name, "fmp-us-equities");
        assert_eq!(event.payload["venue"], "fmp");
        assert_eq!(event.payload["symbol"], "AAPL");
        assert_eq!(event.payload["timeframe"], "1d");
        assert_eq!(event.payload["open_time"], "2024-04-15T00:00:00Z");
        assert_eq!(event.payload["close_time"], "2024-04-15T23:59:59.999Z");
        assert_eq!(event.payload["open"], "172.5");
        assert_eq!(event.payload["high"], "176.63");
        assert_eq!(event.payload["low"], "172.45");
        assert_eq!(event.payload["close"], "175.04");
        assert_eq!(event.payload["volume"], "73531800");
        assert!(
            rx.recv().await.is_none(),
            "future daily bar must be skipped"
        );
    }

    #[tokio::test]
    async fn fmp_poll_sends_apikey_header_auth() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/stable/historical-price-eod/full"))
            .and(query_param("symbol", "AAPL"))
            .and(header("apikey", "test-secret"))
            .respond_with(ResponseTemplate::new(200).set_body_raw("[]", "application/json"))
            .expect(1)
            .mount(&server)
            .await;
        let source = MarketCandleSource::from_config(&fmp_config(
            &server.uri(),
            Some(AuthConfig::Header {
                name:  "apikey".to_owned(),
                value: "test-secret".to_owned(),
            }),
        ))
        .expect("FMP header config should parse");
        let (tx, _rx) = tokio::sync::mpsc::channel(1);

        assert!(source.poll_once(&tx).await);
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
    fn yahoo_provider_builds_public_chart_url() {
        let source = yahoo_source();
        let url = source
            .build_yahoo_chart_url("^GSPC", "1d")
            .expect("URL should build");

        assert_eq!(
            url.as_str(),
            "https://query1.finance.yahoo.com/v8/finance/chart/^GSPC?interval=1d&range=5d&events=history&includePrePost=false"
        );
    }

    #[test]
    fn yahoo_chart_becomes_closed_candle_events_and_skips_open_or_null_bars() {
        let source = yahoo_source();
        let events = source
            .parse_yahoo_candle_events(
                "AAPL",
                "1d",
                br#"{
  "chart": {
    "result": [{
      "meta": {"symbol": "AAPL"},
      "timestamp": [1713139200, 4102444800, 1713312000],
      "indicators": {
        "quote": [{
          "open": [172.50, 190.00, null],
          "high": [176.63, 192.00, 180.00],
          "low": [172.45, 189.50, 175.00],
          "close": [175.04, 191.25, 179.00],
          "volume": [73531800, 1000, 2000]
        }]
      }
    }],
    "error": null
  }
}"#,
            )
            .expect("Yahoo chart should parse");

        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.source_name, "yahoo-public");
        assert_eq!(event.event_type, "market_candle_closed");
        assert!(event.tags.contains(&"venue:yahoo".to_owned()));
        assert!(event.tags.contains(&"symbol:AAPL".to_owned()));
        assert!(event.tags.contains(&"timeframe:1d".to_owned()));
        assert_eq!(event.payload["open"], "172.5");
        assert_eq!(event.payload["high"], "176.63");
        assert_eq!(event.payload["low"], "172.45");
        assert_eq!(event.payload["close"], "175.04");
        assert_eq!(event.payload["volume"], "73531800");
    }

    #[test]
    fn yahoo_chart_normalizes_missing_volume_to_zero() {
        let source = yahoo_source();
        let events = source
            .parse_yahoo_candle_events(
                "AAPL",
                "1d",
                br#"{
  "chart": {
    "result": [{
      "timestamp": [1713139200],
      "indicators": {
        "quote": [{
          "open": [172.50],
          "high": [176.63],
          "low": [172.45],
          "close": [175.04],
          "volume": [null]
        }]
      }
    }],
    "error": null
  }
}"#,
            )
            .expect("Yahoo chart should parse");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].payload["volume"], "0");
    }

    #[tokio::test]
    async fn yahoo_poll_fetches_chart_and_emits_closed_candle() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v8/finance/chart/AAPL"))
            .and(query_param("interval", "1d"))
            .and(query_param("range", "5d"))
            .and(query_param("events", "history"))
            .and(query_param("includePrePost", "false"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{
  "chart": {
    "result": [{
      "timestamp": [1713139200],
      "indicators": {
        "quote": [{
          "open": [172.50],
          "high": [176.63],
          "low": [172.45],
          "close": [175.04],
          "volume": [73531800]
        }]
      }
    }],
    "error": null
  }
}"#,
                "application/json",
            ))
            .expect(1)
            .mount(&server)
            .await;
        let source = yahoo_source_with_base_url(&server.uri(), "AAPL");
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);

        assert!(source.poll_yahoo_once(&tx).await);
        drop(tx);

        let event = rx.recv().await.expect("closed candle event");
        assert_eq!(event.payload["symbol"], "AAPL");
        assert_eq!(event.payload["close"], "175.04");
        assert!(rx.recv().await.is_none());
    }

    #[test]
    fn yahoo_fanout_uses_one_request_per_stream_and_conservative_budget() {
        let transport = serde_json::json!({"interval_secs": 60});
        let symbols = (0..100)
            .map(|index| format!("SYM{index}"))
            .collect::<Vec<_>>();
        let timeframes = vec!["1d".to_owned()];

        let safety = market_candle_fanout_safety(
            Some("yahoo"),
            &transport,
            &symbols,
            &timeframes,
            super::DEFAULT_MARKET_CANDLE_REQUEST_BUDGET_PER_SECOND,
        )
        .expect("fan-out should calculate");

        assert_eq!(safety.poll_request_count, 100);
        assert_eq!(safety.request_budget_per_second, 0.2);
        assert_eq!(safety.minimum_safe_interval_secs, 500);
        assert!(!safety.safe_to_start);

        let one_symbol = market_candle_fanout_safety(
            Some("yahoo"),
            &serde_json::json!({"interval_secs": 5}),
            &["AAPL".to_owned()],
            &timeframes,
            super::DEFAULT_MARKET_CANDLE_REQUEST_BUDGET_PER_SECOND,
        )
        .expect("fan-out should calculate");
        assert_eq!(one_symbol.minimum_safe_interval_secs, 60);
        assert!(!one_symbol.safe_to_start);
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
