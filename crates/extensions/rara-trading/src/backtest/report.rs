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
#[derive(Debug, Clone, Copy, PartialEq, Builder)]
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
