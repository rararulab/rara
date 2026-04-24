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

//! [`StatusReporter`] — callback for persisting runtime status transitions.
//!
//! The kernel's data feed machinery tracks runtime state purely in memory
//! (cancellation tokens in [`DataFeedRegistry`](super::DataFeedRegistry)).
//! Persisting status transitions to the `data_feeds` table is the
//! responsibility of the caller (backend-admin), wired in via this trait
//! so the kernel never reaches into SQLx or the service layer.

use std::sync::Arc;

use async_trait::async_trait;

use super::FeedStatus;

/// Persist a feed's runtime status transition.
///
/// Implementations typically perform a single `UPDATE` on the `data_feeds`
/// table. Errors are expected to be logged by the implementor — callers
/// of this trait do not propagate failures, because a failing status
/// write must never crash the polling loop or task spawner.
#[async_trait]
pub trait StatusReporter: Send + Sync {
    /// Report a status transition for the feed identified by `name`.
    ///
    /// `last_error` is `Some(msg)` only for [`FeedStatus::Error`] — for
    /// [`FeedStatus::Idle`] and [`FeedStatus::Running`] it should be
    /// `None`, which also clears any previous error.
    async fn report(&self, name: &str, status: FeedStatus, last_error: Option<String>);
}

/// Shared reference type for [`StatusReporter`] implementations.
pub type StatusReporterRef = Arc<dyn StatusReporter>;
