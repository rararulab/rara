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

//! [`SvcStatusReporter`] — bridges the kernel's [`StatusReporter`] callback
//! to [`DataFeedSvc::update_status`] so runtime transitions persist to the
//! `data_feeds` table.

use async_trait::async_trait;
use rara_kernel::data_feed::{FeedStatus, StatusReporter};
use tracing::warn;

use super::DataFeedSvc;

/// [`StatusReporter`] that writes transitions through [`DataFeedSvc`].
///
/// Errors are logged and swallowed — a failing status write must never
/// crash the polling loop or task spawner that triggered it.
pub struct SvcStatusReporter {
    svc: DataFeedSvc,
}

impl SvcStatusReporter {
    /// Build a reporter backed by the given service.
    pub fn new(svc: DataFeedSvc) -> Self { Self { svc } }
}

#[async_trait]
impl StatusReporter for SvcStatusReporter {
    async fn report(&self, name: &str, status: FeedStatus, last_error: Option<String>) {
        if let Err(e) = self.svc.update_status(name, status, last_error).await {
            warn!(feed = name, %status, error = %e, "failed to persist feed status");
        }
    }
}
