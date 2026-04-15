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

//! Backend service client trait for Telegram command handlers.
//!
//! Abstracts all HTTP calls to the main service so command handlers
//! remain testable without a running backend.

use async_trait::async_trait;
use serde::Deserialize;
use snafu::Snafu;

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Error returned by [`BotServiceClient`] operations.
///
/// This is the single error surface for command handlers. Handlers should not
/// inspect kernel-specific error types directly.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum BotServiceError {
    /// A generic service-level error.
    #[snafu(display("{message}"))]
    Service { message: String },
    /// Session index operation failed.
    #[snafu(display("session operation failed: {source}"))]
    Session {
        source: rara_kernel::session::SessionError,
    },
    /// Tape operation failed with explicit high-level context.
    #[snafu(display("{context}: {source}"))]
    Tape {
        context: &'static str,
        source:  rara_kernel::memory::TapError,
    },
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Channel-to-session binding.
#[derive(Debug, Clone, Deserialize)]
pub struct ChannelBinding {
    pub session_key: String,
}

/// Summary of a chat session (used in list views).
#[derive(Debug, Clone, Deserialize)]
pub struct SessionListItem {
    pub key:           String,
    pub title:         Option<String>,
    pub preview:       Option<String>,
    pub message_count: i64,
    pub updated_at:    String,
}

/// Detailed information about a single chat session.
#[derive(Debug, Clone, Deserialize)]
pub struct SessionDetail {
    pub key:           String,
    pub title:         Option<String>,
    pub model:         Option<String>,
    pub message_count: i64,
    pub preview:       Option<String>,
    pub created_at:    String,
    pub updated_at:    String,
}

/// A job discovered through the search API.
#[derive(Debug, Clone, Deserialize)]
pub struct DiscoveryJob {
    pub title:           String,
    pub company:         String,
    pub location:        Option<String>,
    pub url:             Option<String>,
    pub salary_min:      Option<i32>,
    pub salary_max:      Option<i32>,
    pub salary_currency: Option<String>,
}
/// Information about a configured MCP server.

#[derive(Debug, Clone, Deserialize)]
pub struct McpServerInfo {
    pub name:   String,
    pub status: McpServerStatus,
}

/// Result of `/checkout` workflow after backend side effects are applied.
///
/// The client implementation owns state mutation (forking sessions and binding
/// channels). Command handlers only map this enum to user-facing text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckoutResult {
    /// Current session has no parent, so no state changes were made.
    NoParent,
    /// Channel switched to parent session.
    SwitchedToParent { session_key: String },
    /// New fork created from anchor and channel switched to the child session.
    ForkedFromAnchor {
        anchor_name: String,
        session_key: String,
    },
}

/// Connection status of an MCP server.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum McpServerStatus {
    Connected,
    Connecting,
    Disconnected,
    Error { message: String },
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Async trait abstracting all backend API calls used by command handlers.
///
/// Implementations may call the real HTTP service or return hardcoded data
/// for testing.
///
/// Design rule: this trait owns business side effects. Command handlers are
/// kept thin (parse input + render output), so orchestration belongs here.
#[async_trait]
pub trait BotServiceClient: Send + Sync {
    // -- Session management --------------------------------------------------

    /// Look up the session bound to a channel.
    ///
    /// `thread_id` narrows the lookup to a specific forum topic (Telegram
    /// supergroup threads).  Pass `None` for non-forum contexts.
    async fn get_channel_session(
        &self,
        channel_type: &str,
        chat_id: &str,
        thread_id: Option<&str>,
    ) -> Result<Option<ChannelBinding>, BotServiceError>;

    /// Create or update a channel-to-session binding.
    ///
    /// `thread_id` associates the binding with a specific forum topic when
    /// present.
    async fn bind_channel(
        &self,
        channel_type: &str,
        chat_id: &str,
        session_key: &str,
        thread_id: Option<&str>,
    ) -> Result<ChannelBinding, BotServiceError>;

    /// Create a new chat session and return the generated session key.
    async fn create_session(&self, title: Option<&str>) -> Result<String, BotServiceError>;

    /// Delete all messages from a session.
    async fn clear_session_messages(&self, session_key: &str) -> Result<(), BotServiceError>;

    /// List recent sessions.
    async fn list_sessions(&self, limit: u32) -> Result<Vec<SessionListItem>, BotServiceError>;

    /// Get detailed information about a session.
    async fn get_session(&self, key: &str) -> Result<SessionDetail, BotServiceError>;

    /// Update session fields (e.g. model).
    async fn update_session(
        &self,
        key: &str,
        model: Option<&str>,
    ) -> Result<SessionDetail, BotServiceError>;

    // -- Tape / anchor tree -------------------------------------------------

    /// Build the full anchor tree for the given session.
    ///
    /// This is used by `/anchors` to render cross-fork topology, including
    /// ancestor and descendant branches of the current session.
    async fn anchor_tree(
        &self,
        session_key: &str,
    ) -> Result<rara_kernel::memory::AnchorTree, BotServiceError>;

    /// Fork a new session at the selected anchor from the current session.
    ///
    /// Low-level primitive used by higher-level checkout flows.
    /// Returns the newly created child session key.
    async fn checkout_anchor(
        &self,
        session_key: &str,
        anchor_name: &str,
    ) -> Result<String, BotServiceError>;

    /// Return parent session key if the current session is a fork.
    ///
    /// Low-level primitive used by higher-level checkout flows.
    async fn parent_session(&self, session_key: &str) -> Result<Option<String>, BotServiceError>;

    /// Execute checkout behavior and bind channel accordingly.
    ///
    /// This is the canonical `/checkout` operation entrypoint.
    /// It applies side effects (fork/switch + binding) atomically from the
    /// command layer's perspective.
    /// - `None` anchor means "switch to parent"
    /// - `Some(anchor)` means "fork from anchor and switch to child"
    async fn checkout_session(
        &self,
        chat_id: &str,
        session_key: &str,
        anchor_name: Option<&str>,
        thread_id: Option<&str>,
    ) -> Result<CheckoutResult, BotServiceError>;

    // -- Job discovery -------------------------------------------------------

    /// Search for jobs matching the given criteria.
    async fn discover_jobs(
        &self,
        keywords: Vec<String>,
        location: Option<String>,
        max_results: u32,
    ) -> Result<Vec<DiscoveryJob>, BotServiceError>;

    /// Submit raw JD text for parsing.
    async fn submit_jd_parse(&self, text: &str) -> Result<(), BotServiceError>;

    // -- MCP servers ---------------------------------------------------------

    /// List all configured MCP servers.
    async fn list_mcp_servers(&self) -> Result<Vec<McpServerInfo>, BotServiceError>;

    /// Get a single MCP server's info.
    async fn get_mcp_server(&self, name: &str) -> Result<McpServerInfo, BotServiceError>;

    /// Add a new MCP server configuration.
    async fn add_mcp_server(
        &self,
        name: &str,
        command: &str,
        args: &[String],
    ) -> Result<McpServerInfo, BotServiceError>;

    /// Start an existing MCP server.
    async fn start_mcp_server(&self, name: &str) -> Result<(), BotServiceError>;

    /// Remove an MCP server configuration.
    async fn remove_mcp_server(&self, name: &str) -> Result<(), BotServiceError>;

    /// Delete a session and all associated data (metadata, tape, bindings).
    async fn delete_session(&self, key: &str) -> Result<(), BotServiceError>;
}
