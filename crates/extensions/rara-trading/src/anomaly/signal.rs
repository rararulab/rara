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

//! The structured output of the anomaly evaluator.

use bon::Builder;

/// Ordered urgency of a detected market anomaly.
///
/// Variants are ordered from least to most urgent so downstream policy can
/// compare against a threshold (issue 2416 will bypass the delivery budget at
/// or above [`Severity::Critical`]). Deriving `Ord` makes that comparison a
/// plain `>=` rather than a hand-rolled ranking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// A single rule fired at a modest magnitude — worth narrating, not
    /// alarming.
    Watch,
    /// Multiple rules fired but the drawdown stayed shallow.
    Elevated,
    /// A deep drawdown coincided with corroborating signals — the
    /// bypass-eligible level (a flash-crash shape).
    Critical,
}

impl Severity {
    /// Lowercase label embedded in the injected directive text.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Severity::Watch => "watch",
            Severity::Elevated => "elevated",
            Severity::Critical => "critical",
        }
    }
}

/// The raw statistics behind an [`AnomalySignal`], kept so every alert is an
/// inspectable trace rather than an opaque verdict.
///
/// Fractions are signed where direction matters ([`Self::window_return`]) and
/// unsigned magnitudes otherwise ([`Self::max_drawdown`]). The `Option` fields
/// are `None` when the window held too few samples to trust the statistic.
#[derive(Debug, Clone, Copy, PartialEq, Builder)]
pub struct AnomalyMetrics {
    /// Signed cumulative return across the window, as a fraction (`0.05` =
    /// +5%).
    pub window_return:     f64,
    /// Deepest peak-to-trough decline across the window, as a positive
    /// fraction.
    pub max_drawdown:      f64,
    /// Newest volume divided by the rolling mean volume, when computable.
    pub volume_surge:      Option<f64>,
    /// MAD-based robust z-score of the newest log-return, when computable.
    pub robust_zscore:     Option<f64>,
    /// BNS jump ratio (realized variance / bipower variation), when computable.
    pub jump_ratio:        Option<f64>,
    /// Whether the jump ratio crossed the discontinuity threshold.
    pub jump_flagged:      bool,
    /// Recent-to-baseline per-bar realized-variance ratio, when computable —
    /// the volatility-regime signal's value.
    pub volatility_regime: Option<f64>,
    /// Signed trailing same-sign run length in bars (`+N` up-run, `-N`
    /// down-run), when computable — the directional-run signal's value.
    pub directional_run:   Option<f64>,
}

/// A detected market anomaly: a severity, a human-readable reason naming the
/// rules/statistics that fired with their magnitudes, and the structured
/// metrics behind them.
#[derive(Debug, Clone, PartialEq, Builder)]
pub struct AnomalySignal {
    /// How urgent the anomaly is.
    pub severity: Severity,
    /// Human-readable summary of which rules fired and by how much.
    pub reason:   String,
    /// The structured statistics the [`Self::reason`] and severity derive from.
    pub metrics:  AnomalyMetrics,
}
