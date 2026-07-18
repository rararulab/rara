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

//! Signal-accuracy backtest harness: the first rung of layer ② (decision
//! support).
//!
//! Layer ① (`anomaly` + `dispatch`) lets rara *see* the market — `evaluate`
//! fires a composite `AnomalySignal` on each closed candle and `dispatch`
//! delivers it. This module answers the next question: **is a signal actually
//! any good?** It replays a single symbol/timeframe stream of stored candles in
//! time order through the same `anomaly::evaluate` and, for every bar the
//! evaluator fires on, applies one **fixed naive rule** — enter a long at the
//! trigger bar's close, exit `HOLD_BARS` bars later at that bar's close — then
//! reports a **fixed metric set** ([`BacktestReport`]): trigger count, win
//! rate, signed mean/median forward return, and the naive strategy's max
//! drawdown. It also reports [`SignalAttributionReport`], the same fixed
//! forward-return metrics grouped by each builtin signal that contributed to
//! composite anomalies.
//!
//! Two seams, mirroring the `anomaly` module:
//! - [`run_backtest`] — the **pure, deterministic core** over an ordered candle
//!   slice (no clock, no I/O); the unit tests bind here.
//! - [`backtest`] — the **thin async entry** that fetches the stream via
//!   [`MarketDataRepository::candles`](crate::market_data::MarketDataRepository::candles)
//!   and delegates to the core.
//!
//! **No look-ahead** is the number-one backtest bug and is enforced
//! structurally: signal evaluation for bar `i` sees only the `EVAL_WINDOW` bars
//! strictly before `i` plus `latest = candles[i]`, and forward return reads
//! only bars strictly after `i`. A trigger without a full `HOLD_BARS` forward
//! window is counted but excluded from the P&L denominator — never fabricated
//! or zero-filled. Because the detectors are tail/volatility signals, a low win
//! rate with a negative mean forward return is a valid, informative result; the
//! signed returns are reported without taking absolute value.
//!
//! This is a deliberately narrow first cut: one fixed rule, one fixed metric
//! set, a single stream. No strategy DSL, parameter search, multi-asset
//! portfolio, cost model, or execution wiring — those stay out of scope.

mod error;
mod report;
mod runner;
pub mod tools;

pub use error::{BacktestError, Result};
pub use report::{BacktestReport, SignalAttribution, SignalAttributionReport};
pub use runner::{HOLD_BARS, backtest, run_backtest, run_signal_attribution};
