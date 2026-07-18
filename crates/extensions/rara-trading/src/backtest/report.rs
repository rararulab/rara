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

//! The fixed metric set the backtest harness reports.

use bon::Builder;
use serde::Serialize;

/// The result of replaying one candle stream through the naive-long rule.
///
/// Every field is stream-wide over the composite `AnomalySignal` triggers.
/// Exposing both [`Self::trigger_count`] and [`Self::evaluated_trade_count`]
/// keeps the end-of-stream exclusion inspectable rather than silently dropped:
/// a trigger too close to the end of the stream to have a full `HOLD_BARS`
/// forward window counts in `trigger_count` but not in `evaluated_trade_count`.
///
/// The forward-return metrics are **signed** — because rara's detectors are
/// tail/volatility signals rather than directional buy signals, a low
/// [`Self::win_rate`] paired with a negative [`Self::mean_forward_return`] is a
/// valid, informative result (the signals lean bearish), not a broken harness.
/// The `Option` fields are `None` — never `NaN` — when there are no evaluated
/// trades to average over.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Builder)]
pub struct BacktestReport {
    /// Bars where `anomaly::evaluate` returned `Some` (the composite signal
    /// fired).
    pub trigger_count:         usize,
    /// Triggers that have a full `HOLD_BARS` forward window — the denominator
    /// for the win-rate and forward-return metrics.
    pub evaluated_trade_count: usize,
    /// Evaluated trades whose forward return is strictly positive.
    pub win_count:             usize,
    /// Fraction of evaluated trades that won (`win_count /
    /// evaluated_trade_count`), or `None` when there are no evaluated
    /// trades.
    pub win_rate:              Option<f64>,
    /// Signed arithmetic mean forward return across evaluated trades, or `None`
    /// when there are no evaluated trades.
    pub mean_forward_return:   Option<f64>,
    /// Signed median forward return across evaluated trades, or `None` when
    /// there are no evaluated trades.
    pub median_forward_return: Option<f64>,
    /// Deepest peak-to-trough fractional decline of the naive strategy's equity
    /// curve — the cumulative product of `(1 + per-trade forward return)` over
    /// evaluated trades in trigger-time order. `0.0` when there are no trades.
    pub max_drawdown:          f64,
}

/// Per-signal attribution for the same replay and naive-long rule.
///
/// The composite [`BacktestReport`] answers "when rara would have emitted an
/// anomaly, what happened next?". This report answers the next research
/// question: "which builtin signals contributed to those anomalies, and what
/// happened after each signal fired?". A bar can contribute to multiple rows
/// when multiple signals fire on the same candle, so per-signal
/// [`SignalAttribution::trigger_count`] values intentionally do not sum to the
/// composite trigger count.
#[derive(Debug, Clone, PartialEq, Serialize, Builder)]
pub struct SignalAttributionReport {
    /// Composite anomaly triggers from the same replay, for context.
    pub composite_trigger_count: usize,
    /// Builtin signals in stable registry order, including zero-count rows so
    /// absence is explicit.
    pub signals:                 Vec<SignalAttribution>,
}

/// One builtin signal's trigger and naive-long forward-return metrics.
#[derive(Debug, Clone, PartialEq, Serialize, Builder)]
pub struct SignalAttribution {
    /// Stable signal identifier such as `volume_surge` or `directional_run`.
    pub signal_name:           String,
    /// Bars where this individual signal fired.
    pub trigger_count:         usize,
    /// Triggers that have a full `HOLD_BARS` forward window.
    pub evaluated_trade_count: usize,
    /// Evaluated trades whose forward return is strictly positive.
    pub win_count:             usize,
    /// Fraction of evaluated trades that won, or `None` when no full forward
    /// window exists.
    pub win_rate:              Option<f64>,
    /// Signed arithmetic mean forward return, or `None` when no full forward
    /// window exists.
    pub mean_forward_return:   Option<f64>,
    /// Signed median forward return, or `None` when no full forward window
    /// exists.
    pub median_forward_return: Option<f64>,
    /// Deepest peak-to-trough fractional decline of this signal's naive
    /// strategy equity curve. `0.0` when there are no evaluated trades.
    pub max_drawdown:          f64,
}
