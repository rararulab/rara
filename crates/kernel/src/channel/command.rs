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

//! Command and callback handler traits for channel adapters.
//!
//! Adapters parse platform-specific command syntax (e.g. `/search keywords`)
//! and delegate execution to registered [`CommandHandler`] implementations.
//! Similarly, interactive callbacks (e.g. inline keyboard button presses)
//! are routed to [`CallbackHandler`] implementations.

use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::types::{ChannelType, ChannelUser, InlineButton};
use crate::error::KernelError;

// ---------------------------------------------------------------------------
// Command types
// ---------------------------------------------------------------------------

/// Parsed command extracted from a channel message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandInfo {
    /// Command name without the leading slash (e.g. "search", "help").
    pub name: String,
    /// Raw argument string after the command name.
    pub args: String,
    /// The complete raw text including the command prefix.
    pub raw:  String,
}

/// Definition of a supported command (for help text generation).
#[derive(Debug, Clone)]
pub struct CommandDefinition {
    /// Command name without slash (e.g. "search").
    pub name:        String,
    /// Human-readable description (e.g. "Search for jobs").
    pub description: String,
    /// Usage example (e.g. "/search `<keywords>` \[@ location\]").
    pub usage:       Option<String>,
}

/// Context available to command handlers.
#[derive(Debug, Clone)]
pub struct CommandContext {
    /// Which channel the command came from.
    pub channel_type: ChannelType,
    /// Session key for the conversation.
    pub session_key:  String,
    /// User who issued the command.
    pub user:         ChannelUser,
    /// Adapter-specific metadata from the original message.
    pub metadata:     HashMap<String, serde_json::Value>,
}

#[derive(Debug)]
/// Result returned by a command handler.
pub enum CommandResult {
    /// Plain text response (adapter will format for the platform).
    Text(String),
    /// HTML-formatted response (adapter sends as-is if platform supports HTML).
    Html(String),
    /// HTML text with inline keyboard buttons.
    ///
    /// The adapter should render the keyboard as platform-specific interactive
    /// elements (e.g. Telegram inline keyboard).
    HtmlWithKeyboard {
        html:     String,
        keyboard: Vec<Vec<InlineButton>>,
    },
    /// Binary photo payload.
    Photo {
        data:    Vec<u8>,
        caption: Option<String>,
    },
    /// No visible response needed (handler sent its own messages).
    None,
}

// ---------------------------------------------------------------------------
// CommandHandler trait
// ---------------------------------------------------------------------------

/// Handles one or more slash commands from channel messages.
///
/// Implementations are registered with adapters at construction time.
/// When the adapter detects a command, it looks up a matching handler
/// and calls [`handle`](Self::handle).
///
/// # Example
///
/// ```ignore
/// struct HelpHandler;
///
/// #[async_trait]
/// impl CommandHandler for HelpHandler {
///     fn commands(&self) -> Vec<CommandDefinition> {
///         vec![CommandDefinition {
///             name: "help".to_owned(),
///             description: "Show available commands".to_owned(),
///             usage: None,
///         }]
///     }
///
///     async fn handle(&self, _cmd: &CommandInfo, _ctx: &CommandContext) -> Result<CommandResult, KernelError> {
///         Ok(CommandResult::Text("Available commands: /help, /start".to_owned()))
///     }
/// }
/// ```
#[async_trait]
pub trait CommandHandler: Send + Sync {
    /// Return the command definitions this handler supports.
    fn commands(&self) -> Vec<CommandDefinition>;

    /// Execute the command and return a result.
    async fn handle(
        &self,
        command: &CommandInfo,
        context: &CommandContext,
    ) -> Result<CommandResult, KernelError>;
}

// ---------------------------------------------------------------------------
// Callback types
// ---------------------------------------------------------------------------

/// Context available to callback handlers.
#[derive(Debug, Clone)]
pub struct CallbackContext {
    /// Which channel the callback came from.
    pub channel_type: ChannelType,
    /// Session key.
    pub session_key:  String,
    /// User who triggered the callback.
    pub user:         ChannelUser,
    /// The callback data string.
    pub data:         String,
    /// Platform-specific message ID that originated the callback.
    pub message_id:   Option<String>,
    /// Adapter-specific metadata.
    pub metadata:     HashMap<String, serde_json::Value>,
}

#[derive(Debug)]
/// Result returned by a callback handler.
pub enum CallbackResult {
    /// Edit the originating message with new text.
    EditMessage { text: String },
    /// Send a new message.
    SendMessage { text: String },
    /// Acknowledge without visible action.
    Ack,
}

// ---------------------------------------------------------------------------
// CallbackHandler trait
// ---------------------------------------------------------------------------

/// Handles callback queries from interactive elements (buttons, etc.).
///
/// Each handler declares a prefix it matches on. When a callback arrives,
/// the adapter finds the handler whose prefix matches the callback data.
#[async_trait]
pub trait CallbackHandler: Send + Sync {
    /// The data prefix this handler matches (e.g. "switch:", "search_more:").
    fn prefix(&self) -> &str;

    /// Handle the callback and return a result.
    async fn handle(&self, context: &CallbackContext) -> Result<CallbackResult, KernelError>;
}
