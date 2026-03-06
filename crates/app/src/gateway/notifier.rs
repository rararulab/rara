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

//! Lightweight Telegram notifier for gateway update lifecycle events.
//!
//! Uses [`teloxide::Bot`] to send messages to a configured notification
//! channel. Does NOT go through the kernel IO subsystem — the gateway
//! runs independently of the kernel.

use teloxide::prelude::*;
use teloxide::types::ChatId;
use tracing::warn;

/// Telegram notifier for gateway auto-update events.
///
/// All errors are logged but never propagated — notifications must not
/// break the update pipeline.
pub struct UpdateNotifier {
    bot: Bot,
    channel_id: i64,
}

impl UpdateNotifier {
    /// Create a new notifier.
    ///
    /// Proxy is automatically picked up from `HTTPS_PROXY` / `HTTP_PROXY` /
    /// `ALL_PROXY` environment variables by the underlying reqwest client.
    pub fn new(bot_token: &str, channel_id: i64) -> Self {
        let bot = Bot::new(bot_token);
        Self { bot, channel_id }
    }

    /// Send a notification message. Errors are logged but never propagated.
    pub async fn notify(&self, message: &str) {
        let result = self
            .bot
            .send_message(ChatId(self.channel_id), message)
            .parse_mode(teloxide::types::ParseMode::Html)
            .await;

        if let Err(e) = result {
            warn!(error = %e, "UpdateNotifier: failed to send Telegram notification");
        }
    }
}
