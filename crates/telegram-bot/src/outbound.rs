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

//! Outbound message abstraction for sending Telegram messages.
//!
//! [`TelegramOutbound`] centralizes all message sending logic with two key
//! features:
//!
//! - **Markdown conversion** — [`send_markdown`](TelegramOutbound::send_markdown)
//!   automatically converts Markdown to Telegram's HTML subset before sending.
//! - **Auto-chunking** — messages exceeding the 4096-character Telegram limit
//!   are split at newline or space boundaries and sent as multiple messages.

use std::sync::{Arc, RwLock};

use snafu::{ResultExt, Snafu};
use teloxide::{payloads::SendMessageSetters, requests::Requester, types::ChatId};
use tracing::instrument;

use crate::{
    markdown::{TELEGRAM_MAX_MESSAGE_LEN, chunk_message, markdown_to_telegram_html},
    state::TelegramRuntimeConfig,
};

/// Errors from outbound message operations.
#[derive(Debug, Snafu)]
pub(crate) enum OutboundError {
    #[snafu(display("failed to send message: {source}"))]
    Send { source: teloxide::RequestError },
    #[snafu(display("failed to send typing indicator: {source}"))]
    Typing { source: teloxide::RequestError },
}

/// Outbound message sender with automatic formatting and chunking.
#[derive(Clone)]
pub(crate) struct TelegramOutbound {
    bot:    teloxide::Bot,
    config: Arc<RwLock<TelegramRuntimeConfig>>,
}

impl TelegramOutbound {
    /// Create a new outbound sender.
    pub(crate) fn new(bot: teloxide::Bot, config: Arc<RwLock<TelegramRuntimeConfig>>) -> Self {
        Self { bot, config }
    }

    /// Send a plain text message (no markdown conversion).
    #[allow(dead_code)]
    #[instrument(level = "info", skip(self, text), fields(chat_id = chat_id.0, text_len = text.len()), err)]
    pub(crate) async fn send_text(
        &self,
        chat_id: ChatId,
        text: &str,
    ) -> Result<(), OutboundError> {
        let chunks = chunk_message(text, TELEGRAM_MAX_MESSAGE_LEN);
        for chunk in chunks {
            self.bot
                .send_message(chat_id, chunk)
                .await
                .context(SendSnafu)?;
        }
        Ok(())
    }

    /// Send a Markdown message — converts to Telegram HTML automatically.
    #[instrument(level = "info", skip(self, markdown), fields(chat_id = chat_id.0, md_len = markdown.len()), err)]
    pub(crate) async fn send_markdown(
        &self,
        chat_id: ChatId,
        markdown: &str,
    ) -> Result<(), OutboundError> {
        let html = markdown_to_telegram_html(markdown);
        let chunks = chunk_message(&html, TELEGRAM_MAX_MESSAGE_LEN);
        for chunk in chunks {
            self.bot
                .send_message(chat_id, chunk)
                .parse_mode(teloxide::types::ParseMode::Html)
                .await
                .context(SendSnafu)?;
        }
        Ok(())
    }

    /// Send a typing indicator to the chat.
    #[allow(dead_code)]
    pub(crate) async fn send_typing(&self, chat_id: ChatId) -> Result<(), OutboundError> {
        self.bot
            .send_chat_action(chat_id, teloxide::types::ChatAction::Typing)
            .await
            .context(TypingSnafu)?;
        Ok(())
    }

    /// Send a plain text message to the configured primary chat.
    #[allow(dead_code)]
    #[instrument(level = "info", skip(self, text), fields(text_len = text.len()), err)]
    pub(crate) async fn send_primary_text(&self, text: &str) -> Result<(), OutboundError> {
        let chat_id = self.primary_chat_id();
        self.send_text(chat_id, text).await
    }

    /// Send a markdown message to the configured primary chat.
    #[allow(dead_code)]
    #[instrument(level = "info", skip(self, markdown), fields(md_len = markdown.len()), err)]
    pub(crate) async fn send_primary_markdown(
        &self,
        markdown: &str,
    ) -> Result<(), OutboundError> {
        let chat_id = self.primary_chat_id();
        self.send_markdown(chat_id, markdown).await
    }

    /// Read a snapshot of the current runtime config.
    pub(crate) fn primary_config(&self) -> TelegramRuntimeConfig {
        match self.config.read() {
            Ok(g) => g.clone(),
            Err(e) => e.into_inner().clone(),
        }
    }

    fn primary_chat_id(&self) -> ChatId {
        ChatId(self.primary_config().primary_chat_id)
    }
}
