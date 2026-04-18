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
//! `/model`, `/rename`.

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
use teloxide::prelude::Requester;

use super::client::BotServiceClient;

/// Maximum number of sessions to display in the `/sessions` list.
const SESSIONS_LIST_LIMIT: u32 = 10;

/// Handles session management commands.
pub struct SessionCommandHandler {
    client: Arc<dyn BotServiceClient>,
    /// Telegram bot handle for direct API calls (e.g. deleting forum topics).
    /// `None` when running outside a Telegram context.
    bot:    Option<teloxide::Bot>,
}

impl SessionCommandHandler {
    pub fn new(client: Arc<dyn BotServiceClient>, bot: Option<teloxide::Bot>) -> Self {
        Self { client, bot }
    }
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
        let (channel_type, chat_id, thread_id) = extract_channel_info(context);

        match self.client.create_session(None).await {
            Ok(key) => {
                let _ = self
                    .client
                    .bind_channel(channel_type, &chat_id, &key, thread_id.as_deref())
                    .await;
                Ok(CommandResult::Text("New chat session started.".to_owned()))
            }
            Err(e) => Ok(CommandResult::Text(format!(
                "Failed to create session: {e}"
            ))),
        }
    }

    /// `/clear` — clear all messages in the current session.
    ///
    /// When executed inside a forum topic (`thread_id` is present), this also
    /// deletes the session + channel binding and removes the Telegram topic
    /// itself so the user gets a clean slate without leftover empty topics.
    async fn handle_clear(&self, context: &CommandContext) -> Result<CommandResult, KernelError> {
        let (channel_type, chat_id, thread_id) = extract_channel_info(context);

        match self
            .client
            .get_channel_session(channel_type, &chat_id, thread_id.as_deref())
            .await
        {
            Ok(Some(binding)) => {
                match self
                    .client
                    .clear_session_messages(&binding.session_key)
                    .await
                {
                    Ok(()) => {
                        // Inside a forum topic: fully tear down the session and
                        // delete the topic so there is no leftover empty thread.
                        if let Some(ref tid) = thread_id {
                            let _ = self.client.delete_session(&binding.session_key).await;
                            self.try_delete_forum_topic(&chat_id, tid).await;
                            // The topic is gone — any reply would fail, so return
                            // None to signal the adapter not to send a response.
                            return Ok(CommandResult::None);
                        }
                        Ok(CommandResult::Text("Session history cleared.".to_owned()))
                    }
                    Err(e) => Ok(CommandResult::Text(format!("Failed to clear: {e}"))),
                }
            }
            Ok(None) => Ok(CommandResult::Text(
                "No active session. Send a message to start one.".to_owned(),
            )),
            Err(e) => Ok(CommandResult::Text(format!("Error: {e}"))),
        }
    }

    /// Best-effort deletion of a Telegram forum topic.
    ///
    /// Silently ignores errors (missing permissions, invalid IDs) because the
    /// session data is already cleaned up at this point — failing to remove the
    /// UI topic is cosmetic, not critical.
    async fn try_delete_forum_topic(&self, chat_id: &str, thread_id: &str) {
        let Some(ref bot) = self.bot else { return };
        let Ok(chat_id_i64) = chat_id.parse::<i64>() else {
            return;
        };
        let Ok(tid_i32) = thread_id.parse::<i32>() else {
            return;
        };

        let thread = teloxide::types::ThreadId(teloxide::types::MessageId(tid_i32));
        let _ = bot
            .delete_forum_topic(teloxide::types::ChatId(chat_id_i64), thread)
            .await;
    }

    /// `/sessions` — list sessions as a pure inline keyboard.
    ///
    /// Active session shows a checkmark and triggers `detail:` callback;
    /// inactive sessions trigger `switch:` callback.
    async fn handle_sessions(
        &self,
        context: &CommandContext,
    ) -> Result<CommandResult, KernelError> {
        let (channel_type, chat_id, thread_id) = extract_channel_info(context);

        let active_key = match self
            .client
            .get_channel_session(channel_type, &chat_id, thread_id.as_deref())
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
                return Ok(CommandResult::Text(format!("Failed to list sessions: {e}")));
            }
        };

        if sessions.is_empty() {
            return Ok(CommandResult::Text(
                "No sessions found. Send a message to create one.".to_owned(),
            ));
        }

        let header = format!("\u{1f4cb} Sessions ({} total)", sessions.len());
        let mut keyboard_rows: Vec<Vec<InlineButton>> = Vec::new();

        for s in &sessions {
            let display_name = session_display_name(s);
            let is_active = active_key.as_deref() == Some(s.key.as_str());
            let time_ago = format_relative_time(&s.updated_at);
            let time_suffix = if time_ago.is_empty() {
                String::new()
            } else {
                format!(" \u{00b7} {time_ago}")
            };

            let (label, cb_data) = if is_active {
                (
                    format!("\u{2705} {display_name}{time_suffix}"),
                    format!("detail:{}", truncate_str(&s.key, 56)),
                )
            } else {
                (
                    format!("{display_name}{time_suffix}"),
                    format!("switch:{}", truncate_str(&s.key, 56)),
                )
            };

            keyboard_rows.push(vec![
                InlineButton {
                    text:          label,
                    callback_data: Some(cb_data),
                    url:           None,
                },
                InlineButton {
                    text:          "\u{1f5d1}".to_owned(),
                    callback_data: Some(format!("delete:{}", truncate_str(&s.key, 56))),
                    url:           None,
                },
            ]);
        }

        Ok(CommandResult::HtmlWithKeyboard {
            html:     header,
            keyboard: keyboard_rows,
        })
    }

    /// `/usage` — show details about the current session.
    async fn handle_usage(&self, context: &CommandContext) -> Result<CommandResult, KernelError> {
        let (channel_type, chat_id, thread_id) = extract_channel_info(context);

        let session_key = match self
            .client
            .get_channel_session(channel_type, &chat_id, thread_id.as_deref())
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
                let title = detail
                    .title
                    .as_deref()
                    .or(detail.preview.as_deref())
                    .unwrap_or("Untitled");
                let _ = writeln!(text, "<b>Session:</b> {}", html_escape(title));
                let _ = writeln!(
                    text,
                    "<b>Key:</b> <code>{}</code>",
                    html_escape(&detail.key)
                );
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
        let (channel_type, chat_id, thread_id) = extract_channel_info(context);

        let session_key = match self
            .client
            .get_channel_session(channel_type, &chat_id, thread_id.as_deref())
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
            // No args: render an inline keyboard listing available models so
            // the user can switch by tapping. The currently selected model is
            // highlighted with a check mark.
            let current_model = match self.client.get_session(&session_key).await {
                Ok(detail) => detail.model,
                Err(e) => {
                    return Ok(CommandResult::Text(format!(
                        "Failed to get session details: {e}"
                    )));
                }
            };

            match self.client.list_chat_models().await {
                Ok(models) if !models.is_empty() => {
                    let mut keyboard_rows: Vec<Vec<InlineButton>> = Vec::new();
                    for m in &models {
                        let is_current = current_model.as_deref() == Some(m.id.as_str());
                        let ctx_suffix = m
                            .context_length
                            .map(|c| format!(" \u{00b7} {}k", c / 1000))
                            .unwrap_or_default();
                        let label = if is_current {
                            format!("\u{2705} {}{ctx_suffix}", m.name)
                        } else {
                            format!("{}{ctx_suffix}", m.name)
                        };
                        keyboard_rows.push(vec![InlineButton {
                            text:          label,
                            callback_data: Some(format!("model:{}", truncate_str(&m.id, 56))),
                            url:           None,
                        }]);
                    }
                    let header = format!("\u{1f916} Models ({} total)", models.len());
                    Ok(CommandResult::HtmlWithKeyboard {
                        html:     header,
                        keyboard: keyboard_rows,
                    })
                }
                // Empty list or error: fall back to showing the current model
                // with a usage hint.
                _ => {
                    let model = current_model.as_deref().unwrap_or("(default)");
                    Ok(CommandResult::Html(format!(
                        "Model: <b>{}</b>\n\nSwitch: <code>/model model-name</code>",
                        html_escape(model),
                    )))
                }
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
        let (channel_type, chat_id, thread_id) = extract_channel_info(context);

        let session_key = match self
            .client
            .get_channel_session(channel_type, &chat_id, thread_id.as_deref())
            .await
        {
            Ok(Some(binding)) => binding.session_key,
            Ok(None) => {
                return Ok(CommandResult::Text("当前没有活跃的会话。".to_owned()));
            }
            Err(_) => {
                return Ok(CommandResult::Text("当前没有活跃的会话。".to_owned()));
            }
        };

        match rara_kernel::session::SessionKey::try_from_raw(&session_key) {
            Ok(key) => {
                let _ = self.handle.send_signal(key, Signal::Interrupt);
                Ok(CommandResult::Text("已中断当前操作。".to_owned()))
            }
            Err(_) => Ok(CommandResult::Text("当前没有活跃的会话。".to_owned())),
        }
    }
}

// ---------------------------------------------------------------------------
// RenameCommandHandler
// ---------------------------------------------------------------------------

/// Maximum length (in Unicode scalar values) for a renamed session title.
///
/// Matches Telegram's `editForumTopic` 128-character topic name cap so the
/// two layers stay consistent.
const RENAME_TITLE_MAX_CHARS: usize = 128;

/// Handles the `/rename <name>` command — sets the session title and
/// propagates the new label to the channel layer (e.g. renames the
/// Telegram forum topic).
pub struct RenameCommandHandler {
    client: Arc<dyn BotServiceClient>,
}

impl RenameCommandHandler {
    pub fn new(client: Arc<dyn BotServiceClient>) -> Self { Self { client } }
}

#[async_trait]
impl CommandHandler for RenameCommandHandler {
    fn commands(&self) -> Vec<CommandDefinition> {
        vec![CommandDefinition {
            name:        "rename".to_owned(),
            description: "Rename the current session (and its forum topic)".to_owned(),
            usage:       Some("/rename <name>".to_owned()),
        }]
    }

    async fn handle(
        &self,
        command: &CommandInfo,
        context: &CommandContext,
    ) -> Result<CommandResult, KernelError> {
        let title = command.args.trim();
        if title.is_empty() {
            return Ok(CommandResult::Text(
                "Usage: /rename <name>\nExample: /rename Project planning".to_owned(),
            ));
        }

        // Clip to the Telegram topic-name cap — both layers agree on the
        // same limit so the DB title matches the rendered topic name.
        let title: String = title.chars().take(RENAME_TITLE_MAX_CHARS).collect();

        let (channel_type, chat_id, thread_id) = extract_channel_info(context);

        let session_key = match self
            .client
            .get_channel_session(channel_type, &chat_id, thread_id.as_deref())
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

        match self.client.rename_session(&session_key, &title).await {
            Ok(_) => Ok(CommandResult::Html(format!(
                "\u{2705} Session renamed to \"{}\"",
                html_escape(&title),
            ))),
            Err(e) => Ok(CommandResult::Text(format!("Failed to rename: {e}"))),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract channel type string and chat_id from command context metadata.
///
/// For Telegram contexts the chat_id comes from the `telegram_chat_id`
/// metadata entry.  For CLI contexts the chat_id comes from `cli_chat_id`.
/// Falls back to `"unknown"` / `"0"` when the expected key is missing.
pub(crate) fn extract_channel_info(
    context: &CommandContext,
) -> (&'static str, String, Option<String>) {
    use rara_kernel::channel::types::ChannelType;

    match context.channel_type {
        ChannelType::Cli => {
            let chat_id = context
                .metadata
                .get("cli_chat_id")
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_else(|| "0".to_owned());
            ("cli", chat_id, None)
        }
        ChannelType::Telegram => {
            let chat_id = context
                .metadata
                .get("telegram_chat_id")
                .and_then(|v| {
                    v.as_i64()
                        .map(|n| n.to_string())
                        .or_else(|| v.as_str().map(String::from))
                })
                .unwrap_or_else(|| "0".to_owned());
            let thread_id = context.metadata.get("telegram_thread_id").and_then(|v| {
                v.as_i64()
                    .map(|n| n.to_string())
                    .or_else(|| v.as_str().map(String::from))
            });
            ("telegram", chat_id, thread_id)
        }
        _ => {
            let chat_id = context
                .metadata
                .get("chat_id")
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_else(|| "0".to_owned());
            ("unknown", chat_id, None)
        }
    }
}

pub(crate) fn html_escape(s: &str) -> String {
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

pub(crate) fn format_timestamp(raw: &str) -> String {
    if raw.len() >= 16 {
        let date_part = &raw[..10];
        let time_part = &raw[11..16];
        if !time_part.is_empty() {
            return format!("{date_part} {time_part}");
        }
    }
    raw.to_owned()
}

/// Format an ISO-8601 timestamp as a relative duration from now.
///
/// Returns compact strings like `"now"`, `"3m ago"`, `"2h ago"`, `"5d ago"`.
fn format_relative_time(updated_at: &str) -> String {
    let Ok(ts) = chrono::DateTime::parse_from_rfc3339(updated_at) else {
        return String::new();
    };
    let delta = chrono::Utc::now().signed_duration_since(ts);
    let secs = delta.num_seconds();
    if secs < 60 {
        "now".to_owned()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        let days = secs / 86400;
        if days > 30 {
            "30d+ ago".to_owned()
        } else {
            format!("{days}d ago")
        }
    }
}

/// Resolve a human-readable display name for a session.
///
/// Priority: title -> preview (truncated 20 chars) -> "Untitled".
fn session_display_name(s: &super::client::SessionListItem) -> String {
    if let Some(ref title) = s.title {
        return truncate_str(title, 30).to_owned();
    }
    if let Some(ref preview) = s.preview {
        return truncate_str(preview, 20).to_owned();
    }
    "Untitled".to_owned()
}
