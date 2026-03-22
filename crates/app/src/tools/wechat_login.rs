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

//! Two-step WeChat iLink Bot login tools for agents.
//!
//! 1. `wechat-login-start` — requests a QR code and saves the PNG to a temp
//!    file. Returns the image path and a `qrcode_id` token immediately so the
//!    agent can send the image to the user.
//!
//! 2. `wechat-login-confirm` — polls until the user scans the QR code, then
//!    persists credentials and returns the account ID.

use std::path::PathBuf;

use anyhow::Context;
use async_trait::async_trait;
use rara_channels::wechat::login::LoginSession;
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// wechat-login-start
// ---------------------------------------------------------------------------

/// Input parameters for the wechat-login-start tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct WechatLoginStartParams {
    /// Optional override for the iLink API base URL.
    #[serde(default)]
    pub base_url: Option<String>,
}

/// Result returned by the wechat-login-start tool.
#[derive(Debug, Clone, Serialize)]
pub struct WechatLoginStartResult {
    /// Path to the QR code PNG image. Send this to the user to scan.
    pub qrcode_image: PathBuf,
    /// Opaque session token. Pass to `wechat-login-confirm`.
    pub qrcode_id:    String,
    /// API base URL for this session.
    pub base_url:     String,
    /// Human-readable instructions.
    pub message:      String,
}

/// Step 1: request a QR code and save it as a PNG image.
///
/// Returns immediately with the image path and a `qrcode_id`.
/// The agent should send the image to the user, then call
/// `wechat-login-confirm` with the `qrcode_id`.
#[derive(ToolDef)]
#[tool(
    name = "wechat-login-start",
    description = "Start WeChat login. Returns a QR code image path and a session token. Send the \
                   image to the user, then call wechat-login-confirm with the token.",
    tier = "deferred"
)]
pub struct WechatLoginStartTool;

impl WechatLoginStartTool {
    /// Create a new instance.
    pub fn new() -> Self { Self }
}

#[async_trait]
impl ToolExecute for WechatLoginStartTool {
    type Output = WechatLoginStartResult;
    type Params = WechatLoginStartParams;

    async fn run(
        &self,
        params: WechatLoginStartParams,
        _context: &ToolContext,
    ) -> anyhow::Result<WechatLoginStartResult> {
        let session = LoginSession::start(params.base_url.as_deref())
            .await
            .context("failed to start wechat login")?;

        let png_bytes = session.qrcode_png().context("failed to render QR code")?;
        let tmp_dir = std::env::temp_dir().join("rara-wechat");
        std::fs::create_dir_all(&tmp_dir).context("failed to create temp dir")?;
        let filename = format!("login-qrcode-{}.png", uuid::Uuid::new_v4());
        let qrcode_path = tmp_dir.join(&filename);
        std::fs::write(&qrcode_path, &png_bytes).context("failed to write QR code image")?;

        Ok(WechatLoginStartResult {
            qrcode_image: qrcode_path,
            qrcode_id:    session.qrcode_id().to_string(),
            base_url:     session.base_url().to_string(),
            message:      "QR code saved. Send the image to the user and ask them to scan it with \
                           WeChat. Then call wechat-login-confirm."
                .to_string(),
        })
    }
}

// ---------------------------------------------------------------------------
// wechat-login-confirm
// ---------------------------------------------------------------------------

/// Input parameters for the wechat-login-confirm tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct WechatLoginConfirmParams {
    /// The `qrcode_id` returned by `wechat-login-start`.
    pub qrcode_id: String,
    /// The `base_url` returned by `wechat-login-start`.
    pub base_url:  String,
}

/// Result returned after a successful WeChat login confirmation.
#[derive(Debug, Clone, Serialize)]
pub struct WechatLoginConfirmResult {
    /// The account ID that was authenticated.
    pub account_id: String,
    /// Human-readable status message.
    pub message:    String,
}

/// Step 2: poll until the user scans the QR code.
///
/// Call this after sending the QR code image to the user. Blocks
/// until the user scans and confirms (up to 5 minutes).
#[derive(ToolDef)]
#[tool(
    name = "wechat-login-confirm",
    description = "Wait for the user to scan the WeChat QR code. Pass the qrcode_id and base_url \
                   from wechat-login-start. Blocks until confirmed (up to 5 minutes).",
    tier = "deferred",
    timeout_secs = 330
)]
pub struct WechatLoginConfirmTool;

impl WechatLoginConfirmTool {
    /// Create a new instance.
    pub fn new() -> Self { Self }
}

#[async_trait]
impl ToolExecute for WechatLoginConfirmTool {
    type Output = WechatLoginConfirmResult;
    type Params = WechatLoginConfirmParams;

    async fn run(
        &self,
        params: WechatLoginConfirmParams,
        _context: &ToolContext,
    ) -> anyhow::Result<WechatLoginConfirmResult> {
        let account_id = rara_channels::wechat::login::wait_for_confirmation_with(
            &params.qrcode_id,
            &params.base_url,
        )
        .await
        .context("wechat login failed")?;

        Ok(WechatLoginConfirmResult {
            message: format!(
                "WeChat login successful. Account {account_id} saved. Restart rara to activate \
                 the WeChat channel."
            ),
            account_id,
        })
    }
}
