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

//! Per-chat Telegram API rate limiter.
//!
//! Telegram rate limits for bots (tdlib/td#3034):
//! - Private chats: 1 message/sec (edit + send share this quota)
//! - Groups / supergroups / forum-topic supergroups: 20 messages/min
//! - Global: 30 messages/sec across all chats
//!
//! `editMessage` counts against the same quota as `sendMessage`. We treat all
//! outbound requests (edit + send) as one stream per chat.
//!
//! All quotas use `Quota::with_period(...)` which yields **burst capacity 1**
//! and paces strictly at the declared period. The alternative
//! `Quota::per_minute(20)` would seed a fresh limiter with 20 cells of burst,
//! allowing an idle chat to drain all 20 edits instantly — which is exactly
//! the bursty plan-streaming case where Telegram will 429 us on the 21st
//! request within the rolling minute.

use std::{sync::Arc, time::Duration};

use dashmap::DashMap;
use governor::{
    Quota, RateLimiter,
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
};

type DirectLimiter = RateLimiter<NotKeyed, InMemoryState, DefaultClock>;

/// Per-chat + global rate limiter for Telegram outbound calls.
///
/// All outbound Telegram calls (`sendMessage`, `editMessageText`, `sendPhoto`,
/// `sendVoice`, `sendChatAction`) MUST pass through [`Self::acquire`] before
/// being sent. Bypassing the limiter causes 429 `FloodWait` errors which (in
/// forum topics) silently drop inline reply keyboards.
#[derive(Clone)]
pub struct ChatRateLimiter {
    /// One limiter per chat_id. Quota depends on chat kind (group vs private),
    /// derived from the sign of `chat_id` at first lookup.
    per_chat: Arc<DashMap<i64, Arc<DirectLimiter>>>,
    /// Global ~30 req/sec cap across all chats (paced, burst 1).
    global:   Arc<DirectLimiter>,
}

impl ChatRateLimiter {
    /// Build a new limiter with the Telegram global cap (~30 req/sec, paced).
    pub fn new() -> Self {
        // 34ms period ≈ 29.4 req/sec — stays safely below the 30/sec global
        // cap while keeping burst 1.
        let global =
            Quota::with_period(Duration::from_millis(34)).expect("34ms period is non-zero");
        Self {
            per_chat: Arc::new(DashMap::new()),
            global:   Arc::new(RateLimiter::direct(global)),
        }
    }

    /// Block until a send/edit against `chat_id` is allowed by both the
    /// per-chat quota and the global quota.
    ///
    /// Order matters: per-chat first so global quota isn't consumed while a
    /// chat-specific wait is still pending.
    pub async fn acquire(&self, chat_id: i64) {
        let per = self
            .per_chat
            .entry(chat_id)
            .or_insert_with(|| {
                let quota = if chat_id < 0 {
                    // Groups, supergroups, forum-topic supergroups:
                    // 1 msg per 3s ≈ 20/min, burst 1. editMessage shares
                    // the sendMessage quota.
                    Quota::with_period(Duration::from_secs(3)).expect("3s period is non-zero")
                } else {
                    // Private chats: 1 msg/sec, burst 1.
                    Quota::with_period(Duration::from_secs(1)).expect("1s period is non-zero")
                };
                Arc::new(RateLimiter::direct(quota))
            })
            .clone();
        per.until_ready().await;
        self.global.until_ready().await;
    }
}

impl Default for ChatRateLimiter {
    fn default() -> Self { Self::new() }
}
