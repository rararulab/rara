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

//! Shared Telegram bot service.

use teloxide::{RequestError, prelude::*, requests::Requester, types::ChatId};
use tracing::instrument;

/// Strongly-typed Telegram service shared across crates.
///
/// Holds the bot and the configured primary chat id used by the backend.
#[derive(Clone)]
pub struct TelegramService {
    bot:             Bot,
    primary_chat_id: ChatId,
}

impl TelegramService {
    pub fn new(bot: Bot, primary_chat_id: i64) -> Self {
        Self {
            bot,
            primary_chat_id: ChatId(primary_chat_id),
        }
    }

    pub fn bot(&self) -> Bot { self.bot.clone() }

    pub fn primary_chat_id(&self) -> ChatId { self.primary_chat_id }

    pub fn is_primary_chat(&self, chat_id: ChatId) -> bool { chat_id == self.primary_chat_id }

    #[instrument(
        level = "info",
        skip(self, text),
        fields(chat_id = self.primary_chat_id.0, text_len = text.len()),
        err
    )]
    pub async fn send_primary_message(&self, text: &str) -> Result<Message, RequestError> {
        self.send_message(self.primary_chat_id, text).await
    }

    #[instrument(level = "info", skip(self, text), fields(chat_id = chat_id.0, text_len = text.len()), err)]
    pub async fn send_message(&self, chat_id: ChatId, text: &str) -> Result<Message, RequestError> {
        self.bot.send_message(chat_id, text).await
    }
}
