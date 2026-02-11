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

use std::sync::Arc;

use job_server::ServiceHandler;
use teloxide::{prelude::*, types::CallbackQuery, utils::command::BotCommands};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::{
    command::Command,
    http_client::{DiscoveryJobResponse, MainServiceHttpClient},
    telegram_service::TelegramService,
};

/// Core runtime object that owns bot domain behavior.
#[derive(Clone)]
pub(crate) struct TelegramBotRuntime {
    pub(crate) telegram: Arc<TelegramService>,
    /// Client for bot -> main-service HTTP calls.
    main_http:           Arc<MainServiceHttpClient>,
}

impl TelegramBotRuntime {
    pub(crate) fn new(
        telegram: Arc<TelegramService>,
        main_http: Arc<MainServiceHttpClient>,
    ) -> Self {
        Self {
            telegram,
            main_http,
        }
    }

    /// Start teloxide dispatcher loop and return lifecycle handle.
    pub(crate) fn start_dispatcher(self: &Arc<Self>) -> ServiceHandler {
        let cancel = CancellationToken::new();
        let (started_tx, started_rx) = oneshot::channel();

        let runtime = self.clone();
        let cancel_clone = cancel.clone();
        let join_handle = tokio::spawn(async move {
            let bot = runtime.telegram.bot();

            let handler = dptree::entry()
                // Telegram message flow:
                // 1) parse supported commands
                // 2) fallback to plain text message handler
                .branch(
                    Update::filter_message()
                        .branch(
                            dptree::entry()
                                .filter_command::<Command>()
                                .endpoint(endpoint_command),
                        )
                        .branch(dptree::entry().endpoint(endpoint_message)),
                )
                .branch(Update::filter_callback_query().endpoint(endpoint_callback_query));

            let _ = bot.delete_webhook().drop_pending_updates(true).await;

            let mut dispatcher = Dispatcher::builder(bot, handler)
                .dependencies(dptree::deps![runtime])
                .build();

            let _ = started_tx.send(());

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

    async fn handle_command(&self, bot: Bot, msg: Message, cmd: Command) -> ResponseResult<()> {
        match cmd {
            Command::Start => {
                bot.send_message(
                    msg.chat.id,
                    "Welcome! I'm the Job Assistant bot.\n• Send me a JD text and I'll parse \
                     it\n• Use /search <keywords> [@ location] to find jobs\n• Use /help to see \
                     all commands",
                )
                .await?;
            }
            Command::Help => {
                bot.send_message(msg.chat.id, Command::descriptions().to_string())
                    .await?;
            }
            Command::Search(args) => {
                self.handle_search(bot, msg, args).await?;
            }
        }
        Ok(())
    }

    async fn handle_search(&self, bot: Bot, msg: Message, args: String) -> ResponseResult<()> {
        // Hard gate by configured primary chat to avoid accidental public usage.
        if !self.telegram.is_primary_chat(msg.chat.id) {
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

        bot.send_message(
            msg.chat.id,
            format!(
                "🔍 Searching: {} @ {} ...",
                keywords.join(" "),
                location.as_deref().unwrap_or("any")
            ),
        )
        .await?;

        let jobs = match self
            .main_http
            .discover_jobs(keywords.clone(), location.clone(), 3)
            .await
        {
            Ok(jobs) => jobs,
            Err(e) => {
                bot.send_message(msg.chat.id, format!("❌ Search failed: {e}"))
                    .await?;
                return Ok(());
            }
        };

        if jobs.is_empty() {
            bot.send_message(msg.chat.id, "No jobs found matching your criteria.")
                .await?;
            return Ok(());
        }

        let text = Self::format_job_results(&jobs, &keywords, location.as_deref());
        let keyboard = Self::load_more_keyboard(
            jobs.len(),
            Self::encode_search_params(&keywords, location.as_deref()),
        );

        bot.send_message(msg.chat.id, text)
            .parse_mode(teloxide::types::ParseMode::Html)
            .reply_markup(keyboard)
            .await?;

        Ok(())
    }

    async fn handle_callback_query(&self, bot: Bot, q: CallbackQuery) -> ResponseResult<()> {
        // Always ack callback query so Telegram client stops spinner quickly.
        bot.answer_callback_query(&q.id).await?;

        let data = match q.data.as_deref() {
            Some(d) => d,
            None => return Ok(()),
        };
        if !data.starts_with("search_more:") {
            return Ok(());
        }

        if let Some(ref msg) = q.message {
            if !self.telegram.is_primary_chat(msg.chat().id) {
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

        let (keywords, location) = Self::decode_search_params(parts[2]);
        let new_max = current_count + 3;

        let jobs = match self
            .main_http
            .discover_jobs(keywords.clone(), location.clone(), new_max)
            .await
        {
            Ok(jobs) => jobs,
            Err(e) => {
                if let Some(ref msg) = q.message {
                    bot.send_message(msg.chat().id, format!("❌ Load more failed: {e}"))
                        .await?;
                }
                return Ok(());
            }
        };

        let text = Self::format_job_results(&jobs, &keywords, location.as_deref());

        if let Some(ref msg) = q.message {
            let msg_id = msg.id();
            let chat_id = msg.chat().id;

            if (jobs.len() as u32) < new_max {
                bot.edit_message_text(chat_id, msg_id, text)
                    .parse_mode(teloxide::types::ParseMode::Html)
                    .await?;
            } else {
                let keyboard = Self::load_more_keyboard(
                    jobs.len(),
                    Self::encode_search_params(&keywords, location.as_deref()),
                );
                bot.edit_message_text(chat_id, msg_id, text)
                    .parse_mode(teloxide::types::ParseMode::Html)
                    .reply_markup(keyboard)
                    .await?;
            }
        }

        Ok(())
    }

    async fn handle_message(&self, bot: Bot, msg: Message) -> ResponseResult<()> {
        if let Some(text) = msg.text() {
            if !self.telegram.is_primary_chat(msg.chat.id) {
                warn!(
                    chat_id = msg.chat.id.0,
                    "ignoring unauthorized telegram chat"
                );
                bot.send_message(msg.chat.id, "Unauthorized chat.").await?;
                return Ok(());
            }

            // Commands are handled by the command branch; if a slash-prefixed
            // unknown command falls through here, do not treat it as JD text.
            if text.trim_start().starts_with('/') {
                bot.send_message(
                    msg.chat.id,
                    "Unknown command. Use /help to see available commands.",
                )
                .await?;
                return Ok(());
            }

            bot.send_message(msg.chat.id, "Received your JD, processing...")
                .await?;

            if let Err(e) = self.main_http.submit_jd_parse(text).await {
                bot.send_message(msg.chat.id, format!("❌ JD parse submit failed: {e}"))
                    .await?;
            }
        }

        Ok(())
    }

    fn load_more_keyboard(
        current_size: usize,
        encoded_params: String,
    ) -> teloxide::types::InlineKeyboardMarkup {
        let callback_data = format!("search_more:{current_size}:{encoded_params}");
        teloxide::types::InlineKeyboardMarkup::new(vec![vec![
            teloxide::types::InlineKeyboardButton::callback("📄 Load More", callback_data),
        ]])
    }

    pub(crate) fn format_job_results(
        jobs: &[DiscoveryJobResponse],
        keywords: &[String],
        location: Option<&str>,
    ) -> String {
        let location_display = location.unwrap_or("any");
        let mut text = format!(
            "Found <b>{}</b> jobs for <i>{}</i> @ <i>{}</i>:\n\n",
            jobs.len(),
            Self::html_escape(keywords.join(" ")),
            Self::html_escape(location_display),
        );

        for (i, job) in jobs.iter().enumerate() {
            text.push_str(&format!(
                "<b>{}.</b> {} - {}\n",
                i + 1,
                Self::html_escape(&job.title),
                Self::html_escape(&job.company),
            ));
            if let Some(loc) = &job.location {
                text.push_str(&format!("   📍 {}", Self::html_escape(loc)));
            }
            if let (Some(min), Some(max)) = (job.salary_min, job.salary_max) {
                let currency = job.salary_currency.as_deref().unwrap_or("");
                text.push_str(&format!(" | 💰 {}-{} {}", min, max, currency));
            }
            text.push('\n');
            if let Some(url) = &job.url {
                text.push_str(&format!("   🔗 {}\n", url));
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
            Some(loc) => format!("{}@{}", kw, loc),
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
}

/// Teloxide endpoint adapter: command path.
async fn endpoint_command(
    bot: Bot,
    msg: Message,
    cmd: Command,
    runtime: Arc<TelegramBotRuntime>,
) -> ResponseResult<()> {
    runtime.handle_command(bot, msg, cmd).await
}

/// Teloxide endpoint adapter: callback query path.
async fn endpoint_callback_query(
    bot: Bot,
    q: CallbackQuery,
    runtime: Arc<TelegramBotRuntime>,
) -> ResponseResult<()> {
    runtime.handle_callback_query(bot, q).await
}

/// Teloxide endpoint adapter: plain text message path.
async fn endpoint_message(
    bot: Bot,
    msg: Message,
    runtime: Arc<TelegramBotRuntime>,
) -> ResponseResult<()> {
    runtime.handle_message(bot, msg).await
}

#[cfg(test)]
mod tests {
    use super::TelegramBotRuntime;

    #[test]
    fn test_encode_decode_search_params_no_location() {
        let keywords = vec!["rust".to_string(), "engineer".to_string()];
        let encoded = TelegramBotRuntime::encode_search_params(&keywords, None);
        assert_eq!(encoded, "rust+engineer");

        let (decoded_kw, decoded_loc) = TelegramBotRuntime::decode_search_params(&encoded);
        assert_eq!(decoded_kw, keywords);
        assert!(decoded_loc.is_none());
    }

    #[test]
    fn test_encode_decode_search_params_with_location() {
        let keywords = vec!["rust".to_string(), "engineer".to_string()];
        let encoded = TelegramBotRuntime::encode_search_params(&keywords, Some("beijing"));
        assert_eq!(encoded, "rust+engineer@beijing");

        let (decoded_kw, decoded_loc) = TelegramBotRuntime::decode_search_params(&encoded);
        assert_eq!(decoded_kw, keywords);
        assert_eq!(decoded_loc, Some("beijing".to_string()));
    }

    #[test]
    fn test_html_escape() {
        assert_eq!(
            TelegramBotRuntime::html_escape("a < b & c > d"),
            "a &lt; b &amp; c &gt; d"
        );
        assert_eq!(
            TelegramBotRuntime::html_escape("normal text"),
            "normal text"
        );
    }

    #[test]
    fn test_format_job_results_empty() {
        let text =
            TelegramBotRuntime::format_job_results(&[], &["rust".to_string()], Some("remote"));
        assert!(text.contains("Found <b>0</b> jobs"));
    }
}
