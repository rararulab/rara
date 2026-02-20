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

//! Layer 2 service tool for capturing web page screenshots via Playwright.

use std::path::PathBuf;

use async_trait::async_trait;
use tool_core::AgentTool;
use rara_domain_shared::{
    notify::{
        client::NotifyClient,
        types::{NotificationPriority, SendTelegramNotificationRequest},
    },
    settings::SettingsSvc,
};
use serde_json::json;
use tracing::{info, warn};
use uuid::Uuid;

pub struct ScreenshotTool {
    notify:       NotifyClient,
    settings_svc: SettingsSvc,
    project_root: PathBuf,
}

impl ScreenshotTool {
    pub fn new(notify: NotifyClient, settings_svc: SettingsSvc, project_root: PathBuf) -> Self {
        Self {
            notify,
            settings_svc,
            project_root,
        }
    }
}

#[async_trait]
impl AgentTool for ScreenshotTool {
    fn name(&self) -> &str { "screenshot" }

    fn description(&self) -> &str {
        "Take a screenshot of a web page using Playwright and optionally send it to Telegram. \
         Useful for previewing frontend work, checking UI changes, or sharing visual results."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to screenshot (e.g. http://localhost:5173/dashboard)"
                },
                "selector": {
                    "type": "string",
                    "description": "CSS selector to screenshot a specific element (optional)"
                },
                "width": {
                    "type": "number",
                    "description": "Viewport width in pixels (default: 1280)"
                },
                "height": {
                    "type": "number",
                    "description": "Viewport height in pixels (default: 720)"
                },
                "full_page": {
                    "type": "boolean",
                    "description": "Capture full scrollable page (default: false)"
                },
                "send": {
                    "type": "boolean",
                    "description": "Send the screenshot to Telegram (default: true)"
                },
                "caption": {
                    "type": "string",
                    "description": "Optional caption for the Telegram photo"
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let url = params.get("url").and_then(|v| v.as_str()).ok_or_else(|| {
            anyhow::anyhow!("missing required parameter: url")
        })?;

        let selector = params
            .get("selector")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let width = params.get("width").and_then(|v| v.as_u64()).unwrap_or(1280);
        let height = params.get("height").and_then(|v| v.as_u64()).unwrap_or(720);
        let full_page = params
            .get("full_page")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let send = params.get("send").and_then(|v| v.as_bool()).unwrap_or(true);
        let caption = params
            .get("caption")
            .and_then(|v| v.as_str())
            .map(String::from);

        let output_path =
            std::env::temp_dir().join(format!("rara-screenshot-{}.png", Uuid::new_v4()));
        let output_str = output_path.to_string_lossy().to_string();

        // Build and run the screenshot script.
        let script_path = self.project_root.join("scripts/screenshot.mjs");
        let mut cmd = tokio::process::Command::new("node");
        cmd.arg(&script_path)
            .arg(url)
            .arg(&output_str)
            .arg(width.to_string())
            .arg(height.to_string())
            .arg(full_page.to_string())
            .arg(selector)
            .current_dir(&self.project_root)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        info!(url, output = %output_str, "taking screenshot");

        let result = cmd
            .output()
            .await
            .map_err(|e| anyhow::anyhow!("failed to run screenshot script: {e}"))?;

        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            return Err(anyhow::anyhow!("screenshot script failed: {stderr}"));
        }

        // Verify the file exists.
        if !output_path.exists() {
            return Err(anyhow::anyhow!("screenshot file was not created"));
        }

        let mut sent = false;

        // Send to Telegram if requested.
        if send {
            let settings = self.settings_svc.current();
            let chat_id = settings.telegram.chat_id;

            let caption_text = caption.unwrap_or_else(|| format!("Screenshot: {url}"));

            let request = SendTelegramNotificationRequest {
                chat_id,
                subject: None,
                body: caption_text,
                priority: NotificationPriority::Normal,
                max_retries: 3,
                reference_type: None,
                reference_id: None,
                metadata: None,
                photo_path: Some(output_str.clone()),
            };

            match self.notify.send_telegram(request).await {
                Ok(_) => {
                    sent = true;
                    info!(url, "screenshot queued for telegram delivery");
                }
                Err(e) => {
                    warn!(url, error = %e, "failed to queue screenshot notification");
                }
            }
        }

        Ok(json!({
            "success": true,
            "path": output_str,
            "sent": sent,
        }))
    }
}
