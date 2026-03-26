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

use snafu::{ResultExt, Whatever};

use super::prompt::{self, SetupMode};

/// Telegram configuration result.
pub struct TelegramResult {
    /// Bot token from @BotFather.
    pub bot_token: String,
    /// Primary chat ID for the bot.
    pub chat_id:   String,
}

/// Guide the user through Telegram bot configuration.
pub async fn setup_telegram(
    existing: Option<&rara_app::flatten::TelegramConfig>,
    mode: SetupMode,
) -> Result<Option<TelegramResult>, Whatever> {
    prompt::print_step("Telegram Bot");

    if mode == SetupMode::FillMissing && existing.is_some() {
        prompt::print_ok("already configured, skipping");
        return Ok(None);
    }

    if !prompt::confirm("Configure Telegram bot?", true) {
        return Ok(None);
    }

    let default_token = existing.and_then(|t| t.bot_token.as_deref());

    let bot_token = loop {
        let token = prompt::ask("Bot Token (from @BotFather)", default_token);

        match validate_bot_token(&token).await {
            Ok(username) => {
                prompt::print_ok(&format!("bot verified: @{username}"));
                break token;
            }
            Err(e) => {
                prompt::print_err(&format!("invalid bot token: {e}"));
                if !prompt::confirm("Retry?", true) {
                    return Ok(None);
                }
            }
        }
    };

    let default_chat = existing.and_then(|t| t.chat_id.as_deref());
    let chat_id = prompt::ask("Primary Chat ID", default_chat);

    // Validate by sending test message
    if prompt::confirm("Send a test message to verify?", true) {
        match send_test_message(&bot_token, &chat_id).await {
            Ok(()) => prompt::print_ok("test message sent"),
            Err(e) => prompt::print_err(&format!("failed to send: {e}")),
        }
    }

    Ok(Some(TelegramResult { bot_token, chat_id }))
}

/// Validate bot token by calling the Telegram `getMe` endpoint.
/// Returns the bot username on success.
async fn validate_bot_token(token: &str) -> Result<String, Whatever> {
    let client = reqwest::Client::new();
    let url = format!("https://api.telegram.org/bot{token}/getMe");

    let resp = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .whatever_context("request failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        snafu::whatever!("Telegram API returned {status}");
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .whatever_context("invalid JSON response")?;

    let username = body
        .pointer("/result/username")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_owned();

    Ok(username)
}

/// Send a test message to the given chat ID.
async fn send_test_message(token: &str, chat_id: &str) -> Result<(), Whatever> {
    let client = reqwest::Client::new();
    let url = format!("https://api.telegram.org/bot{token}/sendMessage");

    let body = serde_json::json!({
        "chat_id": chat_id,
        "text": "rara setup -- test message OK"
    });

    let resp = client
        .post(&url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .whatever_context("request failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        snafu::whatever!("Telegram API returned {status}: {text}");
    }

    Ok(())
}
