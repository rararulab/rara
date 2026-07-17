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

//! Errors surfaced by the signal-accuracy backtest harness.

use snafu::Snafu;

use crate::anomaly::AnomalyError;

/// Failures the backtest harness raises.
///
/// The harness never fabricates a report from bad data: a candle the anomaly
/// evaluator rejects (a non-positive close makes its log-returns undefined)
/// surfaces as [`BacktestError::Evaluate`] carrying the underlying
/// [`AnomalyError`], and a repository read that fails on the async entry
/// surfaces as [`BacktestError::FetchCandles`]. Both stop the replay rather
/// than returning a silently-empty report.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum BacktestError {
    /// The anomaly evaluator rejected a candle while replaying the stream, so
    /// no trustworthy report can be produced from it.
    #[snafu(display("anomaly evaluation failed while replaying the candle stream: {source}"))]
    Evaluate {
        /// The evaluator error (e.g. a non-positive close).
        source: AnomalyError,
    },

    /// Fetching the candle stream from the market-data repository failed on the
    /// async entry point.
    #[snafu(display("failed to fetch candles for backtest: {source}"))]
    FetchCandles {
        /// The underlying repository error, boxed to keep the domain error
        /// independent of the `anyhow` boundary the repository trait uses.
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

/// Convenience alias for backtest results.
pub type Result<T, E = BacktestError> = std::result::Result<T, E>;
