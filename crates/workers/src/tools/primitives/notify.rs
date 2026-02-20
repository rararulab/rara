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
use rara_agents::tool_registry::AgentTool;
use rara_domain_shared::{
    notify::{
        client::NotifyClient,
        types::{NotificationPriority, SendTelegramNotificationRequest},
    },
    settings::SettingsSvc,
};
use serde_json::json;

/// Layer 1 primitive: enqueue a notification message.
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
    fn name(&self) -> &str { "notify" }

    fn description(&self) -> &str {
        "Send a notification message. Currently supports the telegram channel. The chat_id is read \
         from runtime settings automatically. Returns the queued notification id."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "channel": {
                    "type": "string",
                    "enum": ["telegram"],
                    "description": "Notification channel (currently only telegram)"
                },
                "message": {
                    "type": "string",
                    "description": "The notification message body"
                }
            },
            "required": ["channel", "message"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let channel = params
            .get("channel")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: channel"))?;

        let message = params
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: message"))?;

        match channel {
            "telegram" => {
                let settings = self.settings_svc.current();
                let chat_id = settings.telegram.chat_id;

                let request = SendTelegramNotificationRequest {
                    chat_id,
                    subject: None,
                    body: message.to_owned(),
                    priority: NotificationPriority::Normal,
                    max_retries: 3,
                    reference_type: None,
                    reference_id: None,
                    metadata: None,
                    photo_path: None,
                };

                match self.client.send_telegram(request).await {
                    Ok(queued) => Ok(json!({
                        "sent": true,
                        "id": queued.id.to_string(),
                    })),
                    Err(e) => Ok(json!({
                        "error": format!("{e}"),
                    })),
                }
            }
            other => Ok(json!({
                "error": format!("unsupported channel: {other}"),
            })),
        }
    }
}
