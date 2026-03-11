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

//! MCP server management command: `/mcp`.

use std::{fmt::Write, sync::Arc};

use async_trait::async_trait;
use rara_kernel::{
    channel::command::{
        CommandContext, CommandDefinition, CommandHandler, CommandInfo, CommandResult,
    },
    error::KernelError,
};

use super::client::{BotServiceClient, McpServerStatus};

/// Handles the `/mcp` command for MCP server management.
pub struct McpCommandHandler {
    client: Arc<dyn BotServiceClient>,
}

impl McpCommandHandler {
    pub fn new(client: Arc<dyn BotServiceClient>) -> Self { Self { client } }
}

#[async_trait]
impl CommandHandler for McpCommandHandler {
    fn commands(&self) -> Vec<CommandDefinition> {
        vec![CommandDefinition {
            name:        "mcp".to_owned(),
            description: "List MCP servers or install a new one".to_owned(),
            usage:       Some("/mcp [url|name]".to_owned()),
        }]
    }

    async fn handle(
        &self,
        command: &CommandInfo,
        _context: &CommandContext,
    ) -> Result<CommandResult, KernelError> {
        let args = command.args.trim();

        if args.is_empty() {
            return self.show_status().await;
        }

        self.install_or_restart(args).await
    }
}

impl McpCommandHandler {
    /// List all configured MCP servers with status icons.
    async fn show_status(&self) -> Result<CommandResult, KernelError> {
        let servers = match self.client.list_mcp_servers().await {
            Ok(s) => s,
            Err(e) => {
                return Ok(CommandResult::Text(format!(
                    "Failed to fetch MCP status: {e}"
                )));
            }
        };

        if servers.is_empty() {
            return Ok(CommandResult::Text("No MCP servers configured.".to_owned()));
        }

        let mut text = format!("<b>MCP Servers</b> ({})\n\n", servers.len());
        for s in &servers {
            let (icon, status_text) = match &s.status {
                McpServerStatus::Connected => ("\u{25CF}", "connected".to_owned()),
                McpServerStatus::Connecting => ("\u{25D0}", "connecting".to_owned()),
                McpServerStatus::Disconnected => ("\u{25CB}", "disconnected".to_owned()),
                McpServerStatus::Error { message } => {
                    ("\u{2718}", format!("error: {}", html_escape(message)))
                }
            };
            let extra = if s.name == "context-mode" {
                if matches!(s.status, McpServerStatus::Connected) {
                    " (interceptor: \u{2713})"
                } else {
                    " (interceptor: \u{2717})"
                }
            } else {
                ""
            };
            let _ = writeln!(
                text,
                "{icon} <b>{}</b> \u{2014} {status_text}{extra}",
                html_escape(&s.name)
            );
        }

        let connected = servers
            .iter()
            .filter(|s| matches!(s.status, McpServerStatus::Connected))
            .count();
        let _ = write!(text, "\n{connected}/{} connected", servers.len());

        Ok(CommandResult::Html(text))
    }

    /// Install a new MCP server or restart an existing one.
    async fn install_or_restart(&self, input: &str) -> Result<CommandResult, KernelError> {
        let package_name = extract_mcp_package_name(input);

        // Check if already installed.
        if let Ok(servers) = self.client.list_mcp_servers().await {
            if let Some(existing) = servers.iter().find(|s| s.name == package_name) {
                let status = match &existing.status {
                    McpServerStatus::Connected => "connected",
                    McpServerStatus::Connecting => "connecting",
                    McpServerStatus::Disconnected => "disconnected",
                    McpServerStatus::Error { .. } => "error",
                };

                if matches!(
                    existing.status,
                    McpServerStatus::Disconnected | McpServerStatus::Error { .. }
                ) {
                    let _ = self.client.start_mcp_server(&package_name).await;
                    return Ok(CommandResult::Html(format!(
                        "<b>{}</b> already configured (was {status}), restarting...",
                        html_escape(&package_name),
                    )));
                }

                return Ok(CommandResult::Html(format!(
                    "<b>{}</b> already configured ({status}).",
                    html_escape(&package_name),
                )));
            }
        }

        // Not found — install it.
        let args = vec!["-y".to_owned(), package_name.clone()];
        if let Err(e) = self
            .client
            .add_mcp_server(&package_name, "npx", &args)
            .await
        {
            return Ok(CommandResult::Text(format!(
                "Failed to install {}: {e}",
                html_escape(&package_name),
            )));
        }

        // Poll status — check every 2s up to 5 times (10s total).
        let mut final_status = None;
        for _ in 0..5 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            match self.client.get_mcp_server(&package_name).await {
                Ok(info) => match &info.status {
                    McpServerStatus::Connected => {
                        final_status = Some(info.status);
                        break;
                    }
                    McpServerStatus::Error { .. } => {
                        final_status = Some(info.status);
                        break;
                    }
                    _ => {
                        final_status = Some(info.status);
                    }
                },
                Err(_) => break,
            }
        }

        match final_status {
            Some(McpServerStatus::Connected) => Ok(CommandResult::Html(format!(
                "<b>{}</b> installed and connected.",
                html_escape(&package_name),
            ))),
            Some(McpServerStatus::Error { message }) => {
                let _ = self.client.remove_mcp_server(&package_name).await;
                Ok(CommandResult::Html(format!(
                    "Failed to start <b>{}</b>: {}\nConfig removed.",
                    html_escape(&package_name),
                    html_escape(&message),
                )))
            }
            _ => Ok(CommandResult::Html(format!(
                "<b>{}</b> added, still connecting. Use /mcp to check status later.",
                html_escape(&package_name),
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract an MCP package name from a GitHub URL or plain string.
fn extract_mcp_package_name(input: &str) -> String {
    // Try to detect github.com URLs by simple string parsing.
    if input.contains("github.com/") {
        // Extract: https://github.com/org/repo-name[.git]
        if let Some(rest) = input.split("github.com/").nth(1) {
            // rest = "org/repo-name" or "org/repo-name.git"
            if let Some(repo) = rest.split('/').nth(1) {
                let name = repo
                    .trim_end_matches(".git")
                    .trim_end_matches('/')
                    .split('?')
                    .next()
                    .unwrap_or(repo);
                if !name.is_empty() {
                    return name.to_owned();
                }
            }
        }
    }
    input.to_owned()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
