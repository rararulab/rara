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
//! │  send()  ─► bot.send_message() (HTML)   │
//! │  stop()  ─► shutdown signal             │
//! └─────────────────────────────────────────┘
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use rara_kernel::channel::adapter::ChannelAdapter;
use rara_kernel::channel::bridge::ChannelBridge;
use rara_kernel::channel::command::{
    CallbackContext, CallbackHandler, CallbackResult, CommandContext, CommandHandler, CommandInfo,
    CommandResult,
};
use rara_kernel::channel::types::{
    AgentPhase, ChannelMessage, ChannelType, ChannelUser, ContentBlock, MessageContent,
    MessageRole, OutboundMessage,
};
use rara_kernel::error::KernelError;
use teloxide::payloads::{EditMessageTextSetters, GetUpdatesSetters, SendMessageSetters};
use teloxide::requests::{Request, Requester};
use teloxide::types::{
    AllowedUpdate, ChatAction, ChatId, MaybeInaccessibleMessage, Update, UpdateKind,
};
use tokio::sync::{watch, RwLock};
use tracing::{error, info, warn};

/// Long-polling timeout in seconds (Telegram server-side wait).
const POLL_TIMEOUT_SECS: u32 = 30;

/// Groups with this many members or fewer are treated like private chats --
/// the bot responds to every message without requiring an @mention or keyword.
const SMALL_GROUP_THRESHOLD: u32 = 3;

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
///    `bot.send_message()` formatted as Telegram HTML.
/// 3. Call [`stop`](ChannelAdapter::stop) to signal the polling loop to exit
///    gracefully.
pub struct TelegramAdapter {
    bot: teloxide::Bot,
    allowed_chat_ids: Vec<i64>,
    polling_timeout: u32,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
    /// Bot username from getMe (set during start).
    bot_username: Arc<RwLock<Option<String>>>,
    /// Registered command handlers for slash commands.
    command_handlers: Vec<Arc<dyn CommandHandler>>,
    /// Registered callback handlers for interactive elements.
    callback_handlers: Vec<Arc<dyn CallbackHandler>>,
    /// Primary chat ID for privileged commands (e.g. /search, /jd).
    primary_chat_id: Option<i64>,
    /// Allowed group chat ID. If set, only this group is authorized for
    /// group-chat interactions. Other groups receive an "unauthorized" message.
    allowed_group_chat_id: Option<i64>,
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
            bot_username: Arc::new(RwLock::new(None)),
            command_handlers: Vec::new(),
            callback_handlers: Vec::new(),
            primary_chat_id: None,
            allowed_group_chat_id: None,
        }
    }

    /// Create a new Telegram adapter with a custom polling timeout.
    pub fn with_polling_timeout(mut self, timeout_secs: u32) -> Self {
        self.polling_timeout = timeout_secs;
        self
    }

    /// Register command handlers.
    pub fn with_command_handlers(mut self, handlers: Vec<Arc<dyn CommandHandler>>) -> Self {
        self.command_handlers = handlers;
        self
    }

    /// Register callback handlers.
    pub fn with_callback_handlers(mut self, handlers: Vec<Arc<dyn CallbackHandler>>) -> Self {
        self.callback_handlers = handlers;
        self
    }

    /// Set the primary chat ID for privileged commands.
    ///
    /// Commands like `/search` and `/jd` are restricted to this chat only.
    pub fn with_primary_chat_id(mut self, id: i64) -> Self {
        self.primary_chat_id = Some(id);
        self
    }

    /// Set the allowed group chat ID.
    ///
    /// When set, only the specified group is authorized for group-chat
    /// interactions. Messages from other groups receive an "unauthorized"
    /// response and are not dispatched further.
    pub fn with_allowed_group_chat_id(mut self, id: i64) -> Self {
        self.allowed_group_chat_id = Some(id);
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

        // Store bot username for metadata enrichment.
        if let Some(ref username) = me.username {
            *self.bot_username.write().await = Some(username.clone());
        }

        let bot = self.bot.clone();
        let allowed_chat_ids = self.allowed_chat_ids.clone();
        let polling_timeout = self.polling_timeout;
        let mut shutdown_rx = self.shutdown_rx.clone();
        let bot_username = Arc::clone(&self.bot_username);
        let command_handlers = self.command_handlers.clone();
        let callback_handlers = self.callback_handlers.clone();
        let primary_chat_id = self.primary_chat_id;
        let allowed_group_chat_id = self.allowed_group_chat_id;

        tokio::spawn(async move {
            polling_loop(
                bot,
                bridge,
                allowed_chat_ids,
                polling_timeout,
                &mut shutdown_rx,
                bot_username,
                command_handlers,
                callback_handlers,
                primary_chat_id,
                allowed_group_chat_id,
            )
            .await;
        });

        info!("telegram adapter started");
        Ok(())
    }

    async fn send(&self, message: OutboundMessage) -> Result<(), KernelError> {
        let chat_id = parse_chat_id(&message.session_key)?;
        let html = crate::telegram::markdown::markdown_to_telegram_html(&message.content);
        let chunks = crate::telegram::markdown::chunk_message(&html, 4096);

        for chunk in chunks {
            self.bot
                .send_message(ChatId(chat_id), &chunk)
                .parse_mode(teloxide::types::ParseMode::Html)
                .await
                .map_err(|e| KernelError::Other {
                    message: format!("failed to send telegram message: {e}").into(),
                })?;
        }
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
    bot_username: Arc<RwLock<Option<String>>>,
    command_handlers: Vec<Arc<dyn CommandHandler>>,
    callback_handlers: Vec<Arc<dyn CallbackHandler>>,
    primary_chat_id: Option<i64>,
    allowed_group_chat_id: Option<i64>,
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
            .allowed_updates(vec![
                AllowedUpdate::Message,
                AllowedUpdate::EditedMessage,
                AllowedUpdate::CallbackQuery,
            ]);

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
                    let bot_username = Arc::clone(&bot_username);
                    let cmd_handlers = command_handlers.clone();
                    let cb_handlers = callback_handlers.clone();
                    tokio::spawn(async move {
                        handle_update(
                            update,
                            &bridge,
                            &bot,
                            &allowed,
                            &bot_username,
                            &cmd_handlers,
                            &cb_handlers,
                            primary_chat_id,
                            allowed_group_chat_id,
                        )
                        .await;
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
/// dispatches it via the bridge, and sends the response back as Telegram HTML.
/// Also handles photo messages with multimodal content.
async fn handle_update(
    update: Update,
    bridge: &Arc<dyn ChannelBridge>,
    bot: &teloxide::Bot,
    allowed_chat_ids: &[i64],
    bot_username: &Arc<RwLock<Option<String>>>,
    command_handlers: &[Arc<dyn CommandHandler>],
    callback_handlers: &[Arc<dyn CallbackHandler>],
    _primary_chat_id: Option<i64>,
    allowed_group_chat_id: Option<i64>,
) {
    // Handle callback queries (inline keyboard button presses).
    if let UpdateKind::CallbackQuery(query) = &update.kind {
        handle_callback_query(bot, query, callback_handlers, bot_username).await;
        return;
    }

    let msg = match &update.kind {
        UpdateKind::Message(msg) | UpdateKind::EditedMessage(msg) => msg,
        _ => return,
    };

    let chat_id = msg.chat.id.0;

    // Check if this chat is allowed.
    if !allowed_chat_ids.is_empty() && !allowed_chat_ids.contains(&chat_id) {
        warn!(chat_id, "telegram adapter: dropping message from unauthorized chat");
        return;
    }

    // --- Group chat authorization ---
    let group_chat = is_group_chat(msg);

    if group_chat {
        // Check if this is a small group (treat like private chat).
        let trigger_text = msg.text().or_else(|| msg.caption()).unwrap_or_default();
        let username_guard = bot_username.read().await;
        let username_ref = username_guard.as_deref();

        let is_small = matches!(
            bot.get_chat_member_count(msg.chat.id).await,
            Ok(n) if n <= SMALL_GROUP_THRESHOLD
        );

        if !is_small {
            // Large group: only respond to @mentions or rara keywords.
            let should_respond = is_group_mention(msg, trigger_text, username_ref)
                || contains_rara_keyword(trigger_text);
            if !should_respond {
                return;
            }
        }

        // Check allowed group chat authorization.
        if let Some(allowed_id) = allowed_group_chat_id {
            if chat_id != allowed_id {
                warn!(
                    chat_id,
                    allowed_group_chat_id = allowed_id,
                    "telegram adapter: dropping group message from unauthorized group"
                );
                let _ = bot
                    .send_message(
                        ChatId(chat_id),
                        "This group is not authorized. Please configure the allowed group \
                         chat ID in the adapter settings.",
                    )
                    .await;
                return;
            }
        }

        // Release the read lock before continuing.
        drop(username_guard);
    }

    // Photo messages — download and forward as multimodal content.
    if let Some(photos) = msg.photo() {
        // photos is sorted by size, take the largest.
        if let Some(photo) = photos.last() {
            handle_photo(bot, bridge, msg, photo, chat_id, bot_username).await;
            return;
        }
    }

    // Extract text content.
    let Some(raw_text) = msg.text() else {
        return;
    };

    if raw_text.trim().is_empty() {
        return;
    }

    // Strip @mention from text in group chats.
    let text: String;
    {
        let username_guard = bot_username.read().await;
        text = if group_chat {
            strip_group_mention(raw_text, username_guard.as_deref())
        } else {
            raw_text.to_owned()
        };
    }

    if text.trim().is_empty() {
        return;
    }

    // Check for commands (text starting with '/').
    if text.starts_with('/') {
        if let Some(cmd_info) = parse_command(&text, bot_username).await {
            handle_command(bot, bridge, msg, &cmd_info, command_handlers, bot_username).await;
            return;
        }
    }

    // Convert to ChannelMessage.
    let channel_message =
        telegram_msg_to_channel_message(&update, msg, &text, bot_username).await;

    // Send typing indicator.
    let _ = bot.send_chat_action(ChatId(chat_id), ChatAction::Typing).await;

    // Dispatch to the bridge.
    match bridge.dispatch(channel_message).await {
        Ok(response) => {
            if !response.trim().is_empty() {
                // In group chats, prepend @mention to the sender.
                let response = if group_chat {
                    let mention = mention_sender(msg);
                    if mention.is_empty() {
                        response
                    } else {
                        format!("{mention}\n{response}")
                    }
                } else {
                    response
                };
                send_html_chunks(bot, chat_id, &response).await;
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

/// Send a response as Telegram HTML, splitting into chunks if necessary.
async fn send_html_chunks(bot: &teloxide::Bot, chat_id: i64, response: &str) {
    let html = crate::telegram::markdown::markdown_to_telegram_html(response);
    let chunks = crate::telegram::markdown::chunk_message(&html, 4096);
    for chunk in chunks {
        if let Err(e) = bot
            .send_message(ChatId(chat_id), &chunk)
            .parse_mode(teloxide::types::ParseMode::Html)
            .await
        {
            error!(error = %e, chat_id, "telegram adapter: failed to send response chunk");
        }
    }
}

/// Handle a photo message: download, encode as base64, and dispatch as
/// multimodal content.
async fn handle_photo(
    bot: &teloxide::Bot,
    bridge: &Arc<dyn ChannelBridge>,
    msg: &teloxide::types::Message,
    photo: &teloxide::types::PhotoSize,
    chat_id: i64,
    bot_username: &Arc<RwLock<Option<String>>>,
) {
    // 1. Get file path from Telegram.
    let file = match bot.get_file(photo.file.id.clone()).await {
        Ok(f) => f,
        Err(e) => {
            error!(error = %e, "failed to get file info");
            return;
        }
    };

    // 2. Download the file via HTTP.
    let url = format!(
        "https://api.telegram.org/file/bot{}/{}",
        bot.token(),
        file.path
    );
    let bytes = match reqwest::get(&url).await {
        Ok(resp) => match resp.bytes().await {
            Ok(b) => b,
            Err(e) => {
                error!(error = %e, "failed to download photo");
                return;
            }
        },
        Err(e) => {
            error!(error = %e, "failed to fetch photo URL");
            return;
        }
    };

    // 3. Convert to base64 data URL.
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let data_url = format!("data:image/jpeg;base64,{b64}");

    // 4. Build multimodal ChannelMessage.
    let caption = msg
        .caption()
        .unwrap_or("Analyze this image")
        .to_owned();
    let content = MessageContent::Multimodal(vec![
        ContentBlock::Text { text: caption },
        ContentBlock::ImageUrl { url: data_url },
    ]);

    let (platform_id, display_name) = extract_user_info(msg);
    let mut metadata = build_metadata_from_msg(msg);

    // Include bot username in metadata if available.
    if let Some(ref username) = *bot_username.read().await {
        metadata.insert(
            "telegram_bot_username".to_owned(),
            serde_json::Value::String(username.clone()),
        );
    }

    let channel_message = ChannelMessage {
        id: ulid::Ulid::new().to_string(),
        channel_type: ChannelType::Telegram,
        user: ChannelUser {
            platform_id,
            display_name,
        },
        session_key: format_session_key(chat_id),
        role: MessageRole::User,
        content,
        tool_call_id: None,
        tool_name: None,
        timestamp: jiff::Timestamp::now(),
        metadata,
    };

    let _ = bot
        .send_chat_action(ChatId(chat_id), ChatAction::Typing)
        .await;

    match bridge.dispatch(channel_message).await {
        Ok(response) => {
            if !response.trim().is_empty() {
                send_html_chunks(bot, chat_id, &response).await;
            }
        }
        Err(e) => {
            error!(error = %e, chat_id, "bridge dispatch failed for photo");
            let _ = bot
                .send_message(ChatId(chat_id), format!("Error: {e}"))
                .await;
        }
    }
}

/// Convert a Telegram message with text content into a [`ChannelMessage`].
///
/// Extracts user info, chat ID (used as session key), and message text.
/// Includes bot username in metadata when available.
async fn telegram_msg_to_channel_message(
    update: &Update,
    msg: &teloxide::types::Message,
    text: &str,
    bot_username: &Arc<RwLock<Option<String>>>,
) -> ChannelMessage {
    let chat_id = msg.chat.id.0;
    let (platform_id, display_name) = extract_user_info(msg);

    let mut metadata = build_metadata(update, msg);

    // Include bot username in metadata if available.
    if let Some(ref username) = *bot_username.read().await {
        metadata.insert(
            "telegram_bot_username".to_owned(),
            serde_json::Value::String(username.clone()),
        );
    }

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
        metadata,
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

/// Build metadata from a Telegram message (without an Update reference).
///
/// Used for photo messages where the full [`Update`] is not passed through.
fn build_metadata_from_msg(
    msg: &teloxide::types::Message,
) -> HashMap<String, serde_json::Value> {
    let mut meta = HashMap::new();
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

// ---------------------------------------------------------------------------
// Command & callback handling
// ---------------------------------------------------------------------------

/// Parse a command from text, stripping bot mention if present.
///
/// Handles formats like `/search keywords` and `/search@botname keywords`.
async fn parse_command(
    text: &str,
    bot_username: &Arc<RwLock<Option<String>>>,
) -> Option<CommandInfo> {
    if !text.starts_with('/') {
        return None;
    }

    let text = text.trim();
    // Split into command part and args.
    let (cmd_part, args) = match text.find(char::is_whitespace) {
        Some(pos) => (&text[..pos], text[pos..].trim()),
        None => (text, ""),
    };

    // Strip the '/' prefix.
    let cmd_part = &cmd_part[1..];

    // Strip @botname suffix if present.
    let name = if let Some(at_pos) = cmd_part.find('@') {
        let mentioned_bot = &cmd_part[at_pos + 1..];
        // Verify it's our bot.
        if let Some(ref our_username) = *bot_username.read().await {
            if !mentioned_bot.eq_ignore_ascii_case(our_username) {
                return None; // Command for a different bot.
            }
        }
        cmd_part[..at_pos].to_lowercase()
    } else {
        cmd_part.to_lowercase()
    };

    if name.is_empty() {
        return None;
    }

    Some(CommandInfo {
        name,
        args: args.to_owned(),
        raw: text.to_owned(),
    })
}

/// Find a matching command handler and execute the command.
async fn handle_command(
    bot: &teloxide::Bot,
    _bridge: &Arc<dyn ChannelBridge>,
    msg: &teloxide::types::Message,
    cmd: &CommandInfo,
    handlers: &[Arc<dyn CommandHandler>],
    bot_username: &Arc<RwLock<Option<String>>>,
) {
    let chat_id = msg.chat.id.0;

    // Find matching handler.
    let handler = handlers
        .iter()
        .find(|h| h.commands().iter().any(|def| def.name == cmd.name));

    let Some(handler) = handler else {
        // Unknown command — send help text.
        let known: Vec<String> = handlers
            .iter()
            .flat_map(|h| h.commands())
            .map(|d| format!("/{} \u{2014} {}", d.name, d.description))
            .collect();

        let text = if known.is_empty() {
            format!("Unknown command: /{}\nNo commands are registered.", cmd.name)
        } else {
            format!(
                "Unknown command: /{}\n\nAvailable commands:\n{}",
                cmd.name,
                known.join("\n")
            )
        };
        let _ = bot.send_message(ChatId(chat_id), text).await;
        return;
    };

    let (platform_id, display_name) = extract_user_info(msg);
    let mut metadata = build_metadata_from_msg(msg);
    if let Some(ref username) = *bot_username.read().await {
        metadata.insert(
            "telegram_bot_username".to_owned(),
            serde_json::Value::String(username.clone()),
        );
    }

    let context = CommandContext {
        channel_type: ChannelType::Telegram,
        session_key: format_session_key(chat_id),
        user: ChannelUser {
            platform_id,
            display_name,
        },
        metadata,
    };

    // Send typing indicator.
    let _ = bot
        .send_chat_action(ChatId(chat_id), ChatAction::Typing)
        .await;

    match handler.handle(cmd, &context).await {
        Ok(CommandResult::Text(text)) => {
            send_html_chunks(bot, chat_id, &text).await;
        }
        Ok(CommandResult::Html(html)) => {
            let chunks = crate::telegram::markdown::chunk_message(&html, 4096);
            for chunk in chunks {
                let _ = bot
                    .send_message(ChatId(chat_id), &chunk)
                    .parse_mode(teloxide::types::ParseMode::Html)
                    .await;
            }
        }
        Ok(CommandResult::None) => {
            // Handler handled it internally.
        }
        Err(e) => {
            error!(error = %e, "command handler failed for /{}", cmd.name);
            let _ = bot
                .send_message(ChatId(chat_id), format!("Error: {e}"))
                .await;
        }
    }
}

/// Handle a callback query by routing to the appropriate handler.
async fn handle_callback_query(
    bot: &teloxide::Bot,
    query: &teloxide::types::CallbackQuery,
    handlers: &[Arc<dyn CallbackHandler>],
    bot_username: &Arc<RwLock<Option<String>>>,
) {
    let Some(ref data) = query.data else {
        return;
    };

    // Find matching handler by prefix.
    let handler = handlers.iter().find(|h| data.starts_with(h.prefix()));

    let Some(handler) = handler else {
        warn!(data = %data, "no callback handler found for prefix");
        // Answer the callback to dismiss the loading indicator.
        let _ = bot.answer_callback_query(query.id.clone()).await;
        return;
    };

    // Extract chat_id from the callback query message.
    let chat_id = query
        .message
        .as_ref()
        .map(|m| match m {
            MaybeInaccessibleMessage::Regular(msg) => msg.chat.id.0,
            MaybeInaccessibleMessage::Inaccessible(msg) => msg.chat.id.0,
        })
        .unwrap_or(0);

    let message_id = query.message.as_ref().map(|m| match m {
        MaybeInaccessibleMessage::Regular(msg) => msg.id.0.to_string(),
        MaybeInaccessibleMessage::Inaccessible(msg) => msg.message_id.0.to_string(),
    });

    let (platform_id, display_name) = {
        let user = &query.from;
        let id = user.id.0.to_string();
        let name = if let Some(ref last) = user.last_name {
            Some(format!("{} {last}", user.first_name))
        } else {
            Some(user.first_name.clone())
        };
        (id, name)
    };

    let mut metadata = HashMap::new();
    metadata.insert(
        "telegram_chat_id".to_owned(),
        serde_json::json!(chat_id),
    );
    if let Some(ref username) = query.from.username {
        metadata.insert(
            "telegram_username".to_owned(),
            serde_json::Value::String(username.clone()),
        );
    }
    if let Some(ref username) = *bot_username.read().await {
        metadata.insert(
            "telegram_bot_username".to_owned(),
            serde_json::Value::String(username.clone()),
        );
    }

    let context = CallbackContext {
        channel_type: ChannelType::Telegram,
        session_key: format_session_key(chat_id),
        user: ChannelUser {
            platform_id,
            display_name,
        },
        data: data.clone(),
        message_id: message_id.clone(),
        metadata,
    };

    match handler.handle(&context).await {
        Ok(CallbackResult::EditMessage { text }) => {
            if let Some(ref msg) = query.message {
                let html = crate::telegram::markdown::markdown_to_telegram_html(&text);
                let (chat_id_obj, msg_id) = match msg {
                    MaybeInaccessibleMessage::Regular(m) => (m.chat.id, m.id),
                    MaybeInaccessibleMessage::Inaccessible(m) => (m.chat.id, m.message_id),
                };
                let _ = bot
                    .edit_message_text(chat_id_obj, msg_id, &html)
                    .parse_mode(teloxide::types::ParseMode::Html)
                    .await;
            }
        }
        Ok(CallbackResult::SendMessage { text }) => {
            send_html_chunks(bot, chat_id, &text).await;
        }
        Ok(CallbackResult::Ack) => {
            // Just acknowledge.
        }
        Err(e) => {
            error!(error = %e, "callback handler failed");
        }
    }

    // Always answer the callback query to dismiss the loading indicator.
    let _ = bot.answer_callback_query(query.id.clone()).await;
}

// ---------------------------------------------------------------------------
// Group chat helpers
// ---------------------------------------------------------------------------

/// Check whether a Telegram message originates from a group (or supergroup)
/// chat. Returns `false` for private chats.
fn is_group_chat(msg: &teloxide::types::Message) -> bool {
    matches!(msg.chat.kind, teloxide::types::ChatKind::Public(..))
}

/// Check whether the message contains an @mention of the bot via message
/// entities or a plain-text `@botname` substring.
fn is_group_mention(
    msg: &teloxide::types::Message,
    text: &str,
    bot_username: Option<&str>,
) -> bool {
    let Some(username) = bot_username else {
        return false;
    };
    let expected = username.to_lowercase();

    // Check structured entities first (most reliable).
    if let Some(entities) = msg.parse_entities() {
        for entity in entities {
            if matches!(
                entity.kind(),
                teloxide::types::MessageEntityKind::Mention
            ) {
                let mention = entity
                    .text()
                    .trim()
                    .trim_start_matches('@')
                    .to_lowercase();
                if mention == expected {
                    return true;
                }
            }
        }
    }

    // Fallback: substring check.
    let mention = format!("@{expected}");
    text.to_lowercase().contains(&mention)
}

/// Check whether the text contains any "rara" keyword variant.
///
/// Supported variants: "rara" (case-insensitive), Japanese hiragana/katakana,
/// and Chinese characters.
fn contains_rara_keyword(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("rara")
        || lower.contains("らら")
        || lower.contains("ララ")
        || lower.contains("拉拉")
}

/// Strip the bot @mention from message text.
///
/// Removes the `@botname` substring and trims surrounding whitespace.
fn strip_group_mention(text: &str, bot_username: Option<&str>) -> String {
    let Some(username) = bot_username else {
        return text.trim().to_owned();
    };
    let mention = format!("@{username}");
    text.replace(&mention, "").trim().to_owned()
}

/// Build an @username or first-name string for replying to the sender in a
/// group chat. Returns an empty string if the sender is unknown.
fn mention_sender(msg: &teloxide::types::Message) -> String {
    let Some(sender) = msg.from.as_ref() else {
        return String::new();
    };
    if let Some(username) = &sender.username {
        return format!("@{username}");
    }
    sender.first_name.clone()
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

    #[tokio::test]
    async fn parse_command_basic() {
        let bot_username = Arc::new(RwLock::new(Some("testbot".to_owned())));
        let cmd = parse_command("/search rust developer", &bot_username)
            .await
            .unwrap();
        assert_eq!(cmd.name, "search");
        assert_eq!(cmd.args, "rust developer");
    }

    #[tokio::test]
    async fn parse_command_no_args() {
        let bot_username = Arc::new(RwLock::new(None));
        let cmd = parse_command("/help", &bot_username).await.unwrap();
        assert_eq!(cmd.name, "help");
        assert_eq!(cmd.args, "");
    }

    #[tokio::test]
    async fn parse_command_with_bot_mention() {
        let bot_username = Arc::new(RwLock::new(Some("mybot".to_owned())));
        let cmd = parse_command("/search@mybot keywords", &bot_username)
            .await
            .unwrap();
        assert_eq!(cmd.name, "search");
        assert_eq!(cmd.args, "keywords");
    }

    #[tokio::test]
    async fn parse_command_wrong_bot_returns_none() {
        let bot_username = Arc::new(RwLock::new(Some("mybot".to_owned())));
        let result = parse_command("/search@otherbot keywords", &bot_username).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn parse_command_not_a_command() {
        let bot_username = Arc::new(RwLock::new(None));
        let result = parse_command("hello world", &bot_username).await;
        assert!(result.is_none());
    }

    // -----------------------------------------------------------------------
    // Group chat helper tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_contains_rara_keyword_various() {
        // ASCII case-insensitive.
        assert!(contains_rara_keyword("hey Rara, help"));
        assert!(contains_rara_keyword("RARA please"));
        assert!(contains_rara_keyword("hello rara!"));
        // Japanese hiragana.
        assert!(contains_rara_keyword("こんにちは、らら"));
        // Japanese katakana.
        assert!(contains_rara_keyword("ララに聞いて"));
        // Chinese characters.
        assert!(contains_rara_keyword("拉拉你好"));
        // No match.
        assert!(!contains_rara_keyword("hello world"));
        assert!(!contains_rara_keyword("random text"));
    }

    #[test]
    fn test_strip_group_mention() {
        // With bot username present.
        assert_eq!(
            strip_group_mention("@mybot hello world", Some("mybot")),
            "hello world"
        );
        // Mention in middle of text.
        assert_eq!(
            strip_group_mention("hey @mybot what's up", Some("mybot")),
            "hey  what's up"
        );
        // No mention present — returns trimmed original.
        assert_eq!(
            strip_group_mention("hello world", Some("mybot")),
            "hello world"
        );
        // No bot username — returns trimmed original.
        assert_eq!(
            strip_group_mention("  @someone hello  ", None),
            "@someone hello"
        );
    }

    #[test]
    fn test_with_primary_chat_id() {
        let bot = teloxide::Bot::new("fake_token");
        let adapter = TelegramAdapter::new(bot, vec![]).with_primary_chat_id(12345);
        assert_eq!(adapter.primary_chat_id, Some(12345));
    }

    #[test]
    fn test_with_allowed_group_chat_id() {
        let bot = teloxide::Bot::new("fake_token");
        let adapter =
            TelegramAdapter::new(bot, vec![]).with_allowed_group_chat_id(-100_999_888);
        assert_eq!(adapter.allowed_group_chat_id, Some(-100_999_888));
    }

    #[test]
    fn test_small_group_threshold_constant() {
        // Ensure the constant is set to the expected value.
        assert_eq!(SMALL_GROUP_THRESHOLD, 3);
    }

    #[test]
    fn test_strip_group_mention_only_mention() {
        // When the entire message is just the mention.
        assert_eq!(strip_group_mention("@mybot", Some("mybot")), "");
    }

    #[test]
    fn test_strip_group_mention_multiple_mentions() {
        // Multiple mentions of the bot.
        assert_eq!(
            strip_group_mention("@mybot hello @mybot", Some("mybot")),
            "hello"
        );
    }

    #[test]
    fn test_contains_rara_keyword_embedded() {
        // "rara" embedded inside a larger word should still match.
        assert!(contains_rara_keyword("prerara-something"));
        // But these should not match.
        assert!(!contains_rara_keyword("rad"));
        assert!(!contains_rara_keyword("ordinary text"));
    }
}
