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

//! Conversation-facing market candle query tools.

use async_trait::async_trait;
use jiff::Timestamp;
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{
    CandleLatestQuery, CandleRangeQuery, CandleStreamListQuery, CandleStreamSummary, MarketCandle,
    MarketDataRepositoryRef, Timeframe,
};

const DEFAULT_CANDLE_LIMIT: usize = 500;
const MAX_CANDLE_LIMIT: usize = 10_000;
const MAX_CANDLE_STREAM_LIMIT: usize = MAX_CANDLE_LIMIT - 1;
const MAX_SELECTOR_LEN: usize = 128;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FinanceGetLatestCandleParams {
    #[serde(default)]
    pub source_name: Option<String>,
    pub venue:       String,
    pub symbol:      String,
    pub timeframe:   String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FinanceGetLatestCandleResult {
    pub candle: Option<FinanceCandle>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FinanceQueryCandlesParams {
    #[serde(default)]
    pub source_name: Option<String>,
    pub venue:       String,
    pub symbol:      String,
    pub timeframe:   String,
    /// Inclusive candle open time, as an RFC3339 timestamp.
    pub start:       String,
    /// Exclusive candle open time, as an RFC3339 timestamp.
    pub end:         String,
    #[serde(default)]
    pub limit:       Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FinanceQueryCandlesResult {
    pub candles:     Vec<FinanceCandle>,
    pub count:       usize,
    pub query_limit: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FinanceFindCandleGapsParams {
    #[serde(default)]
    pub source_name: Option<String>,
    pub venue:       String,
    pub symbol:      String,
    pub timeframe:   String,
    /// Inclusive candle open time, as an RFC3339 timestamp.
    pub start:       String,
    /// Exclusive candle open time, as an RFC3339 timestamp.
    pub end:         String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FinanceFindCandleGapsResult {
    pub missing_open_times: Vec<String>,
    pub missing_count:      usize,
    pub expected_count:     usize,
    pub complete:           bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FinanceGetCandleFreshnessParams {
    #[serde(default)]
    pub source_name:      Option<String>,
    pub venue:            String,
    pub symbol:           String,
    pub timeframe:        String,
    /// Comparison timestamp. Defaults to server now.
    #[serde(default)]
    pub as_of:            Option<String>,
    /// Stale threshold in seconds. Defaults to 2x the timeframe step.
    #[serde(default)]
    pub stale_after_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FinanceGetCandleFreshnessResult {
    pub latest:           Option<FinanceCandle>,
    pub as_of:            String,
    pub stale_after_secs: u64,
    pub lag_secs:         Option<i64>,
    pub is_stale:         bool,
    pub status:           String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FinanceListCandleStreamsParams {
    #[serde(default)]
    pub source_name: Option<String>,
    #[serde(default)]
    pub venue:       Option<String>,
    #[serde(default)]
    pub symbol:      Option<String>,
    #[serde(default)]
    pub timeframe:   Option<String>,
    #[serde(default)]
    pub limit:       Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FinanceListCandleStreamsResult {
    pub streams:     Vec<FinanceCandleStream>,
    pub count:       usize,
    pub query_limit: usize,
    pub has_more:    bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FinanceCandleStream {
    pub source_name:        String,
    pub venue:              String,
    pub symbol:             String,
    pub timeframe:          String,
    pub candle_count:       usize,
    pub first_open_time:    String,
    pub latest_open_time:   String,
    pub latest_close_time:  String,
    pub latest_ingested_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FinanceCandle {
    pub source_name:       String,
    pub venue:             String,
    pub symbol:            String,
    pub timeframe:         String,
    pub open_time:         String,
    pub close_time:        String,
    pub open:              String,
    pub high:              String,
    pub low:               String,
    pub close:             String,
    pub volume:            String,
    pub ingested_at:       String,
    pub provider_sequence: Option<String>,
}

#[derive(ToolDef)]
#[tool(
    name = "finance_list_candle_streams",
    description = "List stored OHLCV candle streams and their latest watermarks from the \
                   market-data TSDB. Use this to discover available source/venue/symbol/timeframe \
                   combinations before querying candles, freshness, or gaps. This is read-only \
                   and never places trades.",
    tier = "deferred",
    read_only,
    concurrency_safe
)]
pub struct FinanceListCandleStreamsTool {
    repository: MarketDataRepositoryRef,
}

impl FinanceListCandleStreamsTool {
    pub fn new(repository: MarketDataRepositoryRef) -> Self { Self { repository } }
}

#[async_trait]
impl ToolExecute for FinanceListCandleStreamsTool {
    type Output = FinanceListCandleStreamsResult;
    type Params = FinanceListCandleStreamsParams;

    async fn run(
        &self,
        params: FinanceListCandleStreamsParams,
        _context: &ToolContext,
    ) -> anyhow::Result<FinanceListCandleStreamsResult> {
        let limit = validate_stream_limit(params.limit)?;
        let query_limit = limit.saturating_add(1);
        let query = CandleStreamListQuery {
            source_name: normalize_optional_source_name_selector(params.source_name)?,
            venue:       normalize_optional_venue_selector(params.venue)?,
            symbol:      normalize_optional_symbol_selector(params.symbol)?,
            timeframe:   normalize_optional_timeframe_selector(params.timeframe)?,
            limit:       query_limit,
        };
        let mut streams = self.repository.candle_streams(query).await?;
        let has_more = streams.len() > limit;
        streams.truncate(limit);
        let count = streams.len();
        Ok(FinanceListCandleStreamsResult {
            streams: streams.into_iter().map(FinanceCandleStream::from).collect(),
            count,
            query_limit: limit,
            has_more,
        })
    }
}

#[derive(ToolDef)]
#[tool(
    name = "finance_get_latest_candle",
    description = "Return the newest closed OHLCV candle stored in the market-data TSDB for a \
                   venue, symbol, and timeframe. Decimal values are returned as strings to \
                   preserve financial precision. This is read-only and never places trades.",
    tier = "deferred",
    read_only,
    concurrency_safe
)]
pub struct FinanceGetLatestCandleTool {
    repository: MarketDataRepositoryRef,
}

impl FinanceGetLatestCandleTool {
    pub fn new(repository: MarketDataRepositoryRef) -> Self { Self { repository } }
}

#[async_trait]
impl ToolExecute for FinanceGetLatestCandleTool {
    type Output = FinanceGetLatestCandleResult;
    type Params = FinanceGetLatestCandleParams;

    async fn run(
        &self,
        params: FinanceGetLatestCandleParams,
        _context: &ToolContext,
    ) -> anyhow::Result<FinanceGetLatestCandleResult> {
        let query = CandleLatestQuery {
            source_name: normalize_optional_source_name_selector(params.source_name)?,
            venue:       normalize_required_venue_selector(params.venue)?,
            symbol:      normalize_required_symbol_selector(params.symbol)?,
            timeframe:   normalize_required_timeframe_selector(params.timeframe)?,
        };
        let candle = self.repository.latest_closed_candle(query).await?;
        Ok(FinanceGetLatestCandleResult {
            candle: candle.map(FinanceCandle::from),
        })
    }
}

#[derive(ToolDef)]
#[tool(
    name = "finance_query_candles",
    description = "Query ordered closed OHLCV candles from the market-data TSDB for a venue, \
                   symbol, timeframe, and open-time range. Decimal values are returned as strings \
                   to preserve financial precision. This is read-only and never places trades.",
    tier = "deferred",
    read_only,
    concurrency_safe
)]
pub struct FinanceQueryCandlesTool {
    repository: MarketDataRepositoryRef,
}

impl FinanceQueryCandlesTool {
    pub fn new(repository: MarketDataRepositoryRef) -> Self { Self { repository } }
}

#[async_trait]
impl ToolExecute for FinanceQueryCandlesTool {
    type Output = FinanceQueryCandlesResult;
    type Params = FinanceQueryCandlesParams;

    async fn run(
        &self,
        params: FinanceQueryCandlesParams,
        _context: &ToolContext,
    ) -> anyhow::Result<FinanceQueryCandlesResult> {
        let limit = validate_candle_limit(params.limit)?;
        let start = parse_timestamp("start", &params.start)?;
        let end = parse_timestamp("end", &params.end)?;
        anyhow::ensure!(start < end, "start must be before end");

        let query = CandleRangeQuery {
            source_name: normalize_optional_source_name_selector(params.source_name)?,
            venue: normalize_required_venue_selector(params.venue)?,
            symbol: normalize_required_symbol_selector(params.symbol)?,
            timeframe: normalize_required_timeframe_selector(params.timeframe)?,
            start,
            end,
            limit,
        };
        let candles = self.repository.candles(query).await?;
        let count = candles.len();
        Ok(FinanceQueryCandlesResult {
            candles: candles.into_iter().map(FinanceCandle::from).collect(),
            count,
            query_limit: limit,
        })
    }
}

#[derive(ToolDef)]
#[tool(
    name = "finance_find_candle_gaps",
    description = "Find missing closed-candle open times for a venue, symbol, timeframe, and \
                   open-time range in the market-data TSDB. The range is capped to 10,000 \
                   expected candles to keep quality checks bounded. This is read-only and never \
                   places trades.",
    tier = "deferred",
    read_only,
    concurrency_safe
)]
pub struct FinanceFindCandleGapsTool {
    repository: MarketDataRepositoryRef,
}

impl FinanceFindCandleGapsTool {
    pub fn new(repository: MarketDataRepositoryRef) -> Self { Self { repository } }
}

#[async_trait]
impl ToolExecute for FinanceFindCandleGapsTool {
    type Output = FinanceFindCandleGapsResult;
    type Params = FinanceFindCandleGapsParams;

    async fn run(
        &self,
        params: FinanceFindCandleGapsParams,
        _context: &ToolContext,
    ) -> anyhow::Result<FinanceFindCandleGapsResult> {
        let timeframe = normalize_required_timeframe_selector(params.timeframe)?;
        let start = parse_timestamp("start", &params.start)?;
        let end = parse_timestamp("end", &params.end)?;
        anyhow::ensure!(start < end, "start must be before end");
        let expected_count = expected_open_time_count(&timeframe, start, end)?;

        let query = CandleRangeQuery {
            source_name: normalize_optional_source_name_selector(params.source_name)?,
            venue: normalize_required_venue_selector(params.venue)?,
            symbol: normalize_required_symbol_selector(params.symbol)?,
            timeframe,
            start,
            end,
            limit: expected_count,
        };
        let missing = self.repository.missing_open_times(query).await?;
        let missing_count = missing.len();

        Ok(FinanceFindCandleGapsResult {
            missing_open_times: missing.into_iter().map(|ts| ts.to_string()).collect(),
            missing_count,
            expected_count,
            complete: missing_count == 0,
        })
    }
}

#[derive(ToolDef)]
#[tool(
    name = "finance_get_candle_freshness",
    description = "Report whether the latest closed candle for a venue, symbol, and timeframe is \
                   fresh or stale compared with an as_of timestamp. Defaults stale_after_secs to \
                   2x the timeframe step. This is read-only and never places trades.",
    tier = "deferred",
    read_only,
    concurrency_safe
)]
pub struct FinanceGetCandleFreshnessTool {
    repository: MarketDataRepositoryRef,
}

impl FinanceGetCandleFreshnessTool {
    pub fn new(repository: MarketDataRepositoryRef) -> Self { Self { repository } }
}

#[async_trait]
impl ToolExecute for FinanceGetCandleFreshnessTool {
    type Output = FinanceGetCandleFreshnessResult;
    type Params = FinanceGetCandleFreshnessParams;

    async fn run(
        &self,
        params: FinanceGetCandleFreshnessParams,
        _context: &ToolContext,
    ) -> anyhow::Result<FinanceGetCandleFreshnessResult> {
        let timeframe = normalize_required_timeframe_selector(params.timeframe)?;
        let default_stale_after_secs =
            u64::try_from(timeframe.step()?.as_secs())?.saturating_mul(2);
        let stale_after_secs = params.stale_after_secs.unwrap_or(default_stale_after_secs);
        anyhow::ensure!(stale_after_secs > 0, "stale_after_secs must be positive");
        anyhow::ensure!(
            stale_after_secs <= 31_536_000,
            "stale_after_secs must be <= 31536000"
        );
        let as_of = params
            .as_of
            .as_deref()
            .map(|value| parse_timestamp("as_of", value))
            .transpose()?
            .unwrap_or_else(Timestamp::now);

        let query = CandleLatestQuery {
            source_name: normalize_optional_source_name_selector(params.source_name)?,
            venue: normalize_required_venue_selector(params.venue)?,
            symbol: normalize_required_symbol_selector(params.symbol)?,
            timeframe,
        };
        let latest = self.repository.latest_closed_candle(query).await?;
        let Some(candle) = latest else {
            return Ok(FinanceGetCandleFreshnessResult {
                latest: None,
                as_of: as_of.to_string(),
                stale_after_secs,
                lag_secs: None,
                is_stale: true,
                status: "missing".to_owned(),
            });
        };

        let lag_secs = as_of.as_second() - candle.close_time.as_second();
        let is_stale = lag_secs >= 0 && lag_secs as u64 > stale_after_secs;
        let status = if lag_secs < 0 {
            "future"
        } else if is_stale {
            "stale"
        } else {
            "fresh"
        };

        Ok(FinanceGetCandleFreshnessResult {
            latest: Some(FinanceCandle::from(candle)),
            as_of: as_of.to_string(),
            stale_after_secs,
            lag_secs: Some(lag_secs),
            is_stale,
            status: status.to_owned(),
        })
    }
}

impl From<CandleStreamSummary> for FinanceCandleStream {
    fn from(summary: CandleStreamSummary) -> Self {
        Self {
            source_name:        summary.source_name,
            venue:              summary.venue,
            symbol:             summary.symbol,
            timeframe:          summary.timeframe.to_string(),
            candle_count:       summary.candle_count,
            first_open_time:    summary.first_open_time.to_string(),
            latest_open_time:   summary.latest_open_time.to_string(),
            latest_close_time:  summary.latest_close_time.to_string(),
            latest_ingested_at: summary.latest_ingested_at.to_string(),
        }
    }
}

impl From<MarketCandle> for FinanceCandle {
    fn from(candle: MarketCandle) -> Self {
        Self {
            source_name:       candle.source_name,
            venue:             candle.venue,
            symbol:            candle.symbol,
            timeframe:         candle.timeframe.to_string(),
            open_time:         candle.open_time.to_string(),
            close_time:        candle.close_time.to_string(),
            open:              candle.open.to_string(),
            high:              candle.high.to_string(),
            low:               candle.low.to_string(),
            close:             candle.close.to_string(),
            volume:            candle.volume.to_string(),
            ingested_at:       candle.ingested_at.to_string(),
            provider_sequence: candle.provider_sequence,
        }
    }
}

fn normalize_required_selector(name: &str, value: String) -> anyhow::Result<String> {
    let value = value.trim();
    anyhow::ensure!(!value.is_empty(), "{name} must not be empty");
    anyhow::ensure!(
        value.chars().count() <= MAX_SELECTOR_LEN,
        "{name} is too long"
    );
    Ok(value.to_owned())
}

fn normalize_optional_selector(
    name: &str,
    value: Option<String>,
) -> anyhow::Result<Option<String>> {
    value
        .map(|value| normalize_required_selector(name, value))
        .transpose()
}

fn normalize_optional_source_name_selector(
    value: Option<String>,
) -> anyhow::Result<Option<String>> {
    normalize_optional_selector("source_name", value)
}

fn normalize_required_venue_selector(value: String) -> anyhow::Result<String> {
    Ok(normalize_required_selector("venue", value)?.to_ascii_lowercase())
}

fn normalize_optional_venue_selector(value: Option<String>) -> anyhow::Result<Option<String>> {
    value.map(normalize_required_venue_selector).transpose()
}

fn normalize_required_symbol_selector(value: String) -> anyhow::Result<String> {
    Ok(normalize_required_selector("symbol", value)?.to_ascii_uppercase())
}

fn normalize_optional_symbol_selector(value: Option<String>) -> anyhow::Result<Option<String>> {
    value.map(normalize_required_symbol_selector).transpose()
}

fn normalize_required_timeframe_selector(value: String) -> anyhow::Result<Timeframe> {
    Timeframe::parse(normalize_required_selector("timeframe", value)?.to_ascii_lowercase())
}

fn normalize_optional_timeframe_selector(
    value: Option<String>,
) -> anyhow::Result<Option<Timeframe>> {
    value.map(normalize_required_timeframe_selector).transpose()
}

fn validate_candle_limit(limit: Option<usize>) -> anyhow::Result<usize> {
    let limit = limit.unwrap_or(DEFAULT_CANDLE_LIMIT);
    anyhow::ensure!(limit > 0, "limit must be positive");
    anyhow::ensure!(
        limit <= MAX_CANDLE_LIMIT,
        "limit must be <= {MAX_CANDLE_LIMIT}"
    );
    Ok(limit)
}

fn validate_stream_limit(limit: Option<usize>) -> anyhow::Result<usize> {
    let limit = limit.unwrap_or(DEFAULT_CANDLE_LIMIT);
    anyhow::ensure!(limit > 0, "limit must be positive");
    anyhow::ensure!(
        limit <= MAX_CANDLE_STREAM_LIMIT,
        "limit must be <= {MAX_CANDLE_STREAM_LIMIT}"
    );
    Ok(limit)
}

fn parse_timestamp(name: &str, value: &str) -> anyhow::Result<Timestamp> {
    value
        .parse()
        .map_err(|err| anyhow::anyhow!("{name} must be an RFC3339 timestamp: {err}"))
}

fn expected_open_time_count(
    timeframe: &Timeframe,
    start: Timestamp,
    end: Timestamp,
) -> anyhow::Result<usize> {
    let step = timeframe.step()?;
    let mut cursor = start;
    let mut count = 0usize;
    while cursor < end {
        count = count.saturating_add(1);
        anyhow::ensure!(
            count <= MAX_CANDLE_LIMIT,
            "range contains more than {MAX_CANDLE_LIMIT} expected candles"
        );
        cursor = cursor
            .checked_add(step)
            .map_err(|err| anyhow::anyhow!("timeframe addition overflowed: {err}"))?;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rara_kernel::{
        io::MessageId,
        queue::{ShardedEventQueue, ShardedEventQueueConfig},
        session::SessionKey,
        tool::{AgentTool, ToolContext, ToolExecute},
    };
    use rust_decimal::Decimal;

    use super::{
        FinanceFindCandleGapsParams, FinanceFindCandleGapsTool, FinanceGetCandleFreshnessParams,
        FinanceGetCandleFreshnessTool, FinanceGetLatestCandleParams, FinanceGetLatestCandleTool,
        FinanceListCandleStreamsParams, FinanceListCandleStreamsTool, FinanceQueryCandlesParams,
        FinanceQueryCandlesTool,
    };
    use crate::market_data::{
        InMemoryMarketDataRepository, MarketCandle, MarketDataRepository, Timeframe,
    };

    fn context() -> ToolContext {
        ToolContext {
            user_id:               "alice".to_owned(),
            session_key:           SessionKey::new(),
            origin_endpoint:       None,
            origin_user_id:        None,
            event_queue:           Arc::new(ShardedEventQueue::new(ShardedEventQueueConfig {
                num_shards:      0,
                shard_capacity:  1,
                global_capacity: 16,
            })),
            rara_turn_id:          MessageId::new(),
            context_window_tokens: 0,
            tool_registry:         None,
            stream_handle:         None,
            tool_call_id:          None,
        }
    }

    fn ts(value: &str) -> jiff::Timestamp { value.parse().expect("timestamp fixture should parse") }

    fn dec(value: &str) -> Decimal { value.parse().expect("decimal fixture should parse") }

    fn candle(open_time: &str, close: &str) -> MarketCandle {
        MarketCandle {
            source_name:       "binance-spot".to_owned(),
            venue:             "binance".to_owned(),
            symbol:            "BTCUSDT".to_owned(),
            timeframe:         Timeframe::parse("1m").expect("timeframe fixture should parse"),
            open_time:         ts(open_time),
            close_time:        ts("2026-07-10T08:01:00Z"),
            open:              dec("61500.12"),
            high:              dec("61640.00"),
            low:               dec("61480.50"),
            close:             dec(close),
            volume:            dec("124.551"),
            ingested_at:       ts("2026-07-10T08:01:01Z"),
            provider_sequence: Some(open_time.to_owned()),
        }
    }

    async fn repository() -> Arc<InMemoryMarketDataRepository> {
        let repository = Arc::new(InMemoryMarketDataRepository::default());
        repository
            .upsert_closed_candle(candle("2026-07-10T08:00:00Z", "61510.00"))
            .await
            .unwrap();
        repository
            .upsert_closed_candle(candle("2026-07-10T08:01:00Z", "61520.00"))
            .await
            .unwrap();
        repository
    }

    async fn repository_with_gap() -> Arc<InMemoryMarketDataRepository> {
        let repository = Arc::new(InMemoryMarketDataRepository::default());
        repository
            .upsert_closed_candle(candle("2026-07-10T08:00:00Z", "61510.00"))
            .await
            .unwrap();
        repository
            .upsert_closed_candle(candle("2026-07-10T08:02:00Z", "61530.00"))
            .await
            .unwrap();
        repository
    }

    async fn repository_with_multiple_streams() -> Arc<InMemoryMarketDataRepository> {
        let repository = repository().await;
        repository
            .upsert_closed_candle(MarketCandle {
                symbol: "ETHUSDT".to_owned(),
                open_time: ts("2026-07-10T08:03:00Z"),
                close_time: ts("2026-07-10T08:04:00Z"),
                close: dec("3200.00"),
                provider_sequence: Some("eth".to_owned()),
                ..candle("2026-07-10T08:03:00Z", "3200.00")
            })
            .await
            .unwrap();
        repository
    }

    #[tokio::test]
    async fn list_candle_streams_tool_returns_available_stream_watermarks() {
        let repository = repository_with_multiple_streams().await;
        let tool = FinanceListCandleStreamsTool::new(repository);
        let result = tool
            .run(
                FinanceListCandleStreamsParams {
                    source_name: Some("binance-spot".to_owned()),
                    venue:       Some("binance".to_owned()),
                    symbol:      None,
                    timeframe:   Some("1m".to_owned()),
                    limit:       Some(10),
                },
                &context(),
            )
            .await
            .unwrap();

        assert_eq!(result.count, 2);
        assert_eq!(result.query_limit, 10);
        assert_eq!(result.streams[0].symbol, "ETHUSDT");
        assert_eq!(result.streams[0].candle_count, 1);
        assert_eq!(result.streams[0].latest_open_time, "2026-07-10T08:03:00Z");
        assert_eq!(result.streams[1].symbol, "BTCUSDT");
        assert_eq!(result.streams[1].candle_count, 2);
        assert_eq!(result.streams[1].first_open_time, "2026-07-10T08:00:00Z");
    }

    #[tokio::test]
    async fn list_candle_streams_tool_filters_by_symbol() {
        let repository = repository_with_multiple_streams().await;
        let tool = FinanceListCandleStreamsTool::new(repository);
        let result = tool
            .run(
                FinanceListCandleStreamsParams {
                    source_name: None,
                    venue:       Some("binance".to_owned()),
                    symbol:      Some("BTCUSDT".to_owned()),
                    timeframe:   Some("1m".to_owned()),
                    limit:       None,
                },
                &context(),
            )
            .await
            .unwrap();

        assert_eq!(result.count, 1);
        assert_eq!(result.streams[0].symbol, "BTCUSDT");
        assert_eq!(result.streams[0].candle_count, 2);
    }

    #[tokio::test]
    async fn list_candle_streams_tool_reports_when_more_streams_match() {
        let repository = repository_with_multiple_streams().await;
        let tool = FinanceListCandleStreamsTool::new(repository);
        let result = tool
            .run(
                FinanceListCandleStreamsParams {
                    source_name: Some("binance-spot".to_owned()),
                    venue:       Some("binance".to_owned()),
                    symbol:      None,
                    timeframe:   Some("1m".to_owned()),
                    limit:       Some(1),
                },
                &context(),
            )
            .await
            .unwrap();

        assert_eq!(result.count, 1);
        assert_eq!(result.query_limit, 1);
        assert!(result.has_more);
        assert_eq!(result.streams[0].symbol, "ETHUSDT");
    }

    #[tokio::test]
    async fn list_candle_streams_tool_rejects_unprobeable_limit() {
        let repository = repository_with_multiple_streams().await;
        let tool = FinanceListCandleStreamsTool::new(repository);
        let error = tool
            .run(
                FinanceListCandleStreamsParams {
                    source_name: None,
                    venue:       None,
                    symbol:      None,
                    timeframe:   None,
                    limit:       Some(10_000),
                },
                &context(),
            )
            .await
            .unwrap_err();

        assert!(
            error.to_string().contains("limit must be <= 9999"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn list_candle_streams_tool_normalizes_user_selectors() {
        let repository = repository_with_multiple_streams().await;
        let tool = FinanceListCandleStreamsTool::new(repository);
        let result = tool
            .run(
                FinanceListCandleStreamsParams {
                    source_name: Some(" binance-spot ".to_owned()),
                    venue:       Some(" Binance ".to_owned()),
                    symbol:      Some(" btcusdt ".to_owned()),
                    timeframe:   Some(" 1M ".to_owned()),
                    limit:       None,
                },
                &context(),
            )
            .await
            .unwrap();

        assert_eq!(result.count, 1);
        assert_eq!(result.streams[0].venue, "binance");
        assert_eq!(result.streams[0].symbol, "BTCUSDT");
        assert_eq!(result.streams[0].timeframe, "1m");
        assert_eq!(result.streams[0].candle_count, 2);
    }

    #[tokio::test]
    async fn latest_candle_tool_returns_newest_candle_as_strings() {
        let repository = repository().await;
        let tool = FinanceGetLatestCandleTool::new(repository);
        let result = tool
            .run(
                FinanceGetLatestCandleParams {
                    source_name: Some("binance-spot".to_owned()),
                    venue:       "binance".to_owned(),
                    symbol:      "BTCUSDT".to_owned(),
                    timeframe:   "1m".to_owned(),
                },
                &context(),
            )
            .await
            .unwrap();

        let candle = result.candle.expect("latest candle should exist");
        assert_eq!(candle.open_time, "2026-07-10T08:01:00Z");
        assert_eq!(candle.close, "61520.00");
        assert_eq!(candle.volume, "124.551");
    }

    #[tokio::test]
    async fn latest_candle_tool_normalizes_user_selectors() {
        let repository = repository().await;
        let tool = FinanceGetLatestCandleTool::new(repository);
        let result = tool
            .run(
                FinanceGetLatestCandleParams {
                    source_name: Some(" binance-spot ".to_owned()),
                    venue:       " Binance ".to_owned(),
                    symbol:      " btcusdt ".to_owned(),
                    timeframe:   " 1M ".to_owned(),
                },
                &context(),
            )
            .await
            .unwrap();

        let candle = result.candle.expect("latest candle should exist");
        assert_eq!(candle.venue, "binance");
        assert_eq!(candle.symbol, "BTCUSDT");
        assert_eq!(candle.timeframe, "1m");
        assert_eq!(candle.open_time, "2026-07-10T08:01:00Z");
    }

    #[tokio::test]
    async fn query_candles_tool_returns_ordered_range() {
        let repository = repository().await;
        let tool = FinanceQueryCandlesTool::new(repository);
        let result = tool
            .run(
                FinanceQueryCandlesParams {
                    source_name: Some("binance-spot".to_owned()),
                    venue:       "binance".to_owned(),
                    symbol:      "BTCUSDT".to_owned(),
                    timeframe:   "1m".to_owned(),
                    start:       "2026-07-10T08:00:00Z".to_owned(),
                    end:         "2026-07-10T08:02:00Z".to_owned(),
                    limit:       Some(10),
                },
                &context(),
            )
            .await
            .unwrap();

        assert_eq!(result.count, 2);
        assert_eq!(result.query_limit, 10);
        assert_eq!(
            result
                .candles
                .iter()
                .map(|candle| candle.open_time.as_str())
                .collect::<Vec<_>>(),
            vec!["2026-07-10T08:00:00Z", "2026-07-10T08:01:00Z"]
        );
    }

    #[tokio::test]
    async fn candle_quality_tools_normalize_user_selectors() {
        let repository = repository_with_gap().await;
        let query = FinanceQueryCandlesTool::new(repository.clone());
        let gaps = FinanceFindCandleGapsTool::new(repository.clone());
        let freshness = FinanceGetCandleFreshnessTool::new(repository);

        let query_result = query
            .run(
                FinanceQueryCandlesParams {
                    source_name: Some(" binance-spot ".to_owned()),
                    venue:       " Binance ".to_owned(),
                    symbol:      " btcusdt ".to_owned(),
                    timeframe:   " 1M ".to_owned(),
                    start:       "2026-07-10T08:00:00Z".to_owned(),
                    end:         "2026-07-10T08:03:00Z".to_owned(),
                    limit:       Some(10),
                },
                &context(),
            )
            .await
            .unwrap();
        assert_eq!(query_result.count, 2);

        let gap_result = gaps
            .run(
                FinanceFindCandleGapsParams {
                    source_name: Some(" binance-spot ".to_owned()),
                    venue:       " Binance ".to_owned(),
                    symbol:      " btcusdt ".to_owned(),
                    timeframe:   " 1M ".to_owned(),
                    start:       "2026-07-10T08:00:00Z".to_owned(),
                    end:         "2026-07-10T08:03:00Z".to_owned(),
                },
                &context(),
            )
            .await
            .unwrap();
        assert_eq!(gap_result.missing_open_times, vec!["2026-07-10T08:01:00Z"]);

        let freshness_result = freshness
            .run(
                FinanceGetCandleFreshnessParams {
                    source_name:      Some(" binance-spot ".to_owned()),
                    venue:            " Binance ".to_owned(),
                    symbol:           " btcusdt ".to_owned(),
                    timeframe:        " 1M ".to_owned(),
                    as_of:            Some("2026-07-10T08:03:00Z".to_owned()),
                    stale_after_secs: Some(120),
                },
                &context(),
            )
            .await
            .unwrap();
        assert_eq!(freshness_result.status, "fresh");
        assert_eq!(freshness_result.lag_secs, Some(120));
    }

    #[tokio::test]
    async fn query_candles_tool_rejects_invalid_ranges() {
        let tool = FinanceQueryCandlesTool::new(repository().await);
        let err = tool
            .run(
                FinanceQueryCandlesParams {
                    source_name: None,
                    venue:       "binance".to_owned(),
                    symbol:      "BTCUSDT".to_owned(),
                    timeframe:   "1m".to_owned(),
                    start:       "2026-07-10T08:02:00Z".to_owned(),
                    end:         "2026-07-10T08:00:00Z".to_owned(),
                    limit:       Some(10),
                },
                &context(),
            )
            .await
            .unwrap_err();

        assert!(err.to_string().contains("start must be before end"));
    }

    #[tokio::test]
    async fn find_candle_gaps_tool_reports_missing_open_times() {
        let tool = FinanceFindCandleGapsTool::new(repository_with_gap().await);
        let result = tool
            .run(
                FinanceFindCandleGapsParams {
                    source_name: Some("binance-spot".to_owned()),
                    venue:       "binance".to_owned(),
                    symbol:      "BTCUSDT".to_owned(),
                    timeframe:   "1m".to_owned(),
                    start:       "2026-07-10T08:00:00Z".to_owned(),
                    end:         "2026-07-10T08:03:00Z".to_owned(),
                },
                &context(),
            )
            .await
            .unwrap();

        assert_eq!(result.expected_count, 3);
        assert_eq!(result.missing_count, 1);
        assert!(!result.complete);
        assert_eq!(result.missing_open_times, vec!["2026-07-10T08:01:00Z"]);
    }

    #[tokio::test]
    async fn freshness_tool_reports_fresh_and_stale_status() {
        let repository = repository().await;
        let tool = FinanceGetCandleFreshnessTool::new(repository);

        let fresh = tool
            .run(
                FinanceGetCandleFreshnessParams {
                    source_name:      Some("binance-spot".to_owned()),
                    venue:            "binance".to_owned(),
                    symbol:           "BTCUSDT".to_owned(),
                    timeframe:        "1m".to_owned(),
                    as_of:            Some("2026-07-10T08:02:30Z".to_owned()),
                    stale_after_secs: Some(120),
                },
                &context(),
            )
            .await
            .unwrap();
        assert_eq!(fresh.status, "fresh");
        assert!(!fresh.is_stale);
        assert_eq!(fresh.lag_secs, Some(90));

        let stale = tool
            .run(
                FinanceGetCandleFreshnessParams {
                    source_name:      Some("binance-spot".to_owned()),
                    venue:            "binance".to_owned(),
                    symbol:           "BTCUSDT".to_owned(),
                    timeframe:        "1m".to_owned(),
                    as_of:            Some("2026-07-10T08:10:00Z".to_owned()),
                    stale_after_secs: Some(120),
                },
                &context(),
            )
            .await
            .unwrap();
        assert_eq!(stale.status, "stale");
        assert!(stale.is_stale);
        assert_eq!(stale.lag_secs, Some(540));
    }

    #[test]
    fn candle_query_tools_are_read_only() {
        let repository = Arc::new(InMemoryMarketDataRepository::default());
        let streams = FinanceListCandleStreamsTool::new(repository.clone());
        let latest = FinanceGetLatestCandleTool::new(repository.clone());
        let query = FinanceQueryCandlesTool::new(repository.clone());
        let gaps = FinanceFindCandleGapsTool::new(repository.clone());
        let freshness = FinanceGetCandleFreshnessTool::new(repository);

        assert!(streams.is_read_only(&serde_json::json!({})));
        assert!(latest.is_read_only(&serde_json::json!({})));
        assert!(query.is_read_only(&serde_json::json!({})));
        assert!(gaps.is_read_only(&serde_json::json!({})));
        assert!(freshness.is_read_only(&serde_json::json!({})));
    }
}
