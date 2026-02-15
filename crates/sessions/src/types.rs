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

//! Core types for the sessions crate.
//!
//! This module defines the data model for chat sessions, messages, content
//! blocks, and channel bindings. All types derive `Serialize`/`Deserialize`
//! for JSON API transport and database (JSONB) storage.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// SessionKey
// ---------------------------------------------------------------------------

/// Opaque, string-based identifier for a chat session.
///
/// Session keys use a colon-separated format to encode scope and ownership:
///
/// - **Main session**: `<scope>:<owner>` — e.g. `"user:alice"`
/// - **Peer/DM session**: `<scope>:<a>:<b>` — e.g. `"dm:alice:bob"`
///
/// For peer sessions, the two participant identifiers are sorted
/// lexicographically to ensure a canonical key regardless of argument order.
///
/// # Examples
///
/// ```
/// use rara_sessions::types::SessionKey;
///
/// let main = SessionKey::main("user", "alice");
/// assert_eq!(main.as_str(), "user:alice");
///
/// // Peer keys are order-independent.
/// let dm1 = SessionKey::for_peer("dm", "bob", "alice");
/// let dm2 = SessionKey::for_peer("dm", "alice", "bob");
/// assert_eq!(dm1, dm2);
/// ```
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
    /// produce the same key. This guarantees that a DM conversation between
    /// two parties always maps to exactly one session.
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
/// - `User` — single-user session (no peer)
/// - `Dm` — direct message between two peers
/// - `Group` — group conversation
/// - `Channel` — broadcast / channel-based chat
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
// ChatMessage
// ---------------------------------------------------------------------------

/// The role of the entity that produced a [`ChatMessage`].
///
/// Roles are stored as lowercase strings in the database (`"system"`,
/// `"user"`, `"assistant"`, `"tool"`, `"tool_result"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
    ToolResult,
}

impl std::fmt::Display for MessageRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::System => write!(f, "system"),
            Self::User => write!(f, "user"),
            Self::Assistant => write!(f, "assistant"),
            Self::Tool => write!(f, "tool"),
            Self::ToolResult => write!(f, "tool_result"),
        }
    }
}

/// A single message in a session's conversation history.
///
/// Messages are ordered by `seq` (1-based, monotonically increasing per
/// session). The sequence number is assigned by the repository on
/// [`append_message`](crate::repository::SessionRepository::append_message);
/// convenience constructors set `seq` to `0` as a placeholder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Sequence number within the session (1-based, monotonically increasing).
    /// Set to `0` before persistence; the repository assigns the real value.
    pub seq:          i64,
    /// The role that produced this message.
    pub role:         MessageRole,
    /// Message content — either plain text or a list of multimodal blocks.
    pub content:      MessageContent,
    /// Identifier linking a tool invocation to its result. Present on
    /// [`MessageRole::Tool`] and [`MessageRole::ToolResult`] messages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Name of the tool that was invoked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name:    Option<String>,
    /// Timestamp when the message was persisted.
    pub created_at:   DateTime<Utc>,
}

/// Content payload for a [`ChatMessage`].
///
/// Uses `#[serde(untagged)]` so that a plain string serializes as `"hello"`
/// while a multimodal message serializes as `[{"type":"text","text":"..."},
/// {"type":"image_url","url":"..."}]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    /// Plain UTF-8 text.
    Text(String),
    /// Structured / multimodal content consisting of text and image blocks.
    Multimodal(Vec<ContentBlock>),
}

impl MessageContent {
    /// Extract a plain-text representation.
    ///
    /// - For [`Text`](Self::Text), returns the inner string.
    /// - For [`Multimodal`](Self::Multimodal), concatenates all text blocks
    ///   separated by newlines, ignoring image blocks.
    #[must_use]
    pub fn as_text(&self) -> String {
        match self {
            Self::Text(t) => t.clone(),
            Self::Multimodal(blocks) => blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    ContentBlock::ImageUrl { .. } => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

/// A single block within a [`MessageContent::Multimodal`] payload.
///
/// Internally tagged by `"type"` for JSON (de)serialization:
/// - `{"type": "text", "text": "..."}`
/// - `{"type": "image_url", "url": "..."}`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// A text fragment.
    Text { text: String },
    /// A reference to an image by URL.
    ImageUrl { url: String },
}

// -- convenience constructors ------------------------------------------------

impl ChatMessage {
    /// Create a user message with plain text content.
    ///
    /// The `seq` field is set to `0` — it will be assigned by the repository
    /// when the message is persisted.
    #[must_use]
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            seq:          0, // assigned by repository
            role:         MessageRole::User,
            content:      MessageContent::Text(text.into()),
            tool_call_id: None,
            tool_name:    None,
            created_at:   Utc::now(),
        }
    }

    /// Create an assistant message with plain text content.
    #[must_use]
    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            seq:          0,
            role:         MessageRole::Assistant,
            content:      MessageContent::Text(text.into()),
            tool_call_id: None,
            tool_name:    None,
            created_at:   Utc::now(),
        }
    }

    /// Create a system message.
    #[must_use]
    pub fn system(text: impl Into<String>) -> Self {
        Self {
            seq:          0,
            role:         MessageRole::System,
            content:      MessageContent::Text(text.into()),
            tool_call_id: None,
            tool_name:    None,
            created_at:   Utc::now(),
        }
    }

    /// Create a tool-call message representing a tool invocation by the LLM.
    #[must_use]
    pub fn tool(
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            seq:          0,
            role:         MessageRole::Tool,
            content:      MessageContent::Text(content.into()),
            tool_call_id: Some(tool_call_id.into()),
            tool_name:    Some(tool_name.into()),
            created_at:   Utc::now(),
        }
    }

    /// Create a tool-result message carrying the output of a tool execution.
    #[must_use]
    pub fn tool_result(
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            seq:          0,
            role:         MessageRole::ToolResult,
            content:      MessageContent::Text(content.into()),
            tool_call_id: Some(tool_call_id.into()),
            tool_name:    Some(tool_name.into()),
            created_at:   Utc::now(),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_key_main() {
        let key = SessionKey::main("user", "alice");
        assert_eq!(key.as_str(), "user:alice");
    }

    #[test]
    fn session_key_peer_canonical_order() {
        let k1 = SessionKey::for_peer("dm", "bob", "alice");
        let k2 = SessionKey::for_peer("dm", "alice", "bob");
        assert_eq!(k1, k2);
        assert_eq!(k1.as_str(), "dm:alice:bob");
    }

    #[test]
    fn message_content_as_text() {
        let text = MessageContent::Text("hello".to_owned());
        assert_eq!(text.as_text(), "hello");

        let multi = MessageContent::Multimodal(vec![
            ContentBlock::Text {
                text: "line1".to_owned(),
            },
            ContentBlock::ImageUrl {
                url: "http://img".to_owned(),
            },
            ContentBlock::Text {
                text: "line2".to_owned(),
            },
        ]);
        assert_eq!(multi.as_text(), "line1\nline2");
    }

    #[test]
    fn chat_message_constructors() {
        let u = ChatMessage::user("hello");
        assert_eq!(u.role, MessageRole::User);
        assert_eq!(u.content.as_text(), "hello");

        let a = ChatMessage::assistant("hi there");
        assert_eq!(a.role, MessageRole::Assistant);

        let s = ChatMessage::system("you are helpful");
        assert_eq!(s.role, MessageRole::System);
    }
}
