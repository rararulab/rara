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

//! Market-signal dispatch facade.
//!
//! This module is the single, cohesive home of the layer-① market-signal
//! orchestration: it takes a kernel
//! [`FeedEvent`](rara_kernel::data_feed::FeedEvent) and runs the whole
//! persist → upsert → evaluate → match → deliver pipeline that was previously
//! hand-assembled as app glue.
//!
//! The pipeline is:
//!
//! 1. persist the event to the
//!    [`FeedStore`](rara_kernel::data_feed::FeedStore),
//! 2. for a closed market candle, parse + upsert it into the
//!    [`MarketDataRepository`](crate::market_data::MarketDataRepository),
//! 3. pull the rolling window and run [`anomaly::evaluate`](crate::anomaly),
//! 4. feed the evaluated severity into
//!    [`FinanceSubscriptionRegistry::match_event`](crate::finance::registry::FinanceSubscriptionRegistry)
//!    (the registry remains the single home of the cooldown / budget / bypass
//!    delivery policy),
//! 5. deliver the severity-graded directive through the injected
//!    [`FeedDispatchSink`].
//!
//! The only piece that stays in `crates/app` is the production sink that wraps
//! the kernel handle — everything here depends only on `rara-kernel` types and
//! this crate's own domain modules, so the boundary is clean and layer-②/③ can
//! consume [`on_feed_event`] instead of re-plumbing (or forking) the pipeline.

mod pipeline;

pub use pipeline::{FeedDispatchOutcome, FeedDispatchSink, on_feed_event};

#[cfg(test)]
mod tests;
