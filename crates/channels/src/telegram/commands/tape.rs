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

use super::{
    anchor_dot,
    client::{BotServiceClient, CheckoutResult},
    session::extract_channel_info,
};

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
        let (channel_type, chat_id, thread_id) = extract_channel_info(context);
        let session_key = match self
            .client
            .get_channel_session(channel_type, &chat_id, thread_id.as_deref())
            .await
        {
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
        let (channel_type, chat_id, thread_id) = extract_channel_info(context);
        let session_key = match self
            .client
            .get_channel_session(channel_type, &chat_id, thread_id.as_deref())
            .await
        {
            Ok(Some(binding)) => binding.session_key,
            Ok(None) => return Ok(CommandResult::Text("No active session.".to_owned())),
            Err(e) => {
                return Ok(CommandResult::Text(format!(
                    "Failed to resolve session: {e}"
                )));
            }
        };

        let anchor_name = args.trim();
        let checkout = match self
            .client
            .checkout_session(
                &chat_id,
                &session_key,
                if anchor_name.is_empty() {
                    None
                } else {
                    Some(anchor_name)
                },
                thread_id.as_deref(),
            )
            .await
        {
            Ok(checkout) => checkout,
            Err(e) => return Ok(CommandResult::Text(format!("Failed to checkout: {e}"))),
        };

        match checkout {
            CheckoutResult::NoParent => Ok(CommandResult::Text(
                "Current session has no parent session.".to_owned(),
            )),
            CheckoutResult::SwitchedToParent { session_key } => Ok(CommandResult::Text(format!(
                "Switched to parent session: {session_key}"
            ))),
            CheckoutResult::ForkedFromAnchor {
                anchor_name,
                session_key,
            } => Ok(CommandResult::Text(format!(
                "Forked from anchor '{anchor_name}' into session: {session_key}"
            ))),
        }
    }
}

fn count_sessions(branch: &SessionBranch) -> usize {
    1 + branch
        .forks
        .iter()
        .map(|fork| count_sessions(&fork.branch))
        .sum::<usize>()
}
