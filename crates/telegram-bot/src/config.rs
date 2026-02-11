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

use job_domain_shared::settings::{model::Settings, service::RUNTIME_SETTINGS_KV_KEY};
use smart_default::SmartDefault;
use snafu::{ResultExt, Whatever, whatever};
use tokio_util::sync::CancellationToken;
use yunara_store::config::DatabaseConfig;

use crate::{
    app::BotApp, http_client::MainServiceHttpClient, runtime::TelegramBotRuntime,
    telegram_service::TelegramService,
};

/// Telegram credential/config values.
#[derive(Debug, Clone)]
pub struct TelegramConfig {
    /// Telegram bot token.
    pub bot_token: String,
    /// Primary authorized chat id.
    pub chat_id:   i64,
}

/// Runtime configuration for the standalone bot app.
#[derive(Debug, Clone, SmartDefault)]
pub struct BotConfig {
    /// Database configuration used to persist outgoing notification records.
    pub db_config:              DatabaseConfig,
    /// Telegram configuration. Required to start bot runtime.
    pub telegram:               Option<TelegramConfig>,
    /// Main service HTTP endpoint.
    #[default(_code = "\"http://127.0.0.1:3000\".to_owned()")]
    pub main_service_http_base: String,
}

impl BotConfig {
    /// Build config from env vars.
    ///
    /// Required envs:
    /// - `TELEGRAM_BOT_TOKEN`
    /// - `TELEGRAM_CHAT_ID`
    ///
    /// Optional envs:
    /// - `DATABASE_URL`
    /// - `MAIN_SERVICE_HTTP_BASE`
    pub fn from_env() -> Self {
        let db_config =
            DatabaseConfig::builder()
                .database_url(std::env::var("DATABASE_URL").unwrap_or_else(|_| {
                    "postgres://postgres:postgres@localhost:5432/job".to_string()
                }))
                .build();

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

        Self {
            db_config,
            telegram,
            main_service_http_base,
        }
    }

    /// Initialize concrete runtime dependencies.
    ///
    /// Initializes:
    /// - Telegram adapter
    /// - HTTP client to main service
    /// - Shared notify queue adapter (`pgmq`)
    pub async fn open(self) -> Result<BotApp, Whatever> {
        let db_store = self
            .db_config
            .open()
            .await
            .whatever_context("Failed to initialize database for bot")?;
        let kv_store = db_store.kv_store();

        let mut runtime_settings = kv_store
            .get::<Settings>(RUNTIME_SETTINGS_KV_KEY)
            .await
            .whatever_context("Failed to load runtime settings for bot")?
            .unwrap_or_default();
        runtime_settings.normalize();

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
        let chat_id = match runtime_settings
            .telegram
            .chat_id
            .or_else(|| env_telegram.as_ref().map(|cfg| cfg.chat_id))
        {
            Some(chat_id) => chat_id,
            None => {
                whatever!("Telegram chat_id is required: set TELEGRAM_CHAT_ID or /api/v1/settings")
            }
        };

        let telegram = Arc::new(TelegramService::new(bot_token, chat_id));

        let main_http = Arc::new(MainServiceHttpClient::new(
            self.main_service_http_base.clone(),
        ));

        let runtime = Arc::new(TelegramBotRuntime::new(telegram, main_http));

        let notify_client = Arc::new(
            job_domain_shared::notify::client::NotifyClient::new(db_store.pool().clone())
                .await
                .whatever_context("Failed to initialize shared notify queue client")?,
        );

        Ok(BotApp {
            _config: self,
            runtime,
            notify_client,
            kv_store,
            cancellation_token: CancellationToken::new(),
        })
    }
}
