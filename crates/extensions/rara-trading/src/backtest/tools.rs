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

//! Agent-facing tool for the signal-accuracy backtest harness.

use async_trait::async_trait;
use jiff::Timestamp;
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{
    BacktestReport, HOLD_BARS, SignalAttributionReport, run_backtest, run_signal_attribution,
};
use crate::market_data::{CandleRangeQuery, MarketDataRepositoryRef, Timeframe};

const DEFAULT_BACKTEST_LIMIT: usize = 10_000;
const MAX_BACKTEST_LIMIT: usize = 100_000;
const MAX_SELECTOR_LEN: usize = 128;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FinanceBacktestSignalParams {
    #[serde(default)]
    pub source_name: Option<String>,
    pub venue:       String,
    pub symbol:      String,
    pub timeframe:   String,
    /// Inclusive candle open time, as an RFC3339 timestamp.
    pub start:       String,
    /// Exclusive candle open time, as an RFC3339 timestamp.
    pub end:         String,
    /// Maximum candles to replay. Defaults to 10,000 and is capped at 100,000.
    #[serde(default)]
    pub limit:       Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FinanceBacktestSignalResult {
    pub selector:           FinanceBacktestSignalSelector,
    pub start:              String,
    pub end:                String,
    pub candle_count:       usize,
    pub query_limit:        usize,
    pub hold_bars:          usize,
    pub report:             BacktestReport,
    pub signal_attribution: SignalAttributionReport,
    pub diagnostic_hint:    Option<FinanceBacktestSignalDiagnosticHint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FinanceBacktestSignalSelector {
    pub source_name: Option<String>,
    pub venue:       String,
    pub symbol:      String,
    pub timeframe:   String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FinanceBacktestSignalDiagnosticHint {
    pub tool:            String,
    pub default_params:  serde_json::Value,
    pub required_params: Vec<String>,
    pub optional_params: Vec<String>,
}

#[derive(ToolDef)]
#[tool(
    name = "finance_backtest_signal",
    description = "Replay stored closed OHLCV candles through rara's built-in market-anomaly \
                   signal evaluator and report the fixed naive-long signal-accuracy metrics. Use \
                   this after discovering a candle stream when the user asks whether rara's \
                   signal has historical edge. This is read-only and never places trades.",
    tier = "deferred",
    read_only,
    concurrency_safe
)]
pub struct FinanceBacktestSignalTool {
    repository: MarketDataRepositoryRef,
}

impl FinanceBacktestSignalTool {
    pub fn new(repository: MarketDataRepositoryRef) -> Self { Self { repository } }
}

#[async_trait]
impl ToolExecute for FinanceBacktestSignalTool {
    type Output = FinanceBacktestSignalResult;
    type Params = FinanceBacktestSignalParams;

    async fn run(
        &self,
        params: FinanceBacktestSignalParams,
        _context: &ToolContext,
    ) -> anyhow::Result<FinanceBacktestSignalResult> {
        let limit = validate_backtest_limit(params.limit)?;
        let probe_limit = limit.saturating_add(1);
        let start = parse_timestamp("start", &params.start)?;
        let end = parse_timestamp("end", &params.end)?;
        anyhow::ensure!(start < end, "start must be before end");
        let source_name = normalize_optional_source_name_selector(params.source_name)?;
        let venue = normalize_required_venue_selector(params.venue)?;
        let symbol = normalize_required_symbol_selector(params.symbol)?;
        let timeframe = normalize_required_timeframe_selector(params.timeframe)?;
        let selector = backtest_selector(
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

        let candles = self.repository.candles(query).await?;
        anyhow::ensure!(
            candles.len() <= limit,
            "backtest range contains more than {limit} candles; narrow the time range or raise \
             limit up to {MAX_BACKTEST_LIMIT}"
        );

        let candle_count = candles.len();
        let report = run_backtest(&candles)?;
        let signal_attribution = run_signal_attribution(&candles)?;
        let diagnostic_hint =
            source_scoped_empty_candles_diagnostic_hint(source_name.as_deref(), candle_count == 0);

        Ok(FinanceBacktestSignalResult {
            selector,
            start: start.to_string(),
            end: end.to_string(),
            candle_count,
            query_limit: limit,
            hold_bars: HOLD_BARS,
            report,
            signal_attribution,
            diagnostic_hint,
        })
    }
}

fn backtest_selector(
    source_name: Option<String>,
    venue: String,
    symbol: String,
    timeframe: &Timeframe,
) -> FinanceBacktestSignalSelector {
    FinanceBacktestSignalSelector {
        source_name,
        venue,
        symbol,
        timeframe: timeframe.to_string(),
    }
}

fn source_scoped_empty_candles_diagnostic_hint(
    source_name: Option<&str>,
    is_empty: bool,
) -> Option<FinanceBacktestSignalDiagnosticHint> {
    let source_name = source_name.filter(|_| is_empty)?;
    Some(FinanceBacktestSignalDiagnosticHint {
        tool:            "finance_diagnose_candle_subscriptions".to_owned(),
        default_params:  serde_json::json!({
            "source_names": [source_name],
        }),
        required_params: Vec::new(),
        optional_params: [
            "subscription_id",
            "catalog_source_ids",
            "source_names",
            "feed_ids",
            "as_of",
            "stale_after_secs",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
    })
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

fn normalize_required_symbol_selector(value: String) -> anyhow::Result<String> {
    Ok(normalize_required_selector("symbol", value)?.to_ascii_uppercase())
}

fn normalize_required_timeframe_selector(value: String) -> anyhow::Result<Timeframe> {
    Timeframe::parse(normalize_required_selector("timeframe", value)?.to_ascii_lowercase())
}

fn validate_backtest_limit(limit: Option<usize>) -> anyhow::Result<usize> {
    let limit = limit.unwrap_or(DEFAULT_BACKTEST_LIMIT);
    anyhow::ensure!(limit > 0, "limit must be positive");
    anyhow::ensure!(
        limit <= MAX_BACKTEST_LIMIT,
        "limit must be <= {MAX_BACKTEST_LIMIT}"
    );
    Ok(limit)
}

fn parse_timestamp(name: &str, value: &str) -> anyhow::Result<Timestamp> {
    value
        .parse()
        .map_err(|err| anyhow::anyhow!("{name} must be an RFC3339 timestamp: {err}"))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use jiff::{SignedDuration, Timestamp};
    use rara_kernel::{
        io::MessageId,
        queue::{ShardedEventQueue, ShardedEventQueueConfig},
        session::SessionKey,
        tool::{ToolContext, ToolExecute},
    };
    use rust_decimal::Decimal;

    use super::{
        FinanceBacktestSignalParams, FinanceBacktestSignalSelector, FinanceBacktestSignalTool,
    };
    use crate::{
        backtest::HOLD_BARS,
        market_data::{
            InMemoryMarketDataRepository, MarketCandle, MarketDataRepository, Timeframe,
        },
    };

    const STEP_SECS: i64 = 900;

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

    fn candle(index: i64, close: i64, volume: i64) -> MarketCandle {
        let base: Timestamp = "2026-07-10T00:00:00Z"
            .parse()
            .expect("base timestamp parses");
        let open_time = base + SignedDuration::from_secs(index * STEP_SECS);
        MarketCandle {
            source_name: "binance-spot".to_owned(),
            venue: "binance".to_owned(),
            symbol: "BTCUSDT".to_owned(),
            timeframe: Timeframe::parse("15m").expect("timeframe parses"),
            open_time,
            close_time: open_time + SignedDuration::from_secs(STEP_SECS),
            open: Decimal::from(close),
            high: Decimal::from(close),
            low: Decimal::from(close),
            close: Decimal::from(close),
            volume: Decimal::from(volume),
            ingested_at: open_time,
            provider_sequence: Some(index.to_string()),
        }
    }

    fn fixture() -> Vec<MarketCandle> {
        let prices = [
            60000, 60000, 60000, 60000, 60000, 60000, 60200, 60400, 60600, 60600, 60400, 60200,
            60000, 60000, 60000, 60000,
        ];
        let volumes = [
            100, 100, 100, 100, 100, 400, 100, 100, 100, 400, 100, 100, 100, 100, 100, 100,
        ];
        prices
            .iter()
            .zip(volumes)
            .enumerate()
            .map(|(index, (&price, volume))| candle(index as i64, price, volume))
            .collect()
    }

    async fn repository() -> Arc<InMemoryMarketDataRepository> {
        let repository = Arc::new(InMemoryMarketDataRepository::default());
        for candle in fixture() {
            repository
                .upsert_closed_candle(candle)
                .await
                .expect("seed candle");
        }
        repository
    }

    fn default_params() -> FinanceBacktestSignalParams {
        FinanceBacktestSignalParams {
            source_name: Some("binance-spot".to_owned()),
            venue:       "BINANCE".to_owned(),
            symbol:      "btcusdt".to_owned(),
            timeframe:   "15M".to_owned(),
            start:       "2026-07-10T00:00:00Z".to_owned(),
            end:         "2026-07-10T04:00:00Z".to_owned(),
            limit:       None,
        }
    }

    #[tokio::test]
    async fn backtest_signal_tool_reports_existing_harness_metrics() {
        let tool = FinanceBacktestSignalTool::new(repository().await);

        let result = tool
            .run(default_params(), &context())
            .await
            .expect("backtest signal tool");

        assert_eq!(
            result.selector,
            FinanceBacktestSignalSelector {
                source_name: Some("binance-spot".to_owned()),
                venue:       "binance".to_owned(),
                symbol:      "BTCUSDT".to_owned(),
                timeframe:   "15m".to_owned(),
            }
        );
        assert_eq!(result.candle_count, 16);
        assert_eq!(result.query_limit, 10_000);
        assert_eq!(result.hold_bars, HOLD_BARS);
        assert_eq!(result.report.trigger_count, 2);
        assert_eq!(result.report.evaluated_trade_count, 2);
        assert_eq!(result.report.win_count, 1);
        assert_eq!(result.report.win_rate, Some(0.5));
        assert_eq!(result.signal_attribution.composite_trigger_count, 2);
        let volume_surge = result
            .signal_attribution
            .signals
            .iter()
            .find(|signal| signal.signal_name == "volume_surge")
            .expect("volume surge row");
        assert_eq!(volume_surge.trigger_count, 2);
        assert_eq!(volume_surge.evaluated_trade_count, 2);
        assert_eq!(volume_surge.win_count, 1);
        assert_eq!(volume_surge.win_rate, Some(0.5));
        let directional_run = result
            .signal_attribution
            .signals
            .iter()
            .find(|signal| signal.signal_name == "directional_run")
            .expect("directional run row");
        assert_eq!(directional_run.trigger_count, 0);
        assert_eq!(directional_run.evaluated_trade_count, 0);
        assert_eq!(directional_run.win_rate, None);
        assert!(result.diagnostic_hint.is_none());
    }

    #[tokio::test]
    async fn backtest_signal_tool_rejects_ranges_over_limit() {
        let tool = FinanceBacktestSignalTool::new(repository().await);
        let params = FinanceBacktestSignalParams {
            limit: Some(3),
            ..default_params()
        };

        let error = tool
            .run(params, &context())
            .await
            .expect_err("range should exceed the requested limit");

        assert!(error.to_string().contains("more than 3 candles"));
    }

    #[tokio::test]
    async fn empty_source_scoped_backtest_points_to_diagnostics() {
        let repository = Arc::new(InMemoryMarketDataRepository::default());
        let tool = FinanceBacktestSignalTool::new(repository);

        let result = tool
            .run(default_params(), &context())
            .await
            .expect("empty backtest");

        assert_eq!(result.candle_count, 0);
        assert_eq!(result.report.trigger_count, 0);
        assert_eq!(result.signal_attribution.composite_trigger_count, 0);
        assert!(
            result
                .signal_attribution
                .signals
                .iter()
                .all(|signal| signal.trigger_count == 0)
        );
        let hint = result.diagnostic_hint.expect("diagnostic hint");
        assert_eq!(hint.tool, "finance_diagnose_candle_subscriptions");
        assert_eq!(
            hint.default_params,
            serde_json::json!({
                "source_names": ["binance-spot"],
            })
        );
    }
}
