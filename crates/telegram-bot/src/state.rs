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

//! Centralized bot runtime state.

use std::sync::{Arc, RwLock};

use teloxide::types::ChatId;
use tokio_util::sync::CancellationToken;

use crate::http_client::MainServiceHttpClient;

/// Centralized runtime state shared across all bot components.
#[derive(Clone)]
pub(crate) struct BotState {
    /// The teloxide Bot instance.
    pub(crate) bot:          teloxide::Bot,
    /// Bot username obtained from `getMe` on startup.
    #[allow(dead_code)]
    pub(crate) bot_username: Option<String>,
    /// Runtime configuration (token + primary chat id) that may be updated
    /// via settings sync.
    pub(crate) config:       Arc<RwLock<TelegramRuntimeConfig>>,
    /// HTTP client for bot -> main-service API calls.
    pub(crate) http_client:  Arc<MainServiceHttpClient>,
    /// Cancellation token for graceful shutdown.
    pub(crate) cancel:       CancellationToken,
}

/// Runtime configuration that can be updated without restarting the bot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TelegramRuntimeConfig {
    pub(crate) bot_token:       String,
    pub(crate) primary_chat_id: i64,
}

impl BotState {
    /// Create a new BotState.
    pub(crate) fn new(
        bot: teloxide::Bot,
        bot_username: Option<String>,
        bot_token: String,
        primary_chat_id: i64,
        http_client: Arc<MainServiceHttpClient>,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            bot,
            bot_username,
            config: Arc::new(RwLock::new(TelegramRuntimeConfig {
                bot_token,
                primary_chat_id,
            })),
            http_client,
            cancel,
        }
    }

    /// Check whether the given chat id is the configured primary chat.
    pub(crate) fn is_primary_chat(&self, chat_id: ChatId) -> bool {
        let config = self.current_config();
        chat_id.0 == config.primary_chat_id
    }

    /// Read current runtime config snapshot.
    pub(crate) fn current_config(&self) -> TelegramRuntimeConfig {
        match self.config.read() {
            Ok(g) => g.clone(),
            Err(e) => e.into_inner().clone(),
        }
    }

    /// Update runtime credentials and primary chat id.
    /// Returns `true` if the config actually changed.
    pub(crate) fn update_config(&self, bot_token: String, primary_chat_id: i64) -> bool {
        let next = TelegramRuntimeConfig {
            bot_token,
            primary_chat_id,
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
}
