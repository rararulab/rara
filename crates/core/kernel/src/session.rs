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

//! Unified session types and repository trait.
//!
//! This module is the canonical source of truth for session-related types
//! used across the kernel and downstream crates (rara-sessions, rara-boot,
//! etc.). All session persistence goes through [`SessionRepository`].

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use snafu::Snafu;

use crate::channel::types::ChatMessage;

// ---------------------------------------------------------------------------
// SessionKey
// ---------------------------------------------------------------------------

/// Opaque, string-based identifier for a chat session.
///
/// Session keys use a colon-separated format to encode scope and ownership:
///
/// - **Main session**: `<scope>:<owner>` -- e.g. `"user:alice"`
/// - **Peer/DM session**: `<scope>:<a>:<b>` -- e.g. `"dm:alice:bob"`
///
/// For peer sessions, the two participant identifiers are sorted
/// lexicographically to ensure a canonical key regardless of argument order.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionKey(String);

impl SessionKey {
    /// Create a main session key with the format `<scope>:<owner>`.
    #[must_use]
    pub fn main(scope: &str, owner: &str) -> Self { Self(format!("{scope}:{owner}")) }

    /// Create a peer/DM session key with the format `<scope>:<lo>:<hi>`.
    ///
    /// The two participant identifiers are sorted lexicographically so that
    /// `for_peer("dm", "bob", "alice")` and `for_peer("dm", "alice", "bob")`
    /// produce the same key.
    #[must_use]
    pub fn for_peer(scope: &str, a: &str, b: &str) -> Self {
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        Self(format!("{scope}:{lo}:{hi}"))
    }

    /// Create a session key from a raw string without format validation.
    ///
    /// Use this when the key originates from a trusted source (e.g. the
    /// database) and is known to be well-formed.
    #[must_use]
    pub fn from_raw(raw: impl Into<String>) -> Self { Self(raw.into()) }

    /// Create a session key from any string-like value.
    ///
    /// Alias for [`from_raw`](Self::from_raw) with ergonomic naming
    /// for use as a constructor (matches the old `SessionId::new` API).
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self { Self(id.into()) }

    /// Return the underlying string slice.
    #[must_use]
    pub fn as_str(&self) -> &str { &self.0 }
}

impl std::fmt::Display for SessionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(&self.0) }
}

impl From<String> for SessionKey {
    fn from(s: String) -> Self { Self(s) }
}

impl From<&str> for SessionKey {
    fn from(s: &str) -> Self { Self(s.to_owned()) }
}

// ---------------------------------------------------------------------------
// DmScope
// ---------------------------------------------------------------------------

/// Predefined scope values used to construct [`SessionKey`]s.
///
/// Each variant corresponds to a different conversation topology:
///
/// - `User` -- single-user session (no peer)
/// - `Dm` -- direct message between two peers
/// - `Group` -- group conversation
/// - `Channel` -- broadcast / channel-based chat
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DmScope {
    User,
    Dm,
    Group,
    Channel,
}

impl DmScope {
    /// Return the scope as a static string slice (e.g. `"user"`, `"dm"`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Dm => "dm",
            Self::Group => "group",
            Self::Channel => "channel",
        }
    }
}

// ---------------------------------------------------------------------------
// SessionEntry
// ---------------------------------------------------------------------------

/// A persisted chat session with metadata.
///
/// Each session is uniquely identified by its [`SessionKey`] and tracks
/// message count, model configuration, and a short preview of the
/// conversation for UI display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEntry {
    /// Unique session key (serves as primary key in the database).
    pub key:           SessionKey,
    /// Human-readable title / label shown in session lists.
    pub title:         Option<String>,
    /// LLM model name used for this session (e.g. `"gpt-4o"`,
    /// `"claude-sonnet-4-5-20250929"`).
    pub model:         Option<String>,
    /// Optional system prompt override. When `None`, the service-level
    /// default system prompt is used.
    pub system_prompt: Option<String>,
    /// Running total of messages in this session.
    pub message_count: i64,
    /// Short preview text (typically the first user message, truncated)
    /// for display in session listings.
    pub preview:       Option<String>,
    /// Arbitrary JSON metadata for client-specific extensions.
    pub metadata:      Option<serde_json::Value>,
    /// When the session was first created.
    pub created_at:    DateTime<Utc>,
    /// When the session was last modified (message appended, metadata
    /// changed, etc.).
    pub updated_at:    DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// ChannelBinding
// ---------------------------------------------------------------------------

/// Maps an external channel to a [`SessionKey`].
///
/// Channel bindings allow external messaging platforms (Telegram, Slack, etc.)
/// to route incoming messages to the correct session without the caller
/// needing to know the internal session key.
///
/// The composite key `(channel_type, account, chat_id)` is unique; upserting
/// a binding with the same composite key will update the target session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelBinding {
    /// Channel type identifier, e.g. `"telegram"`, `"slack"`, `"web"`.
    pub channel_type: String,
    /// External account or bot identifier within the channel
    /// (e.g. Telegram bot token hash, Slack workspace id).
    pub account:      String,
    /// External chat or conversation identifier within the channel
    /// (e.g. Telegram chat id, Slack channel id).
    pub chat_id:      String,
    /// The internal session key this binding resolves to.
    pub session_key:  SessionKey,
    /// When this binding was first created.
    pub created_at:   DateTime<Utc>,
    /// When this binding was last updated (e.g. re-pointed to a new session).
    pub updated_at:   DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// SessionError
// ---------------------------------------------------------------------------

/// Errors that can occur during session persistence operations.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum SessionError {
    /// The requested session was not found.
    #[snafu(display("session not found: {key}"))]
    NotFound { key: String },

    /// A session with this key already exists.
    #[snafu(display("session already exists: {key}"))]
    AlreadyExists { key: String },

    /// A storage/infrastructure error occurred.
    #[snafu(display("repository error: {source}"))]
    Repository { source: sqlx::Error },

    /// The session key is malformed.
    #[snafu(display("invalid session key: {message}"))]
    InvalidKey { message: String },

    /// The fork point is out of range.
    #[snafu(display("invalid fork point: seq {seq} is out of range for session {key}"))]
    InvalidForkPoint { key: String, seq: i64 },

    /// A file I/O error occurred while reading/writing message JSONL files.
    #[snafu(display("message file I/O error: {source}"))]
    FileIo { source: std::io::Error },

    /// A JSON serialization/deserialization error occurred.
    #[snafu(display("json error: {source}"))]
    Json { source: serde_json::Error },
}

impl From<sqlx::Error> for SessionError {
    fn from(source: sqlx::Error) -> Self { Self::Repository { source } }
}

// ---------------------------------------------------------------------------
// SessionRepository trait
// ---------------------------------------------------------------------------

/// Async persistence contract for chat sessions, messages, and channel
/// bindings.
///
/// All methods are `&self` (shared reference) so that implementations can
/// be wrapped in `Arc` and shared across async tasks.
#[async_trait]
pub trait SessionRepository: Send + Sync + 'static {
    // -- sessions -----------------------------------------------------------

    /// Persist a new session. Returns [`SessionError::AlreadyExists`] if a
    /// session with the same key already exists.
    async fn create_session(&self, entry: &SessionEntry) -> Result<SessionEntry, SessionError>;

    /// Retrieve a session by its key, or `None` if it does not exist.
    async fn get_session(&self, key: &SessionKey) -> Result<Option<SessionEntry>, SessionError>;

    /// List sessions, ordered by `updated_at` descending.
    ///
    /// `limit` caps the result set; `offset` skips the first N rows.
    async fn list_sessions(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<SessionEntry>, SessionError>;

    /// Update mutable session fields (title, model, system_prompt, metadata,
    /// message_count, preview). Returns [`SessionError::NotFound`] if the
    /// session does not exist.
    async fn update_session(&self, entry: &SessionEntry) -> Result<SessionEntry, SessionError>;

    /// Delete a session and all associated messages and channel bindings
    /// (cascade). Returns [`SessionError::NotFound`] if the session does not
    /// exist.
    async fn delete_session(&self, key: &SessionKey) -> Result<(), SessionError>;

    // -- messages -----------------------------------------------------------

    /// Append a message to the session's conversation history.
    ///
    /// The repository assigns the next monotonically increasing `seq` number.
    /// The returned [`ChatMessage`] contains the assigned `seq`.
    async fn append_message(
        &self,
        session_key: &SessionKey,
        message: &ChatMessage,
    ) -> Result<ChatMessage, SessionError>;

    /// Read messages for a session, ordered by `seq` ascending.
    ///
    /// If `after_seq` is provided, only messages with `seq > after_seq` are
    /// returned (useful for incremental fetch).
    async fn read_messages(
        &self,
        session_key: &SessionKey,
        after_seq: Option<i64>,
        limit: Option<i64>,
    ) -> Result<Vec<ChatMessage>, SessionError>;

    /// Delete all messages for a session while keeping the session row itself.
    async fn clear_messages(&self, session_key: &SessionKey) -> Result<(), SessionError>;

    // -- fork ---------------------------------------------------------------

    /// Fork a session at a specific point in its conversation history.
    ///
    /// Creates a new session under `target_key` and copies all messages from
    /// the source session with `seq <= fork_at_seq`. Returns
    /// [`SessionError::InvalidForkPoint`] if `fork_at_seq` is out of range.
    async fn fork_session(
        &self,
        source_key: &SessionKey,
        target_key: &SessionKey,
        fork_at_seq: i64,
    ) -> Result<SessionEntry, SessionError>;

    // -- channel bindings ---------------------------------------------------

    /// Upsert a channel binding.
    ///
    /// If a binding for the same `(channel_type, account, chat_id)` already
    /// exists, the `session_key` is updated to the new value.
    async fn bind_channel(&self, binding: &ChannelBinding) -> Result<ChannelBinding, SessionError>;

    /// Resolve a channel binding to its target session key.
    ///
    /// Returns `None` if no binding exists for the given channel coordinates.
    async fn get_channel_binding(
        &self,
        channel_type: &str,
        account: &str,
        chat_id: &str,
    ) -> Result<Option<ChannelBinding>, SessionError>;
}
