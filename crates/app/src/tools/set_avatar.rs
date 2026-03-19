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
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetAvatarParams {
    /// Image filename in the images directory (e.g. 'avatar.jpg').
    filename: String,
}

/// Change the Telegram bot's profile photo from the images directory.
#[derive(ToolDef)]
#[tool(
    name = "set-avatar",
    description = "Change the Telegram bot's profile photo. The image file must be placed in the \
                   images directory beforehand. Use a filename relative to the images directory.",
    bypass_interceptor
)]
pub struct SetAvatarTool {
    settings: Arc<dyn SettingsProvider>,
}
impl SetAvatarTool {
    pub fn new(settings: Arc<dyn SettingsProvider>) -> Self { Self { settings } }
}

#[async_trait]
impl ToolExecute for SetAvatarTool {
    type Output = Value;
    type Params = SetAvatarParams;

    async fn run(&self, params: SetAvatarParams, _context: &ToolContext) -> anyhow::Result<Value> {
        let ext = std::path::Path::new(&params.filename)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();
        let mime = match ext.as_str() {
            "jpg" | "jpeg" => "image/jpeg",
            "png" => "image/png",
            _ => {
                return Ok(
                    serde_json::json!({"error": "unsupported image format; only jpg, jpeg, and png are allowed"}),
                );
            }
        };
        let path = rara_paths::images_dir().join(&params.filename);
        if !path.is_file() {
            return Ok(serde_json::json!({"error": format!("file not found: {}", path.display())}));
        }
        let data = tokio::fs::read(&path)
            .await
            .map_err(|e| anyhow::anyhow!("failed to read file '{}': {e}", path.display()))?;
        let token = self
            .settings
            .get(keys::TELEGRAM_BOT_TOKEN)
            .await
            .ok_or_else(|| anyhow::anyhow!("telegram.bot_token not configured"))?;
        let url = format!("https://api.telegram.org/bot{token}/setMyProfilePhoto");
        let photo_part = reqwest::multipart::Part::bytes(data)
            .file_name(params.filename.clone())
            .mime_str(mime)?;
        let form = reqwest::multipart::Form::new()
            .text(
                "photo",
                r#"{"type":"static","photo":"attach://photo_file"}"#,
            )
            .part("photo_file", photo_part);
        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .multipart(form)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("telegram API request failed: {e}"))?;
        let status = resp.status();
        let body: Value = resp
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("failed to parse telegram response: {e}"))?;
        if status.is_success() && body.get("ok").and_then(|v| v.as_bool()) == Some(true) {
            tracing::info!(filename = %params.filename, "bot profile photo updated");
            Ok(serde_json::json!({"status": "updated", "filename": params.filename}))
        } else {
            let description = body
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            tracing::error!(filename = %params.filename, error = %description, "failed to set profile photo");
            Ok(serde_json::json!({"error": format!("telegram API error: {description}")}))
        }
    }
}
