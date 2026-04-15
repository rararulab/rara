// Copyright 2025 Rararulab
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

//! Data Feed — external data ingestion for agent perception.
//!
//! This module defines the core types for the data feed subsystem:
//!
//! - [`FeedEvent`] — the atomic event envelope received from external sources.
//! - [`FeedEventId`] — strongly-typed UUID identifier for deduplication.
//! - [`FeedStore`] — async persistence trait for events and read cursors.
//! - [`FeedFilter`] — query criteria for filtered event retrieval.
//! - [`DataFeed`] — trait for external data source implementations.
//! - [`DataFeedConfig`] / [`FeedType`] / [`FeedStatus`] — persisted
//!   configuration types.
//! - [`AuthConfig`] — unified authentication configuration for all transports.
//! - [`DataFeedRegistry`] — runtime registry managing feed configs and tasks.
//! - [`webhook`] — HTTP POST receiver with HMAC verification and dedup.
//! - [`polling`] — config-driven HTTP polling source with pass-through
//!   response.

pub mod config;
mod event;
mod feed;
pub mod polling;
mod registry;
mod store;
pub mod webhook;

pub use config::{AuthConfig, DataFeedConfig, FeedStatus, FeedType};
pub use event::{FeedEvent, FeedEventId};
pub use feed::DataFeed;
pub use registry::DataFeedRegistry;
pub use store::{FeedFilter, FeedStore, FeedStoreRef};
