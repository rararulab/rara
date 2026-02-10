// Copyright 2026 Crrow
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

mod grpc_command;
mod http_client;

use std::sync::Arc;

use grpc_command::TelegramBotCommandGrpcService;
use http_client::{DiscoveryJobResponse, MainServiceHttpClient};
use job_domain_shared::telegram_service::TelegramService;
use job_server::{
    ServiceHandler,
    grpc::{GrpcServerConfig, start_grpc_server},
};
use smart_default::SmartDefault;
use snafu::{ResultExt, Whatever, whatever};
use teloxide::{prelude::*, types::CallbackQuery, utils::command::BotCommands};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

#[derive(Debug, Clone)]
pub struct TelegramConfig {
    pub bot_token: String,
    pub chat_id:   i64,
}

#[derive(Debug, Clone, SmartDefault)]
pub struct BotConfig {
    pub telegram:               Option<TelegramConfig>,
    #[default(_code = "\"http://127.0.0.1:3000\".to_owned()")]
    pub main_service_http_base: String,
    pub grpc_config:            GrpcServerConfig,
}

impl BotConfig {
    pub fn from_env() -> Self {
        let telegram = match (
            std::env::var("TELEGRAM_BOT_TOKEN"),
            std::env::var("TELEGRAM_CHAT_ID"),
        ) {
            (Ok(token), Ok(chat_id)) => {
                let chat_id: i64 = chat_id
                    .parse()
                    .expect("TELEGRAM_CHAT_ID must be an integer");
                Some(TelegramConfig {
                    bot_token: token,
                    chat_id,
                })
            }
            _ => None,
        };

        let main_service_http_base = std::env::var("MAIN_SERVICE_HTTP_BASE")
            .unwrap_or_else(|_| "http://127.0.0.1:3000".to_owned());

        let grpc_bind = std::env::var("TELEGRAM_BOT_GRPC_BIND")
            .unwrap_or_else(|_| "127.0.0.1:50061".to_owned());

        let grpc_config = GrpcServerConfig {
            bind_address: grpc_bind.clone(),
            server_address: grpc_bind,
            ..GrpcServerConfig::default()
        };

        Self {
            telegram,
            main_service_http_base,
            grpc_config,
        }
    }

    pub async fn open(self) -> Result<BotApp, Whatever> {
        let telegram_cfg = match self.telegram.as_ref() {
            Some(cfg) => cfg,
            None => {
                whatever!("Telegram is required: set TELEGRAM_BOT_TOKEN and TELEGRAM_CHAT_ID")
            }
        };

        let telegram = Arc::new(TelegramService::new(
            teloxide::Bot::new(&telegram_cfg.bot_token),
            telegram_cfg.chat_id,
        ));

        let main_http = Arc::new(MainServiceHttpClient::new(
            self.main_service_http_base.clone(),
        ));

        Ok(BotApp {
            config: self,
            telegram,
            main_http,
            cancellation_token: CancellationToken::new(),
        })
    }
}

pub struct BotApp {
    config:             BotConfig,
    telegram:           Arc<TelegramService>,
    main_http:          Arc<MainServiceHttpClient>,
    cancellation_token: CancellationToken,
}

impl BotApp {
    pub async fn run(self) -> Result<(), Whatever> {
        let mut grpc_handle = start_grpc_server(
            &self.config.grpc_config,
            &[Arc::new(TelegramBotCommandGrpcService::new(
                self.telegram.clone(),
            ))],
        )
        .whatever_context("failed to start telegram-bot gRPC command service")?;
        grpc_handle
            .wait_for_start()
            .await
            .whatever_context("telegram-bot gRPC service failed to start")?;

        let mut telegram_handle = start_telegram_dispatcher(self.telegram, self.main_http);
        telegram_handle
            .wait_for_start()
            .await
            .whatever_context("telegram-bot dispatcher failed to start")?;

        let cancel = self.cancellation_token.clone();
        tokio::spawn(async move {
            tokio::signal::ctrl_c()
                .await
                .expect("failed to install Ctrl+C handler");
            cancel.cancel();
        });

        self.cancellation_token.cancelled().await;
        info!("telegram-bot shutdown requested");

        grpc_handle.shutdown();
        telegram_handle.shutdown();

        grpc_handle
            .wait_for_stop()
            .await
            .whatever_context("failed to stop telegram-bot grpc service")?;
        telegram_handle
            .wait_for_stop()
            .await
            .whatever_context("failed to stop telegram dispatcher")?;

        Ok(())
    }
}

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

fn start_telegram_dispatcher(
    telegram: Arc<TelegramService>,
    main_http: Arc<MainServiceHttpClient>,
) -> ServiceHandler {
    let cancel = CancellationToken::new();
    let (started_tx, started_rx) = oneshot::channel();

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

        let _ = bot.delete_webhook().drop_pending_updates(true).await;

        let mut dispatcher = Dispatcher::builder(bot, handler)
            .dependencies(dptree::deps![telegram, main_http])
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

async fn handle_command(
    bot: Bot,
    msg: Message,
    cmd: Command,
    telegram: Arc<TelegramService>,
    main_http: Arc<MainServiceHttpClient>,
) -> ResponseResult<()> {
    match cmd {
        Command::Start => {
            bot.send_message(
                msg.chat.id,
                "Welcome! I'm the Job Assistant bot.\n• Send me a JD text and I'll parse it\n• \
                 Use /search <keywords> [@ location] to find jobs\n• Use /help to see all commands",
            )
            .await?;
        }
        Command::Help => {
            bot.send_message(msg.chat.id, Command::descriptions().to_string())
                .await?;
        }
        Command::Search(args) => {
            handle_search(bot, msg, args, telegram, main_http).await?;
        }
    }
    Ok(())
}

async fn handle_search(
    bot: Bot,
    msg: Message,
    args: String,
    telegram: Arc<TelegramService>,
    main_http: Arc<MainServiceHttpClient>,
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

    let jobs = match main_http
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

    let text = format_job_results(&jobs, &keywords, location.as_deref());
    let callback_data = format!(
        "search_more:{}:{}",
        jobs.len(),
        encode_search_params(&keywords, location.as_deref()),
    );
    let keyboard = teloxide::types::InlineKeyboardMarkup::new(vec![vec![
        teloxide::types::InlineKeyboardButton::callback("📄 Load More", callback_data),
    ]]);

    bot.send_message(msg.chat.id, text)
        .parse_mode(teloxide::types::ParseMode::Html)
        .reply_markup(keyboard)
        .await?;

    Ok(())
}

async fn handle_callback_query(
    bot: Bot,
    q: CallbackQuery,
    telegram: Arc<TelegramService>,
    main_http: Arc<MainServiceHttpClient>,
) -> ResponseResult<()> {
    bot.answer_callback_query(&q.id).await?;

    let data = match q.data.as_deref() {
        Some(d) => d,
        None => return Ok(()),
    };
    if !data.starts_with("search_more:") {
        return Ok(());
    }

    if let Some(ref msg) = q.message {
        if !telegram.is_primary_chat(msg.chat().id) {
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

    let jobs = match main_http
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

    let text = format_job_results(&jobs, &keywords, location.as_deref());

    if let Some(ref msg) = q.message {
        let msg_id = msg.id();
        let chat_id = msg.chat().id;

        if (jobs.len() as u32) < new_max {
            bot.edit_message_text(chat_id, msg_id, text)
                .parse_mode(teloxide::types::ParseMode::Html)
                .await?;
        } else {
            let callback_data = format!(
                "search_more:{}:{}",
                jobs.len(),
                encode_search_params(&keywords, location.as_deref()),
            );
            let keyboard = teloxide::types::InlineKeyboardMarkup::new(vec![vec![
                teloxide::types::InlineKeyboardButton::callback("📄 Load More", callback_data),
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
    telegram: Arc<TelegramService>,
    main_http: Arc<MainServiceHttpClient>,
) -> ResponseResult<()> {
    if let Some(text) = msg.text() {
        if !telegram.is_primary_chat(msg.chat.id) {
            warn!(
                chat_id = msg.chat.id.0,
                "ignoring unauthorized telegram chat"
            );
            bot.send_message(msg.chat.id, "Unauthorized chat.").await?;
            return Ok(());
        }

        bot.send_message(msg.chat.id, "Received your JD, processing...")
            .await?;

        if let Err(e) = main_http.submit_jd_parse(text).await {
            bot.send_message(msg.chat.id, format!("❌ JD parse submit failed: {e}"))
                .await?;
        }
    }

    Ok(())
}

fn format_job_results(
    jobs: &[DiscoveryJobResponse],
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
            "<b>{}.</b> {} - {}\n",
            i + 1,
            html_escape(&job.title),
            html_escape(&job.company),
        ));
        if let Some(loc) = &job.location {
            text.push_str(&format!("   📍 {}", html_escape(loc)));
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

fn html_escape(s: impl AsRef<str>) -> String {
    s.as_ref()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn encode_search_params(keywords: &[String], location: Option<&str>) -> String {
    let kw = keywords.join("+");
    match location {
        Some(loc) => format!("{}@{}", kw, loc),
        None => kw,
    }
}

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
