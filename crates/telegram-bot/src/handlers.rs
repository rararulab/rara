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

//! Message and callback query handlers extracted from the old runtime module.

use std::sync::Arc;

use teloxide::{
    payloads::{EditMessageTextSetters, SendMessageSetters},
    requests::Requester,
    types::{CallbackQuery, Message, Update, UpdateKind},
    utils::command::BotCommands,
};
use tracing::warn;

use crate::{
    command::Command,
    http_client::DiscoveryJobResponse,
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
/// otherwise treats it as JD text for parse submission.
pub(crate) async fn handle_message_direct(
    msg: Message,
    state: &Arc<BotState>,
) -> Result<(), teloxide::RequestError> {
    let Some(text) = extract_text(&msg) else {
        return Ok(());
    };

    // Try parsing as a bot command first.
    if let Ok(cmd) = Command::parse(text, "") {
        return handle_command(msg, cmd, state).await;
    }

    // Gate non-command messages to primary chat only.
    if !state.is_primary_chat(msg.chat.id) {
        warn!(
            chat_id = msg.chat.id.0,
            "ignoring unauthorized telegram chat"
        );
        state
            .bot
            .send_message(msg.chat.id, "Unauthorized chat.")
            .await?;
        return Ok(());
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

    // Plain text -> treat as JD for parse.
    state
        .bot
        .send_message(msg.chat.id, "Received your JD, processing...")
        .await?;

    if let Err(e) = state.http_client.submit_jd_parse(text).await {
        state
            .bot
            .send_message(msg.chat.id, format!("JD parse submit failed: {e}"))
            .await?;
    }

    Ok(())
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
                     \u{2022} Send me a JD text and I'll parse it\n\
                     \u{2022} Use /search <keywords> [@ location] to find jobs\n\
                     \u{2022} Use /help to see all commands",
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
    }
    Ok(())
}

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

/// Extract text content from a Telegram message.
pub(crate) fn extract_text(msg: &Message) -> Option<&str> {
    msg.text()
}

/// Check if the message contains media (photo, video, document, etc.).
#[allow(dead_code)]
pub(crate) fn has_media(msg: &Message) -> bool {
    msg.photo().is_some()
        || msg.video().is_some()
        || msg.document().is_some()
        || msg.audio().is_some()
        || msg.voice().is_some()
}

/// Classify the chat type for logging/routing.
#[allow(dead_code)]
pub(crate) fn classify_chat(msg: &Message) -> &'static str {
    match &msg.chat.kind {
        teloxide::types::ChatKind::Private(..) => "private",
        teloxide::types::ChatKind::Public(..) => "group_or_channel",
    }
}

fn load_more_keyboard(
    current_size: usize,
    encoded_params: &str,
) -> teloxide::types::InlineKeyboardMarkup {
    let callback_data = format!("search_more:{current_size}:{encoded_params}");
    teloxide::types::InlineKeyboardMarkup::new(vec![vec![
        teloxide::types::InlineKeyboardButton::callback("Load More", callback_data),
    ]])
}

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

pub(crate) fn html_escape(s: impl AsRef<str>) -> String {
    s.as_ref()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub(crate) fn encode_search_params(keywords: &[String], location: Option<&str>) -> String {
    let kw = keywords.join("+");
    match location {
        Some(loc) => format!("{kw}@{loc}"),
        None => kw,
    }
}

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
