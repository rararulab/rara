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

//! Errors surfaced by the anomaly evaluator.

use snafu::Snafu;

/// Failures the anomaly evaluator raises for structurally invalid input.
///
/// These are distinct from the ordinary `Ok(None)` outcome (a normal tape
/// with nothing anomalous, or a window too short to judge): they mark candle
/// data that violates the arithmetic preconditions of the statistics — a
/// non-positive close makes a log-return undefined — so the caller can log a
/// concrete, inspectable reason instead of silently swallowing it.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum AnomalyError {
    /// A candle carried a close price that is not strictly positive, so the
    /// log-return series cannot be formed.
    #[snafu(display(
        "candle for {symbol} at {open_time} has non-positive close {close}; cannot compute \
         log-returns"
    ))]
    NonPositivePrice {
        /// Symbol of the offending candle.
        symbol:    String,
        /// Open time of the offending candle.
        open_time: String,
        /// The non-positive close value, rendered for the log line.
        close:     String,
    },
}

/// Convenience alias for evaluator results.
pub type Result<T, E = AnomalyError> = std::result::Result<T, E>;
