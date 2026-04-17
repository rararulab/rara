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

use std::{num::NonZeroU32, sync::Arc};

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
    /// Global 30 req/sec cap across all chats.
    global:   Arc<DirectLimiter>,
}

impl ChatRateLimiter {
    /// Build a new limiter with the Telegram global cap (30 req/sec).
    pub fn new() -> Self {
        let global = Quota::per_second(NonZeroU32::new(30).expect("30 > 0"));
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
                    // 20/min for groups, supergroups, and forum-topic
                    // supergroups (editMessage shares sendMessage quota).
                    Quota::per_minute(NonZeroU32::new(20).expect("20 > 0"))
                } else {
                    // ~60/min for private chats (1/sec).
                    Quota::per_second(NonZeroU32::new(1).expect("1 > 0"))
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
