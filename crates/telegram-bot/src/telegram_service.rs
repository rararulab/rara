// Copyright 2026 Crrow
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

//! Telegram service used inside telegram-bot runtime.

use std::sync::{Arc, RwLock};

use teloxide::{RequestError, prelude::*, requests::Requester, types::ChatId};
use tracing::instrument;

/// Strongly-typed Telegram adapter used by bot runtime.
#[derive(Clone)]
pub(crate) struct TelegramService {
    config: Arc<RwLock<TelegramRuntimeConfig>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TelegramRuntimeConfig {
    bot_token:       String,
    primary_chat_id: ChatId,
}

impl TelegramService {
    /// Construct telegram adapter with fixed primary chat id.
    pub(crate) fn new(bot_token: String, primary_chat_id: i64) -> Self {
        Self {
            config: Arc::new(RwLock::new(TelegramRuntimeConfig {
                bot_token,
                primary_chat_id: ChatId(primary_chat_id),
            })),
        }
    }

    /// Clone raw `teloxide::Bot` for dispatcher wiring.
    pub(crate) fn bot(&self) -> Bot {
        let config = self.current_config();
        Bot::new(config.bot_token)
    }

    /// Authorization helper: accept only configured primary chat.
    pub(crate) fn is_primary_chat(&self, chat_id: ChatId) -> bool {
        chat_id == self.current_config().primary_chat_id
    }

    #[instrument(
        level = "info",
        skip(self, text),
        fields(chat_id = self.current_config().primary_chat_id.0, text_len = text.len()),
        err
    )]
    pub(crate) async fn send_primary_message(&self, text: &str) -> Result<Message, RequestError> {
        self.send_message(self.current_config().primary_chat_id, text)
            .await
    }

    /// Update runtime credentials and primary chat id.
    pub(crate) fn update_config(&self, bot_token: String, primary_chat_id: i64) -> bool {
        let next = TelegramRuntimeConfig {
            bot_token,
            primary_chat_id: ChatId(primary_chat_id),
        };
        if let Ok(mut guard) = self.config.write() {
            if *guard == next {
                return false;
            }
            *guard = next;
            return true;
        }
        false
    }

    #[instrument(level = "info", skip(self, text), fields(chat_id = chat_id.0, text_len = text.len()), err)]
    pub(crate) async fn send_message(
        &self,
        chat_id: ChatId,
        text: &str,
    ) -> Result<Message, RequestError> {
        let config = self.current_config();
        Bot::new(config.bot_token).send_message(chat_id, text).await
    }

    fn current_config(&self) -> TelegramRuntimeConfig {
        match self.config.read() {
            Ok(g) => g.clone(),
            Err(e) => e.into_inner().clone(),
        }
    }
}
