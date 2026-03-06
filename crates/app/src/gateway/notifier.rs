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

//! Lightweight Telegram notifier for gateway update lifecycle events.
//!
//! Sends messages directly via the Telegram Bot API (`sendMessage`).
//! Does NOT go through the kernel IO subsystem — the gateway runs
//! independently of the kernel.

use reqwest::Client;
use tracing::warn;

/// Telegram notifier for gateway auto-update events.
///
/// All errors are logged but never propagated — notifications must not
/// break the update pipeline.
pub struct UpdateNotifier {
    client: Client,
    bot_token: String,
    channel_id: String,
}

impl UpdateNotifier {
    /// Create a new notifier.
    ///
    /// Proxy is automatically picked up from `HTTPS_PROXY` / `HTTP_PROXY` /
    /// `ALL_PROXY` environment variables via reqwest's built-in support.
    pub fn new(bot_token: String, channel_id: String) -> Self {
        let mut builder = Client::builder();

        // Honour proxy env vars (same precedence as crates/app/src/lib.rs:471-475).
        let proxy = std::env::var("HTTPS_PROXY")
            .or_else(|_| std::env::var("HTTP_PROXY"))
            .or_else(|_| std::env::var("ALL_PROXY"))
            .ok()
            .filter(|v| !v.is_empty());

        if let Some(ref proxy_url) = proxy {
            match reqwest::Proxy::all(proxy_url) {
                Ok(p) => {
                    builder = builder.proxy(p);
                    tracing::info!(proxy = %proxy_url, "UpdateNotifier: using proxy");
                }
                Err(e) => {
                    warn!(error = %e, proxy = %proxy_url, "UpdateNotifier: invalid proxy URL, ignoring");
                }
            }
        }

        let client = builder.build().unwrap_or_else(|_| Client::new());

        Self {
            client,
            bot_token,
            channel_id,
        }
    }

    /// Send a notification message. Errors are logged but never propagated.
    pub async fn notify(&self, message: &str) {
        let url = format!(
            "https://api.telegram.org/bot{}/sendMessage",
            self.bot_token
        );

        let result = self
            .client
            .post(&url)
            .json(&serde_json::json!({
                "chat_id": self.channel_id,
                "text": message,
                "parse_mode": "HTML",
            }))
            .send()
            .await;

        match result {
            Ok(resp) if !resp.status().is_success() => {
                warn!(
                    status = %resp.status(),
                    "UpdateNotifier: Telegram API returned non-success status"
                );
            }
            Err(e) => {
                warn!(error = %e, "UpdateNotifier: failed to send Telegram notification");
            }
            _ => {}
        }
    }
}
