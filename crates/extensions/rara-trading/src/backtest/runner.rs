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

//! The replay loop: the pure, deterministic core and the thin async entry.
//!
//! [`run_backtest`] walks an ordered single-stream candle slice, replays each
//! bar through `anomaly::evaluate` under a strict no-look-ahead window, applies
//! one fixed naive-long rule, and computes the [`BacktestReport`]. It is pure
//! (no clock, no I/O), so the whole report is reproducible from a fixture —
//! this is the seam the unit tests bind to. [`backtest`] is the thin async
//! entry that fetches the stream via [`MarketDataRepository::candles`] and
//! delegates to the same core.

use rust_decimal::prelude::ToPrimitive;
use snafu::ResultExt;

use super::{
    error::{BacktestError, EvaluateSnafu, Result},
    report::BacktestReport,
};
use crate::{
    anomaly::{self, EVAL_WINDOW},
    market_data::{CandleRangeQuery, MarketCandle, MarketDataRepository},
};

/// Bars held between the naive rule's entry and exit: enter at a trigger bar's
/// close, exit `HOLD_BARS` bars later at that bar's close.
///
/// This is a **mechanism constant** (`docs/guides/anti-patterns.md`): the hold
/// horizon tunes the single fixed rule, and a deploy operator has no principled
/// reason to retune it — a YAML knob would recreate the #1804→#1817 footgun. A
/// selectable/parameterized horizon is explicitly out of scope (that is the
/// heavier research-desk tool, a separate later issue).
pub const HOLD_BARS: usize = 3;

/// Replay `candles` (an ordered single-stream slice) through
/// `anomaly::evaluate` and the fixed naive-long rule, returning the
/// deterministic report.
///
/// For the bar at index `i`, signal evaluation sees only the
/// up-to-`EVAL_WINDOW` bars strictly before `i`
/// (`candles[i.saturating_sub(EVAL_WINDOW)..i]`) plus `latest = candles[i]` —
/// never a bar at index `> i`. This mirrors the production invariant
/// (`dispatch` queries `recent_candles` with `end = Some(latest.open_time)`),
/// so the same trigger set the live pipeline would have produced is reproduced
/// here. On a trigger, forward return reads only bars strictly after `i` (up to
/// `candles[i + HOLD_BARS]`); a trigger with fewer than `HOLD_BARS` bars
/// remaining is counted in `trigger_count` but excluded from every P&L metric —
/// never zero-filled.
///
/// # Errors
///
/// Returns [`BacktestError::Evaluate`] if `anomaly::evaluate` rejects a candle
/// (e.g. a non-positive close) — the replay stops rather than producing a
/// report from invalid data.
pub fn run_backtest(candles: &[MarketCandle]) -> Result<BacktestReport> {
    // Pass 1: collect the trigger set under the strict no-look-ahead window.
    let mut trigger_indices = Vec::new();
    for index in 0..candles.len() {
        let window = &candles[index.saturating_sub(EVAL_WINDOW)..index];
        let latest = &candles[index];
        if anomaly::evaluate(window, latest)
            .context(EvaluateSnafu)?
            .is_some()
        {
            trigger_indices.push(index);
        }
    }
    let trigger_count = trigger_indices.len();

    // Pass 2: the naive-long forward return for every trigger that has a full
    // HOLD_BARS forward window, in trigger-time order. A trigger without one is
    // silently skipped here (never fabricated) yet still counted above.
    let forward_returns: Vec<f64> = trigger_indices
        .iter()
        .filter_map(|&index| {
            let exit = index + HOLD_BARS;
            (exit < candles.len()).then(|| {
                let entry_close = close_f64(&candles[index]);
                let exit_close = close_f64(&candles[exit]);
                (exit_close - entry_close) / entry_close
            })
        })
        .collect();

    let evaluated_trade_count = forward_returns.len();
    let win_count = forward_returns.iter().filter(|value| **value > 0.0).count();

    let (win_rate, mean_forward_return, median_forward_return) = if evaluated_trade_count == 0 {
        // Well-defined empty result: None, never a NaN from a zero divisor.
        (None, None, None)
    } else {
        let mean = forward_returns.iter().sum::<f64>() / evaluated_trade_count as f64;
        (
            Some(win_count as f64 / evaluated_trade_count as f64),
            Some(mean),
            median(&forward_returns),
        )
    };

    Ok(BacktestReport::builder()
        .trigger_count(trigger_count)
        .evaluated_trade_count(evaluated_trade_count)
        .win_count(win_count)
        .maybe_win_rate(win_rate)
        .maybe_mean_forward_return(mean_forward_return)
        .maybe_median_forward_return(median_forward_return)
        .max_drawdown(strategy_max_drawdown(&forward_returns))
        .build())
}

/// Fetch the single-stream candle history for `query` and delegate to
/// [`run_backtest`].
///
/// This is the `evaluate` / `evaluate_with` seam pattern the crate already
/// uses: the I/O (one repository read) lives here, the deterministic
/// computation lives in the pure core. It reads a single
/// source/venue/symbol/timeframe/range and changes neither the repository nor
/// the evaluator.
///
/// # Errors
///
/// Returns [`BacktestError::FetchCandles`] if the repository read fails, or
/// [`BacktestError::Evaluate`] if the pure core rejects a candle.
pub async fn backtest(
    repo: &dyn MarketDataRepository,
    query: CandleRangeQuery,
) -> Result<BacktestReport> {
    let candles = repo
        .candles(query)
        .await
        .map_err(|source| BacktestError::FetchCandles {
            source: source.into(),
        })?;
    run_backtest(&candles)
}

/// Strictly-positive `f64` close of a candle already validated by
/// `anomaly::evaluate`.
///
/// `run_backtest` calls `evaluate` for every bar as `latest` before this runs,
/// and `evaluate` rejects any non-positive or non-finite close; so by the time
/// forward return reads an entry/exit close it is known valid. The `expect`
/// documents that invariant rather than hiding a fabricated fallback.
fn close_f64(candle: &MarketCandle) -> f64 {
    candle
        .close
        .to_f64()
        .filter(|value| value.is_finite() && *value > 0.0)
        .expect("close validated as strictly positive and finite by anomaly::evaluate")
}

/// Median of `values`, or `None` when empty. Copies into a scratch buffer so
/// the caller's order (trigger-time order) is preserved. Signed — the median of
/// a bearish tail of returns is negative and reported as such.
fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|left, right| left.total_cmp(right));
    let mid = sorted.len() / 2;
    let value = if sorted.len() % 2 == 0 {
        f64::midpoint(sorted[mid - 1], sorted[mid])
    } else {
        sorted[mid]
    };
    Some(value)
}

/// Deepest peak-to-trough fractional decline of the naive strategy's equity
/// curve — the cumulative product of `(1 + forward_return)` over evaluated
/// trades in trigger-time order. `0.0` when there are no trades. This is a
/// whole-strategy drawdown, not the worst single trade.
fn strategy_max_drawdown(forward_returns: &[f64]) -> f64 {
    let mut equity = 1.0_f64;
    let mut peak = 1.0_f64;
    let mut worst = 0.0_f64;
    for &forward_return in forward_returns {
        equity *= 1.0 + forward_return;
        peak = peak.max(equity);
        if peak > 0.0 {
            worst = worst.max((peak - equity) / peak);
        }
    }
    worst
}

#[cfg(test)]
mod tests {
    use jiff::{SignedDuration, Timestamp};
    use rust_decimal::Decimal;

    use super::{HOLD_BARS, backtest, run_backtest};
    use crate::{
        anomaly::AnomalyError,
        backtest::BacktestError,
        market_data::{
            CandleRangeQuery, InMemoryMarketDataRepository, MarketCandle, MarketDataRepository,
            Timeframe,
        },
    };

    /// Tolerance for signed forward-return / drawdown comparisons.
    const APPROX: f64 = 1e-12;
    /// Fixed 15m step used to space fixture candles (seconds).
    const STEP_SECS: i64 = 900;

    /// Build a candle whose OHLC all equal `close` (signals read closes only)
    /// at a distinct, increasing open time, mirroring the anomaly
    /// evaluator's own fixture helper.
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
            provider_sequence: None,
        }
    }

    /// Assemble an ordered single-stream fixture from parallel price/volume
    /// rows.
    fn stream(prices: &[i64], volumes: &[i64]) -> Vec<MarketCandle> {
        assert_eq!(prices.len(), volumes.len(), "price/volume rows must align");
        prices
            .iter()
            .zip(volumes)
            .enumerate()
            .map(|(index, (&price, &volume))| candle(index as i64, price, volume))
            .collect()
    }

    /// The scenario-1 fixture: two volume-surge triggers (bars 5 and 9), each
    /// with a full `HOLD_BARS` forward window, over a gentle price path that
    /// trips no price-based signal. Trade 1 (5→8) rises +1%, trade 2 (9→12)
    /// falls back, so one trade wins and one loses.
    fn winning_and_losing_fixture() -> Vec<MarketCandle> {
        stream(
            &[
                60000, 60000, 60000, 60000, 60000, 60000, 60200, 60400, 60600, 60600, 60400, 60200,
                60000, 60000, 60000, 60000,
            ],
            &[
                100, 100, 100, 100, 100, 400, 100, 100, 100, 400, 100, 100, 100, 100, 100, 100,
            ],
        )
    }

    #[test]
    fn naive_long_backtest_reports_deterministic_metrics_on_fixture() {
        let candles = winning_and_losing_fixture();
        let report = run_backtest(&candles).expect("positive prices");

        // Hand-computed from the fixture: enter at the trigger close, exit
        // HOLD_BARS (=3) bars later at that close.
        let win_return = (60600.0 - 60000.0) / 60000.0; // bar 5 → bar 8: +1%
        let loss_return = (60000.0 - 60600.0) / 60600.0; // bar 9 → bar 12: <0

        assert_eq!(report.trigger_count, 2);
        assert_eq!(report.evaluated_trade_count, 2);
        // A trade wins exactly when its forward return is strictly positive: the
        // +1% trade counts, the negative one does not.
        assert!(win_return > 0.0 && loss_return < 0.0);
        assert_eq!(report.win_count, 1);
        assert!((report.win_rate.expect("evaluated trades") - 0.5).abs() < APPROX);

        let expected_mean = f64::midpoint(win_return, loss_return);
        assert!((report.mean_forward_return.expect("mean") - expected_mean).abs() < APPROX);
        // Two evaluated trades → the median is the midpoint of the two returns.
        assert!((report.median_forward_return.expect("median") - expected_mean).abs() < APPROX);

        // Equity: 1.0 → ×1.01 (peak 1.01) → back to 1.0. Deepest decline is
        // (1.01 − 1.0) / 1.01.
        let expected_drawdown = 0.01 / 1.01;
        assert!((report.max_drawdown - expected_drawdown).abs() < APPROX);
    }

    #[test]
    fn signal_evaluation_never_reads_bars_after_the_trigger() {
        // Both fixtures are byte-identical up to and including the single trigger
        // bar (5); they differ only in the post-trigger forward bars, moved
        // within a band that fires no new signal.
        let flat_tail = stream(
            &[
                60000, 60000, 60000, 60000, 60000, 60000, 60000, 60000, 60000,
            ],
            &[100, 100, 100, 100, 100, 400, 100, 100, 100],
        );
        let moved_tail = stream(
            &[
                60000, 60000, 60000, 60000, 60000, 60000, 60200, 60400, 60600,
            ],
            &[100, 100, 100, 100, 100, 400, 100, 100, 100],
        );

        let flat = run_backtest(&flat_tail).expect("positive prices");
        let moved = run_backtest(&moved_tail).expect("positive prices");

        // The trigger set is identical — one trigger at bar 5 in both. Had
        // evaluation peeked at a future bar, the moved forward bars would have
        // shifted the trigger set between the two runs.
        assert_eq!(flat.trigger_count, moved.trigger_count);
        assert_eq!(flat.trigger_count, 1);
        assert_eq!(flat.evaluated_trade_count, moved.evaluated_trade_count);
        assert_eq!(flat.evaluated_trade_count, 1);

        // The forward-return metrics differ — proving forward return actually
        // reads the post-trigger window, so the trigger-set assertion is not
        // vacuous. Flat tail → 0% (no win); moved tail → +1% (a win).
        assert_eq!(flat.win_count, 0);
        assert_eq!(moved.win_count, 1);
        assert!((flat.mean_forward_return.expect("flat mean")).abs() < APPROX);
        assert!(
            (moved.mean_forward_return.expect("moved mean") - 0.01).abs() < APPROX,
            "moved tail forward return should be +1%"
        );
        assert_ne!(flat.mean_forward_return, moved.mean_forward_return);
    }

    #[test]
    fn forward_return_excludes_triggers_without_full_hold_window() {
        // The lone trigger is at bar 6 of an 8-bar stream: exit bar 6+HOLD_BARS
        // (=9) is past the end, so the trade has no full forward window.
        let candles = stream(
            &[60000, 60000, 60000, 60000, 60000, 60000, 60000, 60000],
            &[100, 100, 100, 100, 100, 100, 400, 100],
        );
        assert!(
            6 + HOLD_BARS >= candles.len(),
            "trigger must lack a full window"
        );

        let report = run_backtest(&candles).expect("positive prices");

        // Counted as a trigger, but excluded from the P&L denominator — never
        // fabricated or zero-filled.
        assert_eq!(report.trigger_count, 1);
        assert_eq!(report.evaluated_trade_count, 0);
        assert_eq!(report.win_count, 0);
        assert_eq!(report.win_rate, None);
        assert_eq!(report.mean_forward_return, None);
        assert_eq!(report.median_forward_return, None);
        assert_eq!(report.max_drawdown, 0.0);
    }

    #[test]
    fn flat_tape_yields_zero_triggers_and_empty_report() {
        // Tiny alternating moves at steady volume: no signal ever fires.
        let candles = stream(
            &[
                60000, 60010, 60000, 60010, 60000, 60010, 60000, 60010, 60000, 60010, 60000, 60010,
                60000, 60010,
            ],
            &[100; 14],
        );

        let report = run_backtest(&candles).expect("positive prices");

        assert_eq!(report.trigger_count, 0);
        assert_eq!(report.evaluated_trade_count, 0);
        assert_eq!(report.win_rate, None);
        assert_eq!(report.mean_forward_return, None);
        assert_eq!(report.median_forward_return, None);
        assert_eq!(report.max_drawdown, 0.0);
        // The empty result is well-defined, never a NaN.
        assert!(!report.max_drawdown.is_nan());
    }

    #[tokio::test]
    async fn backtest_pulls_stream_via_repository_candles_and_matches_pure_core() {
        let candles = winning_and_losing_fixture();

        let repo = InMemoryMarketDataRepository::default();
        for candle in &candles {
            repo.upsert_closed_candle(candle.clone())
                .await
                .expect("seed candle");
        }

        let last_open = candles.last().expect("non-empty fixture").open_time;
        let query = CandleRangeQuery {
            source_name: Some("binance-spot".to_owned()),
            venue:       "binance".to_owned(),
            symbol:      "BTCUSDT".to_owned(),
            timeframe:   Timeframe::parse("15m").expect("timeframe parses"),
            start:       candles.first().expect("non-empty fixture").open_time,
            // Exclusive end one step past the last bar so the full stream is in
            // range.
            end:         last_open + SignedDuration::from_secs(STEP_SECS),
            limit:       1_000,
        };

        let via_repo = backtest(&repo, query).await.expect("repository backtest");
        let via_core = run_backtest(&candles).expect("pure-core backtest");

        // The repository→core wiring produces exactly the pure-core report.
        assert_eq!(via_repo, via_core);
        // Non-vacuous: the fixture actually exercises trades.
        assert_eq!(via_core.evaluated_trade_count, 2);
    }

    #[test]
    fn non_positive_close_fails_backtest_with_typed_error() {
        // Bar 2 carries a zero close; the anomaly evaluator rejects it.
        let candles = stream(&[60000, 60000, 0, 60000, 60000], &[100, 100, 100, 100, 100]);

        let error = run_backtest(&candles).expect_err("non-positive close is invalid");

        // A typed BacktestError propagating the NonPositivePrice cause — not a
        // report computed from the invalid data.
        assert!(matches!(
            error,
            BacktestError::Evaluate {
                source: AnomalyError::NonPositivePrice { .. },
            }
        ));
    }
}
