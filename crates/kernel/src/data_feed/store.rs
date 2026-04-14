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

//! Persistent store trait for feed events.

use async_trait::async_trait;
use jiff::Timestamp;

use super::event::{FeedEvent, FeedEventId};
use crate::session::SessionKey;

/// Persistent store for external feed events.
///
/// Implementations handle event persistence, filtered queries, and
/// per-subscriber read-cursor tracking. The kernel owns one shared `FeedStore`
/// instance; transport layers call [`append`](Self::append) on ingestion and
/// agent sessions call [`query`](Self::query) /
/// [`unread_count`](Self::unread_count) to consume events.
#[async_trait]
pub trait FeedStore: Send + Sync {
    /// Persist a new event.
    ///
    /// Implementations must be idempotent on `event.id` — inserting an event
    /// whose ID already exists should succeed without duplicating the row.
    async fn append(&self, event: &FeedEvent) -> crate::Result<()>;

    /// Query events matching `filter`, returned in chronological order.
    async fn query(&self, filter: FeedFilter) -> crate::Result<Vec<FeedEvent>>;

    /// Mark all events up to (and including) `up_to` as read for `subscriber`.
    ///
    /// The read cursor is per-subscriber, per-source so that multiple agent
    /// sessions can independently track their consumption progress.
    async fn mark_read(&self, subscriber: &SessionKey, up_to: FeedEventId) -> crate::Result<()>;

    /// Return the number of unread events for `subscriber` across all sources.
    async fn unread_count(&self, subscriber: &SessionKey) -> crate::Result<usize>;
}

/// Filter criteria for [`FeedStore::query`].
#[derive(Debug, Clone)]
pub struct FeedFilter {
    /// Only return events from this source. `None` means all sources.
    pub source_name: Option<String>,

    /// Only return events that carry *all* of these tags. Empty means no tag
    /// filter.
    pub tags: Vec<String>,

    /// Only return events received at or after this timestamp.
    pub since: Option<Timestamp>,

    /// Maximum number of events to return. Implementations should clamp this
    /// to a sane upper bound.
    pub limit: usize,
}
