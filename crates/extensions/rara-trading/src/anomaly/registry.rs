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

//! The anomaly signal registry: a minimal [`Signal`] trait, the structured
//! per-signal output, the shared per-evaluation context, and the builtin
//! registry of the five signals the evaluator ships with.
//!
//! This is the extensibility seam for layer-② decision support: a new signal
//! is "implement [`Signal`] + add one line to [`builtin_registry`]", with no
//! edit to the core evaluation loop in [`super::evaluator`]. The five builtin
//! signals are thin adapters over the pure functions in [`super::rules`] and
//! [`super::statistics`]; those pure functions remain the computational core.

use bon::Builder;

use super::{
    rules::{MaxDrawdownSignal, VolumeSurgeSignal, WindowReturnSignal},
    statistics::{JumpSignal, RobustZScoreSignal},
};

/// Shared inputs prepared once per evaluation and handed to every signal, so no
/// signal recomputes the close/return/volume series.
#[derive(Builder)]
pub(crate) struct SignalContext<'a> {
    /// Ordered close series: window history followed by the latest close.
    closes:          &'a [f64],
    /// Log-returns of [`Self::closes`], newest last.
    returns:         &'a [f64],
    /// Rolling historical volumes (excluding the latest candle).
    history_volumes: &'a [f64],
    /// Volume of the latest (newly closed) candle.
    latest_volume:   f64,
}

impl SignalContext<'_> {
    /// Ordered close series (window history followed by the latest close).
    pub(crate) fn closes(&self) -> &[f64] { self.closes }

    /// Log-returns of [`Self::closes`], newest last.
    pub(crate) fn returns(&self) -> &[f64] { self.returns }

    /// Rolling historical volumes (excluding the latest candle).
    pub(crate) fn history_volumes(&self) -> &[f64] { self.history_volumes }

    /// Volume of the latest (newly closed) candle.
    pub(crate) fn latest_volume(&self) -> f64 { self.latest_volume }

    /// The newest return split from the preceding history, or `None` when the
    /// return series is empty (window too short to form a single return).
    pub(crate) fn newest_return(&self) -> Option<(f64, &[f64])> {
        self.returns
            .split_last()
            .map(|(&newest, history)| (newest, history))
    }
}

/// The structured output of one [`Signal`] over a [`SignalContext`].
///
/// Every field has a downstream consumer, so no field is a hollow placeholder
/// (`docs/guides/anti-patterns.md`): [`Self::name`] routes the value into the
/// public [`super::AnomalyMetrics`] trace, [`Self::value`] populates that trace
/// and feeds [`Signal::fragment`], and [`Self::fired`] drives the fired-count,
/// severity, and reason-fragment selection in the classifier.
#[derive(Debug, Clone, Builder)]
pub(crate) struct SignalOutput {
    /// Stable identifier routing this signal's value into the metrics trace.
    pub(crate) name:  &'static str,
    /// The signal's computed value, or `None` when the window was too short to
    /// compute it — mirrors the `Option<f64>` metrics so "not evaluable" is not
    /// flattened into a numeric `0.0`.
    pub(crate) value: Option<f64>,
    /// Whether the signal crossed its trip threshold.
    pub(crate) fired: bool,
}

/// One anomaly signal: it computes a structured output over the prepared
/// context and formats its own reason fragment when it fires.
pub(crate) trait Signal {
    /// Stable identifier used to route this signal's value into the public
    /// [`super::AnomalyMetrics`] trace. Not shown to users.
    fn name(&self) -> &'static str;

    /// Evaluate the prepared context into this signal's structured output.
    fn evaluate(&self, ctx: &SignalContext<'_>) -> SignalOutput;

    /// Human-readable reason fragment for a fired output (e.g. `drawdown
    /// 6.1%`). Only called for outputs whose [`SignalOutput::fired`] is set.
    fn fragment(&self, output: &SignalOutput) -> String;
}

/// An ordered set of signals the evaluator walks. The builtin order encodes the
/// reason-fragment order (drawdown, return, volume, z-score, jump), so the
/// `reason` string is byte-for-byte stable.
pub(crate) type SignalRegistry = Vec<Box<dyn Signal>>;

/// The five signals the anomaly evaluator ships with, in reason-fragment order.
///
/// Adding a sixth signal is a one-line extension here plus its [`Signal`] impl;
/// the core loop in [`super::evaluator::evaluate_with`] does not change.
pub(crate) fn builtin_registry() -> SignalRegistry {
    vec![
        Box::new(MaxDrawdownSignal),
        Box::new(WindowReturnSignal),
        Box::new(VolumeSurgeSignal),
        Box::new(RobustZScoreSignal),
        Box::new(JumpSignal),
    ]
}
