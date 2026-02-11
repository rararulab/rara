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

use std::sync::Arc;

use job_server::grpc::GrpcServerConfig;
use smart_default::SmartDefault;
use snafu::{ResultExt, Whatever, whatever};
use tokio_util::sync::CancellationToken;
use yunara_store::config::DatabaseConfig;

use crate::{
    app::BotApp, http_client::MainServiceHttpClient, outbox::TelegramOutboxRepository,
    runtime::TelegramBotRuntime, telegram_service::TelegramService,
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
    /// Bot command gRPC server config.
    pub grpc_config:            GrpcServerConfig,
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
    /// - `TELEGRAM_BOT_GRPC_BIND`
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

        let grpc_bind = std::env::var("TELEGRAM_BOT_GRPC_BIND")
            .unwrap_or_else(|_| "127.0.0.1:50061".to_owned());

        let grpc_config = GrpcServerConfig {
            bind_address: grpc_bind.clone(),
            server_address: grpc_bind,
            ..GrpcServerConfig::default()
        };

        Self {
            db_config,
            telegram,
            main_service_http_base,
            grpc_config,
        }
    }

    /// Initialize concrete runtime dependencies.
    ///
    /// Initializes:
    /// - Telegram adapter
    /// - HTTP client to main service
    /// - Bot-owned outbox repository (postgres)
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

        let runtime = Arc::new(TelegramBotRuntime::new(telegram, main_http));

        let db_store = yunara_store::db::DBStore::new(self.db_config.clone())
            .await
            .whatever_context("Failed to initialize database for bot")?;
        let outbox_repo = Arc::new(TelegramOutboxRepository::new(db_store.pool().clone()));

        Ok(BotApp {
            config: self,
            runtime,
            outbox_repo,
            cancellation_token: CancellationToken::new(),
        })
    }
}
