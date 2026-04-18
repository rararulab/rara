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

//! Telegram bot command handler implementations.
//!
//! Each module implements
//! [`CommandHandler`](rara_kernel::channel::command::CommandHandler)
//! or [`CallbackHandler`](rara_kernel::channel::command::CallbackHandler) for a
//! group of related bot commands.
//!
//! ## Modules
//!
//! - [`client`]: Backend service client trait and response types.
//! - [`basic`]: `/start` and `/help` commands.
//! - [`session`]: `/new`, `/clear`, `/sessions`, `/usage`, `/model`, `/rename`
//!   commands.
//! - [`kernel_client`]: `/search` and `/jd` commands.
//! - [`mcp`]: `/mcp` command.
//! - [`status`]: `/status` command.
//! - [`tape`]: `/anchors` and `/checkout` commands.
//! - [`callbacks`]: Inline keyboard callback handlers.

pub mod anchor_dot;
pub mod basic;
pub mod callbacks;
pub mod client;
pub mod debug;
pub mod kernel_client;
pub mod mcp;
pub mod session;
pub mod status;
pub mod tape;

pub use basic::BasicCommandHandler;
pub use callbacks::{
    ModelSwitchCallbackHandler, SessionDeleteCallbackHandler, SessionDeleteCancelHandler,
    SessionDeleteConfirmHandler, SessionDetailCallbackHandler, SessionSwitchCallbackHandler,
};
pub use client::{BotServiceClient, ChatModelItem};
pub use debug::DebugCommandHandler;
pub use kernel_client::KernelBotServiceClient;
pub use mcp::McpCommandHandler;
pub use session::{RenameCommandHandler, SessionCommandHandler, StopCommandHandler};
pub use status::{StatusCommandHandler, StatusJobsCallbackHandler};
pub use tape::TapeCommandHandler;

/// Extract Telegram chat ID from command/callback metadata.
///
/// Returns an error if `telegram_chat_id` is missing — never silently
/// falls back, because a wrong chat ID would bind the wrong channel.
pub(crate) fn extract_chat_id(
    metadata: &std::collections::HashMap<String, serde_json::Value>,
) -> Result<String, rara_kernel::error::KernelError> {
    metadata
        .get("telegram_chat_id")
        .and_then(|v| {
            v.as_i64()
                .map(|n| n.to_string())
                .or_else(|| v.as_str().map(String::from))
        })
        .ok_or_else(|| rara_kernel::error::KernelError::Other {
            message: "missing telegram_chat_id in command context".into(),
        })
}

/// Extract optional Telegram thread ID from command/callback metadata.
///
/// Returns `None` for non-forum chats (the common case).
pub(crate) fn extract_thread_id(
    metadata: &std::collections::HashMap<String, serde_json::Value>,
) -> Option<String> {
    metadata.get("telegram_thread_id").and_then(|v| {
        v.as_i64()
            .map(|n| n.to_string())
            .or_else(|| v.as_str().map(String::from))
    })
}
