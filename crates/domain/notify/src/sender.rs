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
use job_api::pb::telegrambot::v1::{
    SendMessageRequest, telegram_bot_command_service_client::TelegramBotCommandServiceClient,
};
use tonic::transport::Channel;
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

/// gRPC sender that forwards Telegram notifications to telegram-bot process.
#[derive(Clone)]
pub struct TelegramBotGrpcSender {
    target: String,
}

impl TelegramBotGrpcSender {
    #[must_use]
    pub fn new(target: String) -> Self { Self { target } }
}

#[async_trait]
impl NotificationSender for TelegramBotGrpcSender {
    async fn send(&self, notification: &Notification) -> Result<(), NotifyError> {
        let endpoint = normalize_grpc_endpoint(&self.target);
        let channel = Channel::from_shared(endpoint.clone())
            .map_err(|e| NotifyError::SendFailed {
                channel: "telegram".to_owned(),
                message: format!("invalid telegram-bot grpc target {endpoint}: {e}"),
            })?
            .connect()
            .await
            .map_err(|e| NotifyError::SendFailed {
                channel: "telegram".to_owned(),
                message: format!("failed to connect telegram-bot grpc service: {e}"),
            })?;

        let mut client = TelegramBotCommandServiceClient::new(channel);
        let chat_id = notification.recipient.parse::<i64>().unwrap_or_default();
        client
            .send_message(SendMessageRequest {
                chat_id,
                text: format_notification(notification),
            })
            .await
            .map_err(|e| NotifyError::SendFailed {
                channel: "telegram".to_owned(),
                message: format!("telegram-bot grpc send failed: {e}"),
            })?;

        Ok(())
    }
}

fn normalize_grpc_endpoint(target: &str) -> String {
    if target.starts_with("http://") || target.starts_with("https://") {
        target.to_owned()
    } else {
        format!("http://{target}")
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
