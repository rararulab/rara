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

//! Agent-facing read-only market-anomaly evaluation tool.

use async_trait::async_trait;
use jiff::Timestamp;
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{AnomalyMetrics, AnomalySignal, EVAL_WINDOW, evaluate};
use crate::market_data::{
    CandleLatestQuery, CandleRangeQuery, CandleRecentQuery, MarketCandle, MarketDataRepositoryRef,
    Timeframe,
};

const MAX_SELECTOR_LEN: usize = 128;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FinanceEvaluateCandleSignalParams {
    #[serde(default)]
    pub source_name: Option<String>,
    pub venue:       String,
    pub symbol:      String,
    pub timeframe:   String,
    /// Optional candle open time to evaluate. Defaults to the latest stored
    /// closed candle for the selected stream.
    #[serde(default)]
    pub open_time:   Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FinanceEvaluateCandleSignalResult {
    pub selector:            FinanceEvaluateCandleSignalSelector,
    pub requested_open_time: Option<String>,
    pub evaluated_candle:    Option<FinanceEvaluatedCandle>,
    pub window_candle_count: usize,
    pub window_limit:        usize,
    pub window_status:       String,
    pub has_signal:          bool,
    pub status:              String,
    pub signal:              Option<FinanceEvaluatedAnomalySignal>,
    pub diagnostic_hint:     Option<FinanceEvaluateCandleSignalDiagnosticHint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FinanceEvaluateCandleSignalSelector {
    pub source_name: Option<String>,
    pub venue:       String,
    pub symbol:      String,
    pub timeframe:   String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FinanceEvaluatedCandle {
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

#[derive(Debug, Clone, Serialize)]
pub struct FinanceEvaluatedAnomalySignal {
    pub severity: String,
    pub reason:   String,
    pub metrics:  FinanceEvaluatedAnomalyMetrics,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct FinanceEvaluatedAnomalyMetrics {
    pub window_return:     f64,
    pub max_drawdown:      f64,
    pub volume_surge:      Option<f64>,
    pub robust_zscore:     Option<f64>,
    pub jump_ratio:        Option<f64>,
    pub jump_flagged:      bool,
    pub volatility_regime: Option<f64>,
    pub directional_run:   Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FinanceEvaluateCandleSignalDiagnosticHint {
    pub tool:            String,
    pub default_params:  serde_json::Value,
    pub required_params: Vec<String>,
    pub optional_params: Vec<String>,
}

#[derive(ToolDef)]
#[tool(
    name = "finance_evaluate_candle_signal",
    description = "Evaluate the latest or a specified stored closed OHLCV candle through rara's \
                   built-in market-anomaly signal evaluator. Use this after discovering a candle \
                   stream when the user asks whether the current stream has an anomaly signal. \
                   This is read-only and never places trades.",
    tier = "deferred",
    read_only,
    concurrency_safe
)]
pub struct FinanceEvaluateCandleSignalTool {
    repository: MarketDataRepositoryRef,
}

impl FinanceEvaluateCandleSignalTool {
    pub fn new(repository: MarketDataRepositoryRef) -> Self { Self { repository } }
}

#[async_trait]
impl ToolExecute for FinanceEvaluateCandleSignalTool {
    type Output = FinanceEvaluateCandleSignalResult;
    type Params = FinanceEvaluateCandleSignalParams;

    async fn run(
        &self,
        params: FinanceEvaluateCandleSignalParams,
        _context: &ToolContext,
    ) -> anyhow::Result<FinanceEvaluateCandleSignalResult> {
        let source_name = normalize_optional_source_name_selector(params.source_name)?;
        let venue = normalize_required_venue_selector(params.venue)?;
        let symbol = normalize_required_symbol_selector(params.symbol)?;
        let timeframe = normalize_required_timeframe_selector(params.timeframe)?;
        let requested_open_time = params
            .open_time
            .as_deref()
            .map(|value| parse_timestamp("open_time", value))
            .transpose()?;

        let candidate = match requested_open_time {
            Some(open_time) => {
                requested_candle(
                    &*self.repository,
                    source_name.clone(),
                    venue.clone(),
                    symbol.clone(),
                    timeframe.clone(),
                    open_time,
                )
                .await?
            }
            None => {
                latest_candle(
                    &*self.repository,
                    source_name.clone(),
                    venue.clone(),
                    symbol.clone(),
                    timeframe.clone(),
                )
                .await?
            }
        };

        let Some(latest) = candidate else {
            return Ok(missing_result(
                source_name,
                venue,
                symbol,
                &timeframe,
                requested_open_time,
            ));
        };

        let window = self
            .repository
            .recent_candles(CandleRecentQuery {
                source_name: Some(latest.source_name.clone()),
                venue:       latest.venue.clone(),
                symbol:      latest.symbol.clone(),
                timeframe:   latest.timeframe.clone(),
                limit:       EVAL_WINDOW,
                end:         Some(latest.open_time),
            })
            .await?;
        let window_candle_count = window.len();
        let signal = evaluate(&window, &latest)?.map(FinanceEvaluatedAnomalySignal::from);
        let has_signal = signal.is_some();
        let status = if has_signal {
            "signal"
        } else if window_candle_count == 0 {
            "insufficient_history"
        } else {
            "normal"
        };
        let window_status = if window_candle_count >= EVAL_WINDOW {
            "complete"
        } else {
            "partial"
        };
        let selector = candle_signal_selector(
            Some(latest.source_name.clone()),
            latest.venue.clone(),
            latest.symbol.clone(),
            &latest.timeframe,
        );

        Ok(FinanceEvaluateCandleSignalResult {
            selector,
            requested_open_time: requested_open_time.map(|value| value.to_string()),
            evaluated_candle: Some(FinanceEvaluatedCandle::from(latest)),
            window_candle_count,
            window_limit: EVAL_WINDOW,
            window_status: window_status.to_owned(),
            has_signal,
            status: status.to_owned(),
            signal,
            diagnostic_hint: None,
        })
    }
}

async fn latest_candle(
    repository: &dyn crate::market_data::MarketDataRepository,
    source_name: Option<String>,
    venue: String,
    symbol: String,
    timeframe: Timeframe,
) -> anyhow::Result<Option<MarketCandle>> {
    repository
        .latest_closed_candle(CandleLatestQuery {
            source_name,
            venue,
            symbol,
            timeframe,
        })
        .await
}

async fn requested_candle(
    repository: &dyn crate::market_data::MarketDataRepository,
    source_name: Option<String>,
    venue: String,
    symbol: String,
    timeframe: Timeframe,
    open_time: Timestamp,
) -> anyhow::Result<Option<MarketCandle>> {
    let end = open_time
        .checked_add(timeframe.step()?)
        .map_err(|err| anyhow::anyhow!("timeframe addition overflowed: {err}"))?;
    let rows = repository
        .candles(CandleRangeQuery {
            source_name,
            venue,
            symbol,
            timeframe,
            start: open_time,
            end,
            limit: 2,
        })
        .await?;
    anyhow::ensure!(
        rows.len() <= 1,
        "open_time matched multiple candles; supply source_name to disambiguate"
    );
    Ok(rows.into_iter().next())
}

fn missing_result(
    source_name: Option<String>,
    venue: String,
    symbol: String,
    timeframe: &Timeframe,
    requested_open_time: Option<Timestamp>,
) -> FinanceEvaluateCandleSignalResult {
    let selector = candle_signal_selector(source_name.clone(), venue, symbol, timeframe);
    FinanceEvaluateCandleSignalResult {
        selector,
        requested_open_time: requested_open_time.map(|value| value.to_string()),
        evaluated_candle: None,
        window_candle_count: 0,
        window_limit: EVAL_WINDOW,
        window_status: "missing".to_owned(),
        has_signal: false,
        status: "missing".to_owned(),
        signal: None,
        diagnostic_hint: source_scoped_missing_candle_diagnostic_hint(source_name.as_deref()),
    }
}

fn candle_signal_selector(
    source_name: Option<String>,
    venue: String,
    symbol: String,
    timeframe: &Timeframe,
) -> FinanceEvaluateCandleSignalSelector {
    FinanceEvaluateCandleSignalSelector {
        source_name,
        venue,
        symbol,
        timeframe: timeframe.to_string(),
    }
}

fn source_scoped_missing_candle_diagnostic_hint(
    source_name: Option<&str>,
) -> Option<FinanceEvaluateCandleSignalDiagnosticHint> {
    let source_name = source_name?;
    Some(FinanceEvaluateCandleSignalDiagnosticHint {
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

impl From<MarketCandle> for FinanceEvaluatedCandle {
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

impl From<AnomalySignal> for FinanceEvaluatedAnomalySignal {
    fn from(signal: AnomalySignal) -> Self {
        Self {
            severity: signal.severity.label().to_owned(),
            reason:   signal.reason,
            metrics:  FinanceEvaluatedAnomalyMetrics::from(signal.metrics),
        }
    }
}

impl From<AnomalyMetrics> for FinanceEvaluatedAnomalyMetrics {
    fn from(metrics: AnomalyMetrics) -> Self {
        Self {
            window_return:     metrics.window_return,
            max_drawdown:      metrics.max_drawdown,
            volume_surge:      metrics.volume_surge,
            robust_zscore:     metrics.robust_zscore,
            jump_ratio:        metrics.jump_ratio,
            jump_flagged:      metrics.jump_flagged,
            volatility_regime: metrics.volatility_regime,
            directional_run:   metrics.directional_run,
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

fn normalize_required_symbol_selector(value: String) -> anyhow::Result<String> {
    Ok(normalize_required_selector("symbol", value)?.to_ascii_uppercase())
}

fn normalize_required_timeframe_selector(value: String) -> anyhow::Result<Timeframe> {
    Timeframe::parse(normalize_required_selector("timeframe", value)?.to_ascii_lowercase())
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
        tool::{AgentTool, ToolContext, ToolExecute},
    };
    use rust_decimal::Decimal;

    use super::{
        FinanceEvaluateCandleSignalParams, FinanceEvaluateCandleSignalSelector,
        FinanceEvaluateCandleSignalTool,
    };
    use crate::market_data::{
        InMemoryMarketDataRepository, MarketCandle, MarketDataRepository, Timeframe,
    };

    const STEP_SECS: i64 = 60;

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
            timeframe: Timeframe::parse("1m").expect("timeframe parses"),
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

    async fn repository(
        candles: impl IntoIterator<Item = MarketCandle>,
    ) -> Arc<InMemoryMarketDataRepository> {
        let repository = Arc::new(InMemoryMarketDataRepository::default());
        for candle in candles {
            repository
                .upsert_closed_candle(candle)
                .await
                .expect("seed candle");
        }
        repository
    }

    fn default_params() -> FinanceEvaluateCandleSignalParams {
        FinanceEvaluateCandleSignalParams {
            source_name: Some("binance-spot".to_owned()),
            venue:       "BINANCE".to_owned(),
            symbol:      "btcusdt".to_owned(),
            timeframe:   "1M".to_owned(),
            open_time:   None,
        }
    }

    #[tokio::test]
    async fn evaluate_candle_signal_tool_reports_latest_signal() {
        let candles = [100, 101, 102, 103, 104, 105, 106]
            .into_iter()
            .enumerate()
            .map(|(index, close)| candle(index as i64, close, 100));
        let tool = FinanceEvaluateCandleSignalTool::new(repository(candles).await);

        let result = tool
            .run(default_params(), &context())
            .await
            .expect("evaluate latest signal");

        assert_eq!(
            result.selector,
            FinanceEvaluateCandleSignalSelector {
                source_name: Some("binance-spot".to_owned()),
                venue:       "binance".to_owned(),
                symbol:      "BTCUSDT".to_owned(),
                timeframe:   "1m".to_owned(),
            }
        );
        assert_eq!(result.status, "signal");
        assert_eq!(result.window_candle_count, 6);
        assert_eq!(result.window_status, "partial");
        assert!(result.has_signal);
        let signal = result.signal.expect("signal should fire");
        assert_eq!(signal.severity, "elevated");
        assert!(
            signal.reason.contains("directional run +6 bars"),
            "unexpected reason: {}",
            signal.reason
        );
        assert_eq!(signal.metrics.directional_run, Some(6.0));
        assert_eq!(
            result.evaluated_candle.expect("evaluated candle").open_time,
            "2026-07-10T00:06:00Z"
        );
        assert!(result.diagnostic_hint.is_none());
    }

    #[tokio::test]
    async fn evaluate_candle_signal_tool_does_not_look_past_requested_open_time() {
        let candles = [
            candle(0, 100, 100),
            candle(1, 100, 100),
            candle(2, 200, 100),
        ];
        let tool = FinanceEvaluateCandleSignalTool::new(repository(candles).await);
        let params = FinanceEvaluateCandleSignalParams {
            open_time: Some("2026-07-10T00:01:00Z".to_owned()),
            ..default_params()
        };

        let result = tool
            .run(params, &context())
            .await
            .expect("evaluate requested candle");

        assert_eq!(result.status, "normal");
        assert!(!result.has_signal);
        assert!(result.signal.is_none());
        assert_eq!(result.window_candle_count, 1);
        assert_eq!(
            result.evaluated_candle.expect("evaluated candle").open_time,
            "2026-07-10T00:01:00Z"
        );
    }

    #[tokio::test]
    async fn empty_source_scoped_signal_evaluation_points_to_diagnostics() {
        let tool = FinanceEvaluateCandleSignalTool::new(repository([]).await);

        let result = tool
            .run(default_params(), &context())
            .await
            .expect("missing stream");

        assert_eq!(result.status, "missing");
        assert_eq!(result.window_status, "missing");
        assert!(!result.has_signal);
        assert!(result.evaluated_candle.is_none());
        let hint = result.diagnostic_hint.expect("diagnostic hint");
        assert_eq!(hint.tool, "finance_diagnose_candle_subscriptions");
        assert_eq!(
            hint.default_params,
            serde_json::json!({
                "source_names": ["binance-spot"],
            })
        );
    }

    #[test]
    fn evaluate_candle_signal_tool_is_read_only() {
        let repository = Arc::new(InMemoryMarketDataRepository::default());
        let tool = FinanceEvaluateCandleSignalTool::new(repository);

        assert!(tool.is_read_only(&serde_json::json!({})));
    }
}
