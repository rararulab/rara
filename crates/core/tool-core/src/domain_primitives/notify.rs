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
//! telegram notification via the `NotifyClient`. When a `recipient` is
//! specified, the contacts allowlist is checked first — unknown or disabled
//! contacts are rejected.

use std::sync::Arc;

use async_trait::async_trait;
use rara_domain_shared::{
    notify::{
        client::NotifyClient,
        types::{NotificationPriority, SendTelegramNotificationRequest},
    },
    settings::model::Settings,
};
use tokio::sync::watch;
use serde_json::json;

use crate::contact_lookup::ContactLookup;
use crate::AgentTool;

/// Layer 1 primitive: send a Telegram message.
///
/// Enqueues a notification via PGMQ which the telegram-bot process consumes
/// and delivers.  Supports plain text, bold subject headers, photos, and
/// sending to arbitrary chat IDs (not just the primary one from settings).
///
/// When `recipient` is set, the contacts allowlist table is checked:
/// - Not found or disabled → error returned
/// - Found with chat_id → uses that chat_id directly
/// - Found without chat_id → enqueues with recipient for bot to resolve
pub struct NotifyTool {
    client:      NotifyClient,
    settings_rx: watch::Receiver<Settings>,
    contacts:    Arc<dyn ContactLookup>,
}

impl NotifyTool {
    pub fn new(
        client: NotifyClient,
        settings_rx: watch::Receiver<Settings>,
        contacts: Arc<dyn ContactLookup>,
    ) -> Self {
        Self {
            client,
            settings_rx,
            contacts,
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
                    "description": "Telegram username of the recipient (without @ prefix). Must be in the contacts allowlist. Omit to send to the default chat."
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

        // When a recipient is specified, enforce the contacts allowlist.
        let (chat_id, resolved_recipient) = if let Some(ref username) = recipient {
            match self.contacts.find_by_username(username).await {
                Ok(Some(contact)) if contact.enabled => {
                    // Contact found and enabled — use their chat_id if known.
                    (contact.chat_id, Some(contact.username))
                }
                Ok(Some(_)) => {
                    // Contact exists but is disabled.
                    return Ok(json!({
                        "error": "recipient not in contacts allowlist (contact is disabled)",
                    }));
                }
                Ok(None) => {
                    // Contact not in allowlist.
                    return Ok(json!({
                        "error": "recipient not in contacts allowlist",
                    }));
                }
                Err(e) => {
                    return Ok(json!({
                        "error": format!("failed to check contacts allowlist: {e}"),
                    }));
                }
            }
        } else {
            // No recipient — use settings chat_id as fallback.
            (self.settings_rx.borrow().telegram.chat_id, None)
        };

        let request = SendTelegramNotificationRequest {
            chat_id,
            recipient: resolved_recipient.clone(),
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
                "recipient": resolved_recipient.or(recipient),
            })),
            Err(e) => Ok(json!({
                "error": format!("{e}"),
            })),
        }
    }
}
