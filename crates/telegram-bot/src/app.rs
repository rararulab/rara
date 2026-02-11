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

use snafu::{ResultExt, Whatever};
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::{
    config::BotConfig, grpc_command::TelegramBotCommandGrpcService,
    outbox::TelegramOutboxRepository, runtime::TelegramBotRuntime,
};

/// Bot process application handle.
pub struct BotApp {
    pub(crate) config:             BotConfig,
    pub(crate) runtime:            Arc<TelegramBotRuntime>,
    /// Bot-owned transport outbox repository.
    pub(crate) outbox_repo:        Arc<TelegramOutboxRepository>,
    pub(crate) cancellation_token: CancellationToken,
}

impl BotApp {
    /// Start both ingress endpoints and block until shutdown.
    pub async fn run(self) -> Result<(), Whatever> {
        // Start gRPC command ingress for main-service -> bot calls.
        let mut grpc_handle = job_server::grpc::start_grpc_server(
            &self.config.grpc_config,
            &[Arc::new(TelegramBotCommandGrpcService::new(
                self.runtime.telegram.clone(),
                self.outbox_repo.clone(),
            ))],
        )
        .whatever_context("failed to start telegram-bot gRPC command service")?;
        grpc_handle
            .wait_for_start()
            .await
            .whatever_context("telegram-bot gRPC service failed to start")?;

        // Start Telegram long-polling dispatcher.
        let mut telegram_handle = self.runtime.start_dispatcher();
        telegram_handle
            .wait_for_start()
            .await
            .whatever_context("telegram-bot dispatcher failed to start")?;

        // Keep process alive until Ctrl+C.
        let cancel = self.cancellation_token.clone();
        tokio::spawn(async move {
            tokio::signal::ctrl_c()
                .await
                .expect("failed to install Ctrl+C handler");
            cancel.cancel();
        });

        self.cancellation_token.cancelled().await;
        info!("telegram-bot shutdown requested");

        // Graceful teardown order: stop ingress first, then wait joins.
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
