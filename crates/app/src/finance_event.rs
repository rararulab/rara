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

//! Kernel-backed delivery sink for the market-signal dispatch facade.
//!
//! The market-signal orchestration itself lives in
//! [`rara_trading::dispatch`]; this module only supplies the production
//! [`FeedDispatchSink`] that wires the facade's delivery decisions into a live
//! kernel session. It stays here because it depends on `KernelHandle`, which
//! is app/kernel territory and must not leak into `rara-trading`.

use async_trait::async_trait;
use rara_kernel::{identity::UserId, notification::Subscription, session::SessionKey};
use rara_trading::dispatch::FeedDispatchSink;

/// Production sink backed by a kernel handle.
pub(crate) struct KernelFeedDispatchSink {
    handle: rara_kernel::handle::KernelHandle,
}

impl KernelFeedDispatchSink {
    #[must_use]
    pub(crate) fn new(handle: rara_kernel::handle::KernelHandle) -> Self { Self { handle } }
}

#[async_trait]
impl FeedDispatchSink for KernelFeedDispatchSink {
    async fn session_active(&self, session: &SessionKey) -> bool {
        self.handle.process_table().contains(session)
    }

    async fn deliver_synthetic(&self, owner: UserId, session: SessionKey, directive: String) {
        let msg = rara_kernel::io::InboundMessage::synthetic(directive, owner, session);
        self.handle.deliver_internal(msg).await;
    }

    async fn append_feed_event(&self, session: SessionKey, payload: serde_json::Value) {
        let _ = self
            .handle
            .tape()
            .store()
            .append(
                &session.to_string(),
                rara_kernel::memory::TapEntryKind::FeedEvent,
                payload,
                None,
            )
            .await;
    }

    async fn generic_matches(&self, tags: &[String]) -> Vec<Subscription> {
        self.handle
            .subscription_registry()
            .match_tags_any_owner(tags)
            .await
    }
}
