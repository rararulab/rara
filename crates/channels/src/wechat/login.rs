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

use snafu::OptionExt;
use tracing::{info, warn};

use super::{
    api::WeixinApiClient,
    errors::{LoginFailedSnafu, QrCodeExpiredSnafu, Result},
    storage::{self, AccountData, DEFAULT_BASE_URL},
};

/// Performs an interactive QR-code login and persists the resulting
/// credentials under `~/.config/rara/wechat/`.
///
/// Returns the account ID on success.
pub async fn login(base_url: Option<&str>) -> Result<String> {
    let base_url = base_url.unwrap_or(DEFAULT_BASE_URL);
    let client = WeixinApiClient::new(base_url, "", None);

    let qr_resp = client.fetch_qr_code().await?;
    let qrcode_url = qr_resp["data"]["qrcode_url"]
        .as_str()
        .context(LoginFailedSnafu {
            reason: "no qrcode_url in response",
        })?;
    let qrcode_id = qr_resp["data"]["qrcode_id"]
        .as_str()
        .context(LoginFailedSnafu {
            reason: "no qrcode_id in response",
        })?;

    // Render QR code as ASCII art in the terminal.
    let qr = qrcode::QrCode::new(qrcode_url.as_bytes()).map_err(|e| {
        LoginFailedSnafu {
            reason: format!("QR generation failed: {e}"),
        }
        .build()
    })?;
    let image = qr
        .render::<char>()
        .quiet_zone(true)
        .module_dimensions(2, 1)
        .build();
    println!("{image}");
    println!("Scan the QR code above with WeChat to login");

    // Poll until the user scans and confirms.
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
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
                let data = &status_resp["data"];
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
                return Ok(account_id);
            }
            other => {
                warn!("Unknown QR status: {other}");
            }
        }
    }
}
