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

use rara_kernel::data_feed::{DataFeedConfig, FeedType};

pub(super) const DEFAULT_MARKET_CANDLE_REQUEST_BUDGET_PER_SECOND: f64 = 10.0;
const MAX_MARKET_CANDLE_REQUEST_BUDGET_PER_SECOND: f64 = 100.0;
const MIN_MARKET_CANDLE_INTERVAL_SECS: u64 = 5;

#[derive(Debug, Clone)]
pub(super) struct MarketCandleFanoutSafety {
    pub(super) stream_count:                  usize,
    pub(super) poll_request_count:            usize,
    pub(super) configured_interval_secs:      u64,
    pub(super) estimated_requests_per_second: f64,
    pub(super) request_budget_per_second:     f64,
    pub(super) minimum_safe_interval_secs:    u64,
    pub(super) safe_to_start:                 bool,
}

pub(super) fn validate_market_candle_request_budget(budget: f64) -> anyhow::Result<()> {
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

pub(super) fn market_candle_fanout_safety(
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
    let poll_request_count = if provider == Some("binance") {
        stream_count
    } else {
        1
    };
    let estimated_requests_per_second = poll_request_count as f64 / configured_interval_secs as f64;
    let minimum_safe_interval_secs = MIN_MARKET_CANDLE_INTERVAL_SECS
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

pub(super) fn market_candle_config_fanout_safety(
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

pub(super) fn unsafe_market_candle_fanout_message(safety: &MarketCandleFanoutSafety) -> String {
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
