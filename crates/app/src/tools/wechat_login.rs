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

//! Agent tool that initiates WeChat iLink Bot QR-code login.
//!
//! When invoked the tool saves a QR code PNG to a temporary file and
//! returns its path so the agent can send it to the user. The tool
//! then polls until the user scans and confirms.

use std::path::PathBuf;

use anyhow::Context;
use async_trait::async_trait;
use rara_channels::wechat::login::LoginSession;
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Input parameters for the wechat-login tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct WechatLoginParams {
    /// Optional override for the iLink API base URL.
    #[serde(default)]
    pub base_url: Option<String>,
}

/// Result returned after a successful WeChat login.
#[derive(Debug, Clone, Serialize)]
pub struct WechatLoginResult {
    /// The account ID that was authenticated.
    pub account_id:   String,
    /// Path to the QR code PNG image for the user to scan.
    pub qrcode_image: PathBuf,
    /// Human-readable status message.
    pub message:      String,
}

/// Initiates an interactive WeChat iLink Bot QR-code login.
///
/// Saves a QR code PNG to a temporary file and returns its path.
/// The user must scan the QR code with WeChat within 5 minutes.
/// On success credentials are saved automatically.
#[derive(ToolDef)]
#[tool(
    name = "wechat-login",
    description = "Start WeChat iLink Bot login. Saves a QR code image to a temp file for the \
                   user to scan with WeChat. Returns the image path and waits for confirmation. \
                   Credentials are saved automatically.",
    tier = "deferred",
    timeout_secs = 330
)]
pub struct WechatLoginTool;

impl WechatLoginTool {
    /// Create a new instance.
    pub fn new() -> Self { Self }
}

#[async_trait]
impl ToolExecute for WechatLoginTool {
    type Output = WechatLoginResult;
    type Params = WechatLoginParams;

    async fn run(
        &self,
        params: WechatLoginParams,
        _context: &ToolContext,
    ) -> anyhow::Result<WechatLoginResult> {
        let session = LoginSession::start(params.base_url.as_deref())
            .await
            .context("failed to start wechat login")?;

        // Save QR code PNG to a temp file the agent can send to the user.
        let png_bytes = session.qrcode_png().context("failed to render QR code")?;
        let tmp_dir = std::env::temp_dir().join("rara-wechat");
        std::fs::create_dir_all(&tmp_dir).context("failed to create temp dir")?;
        let qrcode_path = tmp_dir.join("login-qrcode.png");
        std::fs::write(&qrcode_path, &png_bytes).context("failed to write QR code image")?;

        let account_id = session
            .wait_for_confirmation()
            .await
            .context("wechat login failed")?;

        Ok(WechatLoginResult {
            message: format!(
                "WeChat login successful. Account {account_id} saved. Restart rara to activate \
                 the WeChat channel."
            ),
            qrcode_image: qrcode_path,
            account_id,
        })
    }
}
