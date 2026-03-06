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

//! Session management commands: `/new`, `/clear`, `/sessions`, `/usage`,
//! `/model`.

use std::{fmt::Write, sync::Arc};

use async_trait::async_trait;
use rara_kernel::{
    channel::{
        command::{CommandContext, CommandDefinition, CommandHandler, CommandInfo, CommandResult},
        types::InlineButton,
    },
    error::KernelError,
    handle::KernelHandle,
    session::Signal,
};

use super::client::BotServiceClient;

/// Maximum number of sessions to display in the `/sessions` list.
const SESSIONS_LIST_LIMIT: u32 = 10;

/// Handles session management commands.
pub struct SessionCommandHandler {
    client: Arc<dyn BotServiceClient>,
}

impl SessionCommandHandler {
    pub fn new(client: Arc<dyn BotServiceClient>) -> Self { Self { client } }
}

#[async_trait]
impl CommandHandler for SessionCommandHandler {
    fn commands(&self) -> Vec<CommandDefinition> {
        vec![
            CommandDefinition {
                name:        "new".to_owned(),
                description: "Start a new chat session".to_owned(),
                usage:       Some("/new".to_owned()),
            },
            CommandDefinition {
                name:        "clear".to_owned(),
                description: "Clear current session history".to_owned(),
                usage:       Some("/clear".to_owned()),
            },
            CommandDefinition {
                name:        "sessions".to_owned(),
                description: "List and switch chat sessions".to_owned(),
                usage:       Some("/sessions".to_owned()),
            },
            CommandDefinition {
                name:        "usage".to_owned(),
                description: "Show current session info".to_owned(),
                usage:       Some("/usage".to_owned()),
            },
            CommandDefinition {
                name:        "model".to_owned(),
                description: "Show or switch the AI model".to_owned(),
                usage:       Some("/model [name]".to_owned()),
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
    async fn handle_new(&self, context: &CommandContext) -> Result<CommandResult, KernelError> {
        let chat_id = extract_chat_id(context);

        match self.client.create_session(Some("Telegram Chat")).await {
            Ok(key) => {
                let _ = self.client.bind_channel("telegram", &chat_id, &key).await;
                Ok(CommandResult::Text("New chat session started.".to_owned()))
            }
            Err(e) => Ok(CommandResult::Text(format!(
                "Failed to create session: {e}"
            ))),
        }
    }

    /// `/clear` — clear all messages in the current session.
    async fn handle_clear(&self, context: &CommandContext) -> Result<CommandResult, KernelError> {
        let chat_id = extract_chat_id(context);

        match self.client.get_channel_session(&chat_id).await {
            Ok(Some(binding)) => {
                match self
                    .client
                    .clear_session_messages(&binding.session_key)
                    .await
                {
                    Ok(()) => Ok(CommandResult::Text("Session history cleared.".to_owned())),
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

        // Find the currently active session key.
        let active_key = match self.client.get_channel_session(&chat_id).await {
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
                return Ok(CommandResult::Text(format!("Failed to list sessions: {e}")));
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
                    text:          label,
                    callback_data: Some(cb_data),
                    url:           None,
                }]);
            }
        }

        if keyboard_rows.is_empty() {
            Ok(CommandResult::Html(text))
        } else {
            Ok(CommandResult::HtmlWithKeyboard {
                html:     text,
                keyboard: keyboard_rows,
            })
        }
    }

    /// `/usage` — show details about the current session.
    async fn handle_usage(&self, context: &CommandContext) -> Result<CommandResult, KernelError> {
        let chat_id = extract_chat_id(context);

        let session_key = match self.client.get_channel_session(&chat_id).await {
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
                    let _ = writeln!(text, "\n<b>Last message:</b>\n{}", html_escape(truncated));
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

        let session_key = match self.client.get_channel_session(&chat_id).await {
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
                        "Session <code>{}</code>\nModel: <b>{}</b>\n\nSwitch: <code>/model \
                         model-name</code>",
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
                Err(e) => Ok(CommandResult::Text(format!("Failed to update model: {e}"))),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// StopCommandHandler
// ---------------------------------------------------------------------------

/// Handles the `/stop` command — sends an interrupt signal to the active
/// session so the kernel cancels the in-progress LLM turn.
pub struct StopCommandHandler {
    client: Arc<dyn BotServiceClient>,
    handle: KernelHandle,
}

impl StopCommandHandler {
    pub fn new(client: Arc<dyn BotServiceClient>, handle: KernelHandle) -> Self {
        Self { client, handle }
    }
}

#[async_trait]
impl CommandHandler for StopCommandHandler {
    fn commands(&self) -> Vec<CommandDefinition> {
        vec![CommandDefinition {
            name:        "stop".to_owned(),
            description: "Interrupt the current operation".to_owned(),
            usage:       Some("/stop".to_owned()),
        }]
    }

    async fn handle(
        &self,
        _command: &CommandInfo,
        context: &CommandContext,
    ) -> Result<CommandResult, KernelError> {
        let chat_id = extract_chat_id(context);

        let session_key = match self.client.get_channel_session(&chat_id).await {
            Ok(Some(binding)) => binding.session_key,
            Ok(None) => {
                return Ok(CommandResult::Text(
                    "当前没有活跃的会话。".to_owned(),
                ));
            }
            Err(_) => {
                return Ok(CommandResult::Text(
                    "当前没有活跃的会话。".to_owned(),
                ));
            }
        };

        match rara_kernel::session::SessionKey::try_from_raw(&session_key) {
            Ok(key) => {
                let _ = self.handle.send_signal(key, Signal::Interrupt);
                Ok(CommandResult::Text("已中断当前操作。".to_owned()))
            }
            Err(_) => Ok(CommandResult::Text(
                "当前没有活跃的会话。".to_owned(),
            )),
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
