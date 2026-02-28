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
//! long polling. Inbound messages are converted to [`RawPlatformMessage`] and
//! handed to an [`InboundSink`] in a fire-and-forget fashion. Outbound
//! delivery is handled by the [`EgressAdapter`] implementation.
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
//! │              │     ├── Update → RawPlatformMessage
//! │              │     │     │              │
//! │              │     │     ▼              │
//! │              │     │  sink.ingest()     │
//! │              │     │                    │
//! │              │     └── loop             │
//! │              │                          │
//! │  send()  ─► bot.send_message() (HTML)   │
//! │  stop()  ─► shutdown signal             │
//! └─────────────────────────────────────────┘
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock as StdRwLock;

use async_trait::async_trait;
use rara_kernel::channel::adapter::ChannelAdapter;
use rara_kernel::channel::command::{
    CallbackHandler, CommandHandler,
};
use crate::telegram::contacts::ContactTracker;
use rara_kernel::channel::types::{
    AgentPhase, ChannelType, InlineButton,
    MessageContent, OutboundMessage, ReplyMarkup,
};
use rara_kernel::error::KernelError;
use rara_kernel::io::egress::{EgressAdapter, EgressError, Endpoint, EndpointAddress, PlatformOutbound};
use rara_kernel::io::ingress::{InboundSink, RawPlatformMessage};
use rara_kernel::io::types::{IngestError, InteractionType, ReplyContext as IoReplyContext};
use teloxide::payloads::{
    EditMessageTextSetters, GetUpdatesSetters, SendMessageSetters, SendPhotoSetters,
};
use teloxide::requests::{Request, Requester};
use teloxide::types::{
    AllowedUpdate, ChatAction, ChatId, InlineKeyboardButton, InlineKeyboardMarkup, InputFile,
    MessageId, ParseMode, ReplyParameters, Update, UpdateKind,
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

/// Minimum interval between Telegram `edit_message_text` calls (1.5 seconds)
/// to avoid hitting Telegram API rate limits.

/// Runtime configuration for the Telegram adapter.
///
/// Can be updated at runtime via [`TelegramAdapter::config_handle`] to change
/// authorization settings without restarting the adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramConfig {
    /// Primary chat ID for privileged commands (e.g. /search, /jd).
    pub primary_chat_id: Option<i64>,
    /// Allowed group chat ID. Only this group is authorized for bot interaction.
    pub allowed_group_chat_id: Option<i64>,
}

impl Default for TelegramConfig {
    fn default() -> Self {
        Self {
            primary_chat_id: None,
            allowed_group_chat_id: None,
        }
    }
}

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
/// - `config` — runtime-updatable settings (primary chat ID, allowed group
///   chat ID). Obtain a shared handle via [`config_handle`](Self::config_handle)
///   and mutate through `std::sync::RwLock::write()`.
///
/// # Lifecycle
///
/// 1. Call [`start`](ChannelAdapter::start) with an [`InboundSink`]. This
///    spawns a background tokio task that polls for updates.
/// 2. For each inbound message, the adapter converts the Telegram
///    [`Update`] to a [`RawPlatformMessage`] and hands it to the sink.
///    Outbound delivery is handled separately via [`EgressAdapter::send`].
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
    /// Runtime-updatable configuration (primary chat ID, allowed group chat ID).
    config: Arc<StdRwLock<TelegramConfig>>,
    /// Optional contact tracker for recording username-to-chat_id mappings.
    contact_tracker: Option<Arc<dyn ContactTracker>>,
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
            config: Arc::new(StdRwLock::new(TelegramConfig::default())),
            contact_tracker: None,
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
    /// This is a convenience builder that mutates the internal config.
    pub fn with_primary_chat_id(self, id: i64) -> Self {
        {
            let mut cfg = self.config.write().unwrap_or_else(|e| e.into_inner());
            cfg.primary_chat_id = Some(id);
        }
        self
    }

    /// Set the allowed group chat ID.
    ///
    /// When set, only the specified group is authorized for group-chat
    /// interactions. Messages from other groups receive an "unauthorized"
    /// response and are not dispatched further.
    /// This is a convenience builder that mutates the internal config.
    pub fn with_allowed_group_chat_id(self, id: i64) -> Self {
        {
            let mut cfg = self.config.write().unwrap_or_else(|e| e.into_inner());
            cfg.allowed_group_chat_id = Some(id);
        }
        self
    }

    /// Set the full runtime config.
    ///
    /// Replaces the current config with the provided one.
    pub fn with_config(self, config: TelegramConfig) -> Self {
        {
            let mut cfg = self.config.write().unwrap_or_else(|e| e.into_inner());
            *cfg = config;
        }
        self
    }

    /// Return a shared handle to the runtime config.
    ///
    /// Callers can use this to update configuration at runtime (e.g. change the
    /// primary chat ID) without restarting the adapter. The polling loop reads
    /// the config on every update, so changes take effect immediately.
    pub fn config_handle(&self) -> Arc<StdRwLock<TelegramConfig>> {
        Arc::clone(&self.config)
    }

    /// Read a snapshot of the current config.
    ///
    /// If the lock is poisoned, recovers and returns the inner value.
    pub fn current_config(&self) -> TelegramConfig {
        match self.config.read() {
            Ok(g) => g.clone(),
            Err(e) => e.into_inner().clone(),
        }
    }

    /// Set a contact tracker for recording username-to-chat_id mappings.
    ///
    /// When set, every incoming message from a user with a Telegram username
    /// will trigger a [`ContactTracker::track`] call, recording the mapping
    /// for outbound notification routing.
    pub fn with_contact_tracker(mut self, tracker: Arc<dyn ContactTracker>) -> Self {
        self.contact_tracker = Some(tracker);
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

// ---------------------------------------------------------------------------
// EgressAdapter implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl EgressAdapter for TelegramAdapter {
    fn channel_type(&self) -> ChannelType {
        ChannelType::Telegram
    }

    async fn send(
        &self,
        endpoint: &Endpoint,
        msg: PlatformOutbound,
    ) -> Result<(), EgressError> {
        let (chat_id, _thread_id) = match &endpoint.address {
            EndpointAddress::Telegram { chat_id, thread_id } => (*chat_id, *thread_id),
            _ => {
                return Err(EgressError::DeliveryFailed {
                    message: "not a telegram endpoint".to_string(),
                });
            }
        };

        match msg {
            PlatformOutbound::Reply {
                content,
                reply_context,
                ..
            } => {
                let html = crate::telegram::markdown::markdown_to_telegram_html(&content);
                let chunks = crate::telegram::markdown::chunk_message(&html, 4096);

                for (i, chunk) in chunks.iter().enumerate() {
                    let mut req = self
                        .bot
                        .send_message(ChatId(chat_id), chunk)
                        .parse_mode(ParseMode::Html);

                    // Attach reply-to on the first chunk if available.
                    if i == 0 {
                        if let Some(ref ctx) = reply_context {
                            if let Some(ref reply_id) = ctx.reply_to_platform_msg_id {
                                if let Ok(msg_id) = parse_message_id(reply_id) {
                                    req = req.reply_parameters(ReplyParameters::new(msg_id));
                                }
                            }
                        }
                    }

                    req.await.map_err(|e| EgressError::DeliveryFailed {
                        message: format!("failed to send telegram message: {e}"),
                    })?;
                }
            }
            PlatformOutbound::StreamChunk {
                delta,
                edit_target,
                ..
            } => {
                if let Some(ref target_id) = edit_target {
                    // Edit an existing message with accumulated text.
                    if let Ok(msg_id) = parse_message_id(target_id) {
                        let html =
                            crate::telegram::markdown::markdown_to_telegram_html(&delta);
                        let _ = self
                            .bot
                            .edit_message_text(ChatId(chat_id), msg_id, &html)
                            .parse_mode(ParseMode::Html)
                            .await;
                    }
                } else {
                    // No edit target — send as a new message.
                    let _ = self
                        .bot
                        .send_message(ChatId(chat_id), &delta)
                        .await;
                }
            }
            PlatformOutbound::Progress { text, .. } => {
                // Send a typing indicator for progress updates.
                let _ = self
                    .bot
                    .send_chat_action(ChatId(chat_id), ChatAction::Typing)
                    .await;
                // If there's progress text, send it as a message.
                if !text.is_empty() {
                    let _ = self
                        .bot
                        .send_message(ChatId(chat_id), &text)
                        .await;
                }
            }
        }

        Ok(())
    }
}

#[async_trait]
impl ChannelAdapter for TelegramAdapter {
    fn channel_type(&self) -> ChannelType {
        ChannelType::Telegram
    }

    async fn start(
        &self,
        sink: Arc<dyn InboundSink>,
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
        let config = Arc::clone(&self.config);
        let contact_tracker = self.contact_tracker.clone();

        tokio::spawn(async move {
            polling_loop(
                bot,
                sink,
                allowed_chat_ids,
                polling_timeout,
                &mut shutdown_rx,
                bot_username,
                config,
                contact_tracker,
            )
            .await;
        });

        info!("telegram adapter started");
        Ok(())
    }

    async fn send(&self, message: OutboundMessage) -> Result<(), KernelError> {
        let chat_id = parse_chat_id(&message.session_key)?;

        // Case 1: Edit an existing message.
        if let Some(ref edit_id) = message.edit_message_id {
            let msg_id = parse_message_id(edit_id)?;
            let html = crate::telegram::markdown::markdown_to_telegram_html(&message.content);
            let mut req = self
                .bot
                .edit_message_text(ChatId(chat_id), msg_id, &html)
                .parse_mode(ParseMode::Html);
            if let Some(keyboard) = convert_reply_markup(&message.reply_markup) {
                req = req.reply_markup(keyboard);
            }
            req.await.map_err(|e| KernelError::Other {
                message: format!("failed to edit telegram message: {e}").into(),
            })?;
            return Ok(());
        }

        // Case 2: Send a photo.
        if let Some(ref photo) = message.photo {
            let file_name = mime_to_filename(&photo.mime_type);
            let input_file = InputFile::memory(photo.data.clone()).file_name(file_name);
            let mut req = self.bot.send_photo(ChatId(chat_id), input_file);
            if let Some(ref caption) = photo.caption {
                req = req.caption(caption).parse_mode(ParseMode::Html);
            }
            if let Some(ref reply_id) = message.reply_to_message_id {
                if let Ok(msg_id) = parse_message_id(reply_id) {
                    req = req.reply_parameters(ReplyParameters::new(msg_id));
                }
            }
            if let Some(keyboard) = convert_reply_markup(&message.reply_markup) {
                req = req.reply_markup(keyboard);
            }
            req.await.map_err(|e| KernelError::Other {
                message: format!("failed to send telegram photo: {e}").into(),
            })?;
            return Ok(());
        }

        // Case 3: Send text message (with optional reply-to and keyboard).
        let html = crate::telegram::markdown::markdown_to_telegram_html(&message.content);
        let chunks = crate::telegram::markdown::chunk_message(&html, 4096);

        for (i, chunk) in chunks.iter().enumerate() {
            let mut req = self
                .bot
                .send_message(ChatId(chat_id), chunk)
                .parse_mode(ParseMode::Html);

            // Attach reply-to only on the first chunk.
            if i == 0 {
                if let Some(ref reply_id) = message.reply_to_message_id {
                    if let Ok(msg_id) = parse_message_id(reply_id) {
                        req = req.reply_parameters(ReplyParameters::new(msg_id));
                    }
                }
            }

            // Attach keyboard only on the last chunk.
            if i == chunks.len() - 1 {
                if let Some(keyboard) = convert_reply_markup(&message.reply_markup) {
                    req = req.reply_markup(keyboard);
                }
            }

            req.await.map_err(|e| KernelError::Other {
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

// ---------------------------------------------------------------------------
// Polling loop (I/O Bus model via InboundSink)
// ---------------------------------------------------------------------------

/// The getUpdates long-polling loop.
///
/// Converts each update to a [`RawPlatformMessage`] and hands it to the
/// [`InboundSink`] in a fire-and-forget fashion. The adapter does **not**
/// wait for a response -- egress delivers replies through
/// [`EgressAdapter::send`].
///
/// Commands and callbacks are routed through the kernel like regular
/// messages via [`InteractionType`]. The adapter only performs
/// authorization checks, contact tracking, and group-chat filtering.
async fn polling_loop(
    bot: teloxide::Bot,
    sink: Arc<dyn InboundSink>,
    allowed_chat_ids: Vec<i64>,
    polling_timeout: u32,
    shutdown_rx: &mut watch::Receiver<bool>,
    bot_username: Arc<RwLock<Option<String>>>,
    config: Arc<StdRwLock<TelegramConfig>>,
    contact_tracker: Option<Arc<dyn ContactTracker>>,
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

                    // Spawn handler as a separate task.
                    let sink = Arc::clone(&sink);
                    let bot = bot.clone();
                    let allowed = allowed_chat_ids.clone();
                    let bot_username = Arc::clone(&bot_username);
                    let config = Arc::clone(&config);
                    let tracker = contact_tracker.clone();
                    tokio::spawn(async move {
                        handle_update(
                            update,
                            &sink,
                            &bot,
                            &allowed,
                            &bot_username,
                            &config,
                            tracker.as_ref(),
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

async fn handle_update(
    update: Update,
    sink: &Arc<dyn InboundSink>,
    bot: &teloxide::Bot,
    allowed_chat_ids: &[i64],
    bot_username: &Arc<RwLock<Option<String>>>,
    config: &Arc<StdRwLock<TelegramConfig>>,
    contact_tracker: Option<&Arc<dyn ContactTracker>>,
) {
    // Read a snapshot of the runtime config for this update.
    let cfg = match config.read() {
        Ok(g) => g.clone(),
        Err(e) => e.into_inner().clone(),
    };

    // Skip callback queries for now in sink mode — they need special handling.
    if matches!(&update.kind, UpdateKind::CallbackQuery(_)) {
        // TODO: Convert callbacks to RawPlatformMessage with InteractionType::Callback
        return;
    }

    let msg = match &update.kind {
        UpdateKind::Message(msg) | UpdateKind::EditedMessage(msg) => msg,
        _ => return,
    };

    let chat_id = msg.chat.id.0;

    // Track contact if username is available.
    if let Some(tracker) = contact_tracker {
        if let Some(ref user) = msg.from {
            if let Some(ref username) = user.username {
                tracker.track(username, chat_id).await;
            }
        }
    }

    // Check if this chat is allowed.
    if !allowed_chat_ids.is_empty() && !allowed_chat_ids.contains(&chat_id) {
        warn!(chat_id, "telegram adapter: dropping message from unauthorized chat");
        return;
    }

    // --- Group chat authorization ---
    let group_chat = is_group_chat(msg);

    if group_chat {
        let trigger_text = msg.text().or_else(|| msg.caption()).unwrap_or_default();
        let username_guard = bot_username.read().await;
        let username_ref = username_guard.as_deref();

        let is_small = matches!(
            bot.get_chat_member_count(msg.chat.id).await,
            Ok(n) if n <= SMALL_GROUP_THRESHOLD
        );

        if !is_small {
            let should_respond = is_group_mention(msg, trigger_text, username_ref)
                || contains_rara_keyword(trigger_text);
            if !should_respond {
                return;
            }
        }

        // Check allowed group chat authorization.
        if let Some(allowed_id) = cfg.allowed_group_chat_id {
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

        drop(username_guard);
    }

    // Convert to RawPlatformMessage.
    let username_guard = bot_username.read().await;
    let username_ref = username_guard.as_deref().unwrap_or("");
    let raw = match telegram_to_raw_platform_message(msg, username_ref) {
        Some(raw) => raw,
        None => return,
    };
    drop(username_guard);

    // Fire-and-forget ingest.
    if let Err(e) = sink.ingest(raw).await {
        match e {
            IngestError::SystemBusy => {
                let _ = bot
                    .send_message(
                        ChatId(chat_id),
                        "\u{26a0}\u{fe0f} \u{7cfb}\u{7edf}\u{7e41}\u{5fd9}\u{ff0c}\u{8bf7}\u{7a0d}\u{540e}\u{518d}\u{8bd5}\u{3002}",
                    )
                    .await;
            }
            other => {
                error!(error = %other, "telegram adapter: ingest failed");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// RawPlatformMessage conversion
// ---------------------------------------------------------------------------

/// Convert a Telegram message to a [`RawPlatformMessage`].
///
/// Extracts user ID, chat ID, text/caption content, and reply context from
/// the Telegram message. Returns `None` if the message has no text content
/// (e.g. stickers, voice notes without caption).
pub fn telegram_to_raw_platform_message(
    msg: &teloxide::types::Message,
    bot_username: &str,
) -> Option<RawPlatformMessage> {
    // Extract text — try text first, then caption (for photos/documents).
    let raw_text = msg.text().or_else(|| msg.caption())?;
    if raw_text.trim().is_empty() {
        return None;
    }

    // Strip @mention from text in group chats.
    let text = if is_group_chat(msg) {
        let username = if bot_username.is_empty() {
            None
        } else {
            Some(bot_username)
        };
        strip_group_mention(raw_text, username)
    } else {
        raw_text.to_owned()
    };

    if text.trim().is_empty() {
        return None;
    }

    let platform_user_id = msg
        .from
        .as_ref()
        .map(|u| u.id.0.to_string())
        .unwrap_or_else(|| "unknown".to_owned());

    // Determine the interaction type.
    let interaction_type = if text.starts_with('/') {
        // Extract command name (strip leading '/' and any @botname suffix).
        let cmd_part = text.split_whitespace().next().unwrap_or(&text);
        let cmd_name = cmd_part
            .trim_start_matches('/')
            .split('@')
            .next()
            .unwrap_or("")
            .to_lowercase();
        if cmd_name.is_empty() {
            InteractionType::Message
        } else {
            InteractionType::Command(cmd_name)
        }
    } else {
        InteractionType::Message
    };

    // Build reply context.
    let reply_context = Some(IoReplyContext {
        thread_id: msg.thread_id.map(|t| t.to_string()),
        reply_to_platform_msg_id: msg
            .reply_to_message()
            .map(|r| r.id.0.to_string()),
        interaction_type,
    });

    // Build metadata (adapter-specific).
    let mut metadata = HashMap::new();
    if let Some(ref user) = msg.from {
        if let Some(ref username) = user.username {
            metadata.insert(
                "telegram_username".to_owned(),
                serde_json::Value::String(username.clone()),
            );
        }
        // Include display name for downstream enrichment.
        let display_name = if let Some(ref last) = user.last_name {
            format!("{} {last}", user.first_name)
        } else {
            user.first_name.clone()
        };
        metadata.insert(
            "telegram_display_name".to_owned(),
            serde_json::Value::String(display_name),
        );
    }
    if !bot_username.is_empty() {
        metadata.insert(
            "telegram_bot_username".to_owned(),
            serde_json::Value::String(bot_username.to_owned()),
        );
    }

    Some(RawPlatformMessage {
        channel_type: ChannelType::Telegram,
        platform_message_id: Some(msg.id.0.to_string()),
        platform_user_id,
        platform_chat_id: Some(msg.chat.id.0.to_string()),
        content: MessageContent::Text(text),
        reply_context,
        metadata,
    })
}

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

/// Parse a string into a teloxide [`MessageId`].
pub fn parse_message_id(id: &str) -> Result<MessageId, KernelError> {
    id.parse::<i32>()
        .map(MessageId)
        .map_err(|_| KernelError::Other {
            message: format!("invalid telegram message id: {id}").into(),
        })
}

/// Convert a kernel [`ReplyMarkup`] to a teloxide [`InlineKeyboardMarkup`].
///
/// Returns `None` if the input is `None` or [`ReplyMarkup::RemoveKeyboard`]
/// (which cannot be represented as an inline keyboard).
fn convert_reply_markup(markup: &Option<ReplyMarkup>) -> Option<InlineKeyboardMarkup> {
    match markup {
        Some(ReplyMarkup::InlineKeyboard { rows }) => {
            let tg_rows: Vec<Vec<InlineKeyboardButton>> = rows
                .iter()
                .map(|row| row.iter().map(convert_inline_button).collect())
                .collect();
            Some(InlineKeyboardMarkup::new(tg_rows))
        }
        Some(ReplyMarkup::RemoveKeyboard) | None => None,
    }
}

/// Convert a kernel [`InlineButton`] to a teloxide [`InlineKeyboardButton`].
fn convert_inline_button(button: &InlineButton) -> InlineKeyboardButton {
    if let Some(ref data) = button.callback_data {
        InlineKeyboardButton::callback(&button.text, data)
    } else if let Some(ref url) = button.url {
        match url.parse::<reqwest::Url>() {
            Ok(parsed) => InlineKeyboardButton::url(&button.text, parsed),
            Err(_) => {
                // Fallback to callback with text as data if URL is invalid.
                InlineKeyboardButton::callback(&button.text, &button.text)
            }
        }
    } else {
        // Fallback: use text as callback data.
        InlineKeyboardButton::callback(&button.text, &button.text)
    }
}

fn mime_to_filename(mime: &str) -> String {
    match mime {
        "image/jpeg" | "image/jpg" => "photo.jpg".to_owned(),
        "image/png" => "photo.png".to_owned(),
        "image/gif" => "photo.gif".to_owned(),
        "image/webp" => "photo.webp".to_owned(),
        _ => "photo.bin".to_owned(),
    }
}


// ---------------------------------------------------------------------------
// Group chat helpers
// ---------------------------------------------------------------------------

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
        assert_eq!(
            <TelegramAdapter as ChannelAdapter>::channel_type(&adapter),
            ChannelType::Telegram
        );
    }

    #[test]
    fn with_polling_timeout_sets_value() {
        let bot = teloxide::Bot::new("fake_token");
        let adapter = TelegramAdapter::new(bot, vec![]).with_polling_timeout(60);
        assert_eq!(adapter.polling_timeout, 60);
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
        assert!(contains_rara_keyword("\u{3053}\u{3093}\u{306b}\u{3061}\u{306f}\u{3001}\u{3089}\u{3089}"));
        // Japanese katakana.
        assert!(contains_rara_keyword("\u{30e9}\u{30e9}\u{306b}\u{805e}\u{3044}\u{3066}"));
        // Chinese characters.
        assert!(contains_rara_keyword("\u{62c9}\u{62c9}\u{4f60}\u{597d}"));
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
        // No mention present -- returns trimmed original.
        assert_eq!(
            strip_group_mention("hello world", Some("mybot")),
            "hello world"
        );
        // No bot username -- returns trimmed original.
        assert_eq!(
            strip_group_mention("  @someone hello  ", None),
            "@someone hello"
        );
    }

    #[test]
    fn test_with_primary_chat_id() {
        let bot = teloxide::Bot::new("fake_token");
        let adapter = TelegramAdapter::new(bot, vec![]).with_primary_chat_id(12345);
        assert_eq!(adapter.current_config().primary_chat_id, Some(12345));
    }

    #[test]
    fn test_with_allowed_group_chat_id() {
        let bot = teloxide::Bot::new("fake_token");
        let adapter =
            TelegramAdapter::new(bot, vec![]).with_allowed_group_chat_id(-100_999_888);
        assert_eq!(
            adapter.current_config().allowed_group_chat_id,
            Some(-100_999_888)
        );
    }

    // --- TelegramConfig tests ---

    #[test]
    fn test_default_config() {
        let config = TelegramConfig::default();
        assert_eq!(config.primary_chat_id, None);
        assert_eq!(config.allowed_group_chat_id, None);
    }

    #[test]
    fn test_with_config() {
        let bot = teloxide::Bot::new("fake_token");
        let config = TelegramConfig {
            primary_chat_id: Some(111),
            allowed_group_chat_id: Some(-100_222),
        };
        let adapter = TelegramAdapter::new(bot, vec![]).with_config(config.clone());
        assert_eq!(adapter.current_config(), config);
    }

    #[test]
    fn test_config_handle_returns_shared_ref() {
        let bot = teloxide::Bot::new("fake_token");
        let adapter = TelegramAdapter::new(bot, vec![]).with_primary_chat_id(42);
        let handle = adapter.config_handle();
        // Reading through the handle should see the same config.
        let cfg = handle.read().unwrap().clone();
        assert_eq!(cfg.primary_chat_id, Some(42));
    }

    #[test]
    fn test_config_update_reflected() {
        let bot = teloxide::Bot::new("fake_token");
        let adapter = TelegramAdapter::new(bot, vec![]);
        assert_eq!(adapter.current_config().primary_chat_id, None);

        // Obtain handle and update config externally.
        let handle = adapter.config_handle();
        {
            let mut cfg = handle.write().unwrap();
            cfg.primary_chat_id = Some(999);
            cfg.allowed_group_chat_id = Some(-100_777);
        }

        // Adapter should reflect the new values.
        let cfg = adapter.current_config();
        assert_eq!(cfg.primary_chat_id, Some(999));
        assert_eq!(cfg.allowed_group_chat_id, Some(-100_777));
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

    // --- Rich outbound message helpers ---

    #[test]
    fn test_parse_message_id_valid() {
        let msg_id = parse_message_id("123").unwrap();
        assert_eq!(msg_id, MessageId(123));
    }

    #[test]
    fn test_parse_message_id_negative() {
        let msg_id = parse_message_id("-1").unwrap();
        assert_eq!(msg_id, MessageId(-1));
    }

    #[test]
    fn test_parse_message_id_invalid() {
        let result = parse_message_id("abc");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_message_id_empty() {
        let result = parse_message_id("");
        assert!(result.is_err());
    }

    #[test]
    fn test_convert_reply_markup_none() {
        assert!(convert_reply_markup(&None).is_none());
    }

    #[test]
    fn test_convert_reply_markup_remove_keyboard() {
        let markup = Some(ReplyMarkup::RemoveKeyboard);
        assert!(convert_reply_markup(&markup).is_none());
    }

    #[test]
    fn test_convert_reply_markup_inline_keyboard() {
        let markup = Some(ReplyMarkup::InlineKeyboard {
            rows: vec![vec![
                InlineButton {
                    text: "Yes".to_owned(),
                    callback_data: Some("yes".to_owned()),
                    url: None,
                },
                InlineButton {
                    text: "No".to_owned(),
                    callback_data: Some("no".to_owned()),
                    url: None,
                },
            ]],
        });
        let result = convert_reply_markup(&markup).unwrap();
        assert_eq!(result.inline_keyboard.len(), 1);
        assert_eq!(result.inline_keyboard[0].len(), 2);
        assert_eq!(result.inline_keyboard[0][0].text, "Yes");
        assert_eq!(result.inline_keyboard[0][1].text, "No");
    }

    #[test]
    fn test_convert_reply_markup_url_button() {
        let markup = Some(ReplyMarkup::InlineKeyboard {
            rows: vec![vec![InlineButton {
                text: "Open".to_owned(),
                callback_data: None,
                url: Some("https://example.com".to_owned()),
            }]],
        });
        let result = convert_reply_markup(&markup).unwrap();
        assert_eq!(result.inline_keyboard[0][0].text, "Open");
    }

    #[test]
    fn test_convert_reply_markup_fallback_button() {
        let markup = Some(ReplyMarkup::InlineKeyboard {
            rows: vec![vec![InlineButton {
                text: "Click".to_owned(),
                callback_data: None,
                url: None,
            }]],
        });
        let result = convert_reply_markup(&markup).unwrap();
        assert_eq!(result.inline_keyboard[0][0].text, "Click");
    }

    #[test]
    fn test_convert_reply_markup_multiple_rows() {
        let markup = Some(ReplyMarkup::InlineKeyboard {
            rows: vec![
                vec![InlineButton {
                    text: "A".to_owned(),
                    callback_data: Some("a".to_owned()),
                    url: None,
                }],
                vec![
                    InlineButton {
                        text: "B".to_owned(),
                        callback_data: Some("b".to_owned()),
                        url: None,
                    },
                    InlineButton {
                        text: "C".to_owned(),
                        callback_data: Some("c".to_owned()),
                        url: None,
                    },
                ],
            ],
        });
        let result = convert_reply_markup(&markup).unwrap();
        assert_eq!(result.inline_keyboard.len(), 2);
        assert_eq!(result.inline_keyboard[0].len(), 1);
        assert_eq!(result.inline_keyboard[1].len(), 2);
    }

    #[test]
    fn test_mime_to_filename() {
        assert_eq!(mime_to_filename("image/jpeg"), "photo.jpg");
        assert_eq!(mime_to_filename("image/jpg"), "photo.jpg");
        assert_eq!(mime_to_filename("image/png"), "photo.png");
        assert_eq!(mime_to_filename("image/gif"), "photo.gif");
        assert_eq!(mime_to_filename("image/webp"), "photo.webp");
        assert_eq!(mime_to_filename("application/octet-stream"), "photo.bin");
        assert_eq!(mime_to_filename("unknown"), "photo.bin");
    }

    // --- Contact tracker tests ---

    #[test]
    fn test_with_contact_tracker() {
        use crate::telegram::contacts::NoopContactTracker;

        let bot = teloxide::Bot::new("fake_token");
        let tracker = Arc::new(NoopContactTracker);
        let adapter = TelegramAdapter::new(bot, vec![]).with_contact_tracker(tracker);
        assert!(adapter.contact_tracker.is_some());
    }

    #[test]
    fn test_without_contact_tracker() {
        let bot = teloxide::Bot::new("fake_token");
        let adapter = TelegramAdapter::new(bot, vec![]);
        assert!(adapter.contact_tracker.is_none());
    }

    // -----------------------------------------------------------------------
    // I/O Bus model tests
    // -----------------------------------------------------------------------

    /// Build a minimal teloxide Message from JSON for testing.
    fn make_test_message(text: &str, user_id: u64, chat_id: i64) -> teloxide::types::Message {
        let json = serde_json::json!({
            "message_id": 42,
            "date": 1700000000,
            "chat": {
                "id": chat_id,
                "type": "private",
                "first_name": "Test"
            },
            "from": {
                "id": user_id,
                "is_bot": false,
                "first_name": "Test",
                "last_name": "User",
                "username": "testuser"
            },
            "text": text
        });
        serde_json::from_value(json).expect("valid test message JSON")
    }

    /// Build a minimal group chat message for testing.
    fn make_test_group_message(
        text: &str,
        user_id: u64,
        chat_id: i64,
    ) -> teloxide::types::Message {
        let json = serde_json::json!({
            "message_id": 42,
            "date": 1700000000,
            "chat": {
                "id": chat_id,
                "type": "supergroup",
                "title": "Test Group"
            },
            "from": {
                "id": user_id,
                "is_bot": false,
                "first_name": "Test",
                "last_name": "User",
                "username": "testuser"
            },
            "text": text
        });
        serde_json::from_value(json).expect("valid test group message JSON")
    }

    #[test]
    fn test_telegram_to_raw_platform_message_basic() {
        let msg = make_test_message("Hello world", 12345, 67890);
        let raw = telegram_to_raw_platform_message(&msg, "mybot").unwrap();

        assert_eq!(raw.channel_type, ChannelType::Telegram);
        assert_eq!(raw.platform_user_id, "12345");
        assert_eq!(raw.platform_chat_id, Some("67890".to_string()));
        assert_eq!(raw.platform_message_id, Some("42".to_string()));
        assert_eq!(raw.content.as_text(), "Hello world");

        // Check reply context.
        let ctx = raw.reply_context.unwrap();
        assert!(matches!(ctx.interaction_type, InteractionType::Message));
        assert!(ctx.thread_id.is_none());
        assert!(ctx.reply_to_platform_msg_id.is_none());

        // Check metadata.
        assert_eq!(
            raw.metadata.get("telegram_username"),
            Some(&serde_json::Value::String("testuser".to_owned()))
        );
        assert_eq!(
            raw.metadata.get("telegram_display_name"),
            Some(&serde_json::Value::String("Test User".to_owned()))
        );
        assert_eq!(
            raw.metadata.get("telegram_bot_username"),
            Some(&serde_json::Value::String("mybot".to_owned()))
        );
    }

    #[test]
    fn test_telegram_to_raw_platform_message_command() {
        let msg = make_test_message("/search rust developer", 12345, 67890);
        let raw = telegram_to_raw_platform_message(&msg, "mybot").unwrap();

        assert_eq!(raw.content.as_text(), "/search rust developer");
        let ctx = raw.reply_context.unwrap();
        assert!(matches!(
            ctx.interaction_type,
            InteractionType::Command(ref name) if name == "search"
        ));
    }

    #[test]
    fn test_telegram_to_raw_platform_message_empty_text() {
        let msg = make_test_message("", 12345, 67890);
        let raw = telegram_to_raw_platform_message(&msg, "mybot");
        assert!(raw.is_none());
    }

    #[test]
    fn test_telegram_to_raw_platform_message_whitespace_only() {
        let msg = make_test_message("   ", 12345, 67890);
        let raw = telegram_to_raw_platform_message(&msg, "mybot");
        assert!(raw.is_none());
    }

    #[test]
    fn test_telegram_to_raw_platform_message_group_strips_mention() {
        let msg = make_test_group_message("@mybot hello there", 12345, -100_999);
        let raw = telegram_to_raw_platform_message(&msg, "mybot").unwrap();

        // The @mention should be stripped in group chats.
        assert_eq!(raw.content.as_text(), "hello there");
    }

    #[test]
    fn test_telegram_to_raw_platform_message_no_bot_username() {
        let msg = make_test_message("Hello", 12345, 67890);
        let raw = telegram_to_raw_platform_message(&msg, "").unwrap();

        assert_eq!(raw.content.as_text(), "Hello");
        // No bot username in metadata.
        assert!(!raw.metadata.contains_key("telegram_bot_username"));
    }

    #[test]
    fn test_telegram_to_raw_platform_message_command_with_bot_mention() {
        let msg = make_test_message("/search@mybot keywords", 12345, 67890);
        let raw = telegram_to_raw_platform_message(&msg, "mybot").unwrap();

        let ctx = raw.reply_context.unwrap();
        assert!(matches!(
            ctx.interaction_type,
            InteractionType::Command(ref name) if name == "search"
        ));
    }

    #[test]
    fn test_egress_adapter_channel_type() {
        let bot = teloxide::Bot::new("fake_token");
        let adapter = TelegramAdapter::new(bot, vec![]);
        // Disambiguate: call EgressAdapter's channel_type explicitly.
        let egress_ct = <TelegramAdapter as EgressAdapter>::channel_type(&adapter);
        assert_eq!(egress_ct, ChannelType::Telegram);
    }
}
