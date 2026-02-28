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

//! Pipeline-specific notify tool.
//!
//! Overrides the default `send_telegram` primitive so that pipeline
//! notifications are routed to the dedicated Telegram notification channel
//! when `notification_channel_id` is configured.  Falls back to the
//! PGMQ-based `NotifyClient` otherwise.

use async_trait::async_trait;
use rara_domain_shared::notify::{
    client::NotifyClient,
    types::{NotificationPriority, SendTelegramNotificationRequest},
};
use serde_json::json;
use rara_kernel::tool::AgentTool;
use tracing::warn;

use crate::settings::SettingsSvc;

/// Pipeline-specific notify tool that routes messages to the dedicated
/// Telegram channel (via Bot API) when configured, falling back to the
/// standard PGMQ path.
pub struct PipelineNotifyTool {
    settings_svc:  SettingsSvc,
    notify_client: NotifyClient,
}

impl PipelineNotifyTool {
    pub fn new(settings_svc: SettingsSvc, notify_client: NotifyClient) -> Self {
        Self {
            settings_svc,
            notify_client,
        }
    }
}

#[async_trait]
impl AgentTool for PipelineNotifyTool {
    fn name(&self) -> &str { "send_telegram" }

    fn description(&self) -> &str {
        "Send a message to a Telegram chat. Use this to proactively notify or communicate with \
         users via Telegram. Supports text, bold subject headers, and photo attachments. By \
         default the message is sent to the primary chat from settings; provide chat_id to send to \
         a different user or group."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "The message body text (supports Markdown)"
                },
                "subject": {
                    "type": "string",
                    "description": "Optional bold subject line displayed above the message body"
                },
                "chat_id": {
                    "type": "integer",
                    "description": "Telegram chat ID to send to. Omit to use the primary chat from settings"
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

        let subject = params
            .get("subject")
            .and_then(|v| v.as_str())
            .map(String::from);

        let settings = self.settings_svc.current();

        // Build the full text body, optionally prepending a bold subject line.
        let body = match &subject {
            Some(subj) => format!("*{subj}*\n\n{message}"),
            None => message.to_owned(),
        };

        // Fast path: when notification_channel_id is configured, send directly
        // to the channel via Bot API (fire-and-forget, no PGMQ persistence).
        if let (Some(token), Some(channel_id)) = (
            settings.telegram.bot_token.as_deref(),
            settings.telegram.notification_channel_id,
        ) {
            let url = format!("https://api.telegram.org/bot{token}/sendMessage");
            let payload = json!({
                "chat_id": channel_id,
                "text": body,
                "parse_mode": "Markdown",
            });
            match reqwest::Client::new()
                .post(&url)
                .json(&payload)
                .send()
                .await
            {
                Ok(resp) if !resp.status().is_success() => {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    warn!(
                        %status, body = %text,
                        "pipeline notify tool: telegram channel send failed"
                    );
                    return Ok(json!({
                        "error": format!("telegram API {status}: {text}"),
                    }));
                }
                Err(e) => {
                    warn!(error = %e, "pipeline notify tool: failed to send to channel");
                    return Ok(json!({
                        "error": format!("{e}"),
                    }));
                }
                Ok(_) => {
                    return Ok(json!({
                        "sent": true,
                        "chat_id": channel_id,
                        "destination": "channel",
                    }));
                }
            }
        }

        // Fallback: enqueue via PGMQ-based notify client (same as the
        // default send_telegram primitive).
        let chat_id = params
            .get("chat_id")
            .and_then(|v| v.as_i64())
            .or(settings.telegram.chat_id);

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

        let photo_path = params
            .get("photo_path")
            .and_then(|v| v.as_str())
            .map(String::from);

        let request = SendTelegramNotificationRequest {
            chat_id,
            recipient: None,
            subject,
            body: message.to_owned(),
            priority,
            max_retries: 3,
            reference_type: None,
            reference_id: None,
            metadata: None,
            photo_path,
        };

        match self.notify_client.send_telegram(request).await {
            Ok(queued) => Ok(json!({
                "sent": true,
                "id": queued.id.to_string(),
                "chat_id": chat_id,
                "destination": "pgmq",
            })),
            Err(e) => Ok(json!({
                "error": format!("{e}"),
            })),
        }
    }
}
