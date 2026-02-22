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

//! Send notification primitive.
//!
//! Reads the chat_id from runtime settings at call time and enqueues a
//! telegram notification via the `NotifyClient`.

use async_trait::async_trait;
use rara_domain_shared::{
    notify::{
        client::NotifyClient,
        types::{NotificationPriority, SendTelegramNotificationRequest},
    },
    settings::SettingsSvc,
};
use serde_json::json;

use crate::AgentTool;

/// Layer 1 primitive: send a Telegram message.
///
/// Enqueues a notification via PGMQ which the telegram-bot process consumes
/// and delivers.  Supports plain text, bold subject headers, photos, and
/// sending to arbitrary chat IDs (not just the primary one from settings).
pub struct NotifyTool {
    client:       NotifyClient,
    settings_svc: SettingsSvc,
}

impl NotifyTool {
    pub fn new(client: NotifyClient, settings_svc: SettingsSvc) -> Self {
        Self {
            client,
            settings_svc,
        }
    }
}

#[async_trait]
impl AgentTool for NotifyTool {
    fn name(&self) -> &str { "send_telegram" }

    fn description(&self) -> &str {
        "Send a message to a Telegram chat. Use this to proactively notify or communicate with \
         users via Telegram. Supports text, bold subject headers, and photo attachments. Specify \
         a recipient by Telegram username (e.g. \"ryan\") to send to a specific person. Omit \
         recipient to send to the default chat."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "The message body text (supports Markdown)"
                },
                "recipient": {
                    "type": "string",
                    "description": "Telegram username of the recipient (without @ prefix). The bot resolves this to a chat ID from its known contacts. Omit to send to the default chat."
                },
                "subject": {
                    "type": "string",
                    "description": "Optional bold subject line displayed above the message body"
                },
                "photo_path": {
                    "type": "string",
                    "description": "Local file path of an image to send as a photo message"
                },
                "priority": {
                    "type": "string",
                    "enum": ["low", "normal", "high", "urgent"],
                    "description": "Message priority (default: normal)"
                }
            },
            "required": ["message"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let message = params
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: message"))?;

        let recipient = params
            .get("recipient")
            .and_then(|v| v.as_str())
            .map(|s| s.trim_start_matches('@').to_owned());

        let subject = params.get("subject").and_then(|v| v.as_str()).map(String::from);

        let photo_path = params
            .get("photo_path")
            .and_then(|v| v.as_str())
            .map(String::from);

        let priority = params
            .get("priority")
            .and_then(|v| v.as_str())
            .map(|s| match s {
                "low" => NotificationPriority::Low,
                "high" => NotificationPriority::High,
                "urgent" => NotificationPriority::Urgent,
                _ => NotificationPriority::Normal,
            })
            .unwrap_or(NotificationPriority::Normal);

        // Use settings chat_id as fallback only when no recipient is specified.
        let chat_id = if recipient.is_none() {
            self.settings_svc.current().telegram.chat_id
        } else {
            None
        };

        let request = SendTelegramNotificationRequest {
            chat_id,
            recipient: recipient.clone(),
            subject,
            body: message.to_owned(),
            priority,
            max_retries: 3,
            reference_type: None,
            reference_id: None,
            metadata: None,
            photo_path,
        };

        match self.client.send_telegram(request).await {
            Ok(queued) => Ok(json!({
                "sent": true,
                "id": queued.id.to_string(),
                "recipient": recipient,
            })),
            Err(e) => Ok(json!({
                "error": format!("{e}"),
            })),
        }
    }
}
