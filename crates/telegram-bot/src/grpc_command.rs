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

use async_trait::async_trait;
use job_api::pb::telegrambot::v1::{
    DispatchCommandRequest, DispatchCommandResponse, SendMessageRequest, SendMessageResponse,
    telegram_bot_command_service_server,
};
use job_domain_shared::telegram_service::TelegramService;
use job_server::grpc::GrpcServiceHandler;
use tokio_util::sync::CancellationToken;
use tonic::{Request, Response, Status, service::RoutesBuilder};
use tonic_health::server::HealthReporter;
use tracing::warn;

#[derive(Clone)]
pub struct TelegramBotCommandGrpcService {
    telegram: Arc<TelegramService>,
}

impl TelegramBotCommandGrpcService {
    pub fn new(telegram: Arc<TelegramService>) -> Self { Self { telegram } }
}

#[async_trait]
impl telegram_bot_command_service_server::TelegramBotCommandService
    for TelegramBotCommandGrpcService
{
    async fn send_message(
        &self,
        request: Request<SendMessageRequest>,
    ) -> Result<Response<SendMessageResponse>, Status> {
        let req = request.into_inner();
        if req.text.trim().is_empty() {
            return Err(Status::invalid_argument("text must not be empty"));
        }

        let send_result = if req.chat_id == 0 {
            self.telegram.send_primary_message(&req.text).await
        } else {
            self.telegram
                .send_message(teloxide::types::ChatId(req.chat_id), &req.text)
                .await
        };

        send_result.map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(SendMessageResponse { accepted: true }))
    }

    async fn dispatch_command(
        &self,
        request: Request<DispatchCommandRequest>,
    ) -> Result<Response<DispatchCommandResponse>, Status> {
        let req = request.into_inner();
        match req.command.as_str() {
            "send_message" => {
                if req.payload_json.trim().is_empty() {
                    return Err(Status::invalid_argument("payload_json must not be empty"));
                }
                self.telegram
                    .send_primary_message(&req.payload_json)
                    .await
                    .map_err(|e| Status::internal(e.to_string()))?;
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
