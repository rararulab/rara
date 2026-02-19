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

//! Configuration loading and dependency wiring.
//!
//! [`BotConfig`] gathers all external configuration (env vars, database, KV
//! store) and [`BotConfig::open`] assembles the fully-wired [`BotApp`]
//! ready to run.

use std::sync::Arc;

use rara_domain_shared::settings::SettingsSvc;
use smart_default::SmartDefault;
use snafu::{ResultExt, Whatever, whatever};
use tokio_util::sync::CancellationToken;
use yunara_store::config::DatabaseConfig;

use crate::{
    app::BotApp, http_client::MainServiceHttpClient, outbound::TelegramOutbound, state::BotState,
};

/// Telegram bot credentials.
///
/// `bot_token` is required to start the bot. `chat_id` is loaded only from
/// runtime settings.
#[derive(Debug, Clone)]
pub struct TelegramConfig {
    /// Bot token obtained from [@BotFather](https://t.me/BotFather).
    pub bot_token: String,
}

/// Top-level configuration for the standalone bot process.
///
/// Use [`BotConfig::from_env`] to populate from environment variables, then
/// call [`BotConfig::open`] to initialize all runtime dependencies and get a
/// [`BotApp`] handle.
#[derive(Debug, Clone, SmartDefault)]
pub struct BotConfig {
    /// Database connection settings. Used for pgmq notification queue and
    /// KV-based settings sync.
    pub db_config:              DatabaseConfig,
    /// Telegram credentials. If `None`, the bot will attempt to read them
    /// from the KV store at startup, failing if neither source provides them.
    pub telegram:               Option<TelegramConfig>,
    /// Base URL of the main HTTP service that this bot calls for job
    /// discovery and JD parsing.
    #[default(_code = "\"http://127.0.0.1:3000\".to_owned()")]
    pub main_service_http_base: String,
}

impl BotConfig {
    /// Build configuration from environment variables.
    ///
    /// # Environment Variables
    ///
    /// | Variable                 | Required | Default                                          |
    /// |--------------------------|----------|--------------------------------------------------|
    /// | `TELEGRAM_BOT_TOKEN`     | Yes      | —                                                |
    /// | `TELEGRAM_CHAT_ID`       | No       | (loaded from runtime settings)                   |
    /// | `DATABASE_URL`           | No       | `postgres://postgres:postgres@localhost:5432/job` |
    /// | `MIGRATION_DIRECTORY`    | No       | `crates/rara-model/migrations`                   |
    /// | `MAIN_SERVICE_HTTP_BASE` | No       | `http://127.0.0.1:3000`                          |
    pub fn from_env() -> Self {
        let db_config =
            DatabaseConfig::builder()
                .database_url(std::env::var("DATABASE_URL").unwrap_or_else(|_| {
                    "postgres://postgres:postgres@localhost:5432/job".to_string()
                }))
                .migration_dir(
                    std::env::var("MIGRATION_DIRECTORY")
                        .unwrap_or_else(|_| "crates/rara-model/migrations".to_string()),
                )
                .build();

        let telegram = std::env::var("TELEGRAM_BOT_TOKEN")
            .ok()
            .map(|token| TelegramConfig { bot_token: token });

        let main_service_http_base = std::env::var("MAIN_SERVICE_HTTP_BASE")
            .unwrap_or_else(|_| "http://127.0.0.1:3000".to_owned());

        Self {
            db_config,
            telegram,
            main_service_http_base,
        }
    }

    /// Initialize all runtime dependencies and return a ready-to-run
    /// [`BotApp`].
    ///
    /// This method performs the following steps in order:
    /// 1. Opens the database connection and runs migrations.
    /// 2. Loads runtime settings from the KV store (falls back to env vars).
    /// 3. Builds a `reqwest::Client` with a 45-second timeout (must exceed the
    ///    30-second Telegram long-poll timeout).
    /// 4. Calls [`bot::initialize`] to delete any webhook and verify the token
    ///    via `getMe`.
    /// 5. Wires [`BotState`], [`TelegramOutbound`], and [`NotifyClient`] into a
    ///    [`BotApp`].
    pub async fn open(self) -> Result<BotApp, Whatever> {
        let db_store = self
            .db_config
            .open()
            .await
            .whatever_context("Failed to initialize database for bot")?;
        let kv_store = db_store.kv_store();

        let settings_svc = SettingsSvc::load(kv_store)
            .await
            .whatever_context("Failed to load runtime settings for bot")?;
        let runtime_settings = settings_svc.current();
        let settings_rx = settings_svc.subscribe();

        let env_telegram = self.telegram.clone();
        let bot_token = match runtime_settings
            .telegram
            .bot_token
            .or_else(|| env_telegram.as_ref().map(|cfg| cfg.bot_token.clone()))
        {
            Some(token) => token,
            None => {
                whatever!(
                    "Telegram bot token is required: set TELEGRAM_BOT_TOKEN or /api/v1/settings"
                )
            }
        };
        let chat_id = match runtime_settings.telegram.chat_id {
            Some(chat_id) => chat_id,
            None => {
                whatever!("Telegram chat_id is required: set it via /api/v1/settings")
            }
        };

        // Build reqwest client with extended timeout for long polling.
        // Use teloxide's re-exported reqwest to match the version teloxide expects.
        let http_client = teloxide::net::default_reqwest_settings()
            .timeout(std::time::Duration::from_secs(45))
            .build()
            .whatever_context("failed to build HTTP client")?;

        let bot = teloxide::Bot::with_client(&bot_token, http_client);

        // Verify token and get bot info.
        let bot_username = crate::bot::initialize(&bot)
            .await
            .whatever_context("failed to initialize telegram bot")?;

        let cancel = CancellationToken::new();

        let main_http = Arc::new(MainServiceHttpClient::new(
            self.main_service_http_base.clone(),
        ));

        let state = Arc::new(BotState::new(
            bot.clone(),
            bot_username,
            bot_token.clone(),
            chat_id,
            runtime_settings.telegram.allowed_group_chat_id,
            main_http,
            cancel,
        ));

        let outbound = Arc::new(TelegramOutbound::new(bot, state.config.clone()));

        let notify_client = Arc::new(
            rara_domain_shared::notify::client::NotifyClient::new(db_store.pool().clone())
                .await
                .whatever_context("Failed to initialize shared notify queue client")?,
        );

        Ok(BotApp {
            state,
            outbound,
            notify_client,
            settings_rx,
        })
    }
}
