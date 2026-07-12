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

//! Durable OHLCV market-data storage.

pub mod model;
pub mod repository;
pub mod timescale;
pub mod tools;

pub use model::{
    CandleLatestQuery, CandleRangeQuery, CandleRecentQuery, CandleStreamListQuery,
    CandleStreamSummary, MarketCandle, Timeframe,
};
pub use repository::{
    InMemoryMarketDataRepository, MarketDataRepository, MarketDataRepositoryRef, UpsertOutcome,
};
pub use timescale::TimescaleMarketDataRepository;
