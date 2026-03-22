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

//! Interactive QR-code login flow for WeChat iLink Bot.
//!
//! Usage from CLI:
//! ```ignore
//! let session = LoginSession::start(None).await?;
//! println!("{}", session.qrcode_terminal());
//! let account_id = session.wait_for_confirmation().await?;
//! ```
//!
//! Usage from an agent tool:
//! ```ignore
//! let session = LoginSession::start(None).await?;
//! let png_bytes = session.qrcode_png()?;
//! // save to temp file, send to user
//! let account_id = session.wait_for_confirmation().await?;
//! ```

use std::time::Duration;

use snafu::OptionExt;
use tracing::{info, warn};

use super::{
    api::WeixinApiClient,
    errors::{LoginFailedSnafu, QrCodeExpiredSnafu, Result},
    storage::{self, AccountData, DEFAULT_BASE_URL},
};

/// Maximum time to wait for the user to scan the QR code.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

/// Width/height of the generated QR code PNG in pixels.
const QR_IMAGE_SIZE: u32 = 512;

/// An in-progress login session awaiting QR code scan.
///
/// Created by [`LoginSession::start`]. The caller is responsible for
/// presenting the QR code to the user (via terminal or image file),
/// then calling [`wait_for_confirmation`](Self::wait_for_confirmation).
pub struct LoginSession {
    client:    WeixinApiClient,
    qr:        qrcode::QrCode,
    qrcode_id: String,
    base_url:  String,
}

impl LoginSession {
    /// Initiates a login session by requesting a QR code from the API.
    pub async fn start(base_url: Option<&str>) -> Result<Self> {
        let base_url = base_url.unwrap_or(DEFAULT_BASE_URL);
        let client = WeixinApiClient::new(base_url, "", None);

        let qr_resp = client.fetch_qr_code().await?;
        let qrcode_url = qr_resp["data"]["qrcode_url"]
            .as_str()
            .context(LoginFailedSnafu {
                reason: "no qrcode_url in response",
            })?
            .to_string();
        let qrcode_id = qr_resp["data"]["qrcode_id"]
            .as_str()
            .context(LoginFailedSnafu {
                reason: "no qrcode_id in response",
            })?
            .to_string();

        let qr = qrcode::QrCode::new(qrcode_url.as_bytes()).map_err(|e| {
            LoginFailedSnafu {
                reason: format!("QR generation failed: {e}"),
            }
            .build()
        })?;

        Ok(Self {
            client,
            qr,
            qrcode_id,
            base_url: base_url.to_string(),
        })
    }

    /// Renders the QR code as ASCII art suitable for terminal display.
    pub fn qrcode_terminal(&self) -> String {
        self.qr
            .render::<char>()
            .quiet_zone(true)
            .module_dimensions(2, 1)
            .build()
    }

    /// Renders the QR code as a PNG image and returns the raw bytes.
    pub fn qrcode_png(&self) -> Result<Vec<u8>> {
        let img = self
            .qr
            .render::<image::Luma<u8>>()
            .quiet_zone(true)
            .min_dimensions(QR_IMAGE_SIZE, QR_IMAGE_SIZE)
            .build();

        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png)
            .map_err(|e| {
                LoginFailedSnafu {
                    reason: format!("PNG encoding failed: {e}"),
                }
                .build()
            })?;

        Ok(buf.into_inner())
    }

    /// Returns the QR code ID for this session.
    ///
    /// Can be passed to [`wait_for_confirmation_with`] to resume
    /// polling in a separate tool call.
    pub fn qrcode_id(&self) -> &str { &self.qrcode_id }

    /// Returns the API base URL for this session.
    pub fn base_url(&self) -> &str { &self.base_url }

    /// Polls the API until the user scans and confirms the QR code.
    ///
    /// Times out after 5 minutes. On success the credentials are
    /// persisted to `~/.config/rara/wechat/` and the account ID is
    /// returned.
    pub async fn wait_for_confirmation(self) -> Result<String> {
        wait_for_confirmation_with(&self.qrcode_id, &self.base_url).await
    }
}

/// Resumes polling for a previously started login session.
///
/// This is the two-step counterpart to [`LoginSession::start`]:
/// call `start` to get the QR code, present it to the user, then
/// call this function with the `qrcode_id` and `base_url` to wait
/// for confirmation.
pub async fn wait_for_confirmation_with(qrcode_id: &str, base_url: &str) -> Result<String> {
    let client = WeixinApiClient::new(base_url, "", None);
    tokio::time::timeout(
        LOGIN_TIMEOUT,
        poll_until_confirmed(&client, qrcode_id, base_url),
    )
    .await
    .map_err(|_| {
        LoginFailedSnafu {
            reason: "login timed out after 5 minutes",
        }
        .build()
    })?
}

async fn poll_until_confirmed(
    client: &WeixinApiClient,
    qrcode_id: &str,
    base_url: &str,
) -> Result<String> {
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let status_resp = client.get_qr_code_status(qrcode_id).await?;
        let status = status_resp["data"]["status"].as_str().unwrap_or("unknown");

        match status {
            "wait" => {}
            "scaned" => {
                info!("QR code scanned, waiting for confirmation...");
            }
            "expired" => {
                return Err(QrCodeExpiredSnafu.build());
            }
            "confirmed" => {
                return save_confirmed_credentials(&status_resp["data"], base_url);
            }
            other => {
                warn!("Unknown QR status: {other}");
            }
        }
    }
}

fn save_confirmed_credentials(data: &serde_json::Value, base_url: &str) -> Result<String> {
    let token = data["bot_token"].as_str().context(LoginFailedSnafu {
        reason: "no bot_token",
    })?;
    let bot_id = data["ilink_bot_id"].as_str().context(LoginFailedSnafu {
        reason: "no ilink_bot_id",
    })?;
    let base = data["baseurl"].as_str().unwrap_or(base_url);
    let user_id = data["ilink_user_id"].as_str().unwrap_or("");

    let account_id = bot_id
        .strip_prefix("ilink_bot_")
        .unwrap_or(bot_id)
        .to_string();

    let account_data = AccountData {
        token:    token.to_string(),
        saved_at: chrono::Utc::now().to_rfc3339(),
        base_url: base.to_string(),
        user_id:  user_id.to_string(),
    };
    storage::save_account_data(&account_id, &account_data)?;

    let mut ids = storage::get_account_ids().unwrap_or_default();
    if !ids.contains(&account_id) {
        ids.push(account_id.clone());
        storage::save_account_ids(&ids)?;
    }

    info!("Login successful! Account ID: {account_id}");
    Ok(account_id)
}
