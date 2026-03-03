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

//! Coding task commands: `/code` and `/tasks`.

use std::sync::Arc;

use async_trait::async_trait;
use rara_kernel::{
    channel::command::{
        CommandContext, CommandDefinition, CommandHandler, CommandInfo, CommandResult,
    },
    error::KernelError,
};

use super::client::BotServiceClient;

/// Handles `/code` and `/tasks` commands.
pub struct CodingCommandHandler {
    client: Arc<dyn BotServiceClient>,
}

impl CodingCommandHandler {
    pub fn new(client: Arc<dyn BotServiceClient>) -> Self { Self { client } }
}

#[async_trait]
impl CommandHandler for CodingCommandHandler {
    fn commands(&self) -> Vec<CommandDefinition> {
        vec![
            CommandDefinition {
                name:        "code".to_owned(),
                description: "Dispatch a coding task".to_owned(),
                usage:       Some("/code <prompt>".to_owned()),
            },
            CommandDefinition {
                name:        "tasks".to_owned(),
                description: "List coding tasks".to_owned(),
                usage:       Some("/tasks".to_owned()),
            },
        ]
    }

    async fn handle(
        &self,
        command: &CommandInfo,
        _context: &CommandContext,
    ) -> Result<CommandResult, KernelError> {
        match command.name.as_str() {
            "code" => self.handle_code(&command.args).await,
            "tasks" => self.handle_tasks().await,
            _ => Ok(CommandResult::None),
        }
    }
}

impl CodingCommandHandler {
    /// `/code <prompt>` — dispatch a coding task.
    async fn handle_code(&self, args: &str) -> Result<CommandResult, KernelError> {
        let prompt = args.trim();
        if prompt.is_empty() {
            return Ok(CommandResult::Text(
                "Usage: /code <prompt>\n\nExample: /code fix the login bug".to_owned(),
            ));
        }

        match self.client.dispatch_coding_task(prompt, "Claude").await {
            Ok(task) => Ok(CommandResult::Html(format!(
                "\u{1F680} Coding task dispatched!\n\nID: <code>{}</code>\nBranch: \
                 <code>{}</code>\nTmux: <code>{}</code>\nStatus: {}\n\nYou'll be notified when it \
                 completes.",
                task.id, task.branch, task.tmux_session, task.status,
            ))),
            Err(e) => Ok(CommandResult::Text(format!(
                "\u{274c} Failed to dispatch task: {e}"
            ))),
        }
    }

    /// `/tasks` — list coding tasks.
    async fn handle_tasks(&self) -> Result<CommandResult, KernelError> {
        match self.client.list_coding_tasks().await {
            Ok(tasks) if tasks.is_empty() => Ok(CommandResult::Text(
                "No coding tasks found.\n\nUse /code <prompt> to dispatch one.".to_owned(),
            )),
            Ok(tasks) => {
                let mut text = format!("\u{1F4CB} <b>Coding Tasks</b> ({})\n\n", tasks.len());
                for t in tasks.iter().take(10) {
                    let status_emoji = match t.status.as_str() {
                        "Pending" => "\u{23F3}",
                        "Cloning" => "\u{1F4E6}",
                        "Running" => "\u{1F3C3}",
                        "Completed" => "\u{2705}",
                        "Failed" => "\u{274c}",
                        "Merged" => "\u{1F389}",
                        "MergeFailed" => "\u{26A0}",
                        _ => "\u{2753}",
                    };
                    let short_id = if t.id.len() > 8 { &t.id[..8] } else { &t.id };
                    let prompt_short = if t.prompt.len() > 60 {
                        format!("{}...", &t.prompt[..60])
                    } else {
                        t.prompt.clone()
                    };
                    let pr = t
                        .pr_url
                        .as_deref()
                        .map_or(String::new(), |url| format!("\n   PR: {url}"));
                    text.push_str(&format!(
                        "{status_emoji} <code>{short_id}</code> [{}] {prompt_short}{pr}\n\n",
                        t.agent_type,
                    ));
                }
                if tasks.len() > 10 {
                    text.push_str(&format!("... and {} more", tasks.len() - 10));
                }
                Ok(CommandResult::Html(text))
            }
            Err(e) => Ok(CommandResult::Text(format!(
                "\u{274c} Failed to list tasks: {e}"
            ))),
        }
    }
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

    struct MockCodingClient {
        has_tasks: bool,
    }

    #[async_trait]
    impl BotServiceClient for MockCodingClient {
        async fn get_channel_session(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Option<ChannelBinding>, BotServiceError> {
            Ok(None)
        }

        async fn bind_channel(
            &self,
            _: &str,
            _: &str,
            _: &str,
            k: &str,
        ) -> Result<ChannelBinding, BotServiceError> {
            Ok(ChannelBinding {
                session_key: k.to_owned(),
            })
        }

        async fn create_session(&self, _: Option<&str>) -> Result<String, BotServiceError> {
            Ok("mock-session-key".to_owned())
        }

        async fn clear_session_messages(&self, _: &str) -> Result<(), BotServiceError> { Ok(()) }

        async fn list_sessions(&self, _: u32) -> Result<Vec<SessionListItem>, BotServiceError> {
            Ok(vec![])
        }

        async fn get_session(&self, _: &str) -> Result<SessionDetail, BotServiceError> {
            Err(BotServiceError::Service {
                message: "n/a".into(),
            })
        }

        async fn update_session(
            &self,
            _: &str,
            _: Option<&str>,
        ) -> Result<SessionDetail, BotServiceError> {
            Err(BotServiceError::Service {
                message: "n/a".into(),
            })
        }

        async fn discover_jobs(
            &self,
            _: Vec<String>,
            _: Option<String>,
            _: u32,
        ) -> Result<Vec<DiscoveryJob>, BotServiceError> {
            Ok(vec![])
        }

        async fn submit_jd_parse(&self, _: &str) -> Result<(), BotServiceError> { Ok(()) }

        async fn list_mcp_servers(&self) -> Result<Vec<McpServerInfo>, BotServiceError> {
            Ok(vec![])
        }

        async fn get_mcp_server(&self, _: &str) -> Result<McpServerInfo, BotServiceError> {
            Err(BotServiceError::Service {
                message: "n/a".into(),
            })
        }

        async fn add_mcp_server(
            &self,
            _: &str,
            _: &str,
            _: &[String],
        ) -> Result<McpServerInfo, BotServiceError> {
            Err(BotServiceError::Service {
                message: "n/a".into(),
            })
        }

        async fn start_mcp_server(&self, _: &str) -> Result<(), BotServiceError> { Ok(()) }

        async fn remove_mcp_server(&self, _: &str) -> Result<(), BotServiceError> { Ok(()) }

        async fn dispatch_coding_task(
            &self,
            _prompt: &str,
            _agent: &str,
        ) -> Result<CodingTask, BotServiceError> {
            Ok(CodingTask {
                id:           "task-001".to_owned(),
                branch:       "fix/login-bug".to_owned(),
                tmux_session: "task-001-session".to_owned(),
                status:       "Pending".to_owned(),
            })
        }

        async fn list_coding_tasks(&self) -> Result<Vec<CodingTaskSummary>, BotServiceError> {
            if self.has_tasks {
                Ok(vec![CodingTaskSummary {
                    id:         "task-001".to_owned(),
                    status:     "Running".to_owned(),
                    agent_type: "Claude".to_owned(),
                    branch:     "fix/login-bug".to_owned(),
                    prompt:     "fix the login bug".to_owned(),
                    pr_url:     None,
                }])
            } else {
                Ok(vec![])
            }
        }
    }

    fn make_context() -> CommandContext {
        CommandContext {
            channel_type: ChannelType::Telegram,
            session_key:  "tg:123".to_owned(),
            user:         ChannelUser {
                platform_id:  "123".to_owned(),
                display_name: Some("Test".to_owned()),
            },
            metadata:     HashMap::new(),
        }
    }

    #[tokio::test]
    async fn code_dispatches_task() {
        let handler = CodingCommandHandler::new(Arc::new(MockCodingClient { has_tasks: false }));
        let cmd = CommandInfo {
            name: "code".to_owned(),
            args: "fix the login bug".to_owned(),
            raw:  "/code fix the login bug".to_owned(),
        };
        let result = handler.handle(&cmd, &make_context()).await;
        match result {
            Ok(CommandResult::Html(html)) => {
                assert!(html.contains("task-001"));
                assert!(html.contains("fix/login-bug"));
                assert!(html.contains("Pending"));
            }
            other => panic!("expected Html, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn code_empty_args_shows_usage() {
        let handler = CodingCommandHandler::new(Arc::new(MockCodingClient { has_tasks: false }));
        let cmd = CommandInfo {
            name: "code".to_owned(),
            args: String::new(),
            raw:  "/code".to_owned(),
        };
        let result = handler.handle(&cmd, &make_context()).await;
        match result {
            Ok(CommandResult::Text(text)) => {
                assert!(text.contains("Usage"));
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn tasks_empty_list() {
        let handler = CodingCommandHandler::new(Arc::new(MockCodingClient { has_tasks: false }));
        let cmd = CommandInfo {
            name: "tasks".to_owned(),
            args: String::new(),
            raw:  "/tasks".to_owned(),
        };
        let result = handler.handle(&cmd, &make_context()).await;
        match result {
            Ok(CommandResult::Text(text)) => {
                assert!(text.contains("No coding tasks"));
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn tasks_with_items() {
        let handler = CodingCommandHandler::new(Arc::new(MockCodingClient { has_tasks: true }));
        let cmd = CommandInfo {
            name: "tasks".to_owned(),
            args: String::new(),
            raw:  "/tasks".to_owned(),
        };
        let result = handler.handle(&cmd, &make_context()).await;
        match result {
            Ok(CommandResult::Html(html)) => {
                assert!(html.contains("task-001"));
                assert!(html.contains("Claude"));
                assert!(html.contains("fix the login bug"));
            }
            other => panic!("expected Html, got {other:?}"),
        }
    }
}
