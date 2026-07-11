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
    CandleLatestQuery, CandleRangeQuery, MarketCandle, MarketDataRepositoryRef, Timeframe,
};

const DEFAULT_CANDLE_LIMIT: usize = 500;
const MAX_CANDLE_LIMIT: usize = 10_000;
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
            source_name: normalize_optional_selector("source_name", params.source_name)?,
            venue:       normalize_required_selector("venue", params.venue)?,
            symbol:      normalize_required_selector("symbol", params.symbol)?,
            timeframe:   Timeframe::parse(params.timeframe)?,
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
        let limit = params.limit.unwrap_or(DEFAULT_CANDLE_LIMIT);
        anyhow::ensure!(limit > 0, "limit must be positive");
        anyhow::ensure!(
            limit <= MAX_CANDLE_LIMIT,
            "limit must be <= {MAX_CANDLE_LIMIT}"
        );
        let start = parse_timestamp("start", &params.start)?;
        let end = parse_timestamp("end", &params.end)?;
        anyhow::ensure!(start < end, "start must be before end");

        let query = CandleRangeQuery {
            source_name: normalize_optional_selector("source_name", params.source_name)?,
            venue: normalize_required_selector("venue", params.venue)?,
            symbol: normalize_required_selector("symbol", params.symbol)?,
            timeframe: Timeframe::parse(params.timeframe)?,
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

fn parse_timestamp(name: &str, value: &str) -> anyhow::Result<Timestamp> {
    value
        .parse()
        .map_err(|err| anyhow::anyhow!("{name} must be an RFC3339 timestamp: {err}"))
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
        FinanceGetLatestCandleParams, FinanceGetLatestCandleTool, FinanceQueryCandlesParams,
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

    #[test]
    fn candle_query_tools_are_read_only() {
        let repository = Arc::new(InMemoryMarketDataRepository::default());
        let latest = FinanceGetLatestCandleTool::new(repository.clone());
        let query = FinanceQueryCandlesTool::new(repository);

        assert!(latest.is_read_only(&serde_json::json!({})));
        assert!(query.is_read_only(&serde_json::json!({})));
    }
}
