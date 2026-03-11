// Copyright 2025 Rararulab
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

//! Set bot profile photo via Telegram Bot API.

use std::sync::Arc;

use async_trait::async_trait;
use rara_domain_shared::settings::{SettingsProvider, keys};
use rara_kernel::tool::{AgentTool, ToolOutput};
use serde_json::json;

/// Set the Telegram bot's profile photo from a local file or URL.
pub struct SetAvatarTool {
    settings: Arc<dyn SettingsProvider>,
}

impl SetAvatarTool {
    pub fn new(settings: Arc<dyn SettingsProvider>) -> Self {
        Self { settings }
    }
}

#[async_trait]
impl AgentTool for SetAvatarTool {
    fn name(&self) -> &str {
        "set-avatar"
    }

    fn description(&self) -> &str {
        "Set the bot's Telegram profile photo. Accepts a local file path or an image URL as the \
         source."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "source": {
                    "type": "string",
                    "description": "Local file path or URL of the image to use as the profile photo"
                }
            },
            "required": ["source"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _context: &rara_kernel::tool::ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let source = params
            .get("source")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: source"))?;

        // Read bot token from settings.
        let token = self
            .settings
            .get(keys::TELEGRAM_BOT_TOKEN)
            .await
            .ok_or_else(|| anyhow::anyhow!("telegram.bot_token not configured"))?;

        // Obtain image bytes.
        let image_bytes = if source.starts_with("http://") || source.starts_with("https://") {
            let client = reqwest::Client::new();
            let resp = client
                .get(source)
                .header("Referer", "https://www.pixiv.net/")
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("failed to download image: {e}"))?;

            if !resp.status().is_success() {
                return Ok(
                    json!({ "error": format!("download failed with status {}", resp.status()) })
                        .into(),
                );
            }

            resp.bytes()
                .await
                .map_err(|e| anyhow::anyhow!("failed to read image bytes: {e}"))?
                .to_vec()
        } else {
            tokio::fs::read(source)
                .await
                .map_err(|e| anyhow::anyhow!("failed to read file '{}': {e}", source))?
        };

        // Build multipart form for Telegram setMyProfilePhoto API.
        let photo_file_part = reqwest::multipart::Part::bytes(image_bytes)
            .file_name("photo.png")
            .mime_str("image/png")?;

        let form = reqwest::multipart::Form::new()
            .text(
                "photo",
                r#"{"type":"static","photo":"attach://photo_file"}"#,
            )
            .part("photo_file", photo_file_part);

        let url = format!(
            "https://api.telegram.org/bot{}/setMyProfilePhoto",
            token
        );

        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .multipart(form)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("telegram API request failed: {e}"))?;

        let status = resp.status();
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("failed to parse telegram response: {e}"))?;

        if status.is_success() && body.get("ok").and_then(|v| v.as_bool()) == Some(true) {
            tracing::info!(source = %source, "bot profile photo updated");
            Ok(json!({
                "status": "updated",
                "source": source,
            })
            .into())
        } else {
            let description = body
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            tracing::error!(source = %source, error = %description, "failed to set profile photo");
            Ok(json!({
                "error": format!("telegram API error: {description}"),
            })
            .into())
        }
    }
}
