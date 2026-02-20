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

//! Telegram bot command definitions.
//!
//! Commands are declared using teloxide's [`BotCommands`] derive macro, which
//! provides automatic parsing from message text and help text generation via
//! [`Command::descriptions()`].

use teloxide::utils::command::BotCommands;

/// Telegram slash commands accepted from users.
///
/// Parsed from incoming messages by [`BotCommands::parse`]. The
/// `rename_rule = "lowercase"` attribute ensures `/Start` matches `/start`.
#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Available commands:")]
pub(crate) enum Command {
    /// Display a welcome message explaining bot capabilities.
    #[command(description = "Start the bot")]
    Start,
    /// List all available commands with descriptions.
    #[command(description = "Show help")]
    Help,
    /// Search for jobs. The argument string is parsed as
    /// `<keywords> [@ location]`, e.g. `/search rust engineer @ beijing`.
    #[command(description = "Search jobs: /search <keywords> [@ location]")]
    Search(String),
    #[command(description = "Start a new chat session")]
    New,
    #[command(description = "Clear current session history")]
    Clear,
    #[command(description = "Parse a Job Description: /jd <text>")]
    Jd(String),
    /// List all chat sessions with inline buttons to switch.
    #[command(description = "List & switch chat sessions")]
    Sessions,
    /// Show details of the current active session.
    #[command(description = "Show current session usage")]
    Usage,
    /// Show or switch the model for the current session.
    /// Without arguments, shows the current model.
    /// With an argument, switches to the specified model.
    #[command(description = "Show/switch model: /model [name]")]
    Model(String),
    /// Show MCP server connection status.
    #[command(description = "Show MCP server status")]
    Mcp,
}
