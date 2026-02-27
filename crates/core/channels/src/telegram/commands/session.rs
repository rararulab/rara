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

//! Session management commands: `/new`, `/clear`, `/sessions`, `/usage`, `/model`.

use std::fmt::Write;
use std::sync::Arc;

use async_trait::async_trait;
use rara_kernel::channel::command::{
    CommandContext, CommandDefinition, CommandHandler, CommandInfo, CommandResult,
};
use rara_kernel::channel::types::InlineButton;
use rara_kernel::error::KernelError;

use super::client::BotServiceClient;

/// Maximum number of sessions to display in the `/sessions` list.
const SESSIONS_LIST_LIMIT: u32 = 10;

/// Handles session management commands.
pub struct SessionCommandHandler {
    client: Arc<dyn BotServiceClient>,
}

impl SessionCommandHandler {
    pub fn new(client: Arc<dyn BotServiceClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl CommandHandler for SessionCommandHandler {
    fn commands(&self) -> Vec<CommandDefinition> {
        vec![
            CommandDefinition {
                name: "new".to_owned(),
                description: "Start a new chat session".to_owned(),
                usage: Some("/new".to_owned()),
            },
            CommandDefinition {
                name: "clear".to_owned(),
                description: "Clear current session history".to_owned(),
                usage: Some("/clear".to_owned()),
            },
            CommandDefinition {
                name: "sessions".to_owned(),
                description: "List and switch chat sessions".to_owned(),
                usage: Some("/sessions".to_owned()),
            },
            CommandDefinition {
                name: "usage".to_owned(),
                description: "Show current session info".to_owned(),
                usage: Some("/usage".to_owned()),
            },
            CommandDefinition {
                name: "model".to_owned(),
                description: "Show or switch the AI model".to_owned(),
                usage: Some("/model [name]".to_owned()),
            },
        ]
    }

    async fn handle(
        &self,
        command: &CommandInfo,
        context: &CommandContext,
    ) -> Result<CommandResult, KernelError> {
        match command.name.as_str() {
            "new" => self.handle_new(context).await,
            "clear" => self.handle_clear(context).await,
            "sessions" => self.handle_sessions(context).await,
            "usage" => self.handle_usage(context).await,
            "model" => self.handle_model(&command.args, context).await,
            _ => Ok(CommandResult::None),
        }
    }
}

impl SessionCommandHandler {
    /// `/new` — create a new session and bind the channel to it.
    async fn handle_new(
        &self,
        context: &CommandContext,
    ) -> Result<CommandResult, KernelError> {
        let chat_id = extract_chat_id(context);
        let account = extract_bot_username(context);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let key = format!("tg-{chat_id}-{now}");

        match self.client.create_session(&key, Some("Telegram Chat")).await {
            Ok(()) => {
                let _ = self
                    .client
                    .bind_channel("telegram", &account, &chat_id, &key)
                    .await;
                Ok(CommandResult::Text("New chat session started.".to_owned()))
            }
            Err(e) => Ok(CommandResult::Text(format!(
                "Failed to create session: {e}"
            ))),
        }
    }

    /// `/clear` — clear all messages in the current session.
    async fn handle_clear(
        &self,
        context: &CommandContext,
    ) -> Result<CommandResult, KernelError> {
        let chat_id = extract_chat_id(context);
        let account = extract_bot_username(context);

        match self
            .client
            .get_channel_session(&account, &chat_id)
            .await
        {
            Ok(Some(binding)) => {
                match self
                    .client
                    .clear_session_messages(&binding.session_key)
                    .await
                {
                    Ok(()) => Ok(CommandResult::Text(
                        "Session history cleared.".to_owned(),
                    )),
                    Err(e) => Ok(CommandResult::Text(format!("Failed to clear: {e}"))),
                }
            }
            Ok(None) => Ok(CommandResult::Text(
                "No active session. Send a message to start one.".to_owned(),
            )),
            Err(e) => Ok(CommandResult::Text(format!("Error: {e}"))),
        }
    }

    /// `/sessions` — list sessions with inline keyboard for switching.
    async fn handle_sessions(
        &self,
        context: &CommandContext,
    ) -> Result<CommandResult, KernelError> {
        let chat_id = extract_chat_id(context);
        let account = extract_bot_username(context);

        // Find the currently active session key.
        let active_key = match self
            .client
            .get_channel_session(&account, &chat_id)
            .await
        {
            Ok(Some(binding)) => Some(binding.session_key),
            Ok(None) => None,
            Err(e) => {
                return Ok(CommandResult::Text(format!(
                    "Failed to resolve session: {e}"
                )));
            }
        };

        let sessions = match self.client.list_sessions(SESSIONS_LIST_LIMIT).await {
            Ok(s) => s,
            Err(e) => {
                return Ok(CommandResult::Text(format!(
                    "Failed to list sessions: {e}"
                )));
            }
        };

        if sessions.is_empty() {
            return Ok(CommandResult::Text(
                "No sessions found. Send a message to create one.".to_owned(),
            ));
        }

        let mut text = String::from("<b>Your sessions:</b>\n\n");
        let mut keyboard_rows: Vec<Vec<InlineButton>> = Vec::new();

        for (i, s) in sessions.iter().enumerate() {
            let title = s.title.as_deref().unwrap_or(&s.key);
            let is_active = active_key.as_deref() == Some(s.key.as_str());
            let marker = if is_active { " \u{2705}" } else { "" };

            let _ = writeln!(
                text,
                "{}. <b>{}</b>{marker}\n   <code>{}</code> ({} msgs)",
                i + 1,
                html_escape(title),
                html_escape(&s.key),
                s.message_count,
            );

            if !is_active {
                let label = format!("Switch to: {}", truncate_str(title, 30));
                let cb_data = format!("switch:{}", truncate_str(&s.key, 56));
                keyboard_rows.push(vec![InlineButton {
                    text: label,
                    callback_data: Some(cb_data),
                    url: None,
                }]);
            }
        }

        if keyboard_rows.is_empty() {
            Ok(CommandResult::Html(text))
        } else {
            Ok(CommandResult::HtmlWithKeyboard {
                html: text,
                keyboard: keyboard_rows,
            })
        }
    }

    /// `/usage` — show details about the current session.
    async fn handle_usage(
        &self,
        context: &CommandContext,
    ) -> Result<CommandResult, KernelError> {
        let chat_id = extract_chat_id(context);
        let account = extract_bot_username(context);

        let session_key = match self
            .client
            .get_channel_session(&account, &chat_id)
            .await
        {
            Ok(Some(binding)) => binding.session_key,
            Ok(None) => {
                return Ok(CommandResult::Text(
                    "No active session. Send a message to create one.".to_owned(),
                ));
            }
            Err(e) => {
                return Ok(CommandResult::Text(format!(
                    "Failed to resolve session: {e}"
                )));
            }
        };

        match self.client.get_session(&session_key).await {
            Ok(detail) => {
                let mut text = String::new();
                let title = detail.title.as_deref().unwrap_or("Untitled");
                let _ = writeln!(text, "<b>Session:</b> {}", html_escape(title));
                let _ = writeln!(
                    text,
                    "<b>Key:</b> <code>{}</code>",
                    html_escape(&detail.key)
                );
                let _ = writeln!(text, "<b>Messages:</b> {}", detail.message_count);
                if let Some(ref model) = detail.model {
                    let _ = writeln!(text, "<b>Model:</b> {}", html_escape(model));
                }
                let _ = writeln!(
                    text,
                    "<b>Created:</b> {}",
                    format_timestamp(&detail.created_at)
                );
                let _ = writeln!(
                    text,
                    "<b>Last active:</b> {}",
                    format_timestamp(&detail.updated_at)
                );
                if let Some(ref preview) = detail.preview {
                    let truncated = truncate_str(preview, 200);
                    let _ = writeln!(
                        text,
                        "\n<b>Last message:</b>\n{}",
                        html_escape(truncated)
                    );
                }
                Ok(CommandResult::Html(text))
            }
            Err(e) => Ok(CommandResult::Text(format!(
                "Failed to get session details: {e}"
            ))),
        }
    }

    /// `/model [name]` — show or switch the AI model.
    async fn handle_model(
        &self,
        args: &str,
        context: &CommandContext,
    ) -> Result<CommandResult, KernelError> {
        let chat_id = extract_chat_id(context);
        let account = extract_bot_username(context);

        let session_key = match self
            .client
            .get_channel_session(&account, &chat_id)
            .await
        {
            Ok(Some(binding)) => binding.session_key,
            Ok(None) => {
                return Ok(CommandResult::Text(
                    "No active session. Send a message to create one.".to_owned(),
                ));
            }
            Err(e) => {
                return Ok(CommandResult::Text(format!(
                    "Failed to resolve session: {e}"
                )));
            }
        };

        let new_model = args.trim();

        if new_model.is_empty() {
            // Show current model.
            match self.client.get_session(&session_key).await {
                Ok(detail) => {
                    let model = detail.model.as_deref().unwrap_or("(default)");
                    Ok(CommandResult::Html(format!(
                        "Session <code>{}</code>\nModel: <b>{}</b>\n\n\
                         Switch: <code>/model model-name</code>",
                        html_escape(&detail.key),
                        html_escape(model),
                    )))
                }
                Err(e) => Ok(CommandResult::Text(format!(
                    "Failed to get session details: {e}"
                ))),
            }
        } else {
            // Update model.
            match self
                .client
                .update_session(&session_key, Some(new_model))
                .await
            {
                Ok(detail) => {
                    let model = detail.model.as_deref().unwrap_or("(default)");
                    Ok(CommandResult::Html(format!(
                        "Model updated.\nSession <code>{}</code>\nModel: <b>{}</b>",
                        html_escape(&detail.key),
                        html_escape(model),
                    )))
                }
                Err(e) => Ok(CommandResult::Text(format!(
                    "Failed to update model: {e}"
                ))),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract chat_id string from command context metadata.
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

/// Extract bot username from command context metadata.
fn extract_bot_username(context: &CommandContext) -> String {
    context
        .metadata
        .get("telegram_bot_username")
        .and_then(|v| v.as_str())
        .unwrap_or("default")
        .to_owned()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn truncate_str(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        return s;
    }
    let limit = max_len.saturating_sub(3);
    let end = s
        .char_indices()
        .take_while(|(i, _)| *i <= limit)
        .last()
        .map_or(0, |(i, c)| i + c.len_utf8());
    &s[..end]
}

fn format_timestamp(raw: &str) -> String {
    if raw.len() >= 16 {
        let date_part = &raw[..10];
        let time_part = &raw[11..16];
        if !time_part.is_empty() {
            return format!("{date_part} {time_part}");
        }
    }
    raw.to_owned()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rara_kernel::channel::types::{ChannelType, ChannelUser};

    use super::*;
    use crate::telegram::commands::client::{
        BotServiceError, ChannelBinding, CodingTask, CodingTaskSummary, DiscoveryJob,
        McpServerInfo, SessionDetail, SessionListItem,
    };

    // -----------------------------------------------------------------------
    // Mock client
    // -----------------------------------------------------------------------

    struct MockClient;

    #[async_trait]
    impl BotServiceClient for MockClient {
        async fn get_channel_session(
            &self,
            _account: &str,
            _chat_id: &str,
        ) -> Result<Option<ChannelBinding>, BotServiceError> {
            Ok(Some(ChannelBinding {
                session_key: "tg-123-100".to_owned(),
            }))
        }

        async fn bind_channel(
            &self,
            _channel_type: &str,
            _account: &str,
            _chat_id: &str,
            session_key: &str,
        ) -> Result<ChannelBinding, BotServiceError> {
            Ok(ChannelBinding {
                session_key: session_key.to_owned(),
            })
        }

        async fn create_session(
            &self,
            _key: &str,
            _title: Option<&str>,
        ) -> Result<(), BotServiceError> {
            Ok(())
        }

        async fn clear_session_messages(
            &self,
            _session_key: &str,
        ) -> Result<(), BotServiceError> {
            Ok(())
        }

        async fn list_sessions(
            &self,
            _limit: u32,
        ) -> Result<Vec<SessionListItem>, BotServiceError> {
            Ok(vec![
                SessionListItem {
                    key: "tg-123-100".to_owned(),
                    title: Some("Session A".to_owned()),
                    message_count: 5,
                    updated_at: "2026-01-01T00:00:00Z".to_owned(),
                },
                SessionListItem {
                    key: "tg-123-200".to_owned(),
                    title: Some("Session B".to_owned()),
                    message_count: 3,
                    updated_at: "2026-01-02T00:00:00Z".to_owned(),
                },
            ])
        }

        async fn get_session(
            &self,
            key: &str,
        ) -> Result<SessionDetail, BotServiceError> {
            Ok(SessionDetail {
                key: key.to_owned(),
                title: Some("Test Session".to_owned()),
                model: Some("gpt-4o".to_owned()),
                message_count: 42,
                preview: Some("Hello world".to_owned()),
                created_at: "2026-01-01T00:00:00Z".to_owned(),
                updated_at: "2026-01-02T12:30:00Z".to_owned(),
            })
        }

        async fn update_session(
            &self,
            key: &str,
            model: Option<&str>,
        ) -> Result<SessionDetail, BotServiceError> {
            Ok(SessionDetail {
                key: key.to_owned(),
                title: Some("Test Session".to_owned()),
                model: model.map(String::from),
                message_count: 42,
                preview: None,
                created_at: "2026-01-01T00:00:00Z".to_owned(),
                updated_at: "2026-01-02T12:30:00Z".to_owned(),
            })
        }

        async fn discover_jobs(
            &self,
            _keywords: Vec<String>,
            _location: Option<String>,
            _max_results: u32,
        ) -> Result<Vec<DiscoveryJob>, BotServiceError> {
            Ok(vec![])
        }

        async fn submit_jd_parse(&self, _text: &str) -> Result<(), BotServiceError> {
            Ok(())
        }

        async fn list_mcp_servers(&self) -> Result<Vec<McpServerInfo>, BotServiceError> {
            Ok(vec![])
        }

        async fn get_mcp_server(
            &self,
            _name: &str,
        ) -> Result<McpServerInfo, BotServiceError> {
            Err(BotServiceError::Service {
                message: "not found".to_owned(),
            })
        }

        async fn add_mcp_server(
            &self,
            _name: &str,
            _command: &str,
            _args: &[String],
        ) -> Result<McpServerInfo, BotServiceError> {
            Err(BotServiceError::Service {
                message: "not implemented".to_owned(),
            })
        }

        async fn start_mcp_server(&self, _name: &str) -> Result<(), BotServiceError> {
            Ok(())
        }

        async fn remove_mcp_server(&self, _name: &str) -> Result<(), BotServiceError> {
            Ok(())
        }

        async fn dispatch_coding_task(
            &self,
            _prompt: &str,
            _agent: &str,
        ) -> Result<CodingTask, BotServiceError> {
            Err(BotServiceError::Service {
                message: "not implemented".to_owned(),
            })
        }

        async fn list_coding_tasks(
            &self,
        ) -> Result<Vec<CodingTaskSummary>, BotServiceError> {
            Ok(vec![])
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn make_context() -> CommandContext {
        let mut metadata = HashMap::new();
        metadata.insert(
            "telegram_chat_id".to_owned(),
            serde_json::json!(123),
        );
        metadata.insert(
            "telegram_bot_username".to_owned(),
            serde_json::json!("test_bot"),
        );
        CommandContext {
            channel_type: ChannelType::Telegram,
            session_key: "tg:123".to_owned(),
            user: ChannelUser {
                platform_id: "123".to_owned(),
                display_name: Some("Test".to_owned()),
            },
            metadata,
        }
    }

    fn make_command(name: &str, args: &str) -> CommandInfo {
        CommandInfo {
            name: name.to_owned(),
            args: args.to_owned(),
            raw: if args.is_empty() {
                format!("/{name}")
            } else {
                format!("/{name} {args}")
            },
        }
    }

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn new_creates_session() {
        let handler = SessionCommandHandler::new(Arc::new(MockClient));
        let result = handler
            .handle(&make_command("new", ""), &make_context())
            .await;
        match result {
            Ok(CommandResult::Text(text)) => {
                assert!(text.contains("New chat session started"));
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn clear_clears_messages() {
        let handler = SessionCommandHandler::new(Arc::new(MockClient));
        let result = handler
            .handle(&make_command("clear", ""), &make_context())
            .await;
        match result {
            Ok(CommandResult::Text(text)) => {
                assert!(text.contains("cleared"));
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn sessions_returns_list_with_keyboard() {
        let handler = SessionCommandHandler::new(Arc::new(MockClient));
        let result = handler
            .handle(&make_command("sessions", ""), &make_context())
            .await;
        match result {
            Ok(CommandResult::HtmlWithKeyboard { html, keyboard }) => {
                assert!(html.contains("Session A"));
                assert!(html.contains("Session B"));
                // Session A is active (tg-123-100), so only B gets a button.
                assert_eq!(keyboard.len(), 1);
                assert!(keyboard[0][0].text.contains("Session B"));
            }
            other => panic!("expected HtmlWithKeyboard, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn usage_shows_session_detail() {
        let handler = SessionCommandHandler::new(Arc::new(MockClient));
        let result = handler
            .handle(&make_command("usage", ""), &make_context())
            .await;
        match result {
            Ok(CommandResult::Html(html)) => {
                assert!(html.contains("Test Session"));
                assert!(html.contains("gpt-4o"));
                assert!(html.contains("42"));
            }
            other => panic!("expected Html, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn model_without_args_shows_current() {
        let handler = SessionCommandHandler::new(Arc::new(MockClient));
        let result = handler
            .handle(&make_command("model", ""), &make_context())
            .await;
        match result {
            Ok(CommandResult::Html(html)) => {
                assert!(html.contains("gpt-4o"));
            }
            other => panic!("expected Html, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn model_with_args_updates() {
        let handler = SessionCommandHandler::new(Arc::new(MockClient));
        let result = handler
            .handle(&make_command("model", "claude-3"), &make_context())
            .await;
        match result {
            Ok(CommandResult::Html(html)) => {
                assert!(html.contains("Model updated"));
                assert!(html.contains("claude-3"));
            }
            other => panic!("expected Html, got {other:?}"),
        }
    }
}
