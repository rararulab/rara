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

//! Market-anomaly evaluation: the layer between candle ingestion and directive
//! delivery.
//!
//! Given a newly closed candle plus its rolling window, [`evaluate`] computes
//! structured signals — L1 rules (window return, rolling drawdown, volume
//! surge) and L2 statistics (a MAD-based robust z-score of log-returns and the
//! Barndorff-Nielsen–Shephard jump test) — and produces an [`AnomalySignal`]
//! carrying a [`Severity`], a human-readable `reason`, and the
//! [`AnomalyMetrics`] behind it. `None` means the tape is unremarkable and the
//! caller should leave the directive wording unchanged.
//!
//! The evaluator is pure (no clock, no I/O); the wiring that pulls the window
//! from the market-data repository and enriches the synthetic directive lives
//! in `crates/app/src/finance_event.rs`.

mod error;
mod evaluator;
mod registry;
mod rules;
mod signal;
mod statistics;

pub use error::{AnomalyError, Result};
pub use evaluator::{EVAL_WINDOW, evaluate};
pub use signal::{AnomalyMetrics, AnomalySignal, Severity};
