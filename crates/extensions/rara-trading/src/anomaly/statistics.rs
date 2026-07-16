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

//! L2 robust statistics: a MAD-based z-score and the Barndorff-Nielsen–Shephard
//! (BNS) jump test. Both operate on plain log-return slices so they are pure
//! and unit-testable without candle fixtures.
//!
//! The tuning constants below are **mechanism constants**, not deployment
//! config (`docs/guides/anti-patterns.md`): a deploy operator has no principled
//! reason to pick a different MAD-to-sigma factor or bipower coefficient, and a
//! YAML knob would recreate the #1804→#1817 footgun where a default config
//! silently disables the fix.
//!
//! Each pure statistic is wrapped by a thin [`Signal`] adapter that owns its
//! trip threshold and reason wording, so the registry can walk L1 rules and L2
//! statistics uniformly.

use super::registry::{Signal, SignalContext, SignalOutput};

/// Robust z-score magnitude that trips the return-anomaly statistic.
const ZSCORE_THRESHOLD: f64 = 3.5;
/// BNS jump ratio above which the path is flagged as containing a jump.
const JUMP_RATIO_THRESHOLD: f64 = 1.5;
/// Recent-to-baseline per-bar realized-variance ratio that trips the
/// volatility-regime statistic (recent variance at least this multiple of the
/// baseline).
const VOLATILITY_REGIME_THRESHOLD: f64 = 4.0;

/// Number of trailing returns forming the "recent" variance sample compared
/// against the earlier baseline. Small enough to react to a fresh regime, large
/// enough that a single bar does not dominate the recent per-bar average.
const RECENT_VARIANCE_SAMPLES: usize = 5;

/// Minimum number of returns required before a statistic is trusted. Below this
/// the MAD scale and bipower variation are too noisy to separate signal from
/// sampling error, so the evaluator reports `None` for that statistic.
pub(crate) const MIN_SAMPLES: usize = 8;

/// Consistency constant making the median absolute deviation an unbiased
/// estimator of the standard deviation for normally distributed data
/// (`1 / Φ⁻¹(0.75) ≈ 1.4826`).
const MAD_TO_SIGMA: f64 = 1.4826;

/// Bipower-variation scaling constant `μ₁⁻² = π / 2`, where `μ₁ = E|Z| =
/// √(2/π)` for a standard normal. It rescales the sum of adjacent absolute
/// return products so that, under pure diffusion, bipower variation and
/// realized variance estimate the same integrated variance.
const BIPOWER_SCALE: f64 = std::f64::consts::FRAC_PI_2;

/// Values whose absolute magnitude is at or below this are treated as zero,
/// guarding the divisions in the z-score and jump ratio.
const EPSILON: f64 = 1e-12;

/// Log-returns `ln(cₜ / cₜ₋₁)` of an ordered close series. Empty for fewer than
/// two prices. Callers guarantee all closes are strictly positive, so the
/// logarithm is always defined.
pub(crate) fn log_returns(closes: &[f64]) -> Vec<f64> {
    closes
        .windows(2)
        .map(|pair| (pair[1] / pair[0]).ln())
        .collect()
}

/// Median of a slice, or `None` when empty. Copies into a scratch buffer so the
/// caller's ordering is preserved.
pub(crate) fn median(values: &[f64]) -> Option<f64> {
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

/// Robust z-score of `newest` against the `history` return series, using the
/// median and median absolute deviation (MAD) rather than mean and standard
/// deviation.
///
/// MAD is used deliberately: a single large historical outlier bar inflates the
/// standard deviation and masks a genuinely anomalous fresh move, whereas the
/// median/MAD scale is unmoved by that one bar. Returns `None` when there are
/// fewer than [`MIN_SAMPLES`] history points or the MAD collapses to zero (a
/// perfectly flat history, where no finite z-score is meaningful).
pub(crate) fn robust_zscore(history: &[f64], newest: f64) -> Option<f64> {
    if history.len() < MIN_SAMPLES {
        return None;
    }
    let center = median(history)?;
    let deviations: Vec<f64> = history.iter().map(|value| (value - center).abs()).collect();
    let mad = median(&deviations)?;
    let scale = MAD_TO_SIGMA * mad;
    if scale <= EPSILON {
        return None;
    }
    Some((newest - center).abs() / scale)
}

/// Realized variance: the sum of squared returns. Dominated by any single large
/// jump because each return enters squared.
pub(crate) fn realized_variance(returns: &[f64]) -> f64 {
    returns.iter().map(|value| value * value).sum()
}

/// Ratio of the recent per-bar realized variance (the last
/// [`RECENT_VARIANCE_SAMPLES`] returns) to the baseline per-bar realized
/// variance (the preceding returns). Each side is normalized by its sample
/// count, so a short recent window and a longer baseline compare on a per-bar
/// basis rather than by raw sum.
///
/// This is the sustained-dispersion detector the single-bar statistics miss by
/// construction: the MAD z-score reads a fresh return as in-family once its
/// history is also elevated, and the BNS jump test reads a cluster of moderate
/// bars as diffusive (ratio near one). A regime that shifts from quiet to
/// choppy therefore trips neither, but drives this ratio up.
///
/// Returns `None` when the baseline holds fewer than [`MIN_SAMPLES`] returns
/// (too few to trust as a baseline), the window is too short to spare
/// [`RECENT_VARIANCE_SAMPLES`] recent returns, or the baseline variance
/// collapses to ~zero (a flat baseline, where any recent motion is an undefined
/// multiple).
pub(crate) fn volatility_regime_ratio(returns: &[f64]) -> Option<f64> {
    if returns.len() < RECENT_VARIANCE_SAMPLES + MIN_SAMPLES {
        return None;
    }
    let split = returns.len() - RECENT_VARIANCE_SAMPLES;
    let (baseline, recent) = returns.split_at(split);
    let recent_per_bar = realized_variance(recent) / recent.len() as f64;
    let baseline_per_bar = realized_variance(baseline) / baseline.len() as f64;
    if baseline_per_bar <= EPSILON {
        return None;
    }
    Some(recent_per_bar / baseline_per_bar)
}

/// Bipower variation: `μ₁⁻²` times the sum of products of adjacent absolute
/// returns. A lone jump contributes to only two adjacent products (and there
/// dampened by its small neighbours), so bipower variation is robust to jumps
/// and estimates the continuous (diffusive) part of variance.
pub(crate) fn bipower_variation(returns: &[f64]) -> f64 {
    let paired: f64 = returns
        .windows(2)
        .map(|pair| pair[0].abs() * pair[1].abs())
        .sum();
    BIPOWER_SCALE * paired
}

/// BNS jump statistic: realized variance divided by bipower variation.
///
/// Under pure diffusion the two estimate the same integrated variance so the
/// ratio sits near one; a discontinuous jump inflates realized variance far
/// more than bipower variation, driving the ratio above one. Returns `None`
/// when there are too few returns or bipower variation is ~zero.
pub(crate) fn jump_ratio(returns: &[f64]) -> Option<f64> {
    if returns.len() < MIN_SAMPLES {
        return None;
    }
    let bipower = bipower_variation(returns);
    if bipower <= EPSILON {
        return None;
    }
    Some(realized_variance(returns) / bipower)
}

/// L2 signal: MAD-based robust z-score of the newest log-return against its
/// history. Not evaluable (`None`) below [`MIN_SAMPLES`] history points or on a
/// flat history where the MAD collapses.
pub(crate) struct RobustZScoreSignal;

impl RobustZScoreSignal {
    /// Stable trace identifier (matches the `AnomalyMetrics::robust_zscore`
    /// field it projects onto).
    pub(crate) const NAME: &'static str = "robust_zscore";
}

impl Signal for RobustZScoreSignal {
    fn name(&self) -> &'static str { Self::NAME }

    fn evaluate(&self, ctx: &SignalContext<'_>) -> SignalOutput {
        let value = ctx
            .newest_return()
            .and_then(|(newest, history)| robust_zscore(history, newest));
        SignalOutput {
            value,
            fired: value.is_some_and(|zscore| zscore >= ZSCORE_THRESHOLD),
        }
    }

    fn fragment(&self, output: &SignalOutput) -> String {
        format!("robust z-score {:.1}", output.value.unwrap_or_default())
    }
}

/// L2 signal: Barndorff-Nielsen–Shephard jump ratio (realized variance over
/// bipower variation). Not evaluable (`None`) below [`MIN_SAMPLES`] returns or
/// a ~zero bipower variation. Its `fired` flag is the
/// `AnomalyMetrics::jump_flagged` bit.
pub(crate) struct JumpSignal;

impl JumpSignal {
    /// Stable trace identifier (matches the `AnomalyMetrics::jump_ratio` field
    /// it projects onto).
    pub(crate) const NAME: &'static str = "jump_ratio";
}

impl Signal for JumpSignal {
    fn name(&self) -> &'static str { Self::NAME }

    fn evaluate(&self, ctx: &SignalContext<'_>) -> SignalOutput {
        let value = jump_ratio(ctx.returns());
        SignalOutput {
            value,
            fired: value.is_some_and(|ratio| ratio >= JUMP_RATIO_THRESHOLD),
        }
    }

    fn fragment(&self, output: &SignalOutput) -> String {
        format!("jump ratio {:.1}", output.value.unwrap_or_default())
    }
}

/// L2 signal: ratio of recent to baseline per-bar realized variance, catching a
/// sustained volatility-regime expansion the single-bar statistics miss. Not
/// evaluable (`None`) below [`RECENT_VARIANCE_SAMPLES`] + [`MIN_SAMPLES`]
/// returns or on a ~flat baseline.
pub(crate) struct VolatilityRegimeSignal;

impl VolatilityRegimeSignal {
    /// Stable trace identifier (matches the `AnomalyMetrics::volatility_regime`
    /// field it projects onto).
    pub(crate) const NAME: &'static str = "volatility_regime";
}

impl Signal for VolatilityRegimeSignal {
    fn name(&self) -> &'static str { Self::NAME }

    fn evaluate(&self, ctx: &SignalContext<'_>) -> SignalOutput {
        let value = volatility_regime_ratio(ctx.returns());
        SignalOutput {
            value,
            fired: value.is_some_and(|ratio| ratio >= VOLATILITY_REGIME_THRESHOLD),
        }
    }

    fn fragment(&self, output: &SignalOutput) -> String {
        format!("volatility regime {:.1}x", output.value.unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        VOLATILITY_REGIME_THRESHOLD, VolatilityRegimeSignal, jump_ratio, realized_variance,
        robust_zscore, volatility_regime_ratio,
    };
    use crate::anomaly::registry::{Signal, SignalContext};

    /// Naive mean/standard-deviation z-score, used only to demonstrate the
    /// contrast the robust version is designed to beat.
    fn stddev_zscore(history: &[f64], newest: f64) -> f64 {
        let n = history.len() as f64;
        let mean = history.iter().sum::<f64>() / n;
        let variance = history.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
        (newest - mean).abs() / variance.sqrt()
    }

    #[test]
    fn robust_zscore_uses_mad_not_stddev() {
        // Eight quiet bars plus one large historical outlier (+10%). A fresh
        // +2% move is anomalous relative to the quiet regime.
        let history = [
            0.001, -0.0012, 0.0009, -0.0008, 0.0011, -0.0010, 0.0012, -0.0009, 0.10,
        ];
        let newest = 0.02;

        let robust = robust_zscore(&history, newest).expect("history exceeds min samples");
        let naive = stddev_zscore(&history, newest);

        // Threshold the evaluator applies to the robust score.
        const ZSCORE_THRESHOLD: f64 = 3.5;

        // The MAD scale ignores the lone outlier, so the fresh move stands out.
        assert!(
            robust > ZSCORE_THRESHOLD,
            "robust z-score {robust} should exceed {ZSCORE_THRESHOLD}"
        );
        // The stddev scale is inflated by the outlier and suppresses the move.
        assert!(
            naive < ZSCORE_THRESHOLD,
            "stddev z-score {naive} should be suppressed below {ZSCORE_THRESHOLD}"
        );
    }

    #[test]
    fn bns_jump_test_flags_jump_over_diffusion() {
        // Diffusive window: constant-magnitude alternating returns. Realized
        // variance = 10 * 0.01² = 0.001.
        let diffusive = [
            0.01, -0.01, 0.01, -0.01, 0.01, -0.01, 0.01, -0.01, 0.01, -0.01,
        ];
        // Jump window: quiet ±0.002 background plus one dominant jump bar,
        // tuned so realized variance also equals 0.001.
        let jump = [
            0.002,
            -0.002,
            0.002,
            -0.002,
            0.031_048_349,
            -0.002,
            0.002,
            -0.002,
            0.002,
            -0.002,
        ];

        // Equal realized variance is the whole point: the separation must come
        // from bipower variation, not from a variance difference.
        let rv_diffusive = realized_variance(&diffusive);
        let rv_jump = realized_variance(&jump);
        assert!(
            (rv_diffusive - rv_jump).abs() < 1e-6,
            "realized variance should match: {rv_diffusive} vs {rv_jump}"
        );

        const JUMP_RATIO_THRESHOLD: f64 = 1.5;

        let diffusive_ratio = jump_ratio(&diffusive).expect("enough samples");
        let jump_ratio_value = jump_ratio(&jump).expect("enough samples");

        assert!(
            diffusive_ratio < JUMP_RATIO_THRESHOLD,
            "diffusive ratio {diffusive_ratio} should stay below {JUMP_RATIO_THRESHOLD}"
        );
        assert!(
            jump_ratio_value > JUMP_RATIO_THRESHOLD,
            "jump ratio {jump_ratio_value} should exceed {JUMP_RATIO_THRESHOLD}"
        );
    }

    /// A [`SignalContext`] over a bare return series; the volatility-regime
    /// signal reads only `returns`, so the other inputs are inert.
    fn ctx(returns: &[f64]) -> SignalContext<'_> {
        SignalContext::builder()
            .closes(&[])
            .returns(returns)
            .history_volumes(&[])
            .latest_volume(0.0)
            .build()
    }

    #[test]
    fn volatility_regime_fires_on_sustained_variance_expansion() {
        // Eight quiet ±0.15% baseline bars, then five loud ±0.8% recent bars —
        // a sustained dispersion expansion, not a single outlier.
        let mut returns = vec![
            0.0015, -0.0015, 0.0015, -0.0015, 0.0015, -0.0015, 0.0015, -0.0015,
        ];
        returns.extend_from_slice(&[0.008, -0.008, 0.008, -0.008, 0.008]);

        let output = VolatilityRegimeSignal.evaluate(&ctx(&returns));
        let ratio = output
            .value
            .expect("recent + baseline both exceed the minimum sample count");

        assert!(
            ratio >= VOLATILITY_REGIME_THRESHOLD,
            "recent variance should be >= {VOLATILITY_REGIME_THRESHOLD}x baseline: {ratio}"
        );
        assert!(output.fired, "a sustained variance expansion should fire");
        // The pure function and the signal agree on the value.
        assert_eq!(volatility_regime_ratio(&returns), output.value);
    }

    #[test]
    fn volatility_regime_silent_on_stable_variance() {
        // Recent and baseline bars carry comparable per-bar variance.
        let returns = [
            0.0015, -0.0015, 0.0015, -0.0015, 0.0015, -0.0015, 0.0015, -0.0015, 0.0016, -0.0016,
            0.0015, -0.0016, 0.0015,
        ];
        let output = VolatilityRegimeSignal.evaluate(&ctx(&returns));
        assert!(
            output.value.is_some(),
            "a full-length window is evaluable: {:?}",
            output.value
        );
        assert!(!output.fired, "a stable-variance tape should not fire");

        // Too short to split into recent + baseline: not evaluable.
        let short = [0.001, -0.001, 0.001];
        assert!(
            VolatilityRegimeSignal
                .evaluate(&ctx(&short))
                .value
                .is_none(),
            "a window shorter than recent + baseline reports no value"
        );
    }
}
