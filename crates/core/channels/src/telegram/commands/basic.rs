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

//! Basic command handlers: `/start` and `/help`.

use std::fmt::Write;

use async_trait::async_trait;
use rara_kernel::channel::command::{
    CommandContext, CommandDefinition, CommandHandler, CommandInfo, CommandResult,
};
use rara_kernel::error::KernelError;

/// Handles `/start` and `/help` commands.
///
/// Accepts a snapshot of all registered command definitions so that `/help`
/// can generate a complete listing.
pub struct BasicCommandHandler {
    all_commands: Vec<CommandDefinition>,
}

impl BasicCommandHandler {
    /// Create a new handler with the full list of available commands.
    pub fn new(all_commands: Vec<CommandDefinition>) -> Self {
        Self { all_commands }
    }
}

#[async_trait]
impl CommandHandler for BasicCommandHandler {
    fn commands(&self) -> Vec<CommandDefinition> {
        vec![
            CommandDefinition {
                name: "start".to_owned(),
                description: "Start the bot".to_owned(),
                usage: Some("/start".to_owned()),
            },
            CommandDefinition {
                name: "help".to_owned(),
                description: "Show available commands".to_owned(),
                usage: Some("/help".to_owned()),
            },
        ]
    }

    async fn handle(
        &self,
        command: &CommandInfo,
        _context: &CommandContext,
    ) -> Result<CommandResult, KernelError> {
        match command.name.as_str() {
            "start" => Ok(CommandResult::Text(
                "Welcome! I'm the Job Assistant bot.\n\
                 Send me any message to start a conversation.\n\n\
                 Use /help to see all available commands."
                    .to_owned(),
            )),
            "help" => {
                let mut text = String::from("<b>Available commands:</b>\n\n");
                for def in &self.all_commands {
                    let default_usage = format!("/{}", def.name);
                    let usage = def.usage.as_deref().unwrap_or(&default_usage);
                    let _ = writeln!(
                        text,
                        "<code>{usage}</code>\n  {}\n",
                        html_escape(&def.description),
                    );
                }
                Ok(CommandResult::Html(text))
            }
            _ => Ok(CommandResult::None),
        }
    }
}

/// Escape `&`, `<`, `>` for safe inclusion in HTML messages.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rara_kernel::channel::types::{ChannelType, ChannelUser};

    use super::*;

    fn make_context() -> CommandContext {
        CommandContext {
            channel_type: ChannelType::Telegram,
            session_key: "test-session".to_owned(),
            user: ChannelUser {
                platform_id: "123".to_owned(),
                display_name: Some("Test User".to_owned()),
            },
            metadata: HashMap::new(),
        }
    }

    fn make_command(name: &str) -> CommandInfo {
        CommandInfo {
            name: name.to_owned(),
            args: String::new(),
            raw: format!("/{name}"),
        }
    }

    fn sample_commands() -> Vec<CommandDefinition> {
        vec![
            CommandDefinition {
                name: "start".to_owned(),
                description: "Start the bot".to_owned(),
                usage: Some("/start".to_owned()),
            },
            CommandDefinition {
                name: "help".to_owned(),
                description: "Show available commands".to_owned(),
                usage: Some("/help".to_owned()),
            },
            CommandDefinition {
                name: "search".to_owned(),
                description: "Search for jobs".to_owned(),
                usage: Some("/search <keywords> [@ location]".to_owned()),
            },
        ]
    }

    #[tokio::test]
    async fn start_returns_welcome_text() {
        let handler = BasicCommandHandler::new(sample_commands());
        let result = handler.handle(&make_command("start"), &make_context()).await;
        match result {
            Ok(CommandResult::Text(text)) => {
                assert!(text.contains("Welcome"));
                assert!(text.contains("/help"));
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn help_lists_all_commands() {
        let handler = BasicCommandHandler::new(sample_commands());
        let result = handler.handle(&make_command("help"), &make_context()).await;
        match result {
            Ok(CommandResult::Html(html)) => {
                assert!(html.contains("/start"));
                assert!(html.contains("/help"));
                assert!(html.contains("/search"));
                assert!(html.contains("Search for jobs"));
            }
            other => panic!("expected Html, got {other:?}"),
        }
    }
}
