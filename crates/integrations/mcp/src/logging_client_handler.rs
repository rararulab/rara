// Copyright 2025 Rararulab
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

use futures::future::BoxFuture;
use rmcp::{
    ClientHandler, RoleClient,
    model::{
        CancelledNotificationParam, ClientInfo, CreateElicitationRequestParams,
        CreateElicitationResult, LoggingLevel, LoggingMessageNotificationParam,
        ProgressNotificationParam, RequestId, ResourceUpdatedNotificationParam,
    },
    service::{NotificationContext, RequestContext},
};
use tracing::{debug, error, info, warn};

use crate::manager::log_buffer::McpLogBuffer;

/// Interface for sending elicitation requests to the UI and awaiting a
/// response.
pub type SendElicitation = Box<
    dyn Fn(
            RequestId,
            CreateElicitationRequestParams,
        ) -> BoxFuture<'static, anyhow::Result<CreateElicitationResult>>
        + Send
        + Sync,
>;

#[derive(Clone)]
pub(crate) struct LoggingClientHandler {
    client_info:      ClientInfo,
    send_elicitation: Arc<SendElicitation>,
    log_buffer:       McpLogBuffer,
    server_name:      String,
}

impl LoggingClientHandler {
    pub(crate) fn new(
        client_info: ClientInfo,
        send_elicitation: SendElicitation,
        server_name: String,
        log_buffer: McpLogBuffer,
    ) -> Self {
        Self {
            client_info,
            send_elicitation: Arc::new(send_elicitation),
            log_buffer,
            server_name,
        }
    }
}

impl ClientHandler for LoggingClientHandler {
    async fn create_elicitation(
        &self,
        request: CreateElicitationRequestParams,
        context: RequestContext<RoleClient>,
    ) -> Result<CreateElicitationResult, rmcp::ErrorData> {
        (self.send_elicitation)(context.id, request)
            .await
            .map_err(|err| rmcp::ErrorData::internal_error(err.to_string(), None))
    }

    async fn on_cancelled(
        &self,
        params: CancelledNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        let msg = format!(
            "cancelled request (request_id: {}, reason: {:?})",
            params.request_id, params.reason
        );
        info!("MCP server {msg}");
        self.log_buffer.push(&self.server_name, "warn", msg).await;
    }

    async fn on_progress(
        &self,
        params: ProgressNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        let msg = format!(
            "progress (token: {:?}, progress: {}, total: {:?}, message: {:?})",
            params.progress_token, params.progress, params.total, params.message
        );
        info!("MCP server {msg}");
        self.log_buffer.push(&self.server_name, "debug", msg).await;
    }

    async fn on_resource_updated(
        &self,
        params: ResourceUpdatedNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        let msg = format!("resource updated (uri: {})", params.uri);
        info!("MCP server {msg}");
        self.log_buffer.push(&self.server_name, "info", msg).await;
    }

    async fn on_resource_list_changed(&self, _context: NotificationContext<RoleClient>) {
        info!("MCP server resource list changed");
        self.log_buffer
            .push(&self.server_name, "info", "resource list changed".into())
            .await;
    }

    async fn on_tool_list_changed(&self, _context: NotificationContext<RoleClient>) {
        info!("MCP server tool list changed");
        self.log_buffer
            .push(&self.server_name, "info", "tool list changed".into())
            .await;
    }

    async fn on_prompt_list_changed(&self, _context: NotificationContext<RoleClient>) {
        info!("MCP server prompt list changed");
        self.log_buffer
            .push(&self.server_name, "info", "prompt list changed".into())
            .await;
    }

    fn get_info(&self) -> ClientInfo { self.client_info.clone() }

    async fn on_logging_message(
        &self,
        params: LoggingMessageNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        let LoggingMessageNotificationParam {
            level,
            logger,
            data,
        } = params;
        let logger = logger.as_deref();
        let buf_level = match level {
            LoggingLevel::Emergency
            | LoggingLevel::Alert
            | LoggingLevel::Critical
            | LoggingLevel::Error => {
                error!(
                    "MCP server log message (level: {:?}, logger: {:?}, data: {})",
                    level, logger, data
                );
                "error"
            }
            LoggingLevel::Warning => {
                warn!(
                    "MCP server log message (level: {:?}, logger: {:?}, data: {})",
                    level, logger, data
                );
                "warn"
            }
            LoggingLevel::Notice | LoggingLevel::Info => {
                info!(
                    "MCP server log message (level: {:?}, logger: {:?}, data: {})",
                    level, logger, data
                );
                "info"
            }
            LoggingLevel::Debug => {
                debug!(
                    "MCP server log message (level: {:?}, logger: {:?}, data: {})",
                    level, logger, data
                );
                "debug"
            }
        };
        self.log_buffer
            .push(&self.server_name, buf_level, format!("{data}"))
            .await;
    }
}
