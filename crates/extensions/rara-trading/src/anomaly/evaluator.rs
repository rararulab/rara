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
//! every rule and statistic is reproducible from a fixture window. It prepares
//! the shared per-evaluation context once, walks the builtin [`SignalRegistry`]
//! collecting each signal's [`SignalOutput`], projects those onto the public
//! [`AnomalyMetrics`], and classifies severity — producing a single
//! [`AnomalySignal`], or `None` when the tape is unremarkable.
//!
//! Adding a signal is "implement [`Signal`] + register one line" in
//! [`super::registry`]; the loop below does not change. The seam that makes
//! that testable is [`evaluate_with`], which takes an explicit registry.
//!
//! The drawdown escalation threshold below is a **mechanism constant**
//! (`docs/guides/anti-patterns.md`): a deploy operator has no principled reason
//! to retune it, and a YAML knob would recreate the #1804→#1817 footgun where a
//! default config silently disables the fix. Per-signal trip thresholds live as
//! `const` next to each signal in [`super::rules`] / [`super::statistics`].

use rust_decimal::prelude::ToPrimitive;

use super::{
    error::{NonPositivePriceSnafu, Result},
    registry::{Signal, SignalContext, SignalOutput, SignalRegistry, builtin_registry},
    rules::{DirectionalRunSignal, MaxDrawdownSignal, VolumeSurgeSignal, WindowReturnSignal},
    signal::{AnomalyMetrics, AnomalySignal, Severity},
    statistics::{self, JumpSignal, RobustZScoreSignal, VolatilityRegimeSignal},
};
use crate::market_data::MarketCandle;

/// Number of preceding candles the caller pulls into the rolling window. Wide
/// enough to give the MAD scale and bipower variation a stable sample, small
/// enough to stay responsive to a regime change.
pub const EVAL_WINDOW: usize = 30;

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
    evaluate_with(&builtin_registry(), window, latest)
}

/// Evaluate `latest` against `window` using an explicit signal `registry`.
///
/// [`evaluate`] delegates here with [`builtin_registry`]; this is the seam a
/// test uses to inject an extra signal and prove it participates without any
/// edit to the walk below. The shared inputs (`closes`, log-`returns`,
/// historical/latest volumes) are prepared once and handed to every signal.
///
/// # Errors
///
/// Returns [`super::AnomalyError::NonPositivePrice`] if any close in the window
/// or the latest candle is not strictly positive.
pub(crate) fn evaluate_with(
    registry: &SignalRegistry,
    window: &[MarketCandle],
    latest: &MarketCandle,
) -> Result<Option<AnomalySignal>> {
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

    let ctx = SignalContext::builder()
        .closes(&closes)
        .returns(&returns)
        .history_volumes(&history_volumes)
        .latest_volume(latest_volume)
        .build();

    let evaluated: Vec<(&dyn Signal, SignalOutput)> = registry
        .iter()
        .map(|signal| {
            let output = signal.evaluate(&ctx);
            (signal.as_ref(), output)
        })
        .collect();

    let metrics = metrics_from(&evaluated);
    Ok(classify(&evaluated, metrics))
}

/// Project the collected signal outputs onto the public [`AnomalyMetrics`]
/// trace, routing each builtin signal's value into its field by stable name.
///
/// Each builtin signal projects onto its named field; a signal beyond the
/// builtin set contributes to the reason and severity but not to this trace.
/// `None` values stay `None` (never flattened to `0.0`) so "not evaluable"
/// survives the projection; `window_return` / `max_drawdown` are always
/// computed, so an absent output falls back to their neutral `0.0`.
fn metrics_from(evaluated: &[(&dyn Signal, SignalOutput)]) -> AnomalyMetrics {
    let output = |name: &str| {
        evaluated
            .iter()
            .find(|(signal, _)| signal.name() == name)
            .map(|(_, out)| out)
    };
    let jump = output(JumpSignal::NAME);
    AnomalyMetrics::builder()
        .window_return(
            output(WindowReturnSignal::NAME)
                .and_then(|out| out.value)
                .unwrap_or(0.0),
        )
        .max_drawdown(
            output(MaxDrawdownSignal::NAME)
                .and_then(|out| out.value)
                .unwrap_or(0.0),
        )
        .maybe_volume_surge(output(VolumeSurgeSignal::NAME).and_then(|out| out.value))
        .maybe_robust_zscore(output(RobustZScoreSignal::NAME).and_then(|out| out.value))
        .maybe_jump_ratio(jump.and_then(|out| out.value))
        .jump_flagged(jump.is_some_and(|out| out.fired))
        .maybe_volatility_regime(output(VolatilityRegimeSignal::NAME).and_then(|out| out.value))
        .maybe_directional_run(output(DirectionalRunSignal::NAME).and_then(|out| out.value))
        .build()
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

/// Map the collected signal outputs and their projected metrics to a severity
/// and reason, or `None` when no signal fired.
///
/// Severity is driven by the generic fired-count over the registry plus the
/// concrete flash-crash escalation keyed on the drawdown value (domain logic
/// that stays concrete — not a weighted/scored combination). The reason is the
/// fired signals' own fragments joined in registry order, so a newly registered
/// signal's fragment appears here with no edit to this function.
fn classify(
    evaluated: &[(&dyn Signal, SignalOutput)],
    metrics: AnomalyMetrics,
) -> Option<AnomalySignal> {
    let fired_count = evaluated.iter().filter(|(_, out)| out.fired).count();
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

    let parts: Vec<String> = evaluated
        .iter()
        .filter(|(_, out)| out.fired)
        .map(|(signal, out)| signal.fragment(out))
        .collect();

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

    use super::{evaluate, evaluate_with};
    use crate::{
        anomaly::{
            error::AnomalyError,
            registry::{Signal, SignalContext, SignalOutput, builtin_registry},
            signal::Severity,
        },
        market_data::{MarketCandle, Timeframe},
    };

    /// A test-only signal that always fires with a recognizable fragment, used
    /// to prove a newly registered signal participates in evaluation without
    /// any edit to the core loop. It is `#[cfg(test)]` only — production code
    /// never registers an input-independent signal.
    struct BeaconSignal;

    impl BeaconSignal {
        const NAME: &'static str = "test_beacon";
    }

    impl Signal for BeaconSignal {
        fn name(&self) -> &'static str { Self::NAME }

        fn evaluate(&self, _ctx: &SignalContext<'_>) -> SignalOutput {
            SignalOutput {
                value: Some(1.0),
                fired: true,
            }
        }

        fn fragment(&self, _output: &SignalOutput) -> String { "test beacon fired".to_owned() }
    }

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

    /// Build `(window, latest)` from a close series at steady volume: the last
    /// close is the newly closed `latest` candle, the rest are the window.
    fn steady_volume_window(closes: &[i64]) -> (Vec<MarketCandle>, MarketCandle) {
        let (&last, history) = closes.split_last().expect("at least one close");
        let window: Vec<MarketCandle> = history
            .iter()
            .enumerate()
            .map(|(index, &close)| candle(index as i64, close, 120))
            .collect();
        let latest = candle(history.len() as i64, last, 120);
        (window, latest)
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
    fn registered_signal_participates_in_evaluation() {
        // A flat tape the five builtin signals leave unremarkable.
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

        // Builtin-only registry: the tape is unremarkable, so no signal.
        assert!(
            evaluate(&history, &latest)
                .expect("positive prices")
                .is_none(),
            "the builtin five should leave this flat tape unremarkable"
        );

        // Extend the registry with one always-firing signal — a one-line
        // registration, no edit to the evaluation loop.
        let mut registry = builtin_registry();
        registry.push(Box::new(BeaconSignal));

        let signal = evaluate_with(&registry, &history, &latest)
            .expect("positive prices")
            .expect("the added signal fires, so evaluation now yields a signal");
        assert!(
            signal.reason.contains("test beacon fired"),
            "the added signal's fragment should appear in the collected output: {}",
            signal.reason
        );

        // The same window through the builtin-only registry still returns None,
        // confirming the added signal — not the window — caused the difference.
        assert!(
            evaluate_with(&builtin_registry(), &history, &latest)
                .expect("positive prices")
                .is_none(),
            "the builtin-only registry must stay silent on this window"
        );
    }

    #[test]
    fn single_moderate_rule_produces_watch_severity() {
        // A varied-magnitude rise that nets ~5% (tripping only the
        // window-return rule), ending on a small in-family up-tick preceded by a
        // shallow pullback so the trailing same-sign run is a single bar (below
        // the directional-run threshold). The magnitudes vary, so the MAD scale
        // stays healthy and no bar is a z-score outlier, and recent and baseline
        // per-bar variance match (no regime shift). The old fixture — a 12-bar
        // monotonic rise — is now a persistent grind the directional-run signal
        // legitimately fires on, so it can no longer isolate a single rule; this
        // window restores that isolation.
        let steps = [
            0.004, 0.005, 0.003, 0.006, 0.004, 0.005, 0.003, 0.005, 0.004, 0.006, 0.004, -0.0025,
            0.002,
        ];
        let mut price = 60_000.0_f64;
        let mut closes = vec![price.round() as i64];
        for step in steps {
            price *= 1.0 + step;
            closes.push(price.round() as i64);
        }
        let (history, latest) = steady_volume_window(&closes);

        let signal = evaluate(&history, &latest)
            .expect("positive prices")
            .expect("a > 3% net move trips the return rule");

        assert_eq!(signal.severity, Severity::Watch);
        assert!(signal.metrics.window_return >= 0.03);
        assert!(signal.metrics.max_drawdown < 0.03);
        // Only the window-return rule contributed to the fired count: no other
        // signal's fragment appears in the reason.
        assert!(
            signal.reason.contains("window return"),
            "reason should name the window return: {}",
            signal.reason
        );
        for fragment in [
            "drawdown",
            "volume surge",
            "robust z-score",
            "jump",
            "volatility regime",
            "directional run",
        ] {
            assert!(
                !signal.reason.contains(fragment),
                "only the window-return rule should fire, found {fragment:?}: {}",
                signal.reason
            );
        }
    }

    #[test]
    fn volatility_regime_alone_enriches_unremarkable_tape() {
        // A quiet ±0.15% baseline that shifts into a choppy ±0.8% cluster, with
        // a small in-family newest bar. The dispersion expansion trips only the
        // volatility-regime signal: the net move, drawdown, volume, z-score, and
        // jump path all stay unremarkable, and the alternating cluster leaves no
        // directional run.
        let closes = [
            60_000, 60_090, 60_000, 60_090, 60_000, 60_090, 60_000, 60_090, 60_000, 60_090, 60_480,
            60_000, 60_480, 60_000, 60_090,
        ];
        let (history, latest) = steady_volume_window(&closes);

        let signal = evaluate(&history, &latest)
            .expect("positive prices")
            .expect("the volatility-regime shift now yields a signal");

        assert_eq!(signal.severity, Severity::Watch);
        assert!(
            signal.reason.contains("volatility regime"),
            "reason should carry the volatility-regime fragment: {}",
            signal.reason
        );
        assert!(
            signal.metrics.volatility_regime.expect("ratio computed") >= 4.0,
            "the projected metric carries the ratio: {:?}",
            signal.metrics.volatility_regime
        );
        // None of the five existing signals' fragments, and no directional run.
        for fragment in [
            "window return",
            "drawdown",
            "volume surge",
            "robust z-score",
            "jump",
            "directional run",
        ] {
            assert!(
                !signal.reason.contains(fragment),
                "only the volatility-regime signal should lift the tape, found {fragment:?}: {}",
                signal.reason
            );
        }
    }

    #[test]
    fn directional_run_alone_enriches_unremarkable_tape() {
        // An alternating ±0.4% baseline (no persistent run, uniform variance)
        // followed by six consecutive +0.4% bars. The trailing grind trips only
        // the directional-run signal: the net move stays under 3%, the path has
        // no meaningful drawdown, volume is steady, the newest bar is in-family
        // for the z-score, and the recent variance matches the baseline.
        let closes = [
            60_000, 60_240, 60_000, 60_240, 60_000, 60_240, 60_000, 60_240, 60_000, 60_240, 60_480,
            60_720, 60_960, 61_200, 61_440,
        ];
        let (history, latest) = steady_volume_window(&closes);

        let signal = evaluate(&history, &latest)
            .expect("positive prices")
            .expect("the directional grind now yields a signal");

        assert_eq!(signal.severity, Severity::Watch);
        assert!(
            signal.reason.contains("directional run"),
            "reason should carry the directional-run fragment: {}",
            signal.reason
        );
        assert_eq!(
            signal.metrics.directional_run.expect("run computed"),
            6.0,
            "the projected metric carries the signed run length"
        );
        for fragment in [
            "window return",
            "drawdown",
            "volume surge",
            "robust z-score",
            "jump",
            "volatility regime",
        ] {
            assert!(
                !signal.reason.contains(fragment),
                "only the directional-run signal should lift the tape, found {fragment:?}: {}",
                signal.reason
            );
        }
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
