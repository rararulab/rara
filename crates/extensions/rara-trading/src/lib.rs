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

//! Trading and finance extension.
//!
//! The kernel `data_feed` module owns the generic event envelope, registry,
//! store, and subscription machinery. This crate owns finance-specific
//! ingestion/parsing sources and emits ordinary kernel
//! [`FeedEvent`](rara_kernel::data_feed::FeedEvent) values.

pub mod feed;
pub mod finance;
pub mod market_data;

#[doc(hidden)]
pub use rara_kernel::tool;
