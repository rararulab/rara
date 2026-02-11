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

//! gRPC command ingress for telegram-bot.
//!
//! This service receives commands from main service and applies them on
//! bot-side adapters (currently Telegram message sending).
//!
//! Delivery contract:
//! 1. Persist outgoing message into `telegram_outbox` as `pending`.
//! 2. Attempt Telegram API send.
//! 3. Update outbox row to `sent` or `failed`.
//!
//! This keeps transport-level reliability data local to telegram-bot.

use std::sync::Arc;

use async_trait::async_trait;
use job_api::pb::telegrambot::v1::{
    DispatchCommandRequest, DispatchCommandResponse, SendMessageRequest, SendMessageResponse,
    telegram_bot_command_service_server,
};
use job_server::grpc::GrpcServiceHandler;
use tokio_util::sync::CancellationToken;
use tonic::{Request, Response, Status, service::RoutesBuilder};
use tonic_health::server::HealthReporter;
use tracing::warn;

use crate::{outbox::TelegramOutboxRepository, telegram_service::TelegramService};

/// Concrete gRPC service implementation for `TelegramBotCommandService`.
#[derive(Clone)]
pub struct TelegramBotCommandGrpcService {
    telegram:    Arc<TelegramService>,
    outbox_repo: Arc<TelegramOutboxRepository>,
}

impl TelegramBotCommandGrpcService {
    /// Construct service with a shared Telegram adapter.
    pub fn new(telegram: Arc<TelegramService>, outbox_repo: Arc<TelegramOutboxRepository>) -> Self {
        Self {
            telegram,
            outbox_repo,
        }
    }

    /// Persist one outgoing message to outbox, send it, and update final
    /// status.
    ///
    /// `chat_id == 0` means "use configured primary chat".
    async fn send_and_record(&self, chat_id: i64, text: &str, source: &str) -> Result<(), Status> {
        // Normalize target chat id for persistence.
        let persisted_chat_id = if chat_id == 0 {
            self.telegram.primary_chat_id().0
        } else {
            chat_id
        };

        // Record "pending" before network I/O so crash/restart won't lose intent.
        let outbox_id = self
            .outbox_repo
            .enqueue(persisted_chat_id, text, source)
            .await
            .map_err(|e| Status::internal(format!("failed to persist telegram outbox: {e}")))?;

        // Actual Telegram delivery attempt.
        let send_result = if chat_id == 0 {
            self.telegram.send_primary_message(text).await
        } else {
            self.telegram
                .send_message(teloxide::types::ChatId(chat_id), text)
                .await
        };

        match send_result {
            Ok(_) => {
                // Mark delivered.
                self.outbox_repo
                    .mark_sent(outbox_id)
                    .await
                    .map_err(|e| Status::internal(format!("failed to mark outbox sent: {e}")))?;
            }
            Err(e) => {
                // Mark failed with provider error for later diagnostics/retry tooling.
                self.outbox_repo
                    .mark_failed(outbox_id, &e.to_string())
                    .await
                    .map_err(|repo_err| {
                        Status::internal(format!("failed to mark outbox failed: {repo_err}"))
                    })?;
                return Err(Status::internal(e.to_string()));
            }
        }

        Ok(())
    }
}

#[async_trait]
impl telegram_bot_command_service_server::TelegramBotCommandService
    for TelegramBotCommandGrpcService
{
    /// Send one message directly to a target chat.
    ///
    /// `chat_id = 0` means fallback to configured primary chat.
    async fn send_message(
        &self,
        request: Request<SendMessageRequest>,
    ) -> Result<Response<SendMessageResponse>, Status> {
        let req = request.into_inner();
        if req.text.trim().is_empty() {
            return Err(Status::invalid_argument("text must not be empty"));
        }
        self.send_and_record(req.chat_id, &req.text, "grpc:send_message")
            .await?;
        Ok(Response::new(SendMessageResponse { accepted: true }))
    }

    async fn dispatch_command(
        &self,
        request: Request<DispatchCommandRequest>,
    ) -> Result<Response<DispatchCommandResponse>, Status> {
        // Command envelope dispatch by string is intentionally simple for now.
        // We can evolve this into typed payloads once command surface grows.
        let req = request.into_inner();
        match req.command.as_str() {
            "send_message" => {
                if req.payload_json.trim().is_empty() {
                    return Err(Status::invalid_argument("payload_json must not be empty"));
                }
                self.send_and_record(0, &req.payload_json, "grpc:dispatch_command")
                    .await?;
                Ok(Response::new(DispatchCommandResponse {
                    accepted: true,
                    message:  "send_message executed".to_owned(),
                }))
            }
            other => {
                warn!(command = other, "received unsupported bot command");
                Ok(Response::new(DispatchCommandResponse {
                    accepted: false,
                    message:  format!("unsupported command: {other}"),
                }))
            }
        }
    }
}

#[async_trait]
impl GrpcServiceHandler for TelegramBotCommandGrpcService {
    fn service_name(&self) -> &'static str { "TelegramBotCommandService" }

    fn file_descriptor_set(&self) -> &'static [u8] { job_api::pb::GRPC_DESC }

    fn register_service(self: &Arc<Self>, builder: &mut RoutesBuilder) {
        builder.add_service(
            telegram_bot_command_service_server::TelegramBotCommandServiceServer::from_arc(
                self.clone(),
            ),
        );
    }

    async fn readiness_reporting(
        self: &Arc<Self>,
        _cancellation_token: CancellationToken,
        reporter: HealthReporter,
    ) {
        reporter
            .set_serving::<telegram_bot_command_service_server::TelegramBotCommandServiceServer<Self>>()
            .await;
    }
}
