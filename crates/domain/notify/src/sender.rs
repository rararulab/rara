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

//! Notification sender implementations.

use async_trait::async_trait;
use teloxide::prelude::*;
use teloxide::requests::Requester;
use teloxide::types::ChatId;
use tracing::info;

use crate::{error::NotifyError, service::NotificationSender, types::Notification};

/// A no-op sender that logs but does not actually deliver notifications.
/// Useful for development and testing environments.
pub struct NoopSender;

#[async_trait]
impl NotificationSender for NoopSender {
    async fn send(&self, notification: &Notification) -> Result<(), NotifyError> {
        info!(
            id = %notification.id,
            channel = ?notification.channel,
            recipient = %notification.recipient,
            "noop: notification would be sent"
        );
        Ok(())
    }
}

/// Sends notifications via Telegram using the teloxide bot API.
pub struct TelegramSender {
    bot:     Bot,
    chat_id: ChatId,
}

impl TelegramSender {
    pub fn new(bot_token: &str, chat_id: i64) -> Self {
        Self {
            bot:     Bot::new(bot_token),
            chat_id: ChatId(chat_id),
        }
    }
}

#[async_trait]
impl NotificationSender for TelegramSender {
    async fn send(&self, notification: &Notification) -> Result<(), NotifyError> {
        let message = format_notification(notification);
        self.bot
            .send_message(self.chat_id, message)
            .await
            .map_err(|e| NotifyError::SendFailed {
                channel: "telegram".to_string(),
                message: e.to_string(),
            })?;
        Ok(())
    }
}

fn format_notification(notification: &Notification) -> String {
    let mut msg = String::new();
    if let Some(subject) = &notification.subject {
        msg.push_str(&format!("*{}*\n\n", subject));
    }
    msg.push_str(&notification.body);
    msg
}
