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

//! The anomaly-evaluation entry point.
//!
//! [`evaluate`] is a pure function of its candle inputs — no clock, no I/O — so
//! every rule and statistic is reproducible from a fixture window. It composes
//! the L1 rules ([`super::rules`]) and L2 statistics ([`super::statistics`])
//! into a single [`AnomalySignal`], or `None` when the tape is unremarkable.
//!
//! The thresholds below are **mechanism constants**
//! (`docs/guides/anti-patterns.md`): a deploy operator has no principled reason
//! to retune the drawdown or volume-surge trip points, and a YAML knob would
//! recreate the #1804→#1817 footgun where a default config silently disables
//! the fix. The one value a deployment might legitimately tune (per-symbol
//! sensitivity) is out of scope for this issue and would arrive via
//! `config.example.yaml`, never as a hardcoded Rust default.

use rust_decimal::prelude::ToPrimitive;

use super::{
    error::{NonPositivePriceSnafu, Result},
    rules,
    signal::{AnomalyMetrics, AnomalySignal, Severity},
    statistics,
};
use crate::market_data::MarketCandle;

/// Number of preceding candles the caller pulls into the rolling window. Wide
/// enough to give the MAD scale and bipower variation a stable sample, small
/// enough to stay responsive to a regime change.
pub const EVAL_WINDOW: usize = 30;

/// Absolute cumulative window return (fraction) that trips the return rule.
const WINDOW_RETURN_THRESHOLD: f64 = 0.03;
/// Rolling max-drawdown magnitude (fraction) that trips the drawdown rule.
const DRAWDOWN_THRESHOLD: f64 = 0.03;
/// Volume-vs-rolling-mean multiple that trips the volume-surge rule.
const VOLUME_SURGE_THRESHOLD: f64 = 3.0;
/// Robust z-score magnitude that trips the return-anomaly statistic.
const ZSCORE_THRESHOLD: f64 = 3.5;
/// BNS jump ratio above which the path is flagged as containing a jump.
const JUMP_RATIO_THRESHOLD: f64 = 1.5;
/// Drawdown magnitude (fraction) that escalates a multi-rule anomaly to
/// [`Severity::Critical`] — the flash-crash shape.
const CRITICAL_DRAWDOWN: f64 = 0.06;

/// Evaluate the newly closed `latest` candle against its preceding `window`
/// (ordered oldest-to-newest, excluding `latest`).
///
/// Returns `Ok(None)` when nothing anomalous fired or the window is too short
/// to judge, `Ok(Some(signal))` when at least one rule or statistic tripped,
/// and `Err` only when a candle carries a non-positive close (log-returns
/// undefined) — a structurally invalid input the caller logs rather than
/// silently drops.
///
/// # Errors
///
/// Returns [`super::AnomalyError::NonPositivePrice`] if any close in the window
/// or the latest candle is not strictly positive.
pub fn evaluate(window: &[MarketCandle], latest: &MarketCandle) -> Result<Option<AnomalySignal>> {
    let mut closes = Vec::with_capacity(window.len() + 1);
    for candle in window {
        closes.push(close_f64(candle)?);
    }
    closes.push(close_f64(latest)?);

    // One prior close is the minimum needed to form a single return.
    if closes.len() < 2 {
        return Ok(None);
    }

    let history_volumes: Vec<f64> = window.iter().map(volume_f64).collect();
    let latest_volume = volume_f64(latest);

    let returns = statistics::log_returns(&closes);
    let (&newest_return, history_returns) = returns
        .split_last()
        .expect("returns is non-empty when closes has at least two entries");

    let jump_ratio = statistics::jump_ratio(&returns);
    let metrics = AnomalyMetrics::builder()
        .window_return(rules::window_return(&closes))
        .max_drawdown(rules::max_drawdown(&closes))
        .maybe_volume_surge(rules::volume_surge(&history_volumes, latest_volume))
        .maybe_robust_zscore(statistics::robust_zscore(history_returns, newest_return))
        .maybe_jump_ratio(jump_ratio)
        .jump_flagged(jump_ratio.is_some_and(|ratio| ratio >= JUMP_RATIO_THRESHOLD))
        .build();

    Ok(classify(metrics))
}

/// Extract a strictly-positive `f64` close, or fail with a typed error.
///
/// Statistical intermediates use `f64` deliberately: log-returns and z-scores
/// are dimensionless ratios, not ledger amounts, so the precision the candle's
/// `Decimal` carries is irrelevant once we divide two prices.
fn close_f64(candle: &MarketCandle) -> Result<f64> {
    match candle.close.to_f64().filter(|value| value.is_finite()) {
        Some(value) if value > 0.0 => Ok(value),
        _ => NonPositivePriceSnafu {
            symbol:    candle.symbol.clone(),
            open_time: candle.open_time.to_string(),
            close:     candle.close.to_string(),
        }
        .fail(),
    }
}

/// Volume as `f64`; a missing or non-finite value degrades to `0.0`, which the
/// surge rule treats as "no volume", not as an error.
fn volume_f64(candle: &MarketCandle) -> f64 {
    candle
        .volume
        .to_f64()
        .filter(|value| value.is_finite())
        .unwrap_or(0.0)
}

/// Map computed metrics to a severity and reason, or `None` when no rule fired.
fn classify(metrics: AnomalyMetrics) -> Option<AnomalySignal> {
    let return_fired = metrics.window_return.abs() >= WINDOW_RETURN_THRESHOLD;
    let drawdown_fired = metrics.max_drawdown >= DRAWDOWN_THRESHOLD;
    let volume_fired = metrics
        .volume_surge
        .is_some_and(|value| value >= VOLUME_SURGE_THRESHOLD);
    let zscore_fired = metrics
        .robust_zscore
        .is_some_and(|value| value >= ZSCORE_THRESHOLD);
    let jump_fired = metrics.jump_flagged;

    let fired_count = [
        return_fired,
        drawdown_fired,
        volume_fired,
        zscore_fired,
        jump_fired,
    ]
    .into_iter()
    .filter(|&fired| fired)
    .count();
    if fired_count == 0 {
        return None;
    }

    let severity = if metrics.max_drawdown >= CRITICAL_DRAWDOWN && fired_count >= 2 {
        Severity::Critical
    } else if fired_count >= 2 {
        Severity::Elevated
    } else {
        Severity::Watch
    };

    let mut parts = Vec::new();
    if drawdown_fired {
        parts.push(format!("drawdown {:.1}%", metrics.max_drawdown * 100.0));
    }
    if return_fired {
        parts.push(format!(
            "window return {:+.1}%",
            metrics.window_return * 100.0
        ));
    }
    if let Some(surge) = metrics.volume_surge.filter(|_| volume_fired) {
        parts.push(format!("volume surge {surge:.1}x"));
    }
    if let Some(zscore) = metrics.robust_zscore.filter(|_| zscore_fired) {
        parts.push(format!("robust z-score {zscore:.1}"));
    }
    if let Some(ratio) = metrics.jump_ratio.filter(|_| jump_fired) {
        parts.push(format!("jump ratio {ratio:.1}"));
    }

    let reason = format!("{} anomaly — {}", severity.label(), parts.join(", "));
    Some(
        AnomalySignal::builder()
            .severity(severity)
            .reason(reason)
            .metrics(metrics)
            .build(),
    )
}

#[cfg(test)]
mod tests {
    use jiff::{SignedDuration, Timestamp};
    use rust_decimal::Decimal;

    use super::evaluate;
    use crate::{
        anomaly::{error::AnomalyError, signal::Severity},
        market_data::{MarketCandle, Timeframe},
    };

    /// Build a candle whose open/high/low/close all equal `close` (drawdown and
    /// returns read closes only) at a distinct, increasing open time.
    fn candle(index: i64, close: i64, volume: i64) -> MarketCandle {
        let base: Timestamp = "2026-07-10T00:00:00Z"
            .parse()
            .expect("base timestamp parses");
        let open_time = base + SignedDuration::from_secs(index * 900);
        MarketCandle {
            source_name: "binance-spot".to_owned(),
            venue: "binance".to_owned(),
            symbol: "BTCUSDT".to_owned(),
            timeframe: Timeframe::parse("15m").expect("timeframe parses"),
            open_time,
            close_time: open_time + SignedDuration::from_secs(900),
            open: Decimal::from(close),
            high: Decimal::from(close),
            low: Decimal::from(close),
            close: Decimal::from(close),
            volume: Decimal::from(volume),
            ingested_at: open_time,
            provider_sequence: None,
        }
    }

    fn window(candles: &[(i64, i64)]) -> Vec<MarketCandle> {
        candles
            .iter()
            .enumerate()
            .map(|(index, &(close, volume))| candle(index as i64, close, volume))
            .collect()
    }

    #[test]
    fn crash_window_produces_high_severity_anomaly_signal() {
        // Six flat bars, then a four-bar decline, all at steady volume.
        let history = window(&[
            (61_500, 120),
            (61_500, 120),
            (61_500, 120),
            (61_500, 120),
            (61_500, 120),
            (61_500, 120),
            (61_000, 120),
            (60_000, 120),
            (59_000, 120),
            (58_000, 120),
        ]);
        // The newly closed bar completes a ~7% drop on a 5x volume spike.
        let latest = candle(10, 57_000, 600);

        let signal = evaluate(&history, &latest)
            .expect("positive prices")
            .expect("crash shape yields a signal");

        assert_eq!(signal.severity, Severity::Critical);
        assert!(
            signal.reason.contains("drawdown") && signal.reason.contains("window return"),
            "reason should name the drawdown and return magnitudes: {}",
            signal.reason
        );
        assert!(signal.metrics.max_drawdown >= 0.06);
        assert!(signal.metrics.window_return <= -0.06);
        assert!(signal.metrics.volume_surge.expect("surge computed") >= 3.0);
    }

    #[test]
    fn flat_tape_produces_no_anomaly_signal() {
        let history = window(&[
            (61_500, 120),
            (61_510, 120),
            (61_500, 120),
            (61_510, 120),
            (61_500, 120),
            (61_510, 120),
            (61_500, 120),
            (61_510, 120),
            (61_500, 120),
            (61_510, 120),
            (61_500, 120),
            (61_510, 120),
        ]);
        let latest = candle(12, 61_505, 122);

        let signal = evaluate(&history, &latest).expect("positive prices");
        assert!(signal.is_none(), "flat tape should not enrich: {signal:?}");
    }

    #[test]
    fn single_moderate_rule_produces_watch_severity() {
        // A steady ~4% rise: only the window-return rule trips, no drawdown, no
        // volume surge, no return-outlier.
        let history: Vec<MarketCandle> = (0..12)
            .map(|index| candle(index, 60_000 + 200 * index, 120))
            .collect();
        let latest = candle(12, 62_400, 120);

        let signal = evaluate(&history, &latest)
            .expect("positive prices")
            .expect("a 4% run trips the return rule");

        assert_eq!(signal.severity, Severity::Watch);
        assert!(signal.metrics.window_return >= 0.03);
        assert!(signal.metrics.max_drawdown < 0.03);
    }

    #[test]
    fn two_rules_without_deep_drawdown_produce_elevated_severity() {
        // A ~4% single-bar drop on a volume surge: return + drawdown + volume
        // fire, but the drawdown stays under the critical 6% floor.
        let history = window(&[
            (61_500, 120),
            (61_500, 120),
            (61_500, 120),
            (61_500, 120),
            (61_500, 120),
            (61_500, 120),
            (61_500, 120),
            (61_500, 120),
            (61_500, 120),
            (61_500, 120),
            (61_500, 120),
        ]);
        let latest = candle(11, 59_000, 450);

        let signal = evaluate(&history, &latest)
            .expect("positive prices")
            .expect("multi-rule anomaly");

        assert_eq!(signal.severity, Severity::Elevated);
        assert!(signal.metrics.max_drawdown < 0.06);
        assert!(signal.metrics.volume_surge.expect("surge computed") >= 3.0);
    }

    #[test]
    fn non_positive_close_is_a_typed_error() {
        let history = window(&[(61_500, 120), (61_500, 120)]);
        let latest = candle(2, 0, 120);

        let error = evaluate(&history, &latest).expect_err("zero close is invalid");
        assert!(matches!(error, AnomalyError::NonPositivePrice { .. }));
    }
}
