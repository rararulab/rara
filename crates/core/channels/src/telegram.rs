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

//! Telegram channel adapter.
//!
//! Implements [`ChannelAdapter`] using the Telegram Bot API via `getUpdates`
//! long polling. The polling pattern is inspired by the existing `telegram-bot`
//! crate but adapted for the kernel's channel abstraction.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────┐
//! │         TelegramAdapter                 │
//! │                                         │
//! │  start() ─► spawn polling task          │
//! │              │                          │
//! │              ├── getUpdates (long poll)  │
//! │              │     │                    │
//! │              │     ├── Update → ChannelMessage
//! │              │     │     │              │
//! │              │     │     ▼              │
//! │              │     │  bridge.dispatch() │
//! │              │     │     │              │
//! │              │     │     ▼              │
//! │              │     │  bot.send_message()│
//! │              │     │                    │
//! │              │     └── loop             │
//! │              │                          │
//! │  send()  ─► bot.send_message()          │
//! │  stop()  ─► shutdown signal             │
//! └─────────────────────────────────────────┘
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use rara_kernel::channel::adapter::ChannelAdapter;
use rara_kernel::channel::bridge::ChannelBridge;
use rara_kernel::channel::types::{
    AgentPhase, ChannelMessage, ChannelType, ChannelUser, MessageContent, MessageRole,
    OutboundMessage,
};
use rara_kernel::error::KernelError;
use teloxide::payloads::GetUpdatesSetters;
use teloxide::requests::{Request, Requester};
use teloxide::types::{AllowedUpdate, ChatAction, ChatId, Update, UpdateKind};
use tokio::sync::watch;
use tracing::{error, info, warn};

/// Long-polling timeout in seconds (Telegram server-side wait).
const POLL_TIMEOUT_SECS: u32 = 30;

/// Initial error retry delay.
const INITIAL_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(2);

/// Maximum retry delay for exponential backoff.
const MAX_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(60);

/// Telegram channel adapter using `getUpdates` long polling.
///
/// # Configuration
///
/// - `allowed_chat_ids` — when non-empty, only messages from these chat IDs
///   are processed. Messages from other chats are silently dropped. When empty,
///   all messages are accepted.
///
/// - `polling_timeout` — long-poll timeout in seconds (default: 30). The HTTP
///   client timeout is set 15 seconds higher to avoid premature disconnects.
///
/// # Lifecycle
///
/// 1. Call [`start`](ChannelAdapter::start) with a bridge handle. This spawns
///    a background tokio task that polls for updates.
/// 2. For each inbound text message, the adapter converts the Telegram
///    [`Update`] to a [`ChannelMessage`] and calls `bridge.dispatch()`. The
///    response string is sent back to the originating chat via
///    `bot.send_message()`.
/// 3. Call [`stop`](ChannelAdapter::stop) to signal the polling loop to exit
///    gracefully.
pub struct TelegramAdapter {
    bot: teloxide::Bot,
    allowed_chat_ids: Vec<i64>,
    polling_timeout: u32,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
}

impl TelegramAdapter {
    /// Create a new Telegram adapter.
    ///
    /// # Arguments
    ///
    /// - `bot` — a configured [`teloxide::Bot`] instance
    /// - `allowed_chat_ids` — list of Telegram chat IDs that are permitted to
    ///   interact with the adapter. Pass an empty vec to allow all chats.
    pub fn new(bot: teloxide::Bot, allowed_chat_ids: Vec<i64>) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            bot,
            allowed_chat_ids,
            polling_timeout: POLL_TIMEOUT_SECS,
            shutdown_tx,
            shutdown_rx,
        }
    }

    /// Create a new Telegram adapter with a custom polling timeout.
    pub fn with_polling_timeout(mut self, timeout_secs: u32) -> Self {
        self.polling_timeout = timeout_secs;
        self
    }

    /// Check whether a chat ID is allowed.
    ///
    /// Returns `true` if the allowed list is empty (all chats permitted) or
    /// if the chat ID is explicitly listed.
    fn is_allowed(&self, chat_id: i64) -> bool {
        self.allowed_chat_ids.is_empty() || self.allowed_chat_ids.contains(&chat_id)
    }
}

#[async_trait]
impl ChannelAdapter for TelegramAdapter {
    fn channel_type(&self) -> ChannelType {
        ChannelType::Telegram
    }

    async fn start(
        &self,
        bridge: Arc<dyn ChannelBridge>,
    ) -> Result<(), KernelError> {
        // Delete any existing webhook so getUpdates works.
        self.bot
            .delete_webhook()
            .await
            .map_err(|e| KernelError::Other {
                message: format!("failed to delete webhook: {e}").into(),
            })?;

        // Verify the bot token via getMe.
        let me = self.bot.get_me().await.map_err(|e| KernelError::Other {
            message: format!("failed to verify bot token via getMe: {e}").into(),
        })?;
        info!(
            bot_id = me.id.0,
            bot_username = ?me.username,
            "telegram adapter: bot identity verified"
        );

        let bot = self.bot.clone();
        let allowed_chat_ids = self.allowed_chat_ids.clone();
        let polling_timeout = self.polling_timeout;
        let mut shutdown_rx = self.shutdown_rx.clone();

        tokio::spawn(async move {
            polling_loop(bot, bridge, allowed_chat_ids, polling_timeout, &mut shutdown_rx).await;
        });

        info!("telegram adapter started");
        Ok(())
    }

    async fn send(&self, message: OutboundMessage) -> Result<(), KernelError> {
        let chat_id = parse_chat_id(&message.session_key)?;
        self.bot
            .send_message(ChatId(chat_id), &message.content)
            .await
            .map_err(|e| KernelError::Other {
                message: format!("failed to send telegram message: {e}").into(),
            })?;
        Ok(())
    }

    async fn stop(&self) -> Result<(), KernelError> {
        let _ = self.shutdown_tx.send(true);
        info!("telegram adapter: shutdown signal sent");
        Ok(())
    }

    async fn typing_indicator(&self, session_key: &str) -> Result<(), KernelError> {
        let chat_id = parse_chat_id(session_key)?;
        self.bot
            .send_chat_action(ChatId(chat_id), ChatAction::Typing)
            .await
            .map_err(|e| KernelError::Other {
                message: format!("failed to send typing indicator: {e}").into(),
            })?;
        Ok(())
    }

    async fn set_phase(
        &self,
        _session_key: &str,
        _phase: AgentPhase,
    ) -> Result<(), KernelError> {
        // No-op for now. Could be implemented as emoji reactions in the future.
        Ok(())
    }
}

/// The main getUpdates long-polling loop.
///
/// Runs until the shutdown signal is received or an unrecoverable error
/// (such as `TerminatedByOtherGetUpdates`) is detected. Uses exponential
/// backoff on transient errors.
async fn polling_loop(
    bot: teloxide::Bot,
    bridge: Arc<dyn ChannelBridge>,
    allowed_chat_ids: Vec<i64>,
    polling_timeout: u32,
    shutdown_rx: &mut watch::Receiver<bool>,
) {
    let mut offset: Option<i32> = None;
    let mut retry_delay = INITIAL_RETRY_DELAY;

    info!("telegram adapter: starting getUpdates polling loop");

    loop {
        // Check for shutdown before each poll.
        if *shutdown_rx.borrow() {
            info!("telegram adapter: shutdown received");
            break;
        }

        let mut request = bot
            .get_updates()
            .timeout(polling_timeout)
            .allowed_updates(vec![AllowedUpdate::Message]);

        if let Some(off) = offset {
            request = request.offset(off);
        }

        // Use select to allow shutdown during the long poll.
        let result = tokio::select! {
            _ = shutdown_rx.changed() => {
                info!("telegram adapter: shutdown during getUpdates wait");
                break;
            }
            result = request.send() => result,
        };

        match result {
            Ok(updates) => {
                // Reset retry delay on success.
                retry_delay = INITIAL_RETRY_DELAY;

                for update in updates {
                    // Advance offset past this update.
                    #[allow(clippy::cast_possible_wrap)]
                    let next_offset = update.id.0 as i32 + 1;
                    offset = Some(next_offset);

                    // Spawn handler as a separate task so the polling loop
                    // is never blocked by slow operations (e.g. LLM calls).
                    let bridge = Arc::clone(&bridge);
                    let bot = bot.clone();
                    let allowed = allowed_chat_ids.clone();
                    tokio::spawn(async move {
                        handle_update(update, &bridge, &bot, &allowed).await;
                    });
                }
            }
            Err(teloxide::RequestError::Api(ref api_err)) => {
                let err_str = format!("{api_err}");
                if err_str.contains("terminated by other getUpdates request") {
                    warn!("telegram adapter: another bot instance detected — exiting");
                    break;
                }
                error!(error = %api_err, "telegram adapter: API error in getUpdates");
                tokio::time::sleep(retry_delay).await;
                retry_delay = (retry_delay * 2).min(MAX_RETRY_DELAY);
            }
            Err(e) => {
                error!(error = %e, "telegram adapter: getUpdates request failed");
                tokio::time::sleep(retry_delay).await;
                retry_delay = (retry_delay * 2).min(MAX_RETRY_DELAY);
            }
        }
    }

    info!("telegram adapter: polling loop stopped");
}

/// Handle a single Telegram update.
///
/// Extracts text from the message, converts it to a [`ChannelMessage`],
/// dispatches it via the bridge, and sends the response back.
async fn handle_update(
    update: Update,
    bridge: &Arc<dyn ChannelBridge>,
    bot: &teloxide::Bot,
    allowed_chat_ids: &[i64],
) {
    let UpdateKind::Message(ref msg) = update.kind else {
        return;
    };

    let chat_id = msg.chat.id.0;

    // Check if this chat is allowed.
    if !allowed_chat_ids.is_empty() && !allowed_chat_ids.contains(&chat_id) {
        warn!(chat_id, "telegram adapter: dropping message from unauthorized chat");
        return;
    }

    // Extract text content.
    let Some(text) = msg.text() else {
        return;
    };

    if text.trim().is_empty() {
        return;
    }

    // Convert to ChannelMessage.
    let channel_message = telegram_update_to_channel_message(&update, text);

    // Send typing indicator.
    let _ = bot.send_chat_action(ChatId(chat_id), ChatAction::Typing).await;

    // Dispatch to the bridge.
    match bridge.dispatch(channel_message).await {
        Ok(response) => {
            if !response.trim().is_empty() {
                if let Err(e) = bot.send_message(ChatId(chat_id), &response).await {
                    error!(error = %e, chat_id, "telegram adapter: failed to send response");
                }
            }
        }
        Err(e) => {
            error!(error = %e, chat_id, "telegram adapter: bridge dispatch failed");
            let _ = bot
                .send_message(ChatId(chat_id), format!("Error: {e}"))
                .await;
        }
    }
}

/// Convert a Telegram [`Update`] with text content into a [`ChannelMessage`].
///
/// Extracts user info, chat ID (used as session key), and message text.
fn telegram_update_to_channel_message(update: &Update, text: &str) -> ChannelMessage {
    let msg = match &update.kind {
        UpdateKind::Message(msg) => msg,
        _ => unreachable!("caller ensures this is a Message update"),
    };

    let chat_id = msg.chat.id.0;
    let (platform_id, display_name) = extract_user_info(msg);

    ChannelMessage {
        id: ulid::Ulid::new().to_string(),
        channel_type: ChannelType::Telegram,
        user: ChannelUser {
            platform_id,
            display_name,
        },
        session_key: format_session_key(chat_id),
        role: MessageRole::User,
        content: MessageContent::Text(text.to_owned()),
        tool_call_id: None,
        tool_name: None,
        timestamp: jiff::Timestamp::now(),
        metadata: build_metadata(update, msg),
    }
}

/// Extract user info from a Telegram message.
///
/// Returns `(platform_id, display_name)`. The platform ID is the Telegram
/// user ID; the display name is built from first name + optional last name.
fn extract_user_info(msg: &teloxide::types::Message) -> (String, Option<String>) {
    match msg.from.as_ref() {
        Some(user) => {
            let platform_id = user.id.0.to_string();
            let display_name = if let Some(ref last) = user.last_name {
                Some(format!("{} {last}", user.first_name))
            } else {
                Some(user.first_name.clone())
            };
            (platform_id, display_name)
        }
        None => ("unknown".to_owned(), None),
    }
}

/// Build metadata from a Telegram update for adapter-specific extensions.
fn build_metadata(
    update: &Update,
    msg: &teloxide::types::Message,
) -> HashMap<String, serde_json::Value> {
    let mut meta = HashMap::new();
    meta.insert(
        "telegram_update_id".to_owned(),
        serde_json::Value::Number(update.id.0.into()),
    );
    meta.insert(
        "telegram_message_id".to_owned(),
        serde_json::Value::Number(msg.id.0.into()),
    );
    meta.insert(
        "telegram_chat_id".to_owned(),
        serde_json::json!(msg.chat.id.0),
    );
    if let Some(ref user) = msg.from {
        if let Some(ref username) = user.username {
            meta.insert(
                "telegram_username".to_owned(),
                serde_json::Value::String(username.clone()),
            );
        }
    }
    meta
}

/// Format a Telegram chat ID as a session key.
///
/// The format is `tg:{chat_id}`, which is stable and parseable.
pub fn format_session_key(chat_id: i64) -> String {
    format!("tg:{chat_id}")
}

/// Parse a chat ID from a session key.
///
/// Supports the canonical `tg:{chat_id}` format as well as plain numeric
/// strings for convenience.
pub fn parse_chat_id(session_key: &str) -> Result<i64, KernelError> {
    let id_str = session_key.strip_prefix("tg:").unwrap_or(session_key);
    id_str.parse::<i64>().map_err(|_| KernelError::Other {
        message: format!("invalid telegram session key: {session_key}").into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_and_parse_session_key() {
        let chat_id: i64 = 123_456_789;
        let key = format_session_key(chat_id);
        assert_eq!(key, "tg:123456789");

        let parsed = parse_chat_id(&key).unwrap();
        assert_eq!(parsed, chat_id);
    }

    #[test]
    fn parse_plain_numeric_session_key() {
        let parsed = parse_chat_id("987654321").unwrap();
        assert_eq!(parsed, 987_654_321);
    }

    #[test]
    fn parse_negative_chat_id() {
        // Group chats have negative IDs.
        let key = format_session_key(-100_123_456_789);
        assert_eq!(key, "tg:-100123456789");

        let parsed = parse_chat_id(&key).unwrap();
        assert_eq!(parsed, -100_123_456_789);
    }

    #[test]
    fn parse_invalid_session_key() {
        let result = parse_chat_id("invalid");
        assert!(result.is_err());
    }

    #[test]
    fn is_allowed_empty_list_allows_all() {
        let bot = teloxide::Bot::new("fake_token");
        let adapter = TelegramAdapter::new(bot, vec![]);
        assert!(adapter.is_allowed(12345));
        assert!(adapter.is_allowed(-100999));
    }

    #[test]
    fn is_allowed_checks_list() {
        let bot = teloxide::Bot::new("fake_token");
        let adapter = TelegramAdapter::new(bot, vec![111, 222, -100333]);
        assert!(adapter.is_allowed(111));
        assert!(adapter.is_allowed(222));
        assert!(adapter.is_allowed(-100333));
        assert!(!adapter.is_allowed(999));
    }

    #[test]
    fn channel_type_is_telegram() {
        let bot = teloxide::Bot::new("fake_token");
        let adapter = TelegramAdapter::new(bot, vec![]);
        assert_eq!(adapter.channel_type(), ChannelType::Telegram);
    }

    #[test]
    fn with_polling_timeout_sets_value() {
        let bot = teloxide::Bot::new("fake_token");
        let adapter = TelegramAdapter::new(bot, vec![]).with_polling_timeout(60);
        assert_eq!(adapter.polling_timeout, 60);
    }
}
