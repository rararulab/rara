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

//! Gateway operations command: `/gateway`.

use std::{collections::HashSet, fmt::Write, sync::Arc};

use async_trait::async_trait;
use rara_kernel::{
    channel::command::{
        CommandContext, CommandDefinition, CommandHandler, CommandInfo, CommandResult,
    },
    error::KernelError,
};

use super::client::{BotServiceClient, GatewayCommandOutcome, GatewayStatus};

/// Handles owner-only gateway operations exposed through Telegram.
pub struct GatewayOpsCommandHandler {
    client: Arc<dyn BotServiceClient>,
    allowed_user_ids: HashSet<String>,
}

impl GatewayOpsCommandHandler {
    pub fn new(client: Arc<dyn BotServiceClient>, allowed_user_ids: HashSet<String>) -> Self {
        Self {
            client,
            allowed_user_ids,
        }
    }

    fn is_allowed(&self, context: &CommandContext) -> bool {
        self.allowed_user_ids.contains(&context.user.platform_id)
    }

    fn unauthorized_result() -> CommandResult {
        CommandResult::Text(
            "This command is restricted to configured owner/admin Telegram users.".to_owned(),
        )
    }
}

#[async_trait]
impl CommandHandler for GatewayOpsCommandHandler {
    fn commands(&self) -> Vec<CommandDefinition> {
        vec![CommandDefinition {
            name: "gateway".to_owned(),
            description: "Owner-only gateway operations: status, restart, update".to_owned(),
            usage: Some("/gateway [status|restart|update]".to_owned()),
        }]
    }

    async fn handle(
        &self,
        command: &CommandInfo,
        context: &CommandContext,
    ) -> Result<CommandResult, KernelError> {
        if !self.is_allowed(context) {
            return Ok(Self::unauthorized_result());
        }

        let subcommand = command.args.split_whitespace().next().unwrap_or("status");
        match subcommand {
            "status" | "" => self.handle_status().await,
            "restart" => self.handle_restart().await,
            "update" => self.handle_update().await,
            _ => Ok(CommandResult::Html(
                "<b>Gateway commands</b>\n<code>/gateway status</code>\n<code>/gateway restart</code>\n<code>/gateway update</code>".to_owned(),
            )),
        }
    }
}

impl GatewayOpsCommandHandler {
    async fn handle_status(&self) -> Result<CommandResult, KernelError> {
        match self.client.gateway_status().await {
            Ok(status) => Ok(CommandResult::Html(render_status(&status))),
            Err(e) => Ok(CommandResult::Text(format!(
                "Failed to fetch gateway status: {e}"
            ))),
        }
    }

    async fn handle_restart(&self) -> Result<CommandResult, KernelError> {
        match self.client.gateway_restart().await {
            Ok(outcome) => Ok(CommandResult::Html(render_outcome(&outcome))),
            Err(e) => Ok(CommandResult::Text(format!(
                "Failed to request restart: {e}"
            ))),
        }
    }

    async fn handle_update(&self) -> Result<CommandResult, KernelError> {
        match self.client.gateway_update().await {
            Ok(outcome) => Ok(CommandResult::Html(render_outcome(&outcome))),
            Err(e) => Ok(CommandResult::Text(format!(
                "Failed to request update: {e}"
            ))),
        }
    }
}

fn render_status(status: &GatewayStatus) -> String {
    let mut text = String::from("<b>Gateway status</b>\n");
    let running = if status.agent.running { "yes" } else { "no" };
    let pid = status
        .agent
        .pid
        .map(|pid| pid.to_string())
        .unwrap_or_else(|| "n/a".to_owned());
    let upstream = status
        .update
        .upstream_rev
        .as_deref()
        .map(short_rev)
        .unwrap_or("unknown");
    let last_check = status
        .update
        .last_check_time
        .as_deref()
        .map(format_timestamp)
        .unwrap_or_else(|| "never".to_owned());

    let _ = writeln!(text, "<b>Agent running:</b> {}", running);
    let _ = writeln!(text, "<b>PID:</b> <code>{}</code>", pid);
    let _ = writeln!(text, "<b>Restart count:</b> {}", status.agent.restart_count);
    let _ = writeln!(
        text,
        "<b>Current rev:</b> <code>{}</code>",
        html_escape(short_rev(&status.update.current_rev))
    );
    let _ = writeln!(
        text,
        "<b>Upstream rev:</b> <code>{}</code>",
        html_escape(upstream)
    );
    let _ = writeln!(
        text,
        "<b>Update available:</b> {}",
        if status.update.update_available {
            "yes"
        } else {
            "no"
        }
    );
    let _ = writeln!(text, "<b>Last check:</b> {}", html_escape(&last_check));
    let _ = write!(
        text,
        "\n<code>/gateway restart</code>\n<code>/gateway update</code>"
    );

    text
}

fn render_outcome(outcome: &GatewayCommandOutcome) -> String {
    let mut text = String::new();
    let headline = if outcome.ok {
        "accepted"
    } else {
        "completed with issues"
    };
    let _ = writeln!(
        text,
        "<b>Gateway {}</b>: {}",
        html_escape(&outcome.action),
        headline
    );
    let _ = writeln!(
        text,
        "<b>Status:</b> <code>{}</code>",
        html_escape(&outcome.status)
    );
    let _ = writeln!(text, "<b>Detail:</b> {}", html_escape(&outcome.detail));
    if let Some(target_rev) = outcome.target_rev.as_deref() {
        let _ = writeln!(
            text,
            "<b>Target rev:</b> <code>{}</code>",
            html_escape(short_rev(target_rev))
        );
    }
    if let Some(active_rev) = outcome.active_rev.as_deref() {
        let _ = writeln!(
            text,
            "<b>Active rev:</b> <code>{}</code>",
            html_escape(short_rev(active_rev))
        );
    }
    if let Some(rolled_back) = outcome.rolled_back {
        let _ = writeln!(
            text,
            "<b>Rolled back:</b> {}",
            if rolled_back { "yes" } else { "no" }
        );
    }
    let _ = write!(
        text,
        "\nProgress details are also emitted through the configured gateway notification channel."
    );
    text
}

fn short_rev(rev: &str) -> &str {
    if rev.len() > 12 { &rev[..12] } else { rev }
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

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use super::*;
    use crate::telegram::commands::client::{
        BotServiceError, ChannelBinding, DiscoveryJob, GatewayAgentStatus, GatewayUpdateStatus,
        McpServerInfo, SessionDetail, SessionListItem,
    };

    struct FakeBotServiceClient {
        status_calls: Mutex<u32>,
        restart_calls: Mutex<u32>,
    }

    impl FakeBotServiceClient {
        fn new() -> Self {
            Self {
                status_calls: Mutex::new(0),
                restart_calls: Mutex::new(0),
            }
        }
    }

    #[async_trait]
    impl BotServiceClient for FakeBotServiceClient {
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

        async fn gateway_status(&self) -> Result<GatewayStatus, BotServiceError> {
            *self.status_calls.lock().unwrap() += 1;
            Ok(GatewayStatus {
                agent: GatewayAgentStatus {
                    running: true,
                    restart_count: 2,
                    pid: Some(4242),
                },
                update: GatewayUpdateStatus {
                    current_rev: "abcdef1234567890".to_owned(),
                    upstream_rev: Some("fedcba0987654321".to_owned()),
                    update_available: true,
                    last_check_time: Some("2026-03-09T07:55:00+00:00".to_owned()),
                },
            })
        }

        async fn gateway_restart(&self) -> Result<GatewayCommandOutcome, BotServiceError> {
            *self.restart_calls.lock().unwrap() += 1;
            Ok(GatewayCommandOutcome {
                ok: true,
                action: "restart".to_owned(),
                status: "accepted".to_owned(),
                detail: "restart command sent".to_owned(),
                target_rev: None,
                active_rev: None,
                rolled_back: None,
            })
        }

        async fn gateway_update(&self) -> Result<GatewayCommandOutcome, BotServiceError> {
            unreachable!()
        }
    }

    fn context(user_id: &str) -> CommandContext {
        CommandContext {
            channel_type: rara_kernel::channel::types::ChannelType::Telegram,
            session_key: String::new(),
            user: rara_kernel::channel::types::ChannelUser {
                platform_id: user_id.to_owned(),
                display_name: None,
            },
            metadata: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn rejects_non_owner_user() {
        let handler = GatewayOpsCommandHandler::new(
            Arc::new(FakeBotServiceClient::new()),
            HashSet::from(["owner-1".to_owned()]),
        );
        let result = handler
            .handle(
                &CommandInfo {
                    name: "gateway".to_owned(),
                    args: "status".to_owned(),
                    raw: "/gateway status".to_owned(),
                },
                &context("random-user"),
            )
            .await
            .expect("command result");

        match result {
            CommandResult::Text(text) => assert!(text.contains("restricted")),
            other => panic!("unexpected result: {other:?}"),
        }
    }

    #[tokio::test]
    async fn status_defaults_when_no_subcommand_is_given() {
        let client = Arc::new(FakeBotServiceClient::new());
        let handler =
            GatewayOpsCommandHandler::new(client.clone(), HashSet::from(["owner-1".to_owned()]));
        let result = handler
            .handle(
                &CommandInfo {
                    name: "gateway".to_owned(),
                    args: String::new(),
                    raw: "/gateway".to_owned(),
                },
                &context("owner-1"),
            )
            .await
            .expect("command result");

        assert_eq!(*client.status_calls.lock().unwrap(), 1);
        match result {
            CommandResult::Html(html) => {
                assert!(html.contains("Gateway status"));
                assert!(html.contains("abcdef123456"));
            }
            other => panic!("unexpected result: {other:?}"),
        }
    }

    #[tokio::test]
    async fn restart_calls_gateway_client() {
        let client = Arc::new(FakeBotServiceClient::new());
        let handler =
            GatewayOpsCommandHandler::new(client.clone(), HashSet::from(["owner-1".to_owned()]));
        let result = handler
            .handle(
                &CommandInfo {
                    name: "gateway".to_owned(),
                    args: "restart".to_owned(),
                    raw: "/gateway restart".to_owned(),
                },
                &context("owner-1"),
            )
            .await
            .expect("command result");

        assert_eq!(*client.restart_calls.lock().unwrap(), 1);
        match result {
            CommandResult::Html(html) => {
                assert!(html.contains("Gateway restart"));
                assert!(html.contains("accepted"));
            }
            other => panic!("unexpected result: {other:?}"),
        }
    }
}
