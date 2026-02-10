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

//! Telegram bot worker — receives messages, delegates JD parsing, and
//! handles `/search` commands for job discovery.

use std::sync::Arc;

use job_common_worker::{Notifiable, NotifyHandle};
use job_domain_job_source::service::JobSourceService;
use job_domain_shared::telegram_service::TelegramService;
use job_server::ServiceHandler;
use teloxide::{prelude::*, types::CallbackQuery, utils::command::BotCommands};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::types::JdParseRequest;

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Available commands:")]
enum Command {
    #[command(description = "Start the bot")]
    Start,
    #[command(description = "Show help")]
    Help,
    #[command(description = "Search jobs: /search <keywords> [@ location]")]
    Search(String),
}

/// Start the Telegram bot as a standalone service.
///
/// Returns a [`ServiceHandler`] for lifecycle management (shutdown, wait).
/// The bot runs a teloxide long-polling dispatcher in a spawned task.
pub fn start_telegram_bot(
    telegram: Arc<TelegramService>,
    jd_tx: mpsc::Sender<JdParseRequest>,
    jd_notify: Arc<tokio::sync::Mutex<Option<NotifyHandle>>>,
    job_source_service: Arc<JobSourceService>,
) -> ServiceHandler {
    let cancel = CancellationToken::new();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();

    let cancel_clone = cancel.clone();
    let join_handle = tokio::spawn(async move {
        let bot = telegram.bot();

        let handler = dptree::entry()
            .branch(
                Update::filter_message()
                    .branch(
                        dptree::entry()
                            .filter_command::<Command>()
                            .endpoint(handle_command),
                    )
                    .branch(dptree::entry().endpoint(handle_message)),
            )
            .branch(Update::filter_callback_query().endpoint(handle_callback_query));

        // Drop any pending updates accumulated while the bot was offline,
        // so we don't replay stale messages on every restart.
        let _ = bot.delete_webhook().drop_pending_updates(true).await;

        let mut dispatcher = Dispatcher::builder(bot, handler)
            .dependencies(dptree::deps![
                jd_tx,
                jd_notify,
                telegram,
                job_source_service
            ])
            .build();

        // Signal that the bot is ready
        let _ = started_tx.send(());

        // Graceful shutdown: when the cancellation token fires,
        // shut down the teloxide dispatcher.
        let shutdown_token = dispatcher.shutdown_token();
        tokio::spawn(async move {
            cancel_clone.cancelled().await;
            if let Ok(f) = shutdown_token.shutdown() {
                f.await;
            }
        });

        dispatcher.dispatch().await;
    });

    ServiceHandler::new(join_handle, cancel, started_rx)
}

async fn handle_command(
    bot: Bot,
    msg: Message,
    cmd: Command,
    telegram: Arc<TelegramService>,
    job_source_service: Arc<JobSourceService>,
) -> ResponseResult<()> {
    match cmd {
        Command::Start => {
            bot.send_message(
                msg.chat.id,
                "Welcome! I'm the Job Assistant bot.\n\u{2022} Send me a JD text and I'll parse \
                 it\n\u{2022} Use /search <keywords> [@ location] to find jobs\n\u{2022} Use \
                 /help to see all commands",
            )
            .await?;
        }
        Command::Help => {
            bot.send_message(msg.chat.id, Command::descriptions().to_string())
                .await?;
        }
        Command::Search(args) => {
            handle_search(bot, msg, args, telegram, job_source_service).await?;
        }
    }
    Ok(())
}

/// Parse `/search <keywords> [@ location]` and call JobSourceService.
///
/// Format: `/search rust engineer @ beijing`
/// Or simply: `/search rust engineer` (no location)
async fn handle_search(
    bot: Bot,
    msg: Message,
    args: String,
    telegram: Arc<TelegramService>,
    job_source_service: Arc<JobSourceService>,
) -> ResponseResult<()> {
    if !telegram.is_primary_chat(msg.chat.id) {
        bot.send_message(msg.chat.id, "Unauthorized chat.").await?;
        return Ok(());
    }

    let args = args.trim();
    if args.is_empty() {
        bot.send_message(
            msg.chat.id,
            "Usage: /search <keywords> [@ location]\nExample: /search rust engineer @ beijing",
        )
        .await?;
        return Ok(());
    }

    // Split on " @ " to separate keywords from location
    let (keywords_str, location) = if let Some(idx) = args.find(" @ ") {
        (&args[..idx], Some(args[idx + 3..].trim().to_owned()))
    } else {
        (args, None)
    };

    let keywords: Vec<String> = keywords_str.split_whitespace().map(String::from).collect();

    if keywords.is_empty() {
        bot.send_message(msg.chat.id, "Please provide at least one keyword.")
            .await?;
        return Ok(());
    }

    let location_display = location.as_deref().unwrap_or("any");
    bot.send_message(
        msg.chat.id,
        format!(
            "\u{1f50d} Searching: {} @ {} ...",
            keywords.join(" "),
            location_display
        ),
    )
    .await?;

    let max_results: u32 = 3;
    let criteria = job_domain_job_source::types::DiscoveryCriteria {
        keywords: keywords.clone(),
        location: location.clone(),
        max_results: Some(max_results),
        ..Default::default()
    };

    let svc = job_source_service.clone();
    let result = tokio::task::spawn_blocking(move || {
        let empty_source = std::collections::HashSet::new();
        let empty_fuzzy = std::collections::HashSet::new();
        svc.discover(&criteria, &empty_source, &empty_fuzzy)
    })
    .await;

    let discovery = match result {
        Ok(d) => d,
        Err(e) => {
            bot.send_message(msg.chat.id, format!("\u{274c} Search failed: {e}"))
                .await?;
            return Ok(());
        }
    };

    if let Some(ref err) = discovery.error {
        bot.send_message(msg.chat.id, format!("\u{274c} Search error: {err}"))
            .await?;
        return Ok(());
    }

    if discovery.jobs.is_empty() {
        bot.send_message(msg.chat.id, "No jobs found matching your criteria.")
            .await?;
        return Ok(());
    }

    let text = format_job_results(&discovery.jobs, &keywords, location.as_deref());

    // Build inline keyboard with "Load More" button.
    // Encode search params in callback data:
    // "search_more:<offset>:<keywords>[@<location>]"
    let callback_data = format!(
        "search_more:{}:{}",
        discovery.jobs.len(),
        encode_search_params(&keywords, location.as_deref()),
    );
    let keyboard = teloxide::types::InlineKeyboardMarkup::new(vec![vec![
        teloxide::types::InlineKeyboardButton::callback("\u{1f4c4} Load More", callback_data),
    ]]);

    bot.send_message(msg.chat.id, text)
        .parse_mode(teloxide::types::ParseMode::Html)
        .reply_markup(keyboard)
        .await?;

    Ok(())
}

/// Handle inline keyboard callback queries (e.g. "Load More" button).
async fn handle_callback_query(
    bot: Bot,
    q: CallbackQuery,
    telegram: Arc<TelegramService>,
    job_source_service: Arc<JobSourceService>,
) -> ResponseResult<()> {
    // Acknowledge the callback to remove the "loading" spinner.
    bot.answer_callback_query(&q.id).await?;

    let data = match q.data.as_deref() {
        Some(d) => d,
        None => return Ok(()),
    };

    if !data.starts_with("search_more:") {
        return Ok(());
    }

    // Check authorization via the message's chat
    if let Some(ref msg) = q.message {
        if !telegram.is_primary_chat(msg.chat().id) {
            return Ok(());
        }
    }

    // Parse callback data: "search_more:<current_count>:<encoded_params>"
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
    let criteria = job_domain_job_source::types::DiscoveryCriteria {
        keywords: keywords.clone(),
        location: location.clone(),
        max_results: Some(new_max),
        ..Default::default()
    };

    let svc = job_source_service.clone();
    let result = tokio::task::spawn_blocking(move || {
        let empty_source = std::collections::HashSet::new();
        let empty_fuzzy = std::collections::HashSet::new();
        svc.discover(&criteria, &empty_source, &empty_fuzzy)
    })
    .await;

    let discovery = match result {
        Ok(d) => d,
        Err(e) => {
            if let Some(ref msg) = q.message {
                let chat_id = msg.chat().id;
                bot.send_message(chat_id, format!("\u{274c} Load more failed: {e}"))
                    .await?;
            }
            return Ok(());
        }
    };

    if let Some(ref err) = discovery.error {
        if let Some(ref msg) = q.message {
            let chat_id = msg.chat().id;
            bot.send_message(chat_id, format!("\u{274c} Error: {err}"))
                .await?;
        }
        return Ok(());
    }

    let text = format_job_results(&discovery.jobs, &keywords, location.as_deref());

    // Update the inline keyboard — if we got fewer results than requested,
    // there are no more results, so remove the button.
    if let Some(ref msg) = q.message {
        let msg_id = msg.id();
        let chat_id = msg.chat().id;

        if (discovery.jobs.len() as u32) < new_max {
            // No more results — remove the keyboard
            bot.edit_message_text(chat_id, msg_id, text)
                .parse_mode(teloxide::types::ParseMode::Html)
                .await?;
        } else {
            // Still more — update the button with new offset
            let callback_data = format!(
                "search_more:{}:{}",
                discovery.jobs.len(),
                encode_search_params(&keywords, location.as_deref()),
            );
            let keyboard = teloxide::types::InlineKeyboardMarkup::new(vec![vec![
                teloxide::types::InlineKeyboardButton::callback(
                    "\u{1f4c4} Load More",
                    callback_data,
                ),
            ]]);
            bot.edit_message_text(chat_id, msg_id, text)
                .parse_mode(teloxide::types::ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
    }

    Ok(())
}

async fn handle_message(
    bot: Bot,
    msg: Message,
    jd_tx: mpsc::Sender<JdParseRequest>,
    jd_notify: std::sync::Arc<tokio::sync::Mutex<Option<job_common_worker::NotifyHandle>>>,
    telegram: std::sync::Arc<TelegramService>,
) -> ResponseResult<()> {
    if let Some(text) = msg.text() {
        if !telegram.is_primary_chat(msg.chat.id) {
            warn!(
                chat_id = msg.chat.id.0,
                "ignoring message from unauthorized Telegram chat"
            );
            bot.send_message(msg.chat.id, "Unauthorized chat.").await?;
            return Ok(());
        }

        bot.send_message(msg.chat.id, "Received your JD, processing...")
            .await?;

        let send_result = jd_tx
            .send(JdParseRequest {
                text: text.to_string(),
            })
            .await;
        if send_result.is_ok() {
            if let Some(handle) = jd_notify.lock().await.as_ref() {
                handle.notify();
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Format job results into a Telegram message (HTML parse mode).
fn format_job_results(
    jobs: &[job_domain_job_source::types::NormalizedJob],
    keywords: &[String],
    location: Option<&str>,
) -> String {
    let location_display = location.unwrap_or("any");
    let mut text = format!(
        "Found <b>{}</b> jobs for <i>{}</i> @ <i>{}</i>:\n\n",
        jobs.len(),
        html_escape(keywords.join(" ")),
        html_escape(location_display),
    );

    for (i, job) in jobs.iter().enumerate() {
        text.push_str(&format!(
            "<b>{}.</b> {} \u{2014} {}\n",
            i + 1,
            html_escape(&job.title),
            html_escape(&job.company),
        ));
        if let Some(loc) = &job.location {
            text.push_str(&format!("   \u{1f4cd} {}", html_escape(loc)));
        }
        if let (Some(min), Some(max)) = (job.salary_min, job.salary_max) {
            let currency = job.salary_currency.as_deref().unwrap_or("");
            text.push_str(&format!(" | \u{1f4b0} {}-{} {}", min, max, currency));
        }
        text.push('\n');
        if let Some(url) = &job.url {
            text.push_str(&format!("   \u{1f517} {}\n", url));
        }
        text.push('\n');
    }

    text
}

/// Escape HTML special characters for Telegram HTML parse mode.
fn html_escape(s: impl AsRef<str>) -> String {
    s.as_ref()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Encode search params into a compact callback data string.
/// Format: "kw1+kw2[@location]"
fn encode_search_params(keywords: &[String], location: Option<&str>) -> String {
    let kw = keywords.join("+");
    match location {
        Some(loc) => format!("{}@{}", kw, loc),
        None => kw,
    }
}

/// Decode search params from callback data.
/// Returns (keywords, optional location).
fn decode_search_params(encoded: &str) -> (Vec<String>, Option<String>) {
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
        assert_eq!(html_escape("a < b & c > d"), "a &lt; b &amp; c &gt; d");
        assert_eq!(html_escape("normal text"), "normal text");
    }

    #[test]
    fn test_format_job_results_empty() {
        let text = format_job_results(&[], &["rust".to_string()], Some("remote"));
        assert!(text.contains("Found <b>0</b> jobs"));
    }
}
