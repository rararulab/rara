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

//! Tape tree commands: `/anchors` and `/checkout`.

use std::sync::Arc;

use async_trait::async_trait;
use rara_kernel::{
    channel::command::{
        CommandContext, CommandDefinition, CommandHandler, CommandInfo, CommandResult,
    },
    error::KernelError,
    memory::SessionBranch,
};

use super::{anchor_dot, client::BotServiceClient};

/// Handles tape visualization and anchor-based forking.
pub struct TapeCommandHandler {
    client: Arc<dyn BotServiceClient>,
}

impl TapeCommandHandler {
    pub fn new(client: Arc<dyn BotServiceClient>) -> Self { Self { client } }
}

#[async_trait]
impl CommandHandler for TapeCommandHandler {
    fn commands(&self) -> Vec<CommandDefinition> {
        vec![
            CommandDefinition {
                name:        "anchors".to_owned(),
                description: "Show the anchor tree across forked sessions".to_owned(),
                usage:       Some("/anchors".to_owned()),
            },
            CommandDefinition {
                name:        "checkout".to_owned(),
                description: "Fork from an anchor or switch back to parent".to_owned(),
                usage:       Some("/checkout [anchor_name]".to_owned()),
            },
        ]
    }

    async fn handle(
        &self,
        command: &CommandInfo,
        context: &CommandContext,
    ) -> Result<CommandResult, KernelError> {
        match command.name.as_str() {
            "anchors" => self.handle_anchors(context).await,
            "checkout" => self.handle_checkout(&command.args, context).await,
            _ => Ok(CommandResult::None),
        }
    }
}

impl TapeCommandHandler {
    async fn handle_anchors(&self, context: &CommandContext) -> Result<CommandResult, KernelError> {
        let chat_id = extract_chat_id(context);
        let session_key = match self.client.get_channel_session(&chat_id).await {
            Ok(Some(binding)) => binding.session_key,
            Ok(None) => return Ok(CommandResult::Text("No active session.".to_owned())),
            Err(e) => {
                return Ok(CommandResult::Text(format!(
                    "Failed to resolve session: {e}"
                )));
            }
        };

        let tree = match self.client.anchor_tree(&session_key).await {
            Ok(tree) => tree,
            Err(e) => {
                return Ok(CommandResult::Text(format!(
                    "Failed to build anchor tree: {e}"
                )));
            }
        };
        let dot = anchor_dot::render_dot(&tree);
        let png = match anchor_dot::render_png(&dot) {
            Ok(png) => png,
            Err(e) => {
                // Explicitly no text fallback: /anchors is expected to return
                // an image or a hard error.
                return Ok(CommandResult::Text(format!(
                    "Failed to render anchor tree image: {e}"
                )));
            }
        };

        Ok(CommandResult::Photo {
            data:    png,
            caption: Some(format!(
                "Anchor tree ({} sessions)",
                count_sessions(&tree.root)
            )),
        })
    }

    async fn handle_checkout(
        &self,
        args: &str,
        context: &CommandContext,
    ) -> Result<CommandResult, KernelError> {
        let chat_id = extract_chat_id(context);
        let session_key = match self.client.get_channel_session(&chat_id).await {
            Ok(Some(binding)) => binding.session_key,
            Ok(None) => return Ok(CommandResult::Text("No active session.".to_owned())),
            Err(e) => {
                return Ok(CommandResult::Text(format!(
                    "Failed to resolve session: {e}"
                )));
            }
        };

        let anchor_name = args.trim();
        if anchor_name.is_empty() {
            // `/checkout` with no args means "go to parent session".
            let parent = match self.client.parent_session(&session_key).await {
                Ok(parent) => parent,
                Err(e) => {
                    return Ok(CommandResult::Text(format!(
                        "Failed to resolve parent session: {e}"
                    )));
                }
            };
            let Some(parent_key) = parent else {
                return Ok(CommandResult::Text(
                    "Current session has no parent session.".to_owned(),
                ));
            };
            if let Err(e) = self
                .client
                .bind_channel("telegram", &chat_id, &parent_key)
                .await
            {
                return Ok(CommandResult::Text(format!(
                    "Failed to switch to parent session: {e}"
                )));
            }
            return Ok(CommandResult::Text(format!(
                "Switched to parent session: {parent_key}"
            )));
        }

        // `/checkout <anchor>` creates a child fork and rebinds the chat.
        let new_session = match self.client.checkout_anchor(&session_key, anchor_name).await {
            Ok(new_session) => new_session,
            Err(e) => {
                return Ok(CommandResult::Text(format!(
                    "Failed to checkout anchor: {e}"
                )));
            }
        };

        if let Err(e) = self
            .client
            .bind_channel("telegram", &chat_id, &new_session)
            .await
        {
            return Ok(CommandResult::Text(format!(
                "Fork created but failed to bind channel: {e}"
            )));
        }

        Ok(CommandResult::Text(format!(
            "Forked from anchor '{anchor_name}' into session: {new_session}"
        )))
    }
}

fn count_sessions(branch: &SessionBranch) -> usize {
    1 + branch
        .forks
        .iter()
        .map(|fork| count_sessions(&fork.branch))
        .sum::<usize>()
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
