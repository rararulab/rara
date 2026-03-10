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

//! Admin Telegram bot commands such as `/restart`.

use std::sync::Arc;

use async_trait::async_trait;
use rara_kernel::{
    channel::command::{
        CommandContext, CommandDefinition, CommandHandler, CommandInfo, CommandResult,
    },
    error::KernelError,
};

use super::client::BotServiceClient;

/// Handles privileged admin commands for Telegram operations.
pub struct AdminCommandHandler {
    client: Arc<dyn BotServiceClient>,
    owner_chat_id: Option<String>,
}

impl AdminCommandHandler {
    pub fn new(client: Arc<dyn BotServiceClient>, owner_chat_id: Option<String>) -> Self {
        Self {
            client,
            owner_chat_id,
        }
    }
}

#[async_trait]
impl CommandHandler for AdminCommandHandler {
    fn commands(&self) -> Vec<CommandDefinition> {
        vec![CommandDefinition {
            name: "restart".to_owned(),
            description: "Restart the supervised Rara instance".to_owned(),
            usage: Some("/restart".to_owned()),
        }]
    }

    async fn handle(
        &self,
        command: &CommandInfo,
        context: &CommandContext,
    ) -> Result<CommandResult, KernelError> {
        match command.name.as_str() {
            "restart" => self.handle_restart(context).await,
            _ => Ok(CommandResult::None),
        }
    }
}

impl AdminCommandHandler {
    async fn handle_restart(&self, context: &CommandContext) -> Result<CommandResult, KernelError> {
        let chat_id = extract_chat_id(context);

        let Some(expected_chat_id) = self.owner_chat_id.as_deref() else {
            return Ok(CommandResult::Text(
                "Restart command is unavailable: no owner Telegram chat is configured.".to_owned(),
            ));
        };

        if chat_id != expected_chat_id {
            return Ok(CommandResult::Text(
                "Unauthorized: this command is restricted to the configured owner chat.".to_owned(),
            ));
        }

        match self.client.restart_agent().await {
            Ok(()) => Ok(CommandResult::Text(
                "Restart requested. The supervised instance should come back shortly.".to_owned(),
            )),
            Err(err) => Ok(CommandResult::Text(format!(
                "Failed to request restart: {err}"
            ))),
        }
    }
}

fn extract_chat_id(context: &CommandContext) -> String {
    context
        .metadata
        .get("telegram_chat_id")
        .and_then(|v| {
            v.as_i64()
                .map(|n| n.to_string())
                .or_else(|| v.as_str().map(String::from))
        })
        .unwrap_or_else(|| "0".to_owned())
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use serde_json::json;

    use super::*;
    use crate::telegram::commands::client::{
        BotServiceError, ChannelBinding, DiscoveryJob, McpServerInfo, SessionDetail,
        SessionListItem,
    };

    struct TestClient {
        restarted: AtomicBool,
        restart_error: Option<String>,
    }

    impl TestClient {
        fn ok() -> Self {
            Self {
                restarted: AtomicBool::new(false),
                restart_error: None,
            }
        }

        fn with_error(message: &str) -> Self {
            Self {
                restarted: AtomicBool::new(false),
                restart_error: Some(message.to_owned()),
            }
        }
    }

    #[async_trait]
    impl BotServiceClient for TestClient {
        async fn get_channel_session(
            &self,
            _chat_id: &str,
        ) -> Result<Option<ChannelBinding>, BotServiceError> {
            unreachable!()
        }

        async fn bind_channel(
            &self,
            _channel_type: &str,
            _chat_id: &str,
            _session_key: &str,
        ) -> Result<ChannelBinding, BotServiceError> {
            unreachable!()
        }

        async fn create_session(&self, _title: Option<&str>) -> Result<String, BotServiceError> {
            unreachable!()
        }

        async fn clear_session_messages(&self, _session_key: &str) -> Result<(), BotServiceError> {
            unreachable!()
        }

        async fn list_sessions(
            &self,
            _limit: u32,
        ) -> Result<Vec<SessionListItem>, BotServiceError> {
            unreachable!()
        }

        async fn get_session(&self, _key: &str) -> Result<SessionDetail, BotServiceError> {
            unreachable!()
        }

        async fn update_session(
            &self,
            _key: &str,
            _model: Option<&str>,
        ) -> Result<SessionDetail, BotServiceError> {
            unreachable!()
        }

        async fn restart_agent(&self) -> Result<(), BotServiceError> {
            self.restarted.store(true, Ordering::SeqCst);
            if let Some(message) = &self.restart_error {
                return Err(BotServiceError::Service {
                    message: message.clone(),
                });
            }
            Ok(())
        }

        async fn discover_jobs(
            &self,
            _keywords: Vec<String>,
            _location: Option<String>,
            _max_results: u32,
        ) -> Result<Vec<DiscoveryJob>, BotServiceError> {
            unreachable!()
        }

        async fn submit_jd_parse(&self, _text: &str) -> Result<(), BotServiceError> {
            unreachable!()
        }

        async fn list_mcp_servers(&self) -> Result<Vec<McpServerInfo>, BotServiceError> {
            unreachable!()
        }

        async fn get_mcp_server(&self, _name: &str) -> Result<McpServerInfo, BotServiceError> {
            unreachable!()
        }

        async fn add_mcp_server(
            &self,
            _name: &str,
            _command: &str,
            _args: &[String],
        ) -> Result<McpServerInfo, BotServiceError> {
            unreachable!()
        }

        async fn start_mcp_server(&self, _name: &str) -> Result<(), BotServiceError> {
            unreachable!()
        }

        async fn remove_mcp_server(&self, _name: &str) -> Result<(), BotServiceError> {
            unreachable!()
        }
    }

    fn command_context(chat_id: i64) -> CommandContext {
        CommandContext {
            channel_type: rara_kernel::channel::types::ChannelType::Telegram,
            session_key: String::new(),
            user: rara_kernel::channel::types::ChannelUser {
                platform_id: "user-1".to_owned(),
                display_name: Some("owner".to_owned()),
            },
            metadata: std::collections::HashMap::from([(
                "telegram_chat_id".to_owned(),
                json!(chat_id),
            )]),
        }
    }

    #[tokio::test]
    async fn restart_requires_owner_chat() {
        let client = Arc::new(TestClient::ok());
        let handler = AdminCommandHandler::new(client.clone(), Some("42".to_owned()));

        let result = handler
            .handle(
                &CommandInfo {
                    name: "restart".to_owned(),
                    args: String::new(),
                    raw: "/restart".to_owned(),
                },
                &command_context(7),
            )
            .await
            .expect("handler should succeed");

        match result {
            CommandResult::Text(text) => assert_eq!(
                text,
                "Unauthorized: this command is restricted to the configured owner chat."
            ),
            other => panic!("unexpected command result: {other:?}"),
        }
        assert!(!client.restarted.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn restart_triggers_client_for_owner_chat() {
        let client = Arc::new(TestClient::ok());
        let handler = AdminCommandHandler::new(client.clone(), Some("42".to_owned()));

        let result = handler
            .handle(
                &CommandInfo {
                    name: "restart".to_owned(),
                    args: String::new(),
                    raw: "/restart".to_owned(),
                },
                &command_context(42),
            )
            .await
            .expect("handler should succeed");

        match result {
            CommandResult::Text(text) => {
                assert_eq!(
                    text,
                    "Restart requested. The supervised instance should come back shortly."
                );
            }
            other => panic!("unexpected command result: {other:?}"),
        }
        assert!(client.restarted.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn restart_surfaces_client_errors() {
        let client = Arc::new(TestClient::with_error("gateway unavailable"));
        let handler = AdminCommandHandler::new(client, Some("42".to_owned()));

        let result = handler
            .handle(
                &CommandInfo {
                    name: "restart".to_owned(),
                    args: String::new(),
                    raw: "/restart".to_owned(),
                },
                &command_context(42),
            )
            .await
            .expect("handler should succeed");

        match result {
            CommandResult::Text(text) => {
                assert_eq!(text, "Failed to request restart: gateway unavailable");
            }
            other => panic!("unexpected command result: {other:?}"),
        }
    }
}
