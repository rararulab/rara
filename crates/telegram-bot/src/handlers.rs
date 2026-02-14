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
//! All user-facing operations are gated by the **primary chat ID** check
//! ([`BotState::is_primary_chat`]). Messages from unauthorized chats receive
//! a rejection reply.

use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use teloxide::{
    net::Download,
    payloads::{EditMessageTextSetters, SendMessageSetters},
    requests::Requester,
    types::{CallbackQuery, ChatAction, Message, ParseMode, PhotoSize, Update, UpdateKind},
    utils::command::BotCommands,
};
use tracing::{info, warn};

use crate::{
    command::Command,
    http_client::DiscoveryJobResponse,
    markdown::{TELEGRAM_MAX_MESSAGE_LEN, chunk_message, markdown_to_telegram_html},
    state::BotState,
};

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
    // Photo messages — download and forward to chat as multimodal content.
    if let Some(photos) = msg.photo() {
        let caption = msg.caption().unwrap_or("");
        return handle_photo_message(&msg, photos, caption, state).await;
    }

    let Some(text) = extract_text(&msg) else {
        return Ok(());
    };

    // Try parsing as a bot command first.
    if let Ok(cmd) = Command::parse(text, "") {
        return handle_command(msg, cmd, state).await;
    }

    // Unknown slash commands.
    if text.trim_start().starts_with('/') {
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
    handle_chat_message(&msg, text, state).await
}

/// Handle callback queries (inline keyboard button presses).
pub(crate) async fn handle_callback_query(
    q: CallbackQuery,
    state: &Arc<BotState>,
) -> Result<(), teloxide::RequestError> {
    // Always ack callback query so Telegram client stops spinner quickly.
    state.bot.answer_callback_query(&q.id).await?;

    let Some(data) = q.data.as_deref() else {
        return Ok(());
    };
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
                    "Welcome! I'm the Job Assistant bot.\n\
                     Send me any message to start a conversation.\n\n\
                     Commands:\n\
                     /search <keywords> [@ location] - Search jobs\n\
                     /jd <text> - Parse a Job Description\n\
                     /new - Start a new chat session\n\
                     /clear - Clear current session history\n\
                     /help - Show all commands",
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
    }
    Ok(())
}

/// Route plain text messages to the chat session system.
///
/// Resolves (or auto-creates) a channel binding for this Telegram chat,
/// sends the user message to the chat service, and relays the AI response
/// back to the user with Markdown formatting.
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

    // Send typing indicator while waiting for LLM response.
    let _ = state
        .bot
        .send_chat_action(msg.chat.id, ChatAction::Typing)
        .await;

    // Send to chat service and relay response.
    match state
        .http_client
        .send_chat_message(&session_key, text, vec![])
        .await
    {
        Ok(response) => {
            let reply_text = response.message.text_content();
            if !reply_text.is_empty() {
                let html = markdown_to_telegram_html(&reply_text);
                let chunks = chunk_message(&html, TELEGRAM_MAX_MESSAGE_LEN);
                for chunk in chunks {
                    state
                        .bot
                        .send_message(msg.chat.id, chunk)
                        .parse_mode(ParseMode::Html)
                        .await?;
                }
            }
        }
        Err(e) => {
            state
                .bot
                .send_message(msg.chat.id, format!("Chat error: {e}"))
                .await?;
        }
    }

    Ok(())
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

    // Send typing indicator — download + LLM inference may take a while.
    let _ = state
        .bot
        .send_chat_action(msg.chat.id, ChatAction::Typing)
        .await;

    // Select the highest-resolution photo (last element in the array).
    let photo = match photos.last() {
        Some(p) => p,
        None => {
            state
                .bot
                .send_message(msg.chat.id, "Could not read photo data.")
                .await?;
            return Ok(());
        }
    };

    // Download the photo via the Bot API.
    let file = match state.bot.get_file(&photo.file.id).await {
        Ok(f) => f,
        Err(e) => {
            warn!(error = %e, "failed to get file info for photo");
            state
                .bot
                .send_message(
                    msg.chat.id,
                    format!("Failed to download photo: {e}"),
                )
                .await?;
            return Ok(());
        }
    };

    let mut buf = Vec::new();
    if let Err(e) = state.bot.download_file(&file.path, &mut buf).await {
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
    let text = if caption.is_empty() {
        "What do you see in this image?"
    } else {
        caption
    };

    // Send to chat service with multimodal content.
    match state
        .http_client
        .send_chat_message(&session_key, text, vec![data_url])
        .await
    {
        Ok(response) => {
            let reply_text = response.message.text_content();
            if !reply_text.is_empty() {
                let html = markdown_to_telegram_html(&reply_text);
                let chunks = chunk_message(&html, TELEGRAM_MAX_MESSAGE_LEN);
                for chunk in chunks {
                    state
                        .bot
                        .send_message(msg.chat.id, chunk)
                        .parse_mode(ParseMode::Html)
                        .await?;
                }
            }
        }
        Err(e) => {
            state
                .bot
                .send_message(msg.chat.id, format!("Chat error: {e}"))
                .await?;
        }
    }

    Ok(())
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
            .send_message(
                msg.chat.id,
                "Usage: /jd <paste job description text>",
            )
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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract the text body from a Telegram message, if present.
pub(crate) fn extract_text(msg: &Message) -> Option<&str> {
    msg.text()
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
        assert_eq!(
            html_escape("a < b & c > d"),
            "a &lt; b &amp; c &gt; d"
        );
        assert_eq!(html_escape("normal text"), "normal text");
    }

    #[test]
    fn test_format_job_results_empty() {
        let text = format_job_results(&[], &["rust".to_string()], Some("remote"));
        assert!(text.contains("Found <b>0</b> jobs"));
    }
}
