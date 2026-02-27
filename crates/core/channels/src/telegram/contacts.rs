// Copyright 2025 Crrow
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

//! Contact tracking interface for the Telegram adapter.
//!
//! When the adapter receives a message from a user with a known username,
//! it calls [`ContactTracker::track`] to record the username-to-chat_id
//! mapping. This enables outbound notification routing: given a username,
//! the application can look up the corresponding chat ID and send a
//! notification via [`ChannelAdapter::send`].
//!
//! The trait is defined here (in the channels crate) so that it has no
//! database dependencies. Implementors typically persist the mapping in
//! a database table; the concrete implementation lives in the composition
//! root (`rara-app`).

use async_trait::async_trait;

/// Tracks Telegram user contacts for notification routing.
///
/// When the adapter receives a message from a user with a known username,
/// it calls [`track`](ContactTracker::track) to record the username-to-chat_id
/// mapping. This enables outbound notification routing.
///
/// Implementors typically persist this in a database table.
#[async_trait]
pub trait ContactTracker: Send + Sync {
    /// Record that `username` is reachable at `chat_id`.
    ///
    /// This is called on every incoming message from a user with a username.
    /// Implementations should be idempotent (update only if chat_id changed).
    async fn track(&self, username: &str, chat_id: i64);
}

/// No-op contact tracker that discards all tracking calls.
#[derive(Debug, Clone, Default)]
pub struct NoopContactTracker;

#[async_trait]
impl ContactTracker for NoopContactTracker {
    async fn track(&self, _username: &str, _chat_id: i64) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingTracker {
        count: AtomicUsize,
    }

    impl CountingTracker {
        fn new() -> Self {
            Self {
                count: AtomicUsize::new(0),
            }
        }

        fn count(&self) -> usize {
            self.count.load(Ordering::Relaxed)
        }
    }

    #[async_trait]
    impl ContactTracker for CountingTracker {
        async fn track(&self, _username: &str, _chat_id: i64) {
            self.count.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[tokio::test]
    async fn test_noop_tracker() {
        let tracker = NoopContactTracker;
        tracker.track("testuser", 12345).await;
        // Just verifies it doesn't panic.
    }

    #[tokio::test]
    async fn test_counting_tracker() {
        let tracker = CountingTracker::new();
        tracker.track("user1", 111).await;
        tracker.track("user2", 222).await;
        assert_eq!(tracker.count(), 2);
    }
}
