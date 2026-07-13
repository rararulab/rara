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
    CandleLatestQuery, CandleRangeQuery, CandleRecentQuery, CandleStreamListQuery,
    CandleStreamSummary, MarketCandle, MarketDataRepositoryRef, Timeframe,
};

const DEFAULT_CANDLE_LIMIT: usize = 500;
const DEFAULT_CANDLE_HINT_LIMIT: usize = 20;
const MAX_CANDLE_LIMIT: usize = 10_000;
const MAX_CANDLE_RANGE_LIMIT: usize = MAX_CANDLE_LIMIT - 1;
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
    pub selector: FinanceCandleSelector,
    pub candle:   Option<FinanceCandle>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FinanceGetRecentCandlesParams {
    #[serde(default)]
    pub source_name: Option<String>,
    pub venue:       String,
    pub symbol:      String,
    pub timeframe:   String,
    #[serde(default)]
    pub limit:       Option<usize>,
    /// Exclusive candle open-time upper bound for paging older candles.
    #[serde(default)]
    pub end:         Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FinanceGetRecentCandlesResult {
    pub selector:       FinanceCandleSelector,
    pub candles:        Vec<FinanceCandle>,
    pub count:          usize,
    pub query_limit:    usize,
    pub has_more:       bool,
    pub next_end:       Option<String>,
    pub next_page_hint: Option<FinanceCandleNextPageHint>,
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
    pub selector:       FinanceCandleSelector,
    pub candles:        Vec<FinanceCandle>,
    pub count:          usize,
    pub query_limit:    usize,
    pub has_more:       bool,
    pub next_start:     Option<String>,
    pub next_page_hint: Option<FinanceCandleNextPageHint>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FinanceCandleNextPageHint {
    pub tool:            String,
    pub default_params:  serde_json::Value,
    pub required_params: Vec<String>,
    pub optional_params: Vec<String>,
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
    pub selector:           FinanceCandleSelector,
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
    pub selector:         FinanceCandleSelector,
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
    #[serde(default)]
    pub offset:      Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FinanceListCandleStreamsResult {
    pub streams:        Vec<FinanceCandleStream>,
    pub filters:        FinanceCandleStreamFilters,
    pub count:          usize,
    pub query_limit:    usize,
    pub query_offset:   usize,
    pub has_more:       bool,
    pub next_page_hint: Option<FinanceCandleStreamNextPageHint>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FinanceCandleStream {
    pub source_name:         String,
    pub venue:               String,
    pub symbol:              String,
    pub timeframe:           String,
    pub candle_count:        usize,
    pub first_open_time:     String,
    pub latest_open_time:    String,
    pub latest_close_time:   String,
    pub latest_ingested_at:  String,
    pub latest_candle_hint:  FinanceCandleStreamToolHint,
    pub recent_candles_hint: FinanceCandleStreamToolHint,
    pub freshness_hint:      FinanceCandleStreamToolHint,
    pub gaps_hint:           FinanceCandleStreamToolHint,
    pub query_candles_hint:  FinanceCandleStreamToolHint,
}

#[derive(Debug, Clone, Serialize)]
pub struct FinanceCandleStreamToolHint {
    pub tool:            String,
    pub default_params:  serde_json::Value,
    pub required_params: Vec<String>,
    pub optional_params: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FinanceCandleStreamNextPageHint {
    pub tool:            String,
    pub default_params:  serde_json::Value,
    pub required_params: Vec<String>,
    pub optional_params: Vec<String>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FinanceCandleSelector {
    pub source_name: Option<String>,
    pub venue:       String,
    pub symbol:      String,
    pub timeframe:   String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FinanceCandleStreamFilters {
    pub source_name: Option<String>,
    pub venue:       Option<String>,
    pub symbol:      Option<String>,
    pub timeframe:   Option<String>,
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
        let offset = params.offset.unwrap_or(0);
        let query_limit = limit.saturating_add(1);
        let source_name = normalize_optional_source_name_selector(params.source_name)?;
        let venue = normalize_optional_venue_selector(params.venue)?;
        let symbol = normalize_optional_symbol_selector(params.symbol)?;
        let timeframe = normalize_optional_timeframe_selector(params.timeframe)?;
        let filters = FinanceCandleStreamFilters {
            source_name: source_name.clone(),
            venue:       venue.clone(),
            symbol:      symbol.clone(),
            timeframe:   timeframe.clone().map(|timeframe| timeframe.to_string()),
        };
        let query = CandleStreamListQuery {
            source_name: source_name.clone(),
            venue: venue.clone(),
            symbol: symbol.clone(),
            timeframe: timeframe.clone(),
            limit: query_limit,
            offset,
        };
        let mut streams = self.repository.candle_streams(query).await?;
        let has_more = streams.len() > limit;
        streams.truncate(limit);
        let count = streams.len();
        let next_page_hint = stream_list_next_page_hint(
            source_name.as_deref(),
            venue.as_deref(),
            symbol.as_deref(),
            timeframe.as_ref(),
            limit,
            offset,
            has_more,
        );
        Ok(FinanceListCandleStreamsResult {
            streams: streams.into_iter().map(FinanceCandleStream::from).collect(),
            filters,
            count,
            query_limit: limit,
            query_offset: offset,
            has_more,
            next_page_hint,
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
        let source_name = normalize_optional_source_name_selector(params.source_name)?;
        let venue = normalize_required_venue_selector(params.venue)?;
        let symbol = normalize_required_symbol_selector(params.symbol)?;
        let timeframe = normalize_required_timeframe_selector(params.timeframe)?;
        let selector = candle_selector(
            source_name.clone(),
            venue.clone(),
            symbol.clone(),
            &timeframe,
        );
        let query = CandleLatestQuery {
            source_name,
            venue,
            symbol,
            timeframe,
        };
        let candle = self.repository.latest_closed_candle(query).await?;
        Ok(FinanceGetLatestCandleResult {
            selector,
            candle: candle.map(FinanceCandle::from),
        })
    }
}

#[derive(ToolDef)]
#[tool(
    name = "finance_get_recent_candles",
    description = "Return the newest closed OHLCV candles stored in the market-data TSDB for a \
                   venue, symbol, and timeframe, ordered from oldest to newest. Decimal values \
                   are returned as strings to preserve financial precision. This is read-only and \
                   never places trades.",
    tier = "deferred",
    read_only,
    concurrency_safe
)]
pub struct FinanceGetRecentCandlesTool {
    repository: MarketDataRepositoryRef,
}

impl FinanceGetRecentCandlesTool {
    pub fn new(repository: MarketDataRepositoryRef) -> Self { Self { repository } }
}

#[async_trait]
impl ToolExecute for FinanceGetRecentCandlesTool {
    type Output = FinanceGetRecentCandlesResult;
    type Params = FinanceGetRecentCandlesParams;

    async fn run(
        &self,
        params: FinanceGetRecentCandlesParams,
        _context: &ToolContext,
    ) -> anyhow::Result<FinanceGetRecentCandlesResult> {
        let limit = validate_candle_range_limit(params.limit)?;
        let probe_limit = limit.saturating_add(1);
        let source_name = normalize_optional_source_name_selector(params.source_name)?;
        let venue = normalize_required_venue_selector(params.venue)?;
        let symbol = normalize_required_symbol_selector(params.symbol)?;
        let timeframe = normalize_required_timeframe_selector(params.timeframe)?;
        let selector = candle_selector(
            source_name.clone(),
            venue.clone(),
            symbol.clone(),
            &timeframe,
        );
        let end = params
            .end
            .as_deref()
            .map(|value| parse_timestamp("end", value))
            .transpose()?;
        let query = CandleRecentQuery {
            source_name: source_name.clone(),
            venue: venue.clone(),
            symbol: symbol.clone(),
            timeframe: timeframe.clone(),
            limit: probe_limit,
            end,
        };
        let mut candles = self.repository.recent_candles(query).await?;
        let has_more = candles.len() > limit;
        if has_more {
            candles.remove(0);
        }
        let next_end = candles
            .first()
            .filter(|_| has_more)
            .map(|candle| candle.open_time.to_string());
        let count = candles.len();
        let next_page_hint = recent_candles_next_page_hint(
            source_name.as_deref(),
            &venue,
            &symbol,
            &timeframe,
            next_end.as_deref(),
            limit,
        );
        Ok(FinanceGetRecentCandlesResult {
            selector,
            candles: candles.into_iter().map(FinanceCandle::from).collect(),
            count,
            query_limit: limit,
            has_more,
            next_end,
            next_page_hint,
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
        let limit = validate_candle_range_limit(params.limit)?;
        let probe_limit = limit.saturating_add(1);
        let start = parse_timestamp("start", &params.start)?;
        let end = parse_timestamp("end", &params.end)?;
        anyhow::ensure!(start < end, "start must be before end");
        let source_name = normalize_optional_source_name_selector(params.source_name)?;
        let venue = normalize_required_venue_selector(params.venue)?;
        let symbol = normalize_required_symbol_selector(params.symbol)?;
        let timeframe = normalize_required_timeframe_selector(params.timeframe)?;
        let selector = candle_selector(
            source_name.clone(),
            venue.clone(),
            symbol.clone(),
            &timeframe,
        );

        let query = CandleRangeQuery {
            source_name: source_name.clone(),
            venue: venue.clone(),
            symbol: symbol.clone(),
            timeframe: timeframe.clone(),
            start,
            end,
            limit: probe_limit,
        };
        let mut candles = self.repository.candles(query).await?;
        let has_more = candles.len() > limit;
        let next_start = candles
            .get(limit)
            .map(|candle| candle.open_time.to_string());
        candles.truncate(limit);
        let count = candles.len();
        let next_page_hint = range_candles_next_page_hint(
            source_name.as_deref(),
            &venue,
            &symbol,
            &timeframe,
            next_start.as_deref(),
            &end.to_string(),
            limit,
        );
        Ok(FinanceQueryCandlesResult {
            selector,
            candles: candles.into_iter().map(FinanceCandle::from).collect(),
            count,
            query_limit: limit,
            has_more,
            next_start,
            next_page_hint,
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
        let source_name = normalize_optional_source_name_selector(params.source_name)?;
        let venue = normalize_required_venue_selector(params.venue)?;
        let symbol = normalize_required_symbol_selector(params.symbol)?;
        let selector = candle_selector(
            source_name.clone(),
            venue.clone(),
            symbol.clone(),
            &timeframe,
        );

        let query = CandleRangeQuery {
            source_name,
            venue,
            symbol,
            timeframe,
            start,
            end,
            limit: expected_count,
        };
        let missing = self.repository.missing_open_times(query).await?;
        let missing_count = missing.len();

        Ok(FinanceFindCandleGapsResult {
            selector,
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
        let source_name = normalize_optional_source_name_selector(params.source_name)?;
        let venue = normalize_required_venue_selector(params.venue)?;
        let symbol = normalize_required_symbol_selector(params.symbol)?;
        let selector = candle_selector(
            source_name.clone(),
            venue.clone(),
            symbol.clone(),
            &timeframe,
        );

        let query = CandleLatestQuery {
            source_name,
            venue,
            symbol,
            timeframe,
        };
        let latest = self.repository.latest_closed_candle(query).await?;
        let Some(candle) = latest else {
            return Ok(FinanceGetCandleFreshnessResult {
                selector,
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
            selector,
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
        let source_name = summary.source_name;
        let venue = summary.venue;
        let symbol = summary.symbol;
        let timeframe = summary.timeframe.to_string();
        Self {
            latest_candle_hint: latest_candle_hint_for_stream(
                &source_name,
                &venue,
                &symbol,
                &timeframe,
            ),
            recent_candles_hint: recent_candles_hint_for_stream(
                &source_name,
                &venue,
                &symbol,
                &timeframe,
            ),
            freshness_hint: freshness_hint_for_stream(&source_name, &venue, &symbol, &timeframe),
            gaps_hint: gaps_hint_for_stream(&source_name, &venue, &symbol, &timeframe),
            query_candles_hint: query_candles_hint_for_stream(
                &source_name,
                &venue,
                &symbol,
                &timeframe,
            ),
            source_name,
            venue,
            symbol,
            timeframe,
            candle_count: summary.candle_count,
            first_open_time: summary.first_open_time.to_string(),
            latest_open_time: summary.latest_open_time.to_string(),
            latest_close_time: summary.latest_close_time.to_string(),
            latest_ingested_at: summary.latest_ingested_at.to_string(),
        }
    }
}

fn latest_candle_hint_for_stream(
    source_name: &str,
    venue: &str,
    symbol: &str,
    timeframe: &str,
) -> FinanceCandleStreamToolHint {
    FinanceCandleStreamToolHint {
        tool:            "finance_get_latest_candle".to_owned(),
        default_params:  stream_default_params(source_name, venue, symbol, timeframe),
        required_params: Vec::new(),
        optional_params: Vec::new(),
    }
}

fn recent_candles_hint_for_stream(
    source_name: &str,
    venue: &str,
    symbol: &str,
    timeframe: &str,
) -> FinanceCandleStreamToolHint {
    let mut default_params = stream_default_param_map(source_name, venue, symbol, timeframe);
    default_params.insert(
        "limit".to_owned(),
        serde_json::Value::Number(DEFAULT_CANDLE_HINT_LIMIT.into()),
    );

    FinanceCandleStreamToolHint {
        tool:            "finance_get_recent_candles".to_owned(),
        default_params:  serde_json::Value::Object(default_params),
        required_params: Vec::new(),
        optional_params: ["end", "limit"].into_iter().map(str::to_owned).collect(),
    }
}

fn freshness_hint_for_stream(
    source_name: &str,
    venue: &str,
    symbol: &str,
    timeframe: &str,
) -> FinanceCandleStreamToolHint {
    FinanceCandleStreamToolHint {
        tool:            "finance_get_candle_freshness".to_owned(),
        default_params:  stream_default_params(source_name, venue, symbol, timeframe),
        required_params: Vec::new(),
        optional_params: ["as_of", "stale_after_secs"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
    }
}

fn gaps_hint_for_stream(
    source_name: &str,
    venue: &str,
    symbol: &str,
    timeframe: &str,
) -> FinanceCandleStreamToolHint {
    FinanceCandleStreamToolHint {
        tool:            "finance_find_candle_gaps".to_owned(),
        default_params:  stream_default_params(source_name, venue, symbol, timeframe),
        required_params: ["start", "end"].into_iter().map(str::to_owned).collect(),
        optional_params: Vec::new(),
    }
}

fn query_candles_hint_for_stream(
    source_name: &str,
    venue: &str,
    symbol: &str,
    timeframe: &str,
) -> FinanceCandleStreamToolHint {
    let mut default_params = stream_default_param_map(source_name, venue, symbol, timeframe);
    default_params.insert(
        "limit".to_owned(),
        serde_json::Value::Number(DEFAULT_CANDLE_HINT_LIMIT.into()),
    );

    FinanceCandleStreamToolHint {
        tool:            "finance_query_candles".to_owned(),
        default_params:  serde_json::Value::Object(default_params),
        required_params: ["start", "end"].into_iter().map(str::to_owned).collect(),
        optional_params: vec!["limit".to_owned()],
    }
}

fn stream_list_next_page_hint(
    source_name: Option<&str>,
    venue: Option<&str>,
    symbol: Option<&str>,
    timeframe: Option<&Timeframe>,
    limit: usize,
    offset: usize,
    has_more: bool,
) -> Option<FinanceCandleStreamNextPageHint> {
    if !has_more {
        return None;
    }

    let mut default_params = serde_json::Map::new();
    if let Some(source_name) = source_name {
        default_params.insert(
            "source_name".to_owned(),
            serde_json::Value::String(source_name.to_owned()),
        );
    }
    if let Some(venue) = venue {
        default_params.insert(
            "venue".to_owned(),
            serde_json::Value::String(venue.to_owned()),
        );
    }
    if let Some(symbol) = symbol {
        default_params.insert(
            "symbol".to_owned(),
            serde_json::Value::String(symbol.to_owned()),
        );
    }
    if let Some(timeframe) = timeframe {
        default_params.insert(
            "timeframe".to_owned(),
            serde_json::Value::String(timeframe.to_string()),
        );
    }
    default_params.insert("limit".to_owned(), serde_json::json!(limit));
    default_params.insert(
        "offset".to_owned(),
        serde_json::json!(offset.saturating_add(limit)),
    );

    Some(FinanceCandleStreamNextPageHint {
        tool:            "finance_list_candle_streams".to_owned(),
        default_params:  serde_json::Value::Object(default_params),
        required_params: Vec::new(),
        optional_params: [
            "source_name",
            "venue",
            "symbol",
            "timeframe",
            "limit",
            "offset",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
    })
}

fn recent_candles_next_page_hint(
    source_name: Option<&str>,
    venue: &str,
    symbol: &str,
    timeframe: &Timeframe,
    next_end: Option<&str>,
    limit: usize,
) -> Option<FinanceCandleNextPageHint> {
    let next_end = next_end?;
    let mut default_params = candle_page_default_param_map(source_name, venue, symbol, timeframe);
    default_params.insert(
        "end".to_owned(),
        serde_json::Value::String(next_end.to_owned()),
    );
    default_params.insert("limit".to_owned(), serde_json::json!(limit));

    Some(FinanceCandleNextPageHint {
        tool:            "finance_get_recent_candles".to_owned(),
        default_params:  serde_json::Value::Object(default_params),
        required_params: Vec::new(),
        optional_params: ["end", "limit"].into_iter().map(str::to_owned).collect(),
    })
}

fn range_candles_next_page_hint(
    source_name: Option<&str>,
    venue: &str,
    symbol: &str,
    timeframe: &Timeframe,
    next_start: Option<&str>,
    end: &str,
    limit: usize,
) -> Option<FinanceCandleNextPageHint> {
    let next_start = next_start?;
    let mut default_params = candle_page_default_param_map(source_name, venue, symbol, timeframe);
    default_params.insert(
        "start".to_owned(),
        serde_json::Value::String(next_start.to_owned()),
    );
    default_params.insert("end".to_owned(), serde_json::Value::String(end.to_owned()));
    default_params.insert("limit".to_owned(), serde_json::json!(limit));

    Some(FinanceCandleNextPageHint {
        tool:            "finance_query_candles".to_owned(),
        default_params:  serde_json::Value::Object(default_params),
        required_params: Vec::new(),
        optional_params: ["start", "end", "limit"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
    })
}

fn candle_page_default_param_map(
    source_name: Option<&str>,
    venue: &str,
    symbol: &str,
    timeframe: &Timeframe,
) -> serde_json::Map<String, serde_json::Value> {
    let mut default_params = serde_json::Map::from_iter([
        (
            "venue".to_owned(),
            serde_json::Value::String(venue.to_owned()),
        ),
        (
            "symbol".to_owned(),
            serde_json::Value::String(symbol.to_owned()),
        ),
        (
            "timeframe".to_owned(),
            serde_json::Value::String(timeframe.to_string()),
        ),
    ]);
    if let Some(source_name) = source_name {
        default_params.insert(
            "source_name".to_owned(),
            serde_json::Value::String(source_name.to_owned()),
        );
    }
    default_params
}

fn stream_default_params(
    source_name: &str,
    venue: &str,
    symbol: &str,
    timeframe: &str,
) -> serde_json::Value {
    serde_json::Value::Object(stream_default_param_map(
        source_name,
        venue,
        symbol,
        timeframe,
    ))
}

fn stream_default_param_map(
    source_name: &str,
    venue: &str,
    symbol: &str,
    timeframe: &str,
) -> serde_json::Map<String, serde_json::Value> {
    serde_json::Map::from_iter([
        (
            "source_name".to_owned(),
            serde_json::Value::String(source_name.to_owned()),
        ),
        (
            "venue".to_owned(),
            serde_json::Value::String(venue.to_owned()),
        ),
        (
            "symbol".to_owned(),
            serde_json::Value::String(symbol.to_owned()),
        ),
        (
            "timeframe".to_owned(),
            serde_json::Value::String(timeframe.to_owned()),
        ),
    ])
}

fn candle_selector(
    source_name: Option<String>,
    venue: String,
    symbol: String,
    timeframe: &Timeframe,
) -> FinanceCandleSelector {
    FinanceCandleSelector {
        source_name,
        venue,
        symbol,
        timeframe: timeframe.to_string(),
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

fn validate_candle_range_limit(limit: Option<usize>) -> anyhow::Result<usize> {
    let limit = limit.unwrap_or(DEFAULT_CANDLE_LIMIT);
    anyhow::ensure!(limit > 0, "limit must be positive");
    anyhow::ensure!(
        limit <= MAX_CANDLE_RANGE_LIMIT,
        "limit must be <= {MAX_CANDLE_RANGE_LIMIT}"
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
        FinanceCandleSelector, FinanceCandleStreamFilters, FinanceFindCandleGapsParams,
        FinanceFindCandleGapsTool, FinanceGetCandleFreshnessParams, FinanceGetCandleFreshnessTool,
        FinanceGetLatestCandleParams, FinanceGetLatestCandleTool, FinanceGetRecentCandlesParams,
        FinanceGetRecentCandlesTool, FinanceListCandleStreamsParams, FinanceListCandleStreamsTool,
        FinanceQueryCandlesParams, FinanceQueryCandlesTool,
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

    fn normalized_selector(source_name: Option<&str>) -> FinanceCandleSelector {
        FinanceCandleSelector {
            source_name: source_name.map(str::to_owned),
            venue:       "binance".to_owned(),
            symbol:      "BTCUSDT".to_owned(),
            timeframe:   "1m".to_owned(),
        }
    }

    fn normalized_stream_filters(
        source_name: Option<&str>,
        venue: Option<&str>,
        symbol: Option<&str>,
        timeframe: Option<&str>,
    ) -> FinanceCandleStreamFilters {
        FinanceCandleStreamFilters {
            source_name: source_name.map(str::to_owned),
            venue:       venue.map(str::to_owned),
            symbol:      symbol.map(str::to_owned),
            timeframe:   timeframe.map(str::to_owned),
        }
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
                    offset:      None,
                },
                &context(),
            )
            .await
            .unwrap();

        assert_eq!(result.count, 2);
        assert_eq!(
            result.filters,
            normalized_stream_filters(Some("binance-spot"), Some("binance"), None, Some("1m"))
        );
        assert_eq!(result.query_limit, 10);
        assert_eq!(result.streams[0].symbol, "ETHUSDT");
        assert_eq!(result.streams[0].candle_count, 1);
        assert_eq!(result.streams[0].latest_open_time, "2026-07-10T08:03:00Z");
        assert_eq!(
            result.streams[0].latest_candle_hint.default_params,
            serde_json::json!({
                "source_name": "binance-spot",
                "venue": "binance",
                "symbol": "ETHUSDT",
                "timeframe": "1m",
            })
        );
        assert_eq!(
            result.streams[0].recent_candles_hint.default_params,
            serde_json::json!({
                "source_name": "binance-spot",
                "venue": "binance",
                "symbol": "ETHUSDT",
                "timeframe": "1m",
                "limit": 20,
            })
        );
        assert_eq!(
            result.streams[0].recent_candles_hint.optional_params,
            ["end", "limit"]
        );
        assert_eq!(
            result.streams[0].freshness_hint.default_params,
            serde_json::json!({
                "source_name": "binance-spot",
                "venue": "binance",
                "symbol": "ETHUSDT",
                "timeframe": "1m",
            })
        );
        assert_eq!(
            result.streams[0].gaps_hint.required_params,
            ["start", "end"]
        );
        assert_eq!(
            result.streams[0].query_candles_hint.default_params,
            serde_json::json!({
                "source_name": "binance-spot",
                "venue": "binance",
                "symbol": "ETHUSDT",
                "timeframe": "1m",
                "limit": 20,
            })
        );
        assert_eq!(
            result.streams[0].query_candles_hint.required_params,
            ["start", "end"]
        );
        assert_eq!(
            result.streams[0].query_candles_hint.optional_params,
            ["limit"]
        );
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
                    offset:      None,
                },
                &context(),
            )
            .await
            .unwrap();

        assert_eq!(result.count, 1);
        assert_eq!(
            result.filters,
            normalized_stream_filters(None, Some("binance"), Some("BTCUSDT"), Some("1m"))
        );
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
                    offset:      None,
                },
                &context(),
            )
            .await
            .unwrap();

        assert_eq!(result.count, 1);
        assert_eq!(result.query_limit, 1);
        assert!(result.has_more);
        assert_eq!(result.streams[0].symbol, "ETHUSDT");
        assert_eq!(result.query_offset, 0);
        let next_page_hint = result
            .next_page_hint
            .as_ref()
            .expect("paginated stream list should include next-page hint");
        assert_eq!(next_page_hint.tool, "finance_list_candle_streams");
        assert_eq!(
            next_page_hint.default_params,
            serde_json::json!({
                "source_name": "binance-spot",
                "venue": "binance",
                "timeframe": "1m",
                "limit": 1,
                "offset": 1,
            })
        );
        assert!(next_page_hint.required_params.is_empty());
        assert_eq!(
            next_page_hint.optional_params,
            [
                "source_name",
                "venue",
                "symbol",
                "timeframe",
                "limit",
                "offset"
            ]
        );

        let second_page = tool
            .run(
                FinanceListCandleStreamsParams {
                    source_name: Some("binance-spot".to_owned()),
                    venue:       Some("binance".to_owned()),
                    symbol:      None,
                    timeframe:   Some("1m".to_owned()),
                    limit:       Some(1),
                    offset:      Some(1),
                },
                &context(),
            )
            .await
            .unwrap();
        assert_eq!(second_page.count, 1);
        assert_eq!(second_page.query_limit, 1);
        assert_eq!(second_page.query_offset, 1);
        assert!(!second_page.has_more);
        assert!(second_page.next_page_hint.is_none());
        assert_eq!(second_page.streams[0].symbol, "BTCUSDT");
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
                    offset:      None,
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
                    offset:      None,
                },
                &context(),
            )
            .await
            .unwrap();

        assert_eq!(result.count, 1);
        assert_eq!(
            result.filters,
            normalized_stream_filters(
                Some("binance-spot"),
                Some("binance"),
                Some("BTCUSDT"),
                Some("1m")
            )
        );
        assert_eq!(result.streams[0].venue, "binance");
        assert_eq!(result.streams[0].symbol, "BTCUSDT");
        assert_eq!(result.streams[0].timeframe, "1m");
        assert_eq!(result.streams[0].candle_count, 2);
    }

    #[tokio::test]
    async fn list_candle_streams_tool_echoes_filters_when_no_streams_match() {
        let repository = repository_with_multiple_streams().await;
        let tool = FinanceListCandleStreamsTool::new(repository);
        let result = tool
            .run(
                FinanceListCandleStreamsParams {
                    source_name: Some(" binance-spot ".to_owned()),
                    venue:       Some(" Binance ".to_owned()),
                    symbol:      Some(" dogeusdt ".to_owned()),
                    timeframe:   Some(" 1M ".to_owned()),
                    limit:       Some(10),
                    offset:      None,
                },
                &context(),
            )
            .await
            .unwrap();

        assert_eq!(result.count, 0);
        assert!(result.streams.is_empty());
        assert_eq!(
            result.filters,
            normalized_stream_filters(
                Some("binance-spot"),
                Some("binance"),
                Some("DOGEUSDT"),
                Some("1m")
            )
        );
        assert_eq!(result.query_limit, 10);
        assert_eq!(result.query_offset, 0);
        assert!(!result.has_more);
        assert!(result.next_page_hint.is_none());
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
        assert_eq!(result.selector, normalized_selector(Some("binance-spot")));
        assert_eq!(candle.venue, "binance");
        assert_eq!(candle.symbol, "BTCUSDT");
        assert_eq!(candle.timeframe, "1m");
        assert_eq!(candle.open_time, "2026-07-10T08:01:00Z");
    }

    #[tokio::test]
    async fn recent_candles_tool_returns_latest_candles_in_order() {
        let repository = repository().await;
        let tool = FinanceGetRecentCandlesTool::new(repository);
        let result = tool
            .run(
                FinanceGetRecentCandlesParams {
                    source_name: Some("binance-spot".to_owned()),
                    venue:       "binance".to_owned(),
                    symbol:      "BTCUSDT".to_owned(),
                    timeframe:   "1m".to_owned(),
                    limit:       Some(1),
                    end:         None,
                },
                &context(),
            )
            .await
            .unwrap();

        assert_eq!(result.count, 1);
        assert_eq!(result.query_limit, 1);
        assert!(result.has_more);
        assert_eq!(result.next_end.as_deref(), Some("2026-07-10T08:01:00Z"));
        let next_page_hint = result
            .next_page_hint
            .as_ref()
            .expect("paginated recent-candle result should include next-page hint");
        assert_eq!(next_page_hint.tool, "finance_get_recent_candles");
        assert_eq!(
            next_page_hint.default_params,
            serde_json::json!({
                "source_name": "binance-spot",
                "venue": "binance",
                "symbol": "BTCUSDT",
                "timeframe": "1m",
                "end": "2026-07-10T08:01:00Z",
                "limit": 1,
            })
        );
        assert!(next_page_hint.required_params.is_empty());
        assert_eq!(next_page_hint.optional_params, ["end", "limit"]);
        assert_eq!(result.candles[0].open_time, "2026-07-10T08:01:00Z");
        assert_eq!(result.candles[0].close, "61520.00");

        let older_page = tool
            .run(
                FinanceGetRecentCandlesParams {
                    source_name: Some("binance-spot".to_owned()),
                    venue:       "binance".to_owned(),
                    symbol:      "BTCUSDT".to_owned(),
                    timeframe:   "1m".to_owned(),
                    limit:       Some(1),
                    end:         Some("2026-07-10T08:01:00Z".to_owned()),
                },
                &context(),
            )
            .await
            .unwrap();
        assert_eq!(older_page.count, 1);
        assert_eq!(older_page.query_limit, 1);
        assert!(!older_page.has_more);
        assert_eq!(older_page.next_end, None);
        assert!(older_page.next_page_hint.is_none());
        assert_eq!(older_page.candles[0].open_time, "2026-07-10T08:00:00Z");
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
        assert!(!result.has_more);
        assert_eq!(result.next_start, None);
        assert!(
            result.next_page_hint.is_none(),
            "complete range result should not include next-page hint"
        );
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
    async fn query_candles_tool_reports_when_more_candles_match() {
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
                    limit:       Some(1),
                },
                &context(),
            )
            .await
            .unwrap();

        assert_eq!(result.count, 1);
        assert_eq!(result.query_limit, 1);
        assert!(result.has_more);
        assert_eq!(result.next_start.as_deref(), Some("2026-07-10T08:01:00Z"));
        let next_page_hint = result
            .next_page_hint
            .as_ref()
            .expect("paginated range result should include next-page hint");
        assert_eq!(next_page_hint.tool, "finance_query_candles");
        assert_eq!(
            next_page_hint.default_params,
            serde_json::json!({
                "source_name": "binance-spot",
                "venue": "binance",
                "symbol": "BTCUSDT",
                "timeframe": "1m",
                "start": "2026-07-10T08:01:00Z",
                "end": "2026-07-10T08:02:00Z",
                "limit": 1,
            })
        );
        assert!(next_page_hint.required_params.is_empty());
        assert_eq!(next_page_hint.optional_params, ["start", "end", "limit"]);
        assert_eq!(result.candles[0].open_time, "2026-07-10T08:00:00Z");
    }

    #[tokio::test]
    async fn query_candles_tool_rejects_unprobeable_limit() {
        let repository = repository().await;
        let tool = FinanceQueryCandlesTool::new(repository);
        let error = tool
            .run(
                FinanceQueryCandlesParams {
                    source_name: Some("binance-spot".to_owned()),
                    venue:       "binance".to_owned(),
                    symbol:      "BTCUSDT".to_owned(),
                    timeframe:   "1m".to_owned(),
                    start:       "2026-07-10T08:00:00Z".to_owned(),
                    end:         "2026-07-10T08:02:00Z".to_owned(),
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
        assert_eq!(
            query_result.selector,
            normalized_selector(Some("binance-spot"))
        );

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
        assert_eq!(
            gap_result.selector,
            normalized_selector(Some("binance-spot"))
        );
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
        assert_eq!(
            freshness_result.selector,
            normalized_selector(Some("binance-spot"))
        );
        assert_eq!(freshness_result.status, "fresh");
        assert_eq!(freshness_result.lag_secs, Some(120));
    }

    #[tokio::test]
    async fn single_stream_candle_tools_echo_selector_when_no_rows_match() {
        let repository = repository().await;
        let latest = FinanceGetLatestCandleTool::new(repository.clone());
        let recent = FinanceGetRecentCandlesTool::new(repository.clone());
        let query = FinanceQueryCandlesTool::new(repository.clone());
        let gaps = FinanceFindCandleGapsTool::new(repository.clone());
        let freshness = FinanceGetCandleFreshnessTool::new(repository);
        let expected = FinanceCandleSelector {
            source_name: Some("binance-spot".to_owned()),
            venue:       "binance".to_owned(),
            symbol:      "DOGEUSDT".to_owned(),
            timeframe:   "1m".to_owned(),
        };

        let latest_result = latest
            .run(
                FinanceGetLatestCandleParams {
                    source_name: Some(" binance-spot ".to_owned()),
                    venue:       " Binance ".to_owned(),
                    symbol:      " dogeusdt ".to_owned(),
                    timeframe:   " 1M ".to_owned(),
                },
                &context(),
            )
            .await
            .unwrap();
        assert_eq!(latest_result.selector, expected);
        assert!(latest_result.candle.is_none());

        let recent_result = recent
            .run(
                FinanceGetRecentCandlesParams {
                    source_name: Some(" binance-spot ".to_owned()),
                    venue:       " Binance ".to_owned(),
                    symbol:      " dogeusdt ".to_owned(),
                    timeframe:   " 1M ".to_owned(),
                    limit:       Some(10),
                    end:         None,
                },
                &context(),
            )
            .await
            .unwrap();
        assert_eq!(recent_result.selector, expected);
        assert!(recent_result.candles.is_empty());

        let query_result = query
            .run(
                FinanceQueryCandlesParams {
                    source_name: Some(" binance-spot ".to_owned()),
                    venue:       " Binance ".to_owned(),
                    symbol:      " dogeusdt ".to_owned(),
                    timeframe:   " 1M ".to_owned(),
                    start:       "2026-07-10T08:00:00Z".to_owned(),
                    end:         "2026-07-10T08:03:00Z".to_owned(),
                    limit:       Some(10),
                },
                &context(),
            )
            .await
            .unwrap();
        assert_eq!(query_result.selector, expected);
        assert!(query_result.candles.is_empty());

        let gaps_result = gaps
            .run(
                FinanceFindCandleGapsParams {
                    source_name: Some(" binance-spot ".to_owned()),
                    venue:       " Binance ".to_owned(),
                    symbol:      " dogeusdt ".to_owned(),
                    timeframe:   " 1M ".to_owned(),
                    start:       "2026-07-10T08:00:00Z".to_owned(),
                    end:         "2026-07-10T08:03:00Z".to_owned(),
                },
                &context(),
            )
            .await
            .unwrap();
        assert_eq!(gaps_result.selector, expected);
        assert_eq!(gaps_result.expected_count, 3);
        assert_eq!(gaps_result.missing_count, 3);

        let freshness_result = freshness
            .run(
                FinanceGetCandleFreshnessParams {
                    source_name:      Some(" binance-spot ".to_owned()),
                    venue:            " Binance ".to_owned(),
                    symbol:           " dogeusdt ".to_owned(),
                    timeframe:        " 1M ".to_owned(),
                    as_of:            Some("2026-07-10T08:03:00Z".to_owned()),
                    stale_after_secs: Some(120),
                },
                &context(),
            )
            .await
            .unwrap();
        assert_eq!(freshness_result.selector, expected);
        assert_eq!(freshness_result.status, "missing");
        assert!(freshness_result.latest.is_none());
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
        let recent = FinanceGetRecentCandlesTool::new(repository.clone());
        let query = FinanceQueryCandlesTool::new(repository.clone());
        let gaps = FinanceFindCandleGapsTool::new(repository.clone());
        let freshness = FinanceGetCandleFreshnessTool::new(repository);

        assert!(streams.is_read_only(&serde_json::json!({})));
        assert!(latest.is_read_only(&serde_json::json!({})));
        assert!(recent.is_read_only(&serde_json::json!({})));
        assert!(query.is_read_only(&serde_json::json!({})));
        assert!(gaps.is_read_only(&serde_json::json!({})));
        assert!(freshness.is_read_only(&serde_json::json!({})));
    }
}
