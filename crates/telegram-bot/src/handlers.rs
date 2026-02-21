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

//! Telegram message and callback query handlers.
//!
//! This module contains all bot business logic. The entry point is
//! [`handle_update`], which dispatches incoming Telegram updates to the
//! appropriate handler:
//!
//! - **Commands** (`/start`, `/help`, `/search`) are parsed via teloxide's
//!   `BotCommands` derive and routed to dedicated handlers.
//! - **Plain text** is treated as a raw Job Description and submitted to the
//!   main service for parsing via the HTTP client.
//! - **Callback queries** handle the "Load More" pagination button for job
//!   search results.
//!
//! Group chats are handled in mention mode: the bot responds only when
//! explicitly mentioned (e.g. `@botname ...`). Private chats remain fully
//! interactive.

use std::sync::Arc;
use std::time::Instant;

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use teloxide::{
    net::Download,
    payloads::{EditMessageTextSetters, SendMessageSetters},
    requests::Requester,
    sugar::request::RequestReplyExt,
    types::{CallbackQuery, ChatAction, ChatId, Message, ParseMode, PhotoSize, Update, UpdateKind},
    utils::command::BotCommands,
};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::{
    command::Command,
    http_client::{ChatStreamEvent, DiscoveryJobResponse},
    markdown::{TELEGRAM_MAX_MESSAGE_LEN, chunk_message, markdown_to_telegram_html},
    state::BotState,
};

/// Maximum number of sessions to display in the `/sessions` list.
const SESSIONS_LIST_LIMIT: u32 = 10;

/// Groups with this many members or fewer are treated like private chats —
/// the bot responds to every message without requiring an @mention or keyword.
const SMALL_GROUP_THRESHOLD: u32 = 3;

/// Top-level update dispatcher: routes to message or callback query handlers.
pub(crate) async fn handle_update(update: Update, state: &Arc<BotState>) {
    let result = match update.kind {
        UpdateKind::Message(msg) => handle_message_direct(msg, state).await,
        UpdateKind::CallbackQuery(query) => handle_callback_query(query, state).await,
        _ => Ok(()),
    };

    if let Err(e) = result {
        warn!(error = %e, "error handling telegram update");
    }
}

/// Handle an incoming Telegram message.
///
/// Routes to command handlers if the message matches a known command,
/// otherwise forwards to the chat session system for AI conversation.
pub(crate) async fn handle_message_direct(
    msg: Message,
    state: &Arc<BotState>,
) -> Result<(), teloxide::RequestError> {
    let chat_is_public = matches!(msg.chat.kind, teloxide::types::ChatKind::Public(..));
    let bot_username = state.bot_username.as_deref();
    info!("receive message: {:?}", msg);
    if chat_is_public {
        // In small groups (≤ SMALL_GROUP_THRESHOLD members), respond to all
        // messages like a private chat. In larger groups, require @mention or
        // keyword trigger.
        let is_small_group = matches!(
            state.bot.get_chat_member_count(msg.chat.id).await,
            Ok(n) if n <= SMALL_GROUP_THRESHOLD
        );

        if !is_small_group {
            let trigger_text = msg.text().or_else(|| msg.caption()).unwrap_or_default();
            let bubble = should_bubble_in_group(&msg, trigger_text, bot_username);
            if !bubble {
                return Ok(());
            }
        }

        if !state.is_allowed_group_chat(msg.chat.id) {
            warn!(
                chat_id = msg.chat.id.0,
                allowed_group_chat_id = ?state.current_config().allowed_group_chat_id,
                "dropping group message: group is not authorized"
            );
            state
                .bot
                .send_message(
                    msg.chat.id,
                    "这个群还没授权给我。请在 Settings -> Telegram Bot 里把 Allowed Group Chat ID \
                     设置为当前群的 chat id（通常是 -100 开头）。",
                )
                .await?;
            return Ok(());
        }
    }

    // Photo messages — download and forward to chat as multimodal content.
    if let Some(photos) = msg.photo() {
        let caption = msg.caption().unwrap_or("");
        return handle_photo_message(&msg, photos, caption, state).await;
    }

    let Some(text) = extract_text(&msg) else {
        return Ok(());
    };

    let user_text = strip_group_mention(text, bot_username);

    // Try parsing as a bot command first.
    if let Ok(cmd) = Command::parse(&user_text, bot_username.unwrap_or("")) {
        return handle_command(msg, cmd, state).await;
    }

    // Unknown slash commands.
    if user_text.trim_start().starts_with('/') {
        state
            .bot
            .send_message(
                msg.chat.id,
                "Unknown command. Use /help to see available commands.",
            )
            .await?;
        return Ok(());
    }

    // Plain text -> route to chat session.
    handle_chat_message(&msg, &user_text, state).await
}

/// Handle callback queries (inline keyboard button presses).
pub(crate) async fn handle_callback_query(
    q: CallbackQuery,
    state: &Arc<BotState>,
) -> Result<(), teloxide::RequestError> {
    // Always ack callback query so Telegram client stops spinner quickly.
    state.bot.answer_callback_query(q.id.clone()).await?;

    let Some(data) = q.data.as_deref() else {
        return Ok(());
    };
    if let Some(ref msg) = q.message
        && matches!(msg.chat().kind, teloxide::types::ChatKind::Public(..))
        && !state.is_allowed_group_chat(msg.chat().id)
    {
        return Ok(());
    }

    // Handle session switching from inline keyboard buttons.
    if data.starts_with("switch:") {
        let key = &data["switch:".len()..];
        if let Some(ref msg) = q.message {
            let chat_id = msg.chat().id;
            let account = "default";
            let chat_id_str = chat_id.0.to_string();

            match state
                .http_client
                .bind_channel("telegram", account, &chat_id_str, key)
                .await
            {
                Ok(_) => {
                    state
                        .bot
                        .send_message(
                            chat_id,
                            format!("Switched to session: <code>{}</code>", html_escape(key)),
                        )
                        .parse_mode(ParseMode::Html)
                        .await?;
                }
                Err(e) => {
                    state
                        .bot
                        .send_message(chat_id, format!("Failed to switch session: {e}"))
                        .await?;
                }
            }
        }
        return Ok(());
    }

    if !data.starts_with("search_more:") {
        return Ok(());
    }

    if let Some(ref msg) = q.message {
        if !state.is_primary_chat(msg.chat().id) {
            return Ok(());
        }
    }

    let parts: Vec<&str> = data.splitn(3, ':').collect();
    if parts.len() != 3 {
        return Ok(());
    }

    let current_count: u32 = match parts[1].parse() {
        Ok(n) => n,
        Err(_) => return Ok(()),
    };

    let (keywords, location) = decode_search_params(parts[2]);
    let new_max = current_count + 3;

    let jobs = match state
        .http_client
        .discover_jobs(keywords.clone(), location.clone(), new_max)
        .await
    {
        Ok(jobs) => jobs,
        Err(e) => {
            if let Some(ref msg) = q.message {
                state
                    .bot
                    .send_message(msg.chat().id, format!("Load more failed: {e}"))
                    .await?;
            }
            return Ok(());
        }
    };

    let text = format_job_results(&jobs, &keywords, location.as_deref());

    if let Some(ref msg) = q.message {
        let msg_id = msg.id();
        let chat_id = msg.chat().id;

        #[allow(clippy::cast_possible_truncation)]
        if (jobs.len() as u32) < new_max {
            state
                .bot
                .edit_message_text(chat_id, msg_id, text)
                .parse_mode(teloxide::types::ParseMode::Html)
                .await?;
        } else {
            let keyboard = load_more_keyboard(
                jobs.len(),
                &encode_search_params(&keywords, location.as_deref()),
            );
            state
                .bot
                .edit_message_text(chat_id, msg_id, text)
                .parse_mode(teloxide::types::ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
    }

    Ok(())
}

/// Start a background loop that sends `ChatAction::Typing` every 4 seconds.
///
/// Returns a join handle and a [`CancellationToken`]. Cancel the token to
/// stop the loop (e.g. once the LLM response is ready), then optionally
/// await the handle to ensure clean shutdown.
fn start_typing_loop(
    bot: teloxide::Bot,
    chat_id: ChatId,
) -> (tokio::task::JoinHandle<()>, CancellationToken) {
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    let handle = tokio::spawn(async move {
        loop {
            let _ = bot.send_chat_action(chat_id, ChatAction::Typing).await;
            tokio::select! {
                () = tokio::time::sleep(std::time::Duration::from_secs(4)) => {}
                () = token.cancelled() => break,
            }
        }
    });
    (handle, cancel)
}

/// Minimum interval between Telegram `edit_message_text` calls (1.5 seconds)
/// to avoid hitting Telegram API rate limits.
const EDIT_THROTTLE: std::time::Duration = std::time::Duration::from_millis(1500);

/// Stream AI response via SSE and progressively update a Telegram message.
///
/// 1. Sends a `ChatAction::Typing` indicator (no placeholder message).
/// 2. Consumes SSE events from the streaming endpoint.
/// 3. On `TextDelta`: accumulates text; on the first non-empty content, sends
///    a real message and records its `message_id`. Subsequent deltas edit the
///    message every ~1.5 seconds (throttled).
/// 4. On `ToolCallStart`: appends a status line like "Using tool: search_web".
/// 5. On `Done`: final edit with the complete response (Markdown -> HTML).
/// 6. On `Error`: edits (or sends) the message to show the error.
///
/// Falls back to the synchronous `send_chat_message()` if SSE connection fails.
async fn stream_and_relay(
    msg: &Message,
    session_key: &str,
    text: &str,
    image_urls: Vec<String>,
    state: &Arc<BotState>,
) -> Result<(), teloxide::RequestError> {
    // Try SSE streaming first.
    let mut rx = match state
        .http_client
        .send_chat_message_streaming(session_key, text, image_urls.clone())
        .await
    {
        Ok(rx) => rx,
        Err(e) => {
            // Fallback to synchronous endpoint.
            warn!(error = %e, "SSE streaming failed, falling back to sync");
            return fallback_sync_reply(msg, session_key, text, image_urls, state).await;
        }
    };

    // Send typing indicator instead of a "..." placeholder message.
    state
        .bot
        .send_chat_action(msg.chat.id, ChatAction::Typing)
        .await
        .ok();

    // Message ID is None until we have real content to show.
    let mut message_id: Option<teloxide::types::MessageId> = None;

    let mut accumulated_text = String::new();
    let mut last_edit = Instant::now();
    let mut tool_status: Vec<String> = Vec::new();
    let mut final_text: Option<String> = None;
    let mut errored = false;

    while let Some(event) = rx.recv().await {
        match event {
            ChatStreamEvent::TextDelta { text: delta } => {
                accumulated_text.push_str(&delta);
                // Throttled edit: only send if enough time has passed and there's
                // content.
                if last_edit.elapsed() >= EDIT_THROTTLE && !accumulated_text.trim().is_empty() {
                    let display = build_progress_text(&accumulated_text, &tool_status);
                    message_id =
                        send_or_edit(state, msg, message_id, &display, None).await;
                    last_edit = Instant::now();
                }
            }
            ChatStreamEvent::ToolCallStart { name, .. } => {
                tool_status.push(format!("\u{1f527} {name}"));
                let display = build_progress_text(&accumulated_text, &tool_status);
                // Always send/edit on tool start (important UX feedback).
                message_id =
                    send_or_edit(state, msg, message_id, &display, None).await;
                last_edit = Instant::now();
            }
            ChatStreamEvent::ToolCallEnd {
                name,
                success,
                error,
                ..
            } => {
                // Update the tool status line.
                if let Some(entry) = tool_status.iter_mut().find(|s| s.contains(&name)) {
                    if success {
                        *entry = format!("\u{2705} {name}");
                    } else {
                        let err_msg = error.as_deref().unwrap_or("failed");
                        *entry = format!("\u{274c} {name}: {err_msg}");
                    }
                }
            }
            ChatStreamEvent::Done { text: done_text } => {
                final_text = Some(done_text);
                break;
            }
            ChatStreamEvent::Error { message: err_msg } => {
                let error_text = format!("Error: {err_msg}");
                send_or_edit(state, msg, message_id, &error_text, None).await;
                errored = true;
                break;
            }
            // Ignore thinking/iteration/reasoning events.
            _ => {}
        }
    }

    if errored {
        return Ok(());
    }

    // Final edit with the complete text.
    let response_text = final_text.unwrap_or(accumulated_text);
    if response_text.trim().is_empty() {
        send_or_edit(state, msg, message_id, "(empty response)", None).await;
        return Ok(());
    }

    // Add @mention for group chats.
    let response_text = if matches!(msg.chat.kind, teloxide::types::ChatKind::Public(..)) {
        let mention = mention_sender(msg);
        if mention.is_empty() {
            response_text
        } else {
            format!("{mention}\n{response_text}")
        }
    } else {
        response_text
    };

    let html = markdown_to_telegram_html(&response_text);
    let chunks = chunk_message(&html, TELEGRAM_MAX_MESSAGE_LEN);

    if chunks.len() == 1 {
        // Single chunk: edit or send the message.
        send_or_edit(
            state,
            msg,
            message_id,
            &chunks[0],
            Some(ParseMode::Html),
        )
        .await;
    } else {
        // Multiple chunks: edit/send first chunk, send rest as new messages.
        send_or_edit(
            state,
            msg,
            message_id,
            &chunks[0],
            Some(ParseMode::Html),
        )
        .await;
        for chunk in &chunks[1..] {
            state
                .bot
                .send_message(msg.chat.id, chunk)
                .parse_mode(ParseMode::Html)
                .await?;
        }
    }

    Ok(())
}

/// Send a new message or edit an existing one.
///
/// If `message_id` is `None`, sends a new reply message and returns `Some(id)`.
/// If `message_id` is `Some`, edits the existing message and returns the same
/// `Some(id)`. Errors are silently ignored (matching the existing behaviour for
/// intermediate progress edits).
async fn send_or_edit(
    state: &Arc<BotState>,
    msg: &Message,
    message_id: Option<teloxide::types::MessageId>,
    text: &str,
    parse_mode: Option<ParseMode>,
) -> Option<teloxide::types::MessageId> {
    match message_id {
        Some(id) => {
            let mut req = state.bot.edit_message_text(msg.chat.id, id, text);
            if let Some(mode) = parse_mode {
                req = req.parse_mode(mode);
            }
            let _ = req.await;
            Some(id)
        }
        None => {
            let mut req = state.bot.send_message(msg.chat.id, text).reply_to(msg.id);
            if let Some(mode) = parse_mode {
                req = req.parse_mode(mode);
            }
            match req.await {
                Ok(sent) => Some(sent.id),
                Err(_) => None,
            }
        }
    }
}

/// Build a progress display string showing accumulated text and tool status.
fn build_progress_text(text: &str, tool_status: &[String]) -> String {
    let mut display = String::new();
    if !tool_status.is_empty() {
        for status in tool_status {
            display.push_str(status);
            display.push('\n');
        }
        display.push('\n');
    }
    if !text.trim().is_empty() {
        display.push_str(text);
    }
    if display.is_empty() {
        display.push_str("...");
    }
    // Telegram edit_message_text won't accept identical content,
    // and we don't want to exceed message size limits for intermediate edits.
    if display.len() > 4000 {
        display.truncate(4000);
        display.push_str("...");
    }
    display
}

/// Fallback: use the synchronous `send_chat_message` endpoint.
async fn fallback_sync_reply(
    msg: &Message,
    session_key: &str,
    text: &str,
    image_urls: Vec<String>,
    state: &Arc<BotState>,
) -> Result<(), teloxide::RequestError> {
    let (typing_handle, typing_cancel) = start_typing_loop(state.bot.clone(), msg.chat.id);

    let result = state
        .http_client
        .send_chat_message(session_key, text, image_urls)
        .await;

    typing_cancel.cancel();
    let _ = typing_handle.await;

    match result {
        Ok(response) => {
            let reply_text = response.message.text_content();
            if !reply_text.is_empty() {
                let reply_text =
                    if matches!(msg.chat.kind, teloxide::types::ChatKind::Public(..)) {
                        let mention = mention_sender(msg);
                        if mention.is_empty() {
                            reply_text
                        } else {
                            format!("{mention}\n{reply_text}")
                        }
                    } else {
                        reply_text
                    };

                let html = markdown_to_telegram_html(&reply_text);
                let chunks = chunk_message(&html, TELEGRAM_MAX_MESSAGE_LEN);
                for (i, chunk) in chunks.into_iter().enumerate() {
                    let mut req = state
                        .bot
                        .send_message(msg.chat.id, chunk)
                        .parse_mode(ParseMode::Html);
                    if i == 0 {
                        req = req.reply_to(msg.id);
                    }
                    req.await?;
                }
            }
        }
        Err(e) => {
            state
                .bot
                .send_message(msg.chat.id, format!("Chat error: {e}"))
                .reply_to(msg.id)
                .await?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Command handlers
// ---------------------------------------------------------------------------

/// Dispatch a parsed command to the appropriate handler.
async fn handle_command(
    msg: Message,
    cmd: Command,
    state: &Arc<BotState>,
) -> Result<(), teloxide::RequestError> {
    match cmd {
        Command::Start => {
            state
                .bot
                .send_message(
                    msg.chat.id,
                    "Welcome! I'm the Job Assistant bot.\nSend me any message to start a \
                     conversation.\n\nCommands:\n/search <keywords> [@ location] - Search \
                     jobs\n/jd <text> - Parse a Job Description\n/new - Start a new chat \
                     session\n/clear - Clear current session history\n/sessions - List & switch \
                     chat sessions\n/usage - Show current session info\n/help - Show all commands",
                )
                .await?;
        }
        Command::Help => {
            state
                .bot
                .send_message(msg.chat.id, Command::descriptions().to_string())
                .await?;
        }
        Command::Search(args) => {
            handle_search(msg, args, state).await?;
        }
        Command::New => {
            handle_new_session(&msg, state).await?;
        }
        Command::Clear => {
            handle_clear_session(&msg, state).await?;
        }
        Command::Jd(jd_text) => {
            handle_jd_parse(&msg, &jd_text, state).await?;
        }
        Command::Sessions => {
            handle_sessions(&msg, state).await?;
        }
        Command::Usage => {
            handle_usage(&msg, state).await?;
        }
        Command::Model(args) => {
            handle_model(&msg, &args, state).await?;
        }
        Command::Mcp => {
            handle_mcp(&msg, state).await?;
        }
    }
    Ok(())
}

/// Route plain text messages to the chat session system.
///
/// Resolves (or auto-creates) a channel binding for this Telegram chat,
/// then streams the AI response via SSE with progressive Telegram message
/// updates.
async fn handle_chat_message(
    msg: &Message,
    text: &str,
    state: &Arc<BotState>,
) -> Result<(), teloxide::RequestError> {
    let account = "default";
    let chat_id_str = msg.chat.id.0.to_string();

    // Resolve or auto-create session binding.
    let session_key = match state
        .http_client
        .get_channel_session(account, &chat_id_str)
        .await
    {
        Ok(Some(binding)) => binding.session_key,
        Ok(None) => {
            // Auto-create: generate key, create session, bind channel.
            let key = format!("tg-{}", msg.chat.id.0);
            let _ = state
                .http_client
                .create_session(&key, Some("Telegram Chat"))
                .await;
            let _ = state
                .http_client
                .bind_channel("telegram", account, &chat_id_str, &key)
                .await;
            key
        }
        Err(e) => {
            state
                .bot
                .send_message(msg.chat.id, format!("Failed to resolve session: {e}"))
                .await?;
            return Ok(());
        }
    };

    stream_and_relay(msg, &session_key, text, vec![], state).await
}

/// Handle a photo message by downloading the image and forwarding it to the
/// chat service as multimodal content (base64 data URL).
///
/// Selects the highest-resolution variant from the `PhotoSize` array (the
/// last element), downloads the file via the Bot API, converts it to a
/// `data:image/jpeg;base64,...` URL, and sends it alongside the caption text
/// to the chat service.
async fn handle_photo_message(
    msg: &Message,
    photos: &[PhotoSize],
    caption: &str,
    state: &Arc<BotState>,
) -> Result<(), teloxide::RequestError> {
    let account = "default";
    let chat_id_str = msg.chat.id.0.to_string();

    // Resolve or auto-create session binding (same logic as text messages).
    let session_key = match state
        .http_client
        .get_channel_session(account, &chat_id_str)
        .await
    {
        Ok(Some(binding)) => binding.session_key,
        Ok(None) => {
            let key = format!("tg-{}", msg.chat.id.0);
            let _ = state
                .http_client
                .create_session(&key, Some("Telegram Chat"))
                .await;
            let _ = state
                .http_client
                .bind_channel("telegram", account, &chat_id_str, &key)
                .await;
            key
        }
        Err(e) => {
            state
                .bot
                .send_message(msg.chat.id, format!("Failed to resolve session: {e}"))
                .await?;
            return Ok(());
        }
    };

    // Start continuous typing indicator — download + LLM inference may take a
    // while.
    let (typing_handle, typing_cancel) = start_typing_loop(state.bot.clone(), msg.chat.id);

    // Select the highest-resolution photo (last element in the array).
    let Some(photo) = photos.last() else {
        typing_cancel.cancel();
        let _ = typing_handle.await;
        state
            .bot
            .send_message(msg.chat.id, "Could not read photo data.")
            .await?;
        return Ok(());
    };

    // Download the photo via the Bot API.
    let file = match state.bot.get_file(photo.file.id.clone()).await {
        Ok(f) => f,
        Err(e) => {
            typing_cancel.cancel();
            let _ = typing_handle.await;
            warn!(error = %e, "failed to get file info for photo");
            state
                .bot
                .send_message(msg.chat.id, format!("Failed to download photo: {e}"))
                .await?;
            return Ok(());
        }
    };

    let mut buf = Vec::new();
    if let Err(e) = state.bot.download_file(&file.path, &mut buf).await {
        typing_cancel.cancel();
        let _ = typing_handle.await;
        warn!(error = %e, "failed to download photo file");
        state
            .bot
            .send_message(msg.chat.id, format!("Failed to download photo: {e}"))
            .await?;
        return Ok(());
    }

    info!(
        file_id = %photo.file.id,
        size_bytes = buf.len(),
        "photo downloaded from Telegram"
    );

    // Convert to base64 data URL for multimodal LLM input.
    let data_url = format!("data:image/jpeg;base64,{}", BASE64.encode(&buf));

    // Use caption as text; fall back to a generic prompt if empty.
    let text = if caption.trim().is_empty() {
        "Please analyze this image and reply in the same language and tone as this conversation."
    } else {
        caption
    };

    // Stop typing since streaming will show progress inline.
    typing_cancel.cancel();
    let _ = typing_handle.await;

    // Stream AI response and progressively update Telegram message.
    stream_and_relay(msg, &session_key, text, vec![data_url], state).await
}

/// Handle `/new` — start a new chat session and re-bind the channel.
async fn handle_new_session(
    msg: &Message,
    state: &Arc<BotState>,
) -> Result<(), teloxide::RequestError> {
    let account = "default";
    let chat_id_str = msg.chat.id.0.to_string();

    // Generate a unique key using the chat id and current timestamp.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let key = format!("tg-{}-{now}", msg.chat.id.0);

    match state
        .http_client
        .create_session(&key, Some("Telegram Chat"))
        .await
    {
        Ok(()) => {
            let _ = state
                .http_client
                .bind_channel("telegram", account, &chat_id_str, &key)
                .await;
            state
                .bot
                .send_message(msg.chat.id, "New chat session started.")
                .await?;
        }
        Err(e) => {
            state
                .bot
                .send_message(msg.chat.id, format!("Failed to create session: {e}"))
                .await?;
        }
    }

    Ok(())
}

/// Handle `/clear` — clear all messages in the current session.
async fn handle_clear_session(
    msg: &Message,
    state: &Arc<BotState>,
) -> Result<(), teloxide::RequestError> {
    let account = "default";
    let chat_id_str = msg.chat.id.0.to_string();

    match state
        .http_client
        .get_channel_session(account, &chat_id_str)
        .await
    {
        Ok(Some(binding)) => {
            match state
                .http_client
                .clear_session_messages(&binding.session_key)
                .await
            {
                Ok(()) => {
                    state
                        .bot
                        .send_message(msg.chat.id, "Session history cleared.")
                        .await?;
                }
                Err(e) => {
                    state
                        .bot
                        .send_message(msg.chat.id, format!("Failed to clear: {e}"))
                        .await?;
                }
            }
        }
        Ok(None) => {
            state
                .bot
                .send_message(
                    msg.chat.id,
                    "No active session. Send a message to start one.",
                )
                .await?;
        }
        Err(e) => {
            state
                .bot
                .send_message(msg.chat.id, format!("Error: {e}"))
                .await?;
        }
    }

    Ok(())
}

/// Handle `/jd <text>` — parse a Job Description (primary chat only).
async fn handle_jd_parse(
    msg: &Message,
    jd_text: &str,
    state: &Arc<BotState>,
) -> Result<(), teloxide::RequestError> {
    if !state.is_primary_chat(msg.chat.id) {
        state
            .bot
            .send_message(msg.chat.id, "Unauthorized chat.")
            .await?;
        return Ok(());
    }

    let jd_text = jd_text.trim();
    if jd_text.is_empty() {
        state
            .bot
            .send_message(msg.chat.id, "Usage: /jd <paste job description text>")
            .await?;
        return Ok(());
    }

    state
        .bot
        .send_message(msg.chat.id, "Received your JD, processing...")
        .await?;

    if let Err(e) = state.http_client.submit_jd_parse(jd_text).await {
        state
            .bot
            .send_message(msg.chat.id, format!("JD parse failed: {e}"))
            .await?;
    }

    Ok(())
}

/// Handle `/search <keywords> [@ location]`.
///
/// Parses keyword and optional location arguments, calls the main service
/// discovery API, and sends results as an HTML-formatted message with an
/// inline "Load More" button for pagination.
async fn handle_search(
    msg: Message,
    args: String,
    state: &Arc<BotState>,
) -> Result<(), teloxide::RequestError> {
    // Hard gate by configured primary chat to avoid accidental public usage.
    if !state.is_primary_chat(msg.chat.id) {
        state
            .bot
            .send_message(msg.chat.id, "Unauthorized chat.")
            .await?;
        return Ok(());
    }

    let args = args.trim();
    if args.is_empty() {
        state
            .bot
            .send_message(
                msg.chat.id,
                "Usage: /search <keywords> [@ location]\nExample: /search rust engineer @ beijing",
            )
            .await?;
        return Ok(());
    }

    let (keywords_str, location) = if let Some(idx) = args.find(" @ ") {
        (&args[..idx], Some(args[idx + 3..].trim().to_owned()))
    } else {
        (args, None)
    };

    let keywords: Vec<String> = keywords_str.split_whitespace().map(String::from).collect();
    if keywords.is_empty() {
        state
            .bot
            .send_message(msg.chat.id, "Please provide at least one keyword.")
            .await?;
        return Ok(());
    }

    state
        .bot
        .send_message(
            msg.chat.id,
            format!(
                "Searching: {} @ {} ...",
                keywords.join(" "),
                location.as_deref().unwrap_or("any")
            ),
        )
        .await?;

    let jobs = match state
        .http_client
        .discover_jobs(keywords.clone(), location.clone(), 3)
        .await
    {
        Ok(jobs) => jobs,
        Err(e) => {
            state
                .bot
                .send_message(msg.chat.id, format!("Search failed: {e}"))
                .await?;
            return Ok(());
        }
    };

    if jobs.is_empty() {
        state
            .bot
            .send_message(msg.chat.id, "No jobs found matching your criteria.")
            .await?;
        return Ok(());
    }

    let text = format_job_results(&jobs, &keywords, location.as_deref());
    let keyboard = load_more_keyboard(
        jobs.len(),
        &encode_search_params(&keywords, location.as_deref()),
    );

    state
        .bot
        .send_message(msg.chat.id, text)
        .parse_mode(teloxide::types::ParseMode::Html)
        .reply_markup(keyboard)
        .await?;

    Ok(())
}

/// Handle `/sessions` — list all sessions, marking the active one.
///
/// Also sends an inline keyboard so the user can tap a button to switch
/// sessions without typing the key manually.
async fn handle_sessions(
    msg: &Message,
    state: &Arc<BotState>,
) -> Result<(), teloxide::RequestError> {
    let account = "default";
    let chat_id_str = msg.chat.id.0.to_string();

    // Find the currently active session key (if any).
    let active_key = match state
        .http_client
        .get_channel_session(account, &chat_id_str)
        .await
    {
        Ok(Some(binding)) => Some(binding.session_key),
        Ok(None) => None,
        Err(e) => {
            state
                .bot
                .send_message(msg.chat.id, format!("Failed to resolve session: {e}"))
                .await?;
            return Ok(());
        }
    };

    // Fetch the list of sessions.
    let sessions = match state.http_client.list_sessions(SESSIONS_LIST_LIMIT).await {
        Ok(s) => s,
        Err(e) => {
            state
                .bot
                .send_message(msg.chat.id, format!("Failed to list sessions: {e}"))
                .await?;
            return Ok(());
        }
    };

    if sessions.is_empty() {
        state
            .bot
            .send_message(
                msg.chat.id,
                "No sessions found. Send a message to create one.",
            )
            .await?;
        return Ok(());
    }

    // Build the text listing.
    use std::fmt::Write;
    let mut text = String::from("<b>Your sessions:</b>\n\n");
    let mut buttons: Vec<Vec<teloxide::types::InlineKeyboardButton>> = Vec::new();

    for (i, s) in sessions.iter().enumerate() {
        let title = s.title.as_deref().unwrap_or(&s.key);
        let is_active = active_key.as_deref() == Some(&s.key);
        let marker = if is_active { " \u{2705}" } else { "" };

        let _ = writeln!(
            text,
            "{}. <b>{}</b>{marker}\n   <code>{}</code> ({} msgs)",
            i + 1,
            html_escape(title),
            html_escape(&s.key),
            s.message_count,
        );

        // Add an inline button for each non-active session to allow switching.
        if !is_active {
            let label = format!("Switch to: {}", truncate_str(title, 30),);
            // Callback data limit is 64 bytes; prefix "switch:" is 7 bytes.
            let cb_data = format!("switch:{}", truncate_str(&s.key, 56));
            buttons.push(vec![teloxide::types::InlineKeyboardButton::callback(
                label, cb_data,
            )]);
        }
    }

    let keyboard = teloxide::types::InlineKeyboardMarkup::new(buttons);

    state
        .bot
        .send_message(msg.chat.id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;

    Ok(())
}

/// Handle `/usage` — show details about the current active session.
async fn handle_usage(msg: &Message, state: &Arc<BotState>) -> Result<(), teloxide::RequestError> {
    let account = "default";
    let chat_id_str = msg.chat.id.0.to_string();

    let session_key = match state
        .http_client
        .get_channel_session(account, &chat_id_str)
        .await
    {
        Ok(Some(binding)) => binding.session_key,
        Ok(None) => {
            state
                .bot
                .send_message(
                    msg.chat.id,
                    "No active session. Send a message to create one.",
                )
                .await?;
            return Ok(());
        }
        Err(e) => {
            state
                .bot
                .send_message(msg.chat.id, format!("Failed to resolve session: {e}"))
                .await?;
            return Ok(());
        }
    };

    match state.http_client.get_session(&session_key).await {
        Ok(detail) => {
            use std::fmt::Write;
            let mut text = String::new();

            let title = detail.title.as_deref().unwrap_or("Untitled");
            let _ = writeln!(text, "<b>Session:</b> {}", html_escape(title));
            let _ = writeln!(
                text,
                "<b>Key:</b> <code>{}</code>",
                html_escape(&detail.key)
            );
            let _ = writeln!(text, "<b>Messages:</b> {}", detail.message_count);
            if let Some(ref model) = detail.model {
                let _ = writeln!(text, "<b>Model:</b> {}", html_escape(model));
            }
            let _ = writeln!(
                text,
                "<b>Created:</b> {}",
                format_timestamp(&detail.created_at)
            );
            let _ = writeln!(
                text,
                "<b>Last active:</b> {}",
                format_timestamp(&detail.updated_at)
            );

            if let Some(ref preview) = detail.preview {
                let truncated = truncate_str(preview, 200);
                let _ = writeln!(text, "\n<b>Last message:</b>\n{}", html_escape(truncated));
            }

            state
                .bot
                .send_message(msg.chat.id, text)
                .parse_mode(ParseMode::Html)
                .await?;
        }
        Err(e) => {
            state
                .bot
                .send_message(msg.chat.id, format!("Failed to get session details: {e}"))
                .await?;
        }
    }

    Ok(())
}

/// Handle `/model [name]` — show or switch the model for the current session.
///
/// Without arguments, displays the current model. With an argument, updates
/// the session model to the specified value.
async fn handle_model(
    msg: &Message,
    args: &str,
    state: &Arc<BotState>,
) -> Result<(), teloxide::RequestError> {
    let account = "default";
    let chat_id_str = msg.chat.id.0.to_string();

    let session_key = match state
        .http_client
        .get_channel_session(account, &chat_id_str)
        .await
    {
        Ok(Some(binding)) => binding.session_key,
        Ok(None) => {
            state
                .bot
                .send_message(
                    msg.chat.id,
                    "No active session. Send a message to create one.",
                )
                .await?;
            return Ok(());
        }
        Err(e) => {
            state
                .bot
                .send_message(msg.chat.id, format!("Failed to resolve session: {e}"))
                .await?;
            return Ok(());
        }
    };

    let new_model = args.trim();

    if new_model.is_empty() {
        // Show current model.
        match state.http_client.get_session(&session_key).await {
            Ok(detail) => {
                let model = detail.model.as_deref().unwrap_or("(default)");
                state
                    .bot
                    .send_message(
                        msg.chat.id,
                        format!(
                            "Session <code>{}</code>\nModel: <b>{}</b>\n\nSwitch: <code>/model \
                             model-name</code>",
                            html_escape(&detail.key),
                            html_escape(model),
                        ),
                    )
                    .parse_mode(ParseMode::Html)
                    .await?;
            }
            Err(e) => {
                state
                    .bot
                    .send_message(msg.chat.id, format!("Failed to get session details: {e}"))
                    .await?;
            }
        }
    } else {
        // Switch model.
        match state
            .http_client
            .update_session(&session_key, Some(new_model))
            .await
        {
            Ok(detail) => {
                let model = detail.model.as_deref().unwrap_or("(default)");
                state
                    .bot
                    .send_message(
                        msg.chat.id,
                        format!(
                            "Model updated.\nSession <code>{}</code>\nModel: <b>{}</b>",
                            html_escape(&detail.key),
                            html_escape(model),
                        ),
                    )
                    .parse_mode(ParseMode::Html)
                    .await?;
            }
            Err(e) => {
                state
                    .bot
                    .send_message(msg.chat.id, format!("Failed to update model: {e}"))
                    .await?;
            }
        }
    }

    Ok(())
}

/// Handle `/mcp` — show MCP server connection status.
async fn handle_mcp(msg: &Message, state: &Arc<BotState>) -> Result<(), teloxide::RequestError> {
    use crate::http_client::McpServerStatus;

    match state.http_client.list_mcp_servers().await {
        Ok(servers) => {
            if servers.is_empty() {
                state
                    .bot
                    .send_message(msg.chat.id, "No MCP servers configured.")
                    .await?;
                return Ok(());
            }

            use std::fmt::Write;
            let mut text = format!("<b>MCP Servers</b> ({})\n\n", servers.len());
            for s in &servers {
                let (icon, status_text) = match &s.status {
                    McpServerStatus::Connected => ("●", "connected".to_owned()),
                    McpServerStatus::Disconnected => ("○", "disconnected".to_owned()),
                    McpServerStatus::Error { message } => {
                        ("✘", format!("error: {}", html_escape(message)))
                    }
                };
                let _ = writeln!(
                    text,
                    "{icon} <b>{}</b> — {status_text}",
                    html_escape(&s.name)
                );
            }

            let connected = servers
                .iter()
                .filter(|s| matches!(s.status, McpServerStatus::Connected))
                .count();
            let _ = write!(text, "\n{connected}/{} connected", servers.len());

            state
                .bot
                .send_message(msg.chat.id, text)
                .parse_mode(ParseMode::Html)
                .await?;
        }
        Err(e) => {
            state
                .bot
                .send_message(msg.chat.id, format!("Failed to fetch MCP status: {e}"))
                .await?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract the text body from a Telegram message, if present.
pub(crate) fn extract_text(msg: &Message) -> Option<&str> { msg.text() }

fn is_group_mention(msg: &Message, text: &str, bot_username: Option<&str>) -> bool {
    let Some(username) = bot_username else {
        return false;
    };
    let expected = username.to_lowercase();
    if let Some(entities) = msg.parse_entities() {
        for entity in entities {
            if matches!(entity.kind(), teloxide::types::MessageEntityKind::Mention) {
                let mention = entity.text().trim().trim_start_matches('@').to_lowercase();
                if mention == expected {
                    return true;
                }
            }
        }
    }

    let mention = format!("@{expected}");
    text.to_lowercase().contains(&mention)
}

fn contains_rara_keyword(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("rara")
        || lower.contains("らら")
        || lower.contains("ララ")
        || lower.contains("拉拉")
}

fn should_bubble_in_group(msg: &Message, text: &str, bot_username: Option<&str>) -> bool {
    is_group_mention(msg, text, bot_username) || contains_rara_keyword(text)
}

fn strip_group_mention(text: &str, bot_username: Option<&str>) -> String {
    let Some(username) = bot_username else {
        return text.trim().to_owned();
    };
    let mention = format!("@{username}");
    text.replace(&mention, "").trim().to_owned()
}

fn mention_sender(msg: &Message) -> String {
    let Some(sender) = msg.from.as_ref() else {
        return String::new();
    };
    if let Some(username) = &sender.username {
        return format!("@{username}");
    }
    sender.first_name.clone()
}

/// Returns `true` if the message contains any media attachment (photo, video,
/// document, audio, or voice).
#[allow(dead_code)]
pub(crate) fn has_media(msg: &Message) -> bool {
    msg.photo().is_some()
        || msg.video().is_some()
        || msg.document().is_some()
        || msg.audio().is_some()
        || msg.voice().is_some()
}

/// Classify the chat type as `"private"` or `"group_or_channel"` for
/// logging and routing purposes.
#[allow(dead_code)]
pub(crate) fn classify_chat(msg: &Message) -> &'static str {
    match &msg.chat.kind {
        teloxide::types::ChatKind::Private(..) => "private",
        teloxide::types::ChatKind::Public(..) => "group_or_channel",
    }
}

/// Build an inline keyboard with a single "Load More" button.
///
/// The callback data encodes the current result count and search parameters
/// so the handler can fetch the next page without storing server-side state.
fn load_more_keyboard(
    current_size: usize,
    encoded_params: &str,
) -> teloxide::types::InlineKeyboardMarkup {
    let callback_data = format!("search_more:{current_size}:{encoded_params}");
    teloxide::types::InlineKeyboardMarkup::new(vec![vec![
        teloxide::types::InlineKeyboardButton::callback("Load More", callback_data),
    ]])
}

/// Format a list of discovered jobs as an HTML message for Telegram.
///
/// Each job is numbered and includes title, company, location, salary range,
/// and URL (when available).
pub(crate) fn format_job_results(
    jobs: &[DiscoveryJobResponse],
    keywords: &[String],
    location: Option<&str>,
) -> String {
    use std::fmt::Write;

    let location_display = location.unwrap_or("any");
    let kw_escaped = html_escape(keywords.join(" "));
    let loc_escaped = html_escape(location_display);
    let mut text = format!(
        "Found <b>{}</b> jobs for <i>{kw_escaped}</i> @ <i>{loc_escaped}</i>:\n\n",
        jobs.len(),
    );

    for (i, job) in jobs.iter().enumerate() {
        let title = html_escape(&job.title);
        let company = html_escape(&job.company);
        let _ = write!(text, "<b>{}.</b> {title} - {company}\n", i + 1);
        if let Some(loc) = &job.location {
            let _ = write!(text, "   {}", html_escape(loc));
        }
        if let (Some(min), Some(max)) = (job.salary_min, job.salary_max) {
            let currency = job.salary_currency.as_deref().unwrap_or("");
            let _ = write!(text, " | {min}-{max} {currency}");
        }
        text.push('\n');
        if let Some(url) = &job.url {
            let _ = write!(text, "   {url}\n");
        }
        text.push('\n');
    }

    text
}

/// Escape `&`, `<`, `>` for safe inclusion in Telegram HTML messages.
pub(crate) fn html_escape(s: impl AsRef<str>) -> String {
    s.as_ref()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Encode search parameters into a compact string for callback data.
///
/// Format: `keyword1+keyword2@location` (location part omitted if `None`).
/// This is stored in the inline keyboard callback data to enable stateless
/// pagination.
pub(crate) fn encode_search_params(keywords: &[String], location: Option<&str>) -> String {
    let kw = keywords.join("+");
    match location {
        Some(loc) => format!("{kw}@{loc}"),
        None => kw,
    }
}

/// Decode search parameters from the compact callback data string.
///
/// Inverse of [`encode_search_params`].
pub(crate) fn decode_search_params(encoded: &str) -> (Vec<String>, Option<String>) {
    if let Some(idx) = encoded.find('@') {
        let kw = encoded[..idx].split('+').map(String::from).collect();
        let loc = encoded[idx + 1..].to_owned();
        (kw, Some(loc))
    } else {
        let kw = encoded.split('+').map(String::from).collect();
        (kw, None)
    }
}

/// Truncate a string to at most `max_len` characters, appending "..." if
/// truncation occurs. Works on char boundaries to avoid panics on
/// multi-byte strings.
fn truncate_str(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        return s;
    }
    // Find the last char boundary at or before max_len - 3 (room for "...").
    let limit = max_len.saturating_sub(3);
    let end = s
        .char_indices()
        .take_while(|(i, _)| *i <= limit)
        .last()
        .map_or(0, |(i, c)| i + c.len_utf8());
    &s[..end]
}

/// Format an ISO-8601 / RFC-3339 timestamp string into a short
/// human-readable date-time. Falls back to the raw string on parse failure.
fn format_timestamp(raw: &str) -> String {
    // Try to parse as RFC-3339 (e.g. "2026-02-14T10:30:00Z").
    if raw.len() >= 16 {
        // Return "YYYY-MM-DD HH:MM" for brevity.
        let date_part = &raw[..10];
        let time_part = if raw.len() >= 16 { &raw[11..16] } else { "" };
        if !time_part.is_empty() {
            return format!("{date_part} {time_part}");
        }
    }
    raw.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_search_params_no_location() {
        let keywords = vec!["rust".to_string(), "engineer".to_string()];
        let encoded = encode_search_params(&keywords, None);
        assert_eq!(encoded, "rust+engineer");

        let (decoded_kw, decoded_loc) = decode_search_params(&encoded);
        assert_eq!(decoded_kw, keywords);
        assert!(decoded_loc.is_none());
    }

    #[test]
    fn test_encode_decode_search_params_with_location() {
        let keywords = vec!["rust".to_string(), "engineer".to_string()];
        let encoded = encode_search_params(&keywords, Some("beijing"));
        assert_eq!(encoded, "rust+engineer@beijing");

        let (decoded_kw, decoded_loc) = decode_search_params(&encoded);
        assert_eq!(decoded_kw, keywords);
        assert_eq!(decoded_loc, Some("beijing".to_string()));
    }

    #[test]
    fn test_html_escape() {
        assert_eq!(html_escape("a < b & c > d"), "a &lt; b &amp; c &gt; d");
        assert_eq!(html_escape("normal text"), "normal text");
    }

    #[test]
    fn test_format_job_results_empty() {
        let text = format_job_results(&[], &["rust".to_string()], Some("remote"));
        assert!(text.contains("Found <b>0</b> jobs"));
    }

    #[test]
    fn test_truncate_str_short() {
        assert_eq!(truncate_str("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_str_exact() {
        assert_eq!(truncate_str("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_str_long() {
        let result = truncate_str("hello world, this is long", 10);
        // max_len=10, limit=7, last char at or before index 7 is 'o' (index 7),
        // so end = 8, result is "hello wo".
        assert!(result.len() <= 10);
        assert_eq!(result, "hello wo");
    }

    #[test]
    fn test_format_timestamp_rfc3339() {
        assert_eq!(format_timestamp("2026-02-14T10:30:00Z"), "2026-02-14 10:30");
    }

    #[test]
    fn test_format_timestamp_with_offset() {
        assert_eq!(
            format_timestamp("2026-02-14T10:30:45+08:00"),
            "2026-02-14 10:30"
        );
    }

    #[test]
    fn test_format_timestamp_short_fallback() {
        assert_eq!(format_timestamp("2026-02"), "2026-02");
    }
}
