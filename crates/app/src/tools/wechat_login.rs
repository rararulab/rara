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
//! When invoked the tool prints a QR code to stdout for the user to
//! scan with WeChat. On success the credentials are persisted to
//! `~/.config/rara/wechat/` and the account ID is returned so the
//! agent can confirm the setup is complete.

use anyhow::Context;
use async_trait::async_trait;
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
    pub account_id: String,
    /// Human-readable status message.
    pub message:    String,
}

/// Initiates an interactive WeChat iLink Bot QR-code login.
///
/// A QR code is displayed in the terminal for the user to scan with
/// WeChat. Credentials are saved automatically and the adapter will
/// pick them up on next restart.
#[derive(ToolDef)]
#[tool(
    name = "wechat-login",
    description = "Start WeChat iLink Bot login. Displays a QR code in the terminal for the user \
                   to scan with WeChat. Credentials are saved automatically.",
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
        let account_id = rara_channels::wechat::login::login(params.base_url.as_deref())
            .await
            .context("wechat login failed")?;

        Ok(WechatLoginResult {
            message: format!(
                "WeChat login successful. Account {account_id} saved. Restart rara to activate \
                 the WeChat channel."
            ),
            account_id,
        })
    }
}
