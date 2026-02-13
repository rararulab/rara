//! Core types for the sessions domain.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// SessionKey
// ---------------------------------------------------------------------------

/// Opaque session identifier.
///
/// Format: `<scope>:<owner>` for main sessions (e.g. `user:alice`),
/// or `<scope>:<a>:<b>` for peer sessions (e.g. `dm:alice:bob`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionKey(String);

impl SessionKey {
    /// Create a main session key: `<scope>:<owner>`.
    #[must_use]
    pub fn main(scope: &str, owner: &str) -> Self {
        Self(format!("{scope}:{owner}"))
    }

    /// Create a peer/DM session key.
    ///
    /// The two identifiers are sorted to ensure a canonical order so that
    /// `dm(a, b)` and `dm(b, a)` resolve to the same key.
    #[must_use]
    pub fn for_peer(scope: &str, a: &str, b: &str) -> Self {
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        Self(format!("{scope}:{lo}:{hi}"))
    }

    /// Create a session key from a raw string without validation.
    #[must_use]
    pub fn from_raw(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    /// Inner string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SessionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for SessionKey {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for SessionKey {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

// ---------------------------------------------------------------------------
// DmScope
// ---------------------------------------------------------------------------

/// Predefined scope values for session keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DmScope {
    User,
    Dm,
    Group,
    Channel,
}

impl DmScope {
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

/// A persisted chat session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEntry {
    /// Unique session key (primary key).
    pub key:           SessionKey,
    /// Human-readable title / label.
    pub title:         Option<String>,
    /// Model name used for this session (e.g. `gpt-4o`).
    pub model:         Option<String>,
    /// System prompt override for this session.
    pub system_prompt: Option<String>,
    /// Running message count.
    pub message_count: i64,
    /// Short preview of the last assistant message.
    pub preview:       Option<String>,
    /// Arbitrary metadata.
    pub metadata:      Option<serde_json::Value>,
    pub created_at:    DateTime<Utc>,
    pub updated_at:    DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// ChatMessage
// ---------------------------------------------------------------------------

/// Role of a chat message participant.
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Sequence number within the session (1-based, monotonically increasing).
    pub seq:     i64,
    /// The role that produced this message.
    pub role:    MessageRole,
    /// Message content (text or structured).
    pub content: MessageContent,
    /// Optional tool-call id (for tool / tool_result messages).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Optional tool name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// When the message was persisted.
    pub created_at: DateTime<Utc>,
}

/// Content payload for a chat message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    /// Plain text.
    Text(String),
    /// Structured / multimodal content.
    Multimodal(Vec<ContentBlock>),
}

impl MessageContent {
    /// Extract a plain-text representation (returns the text if `Text`, or
    /// concatenates all text blocks if `Multimodal`).
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

/// A single block within multimodal content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
    ImageUrl { url: String },
}

// -- convenience constructors ------------------------------------------------

impl ChatMessage {
    /// Create a user message with plain text.
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

    /// Create an assistant message with plain text.
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

    /// Create a tool-call message.
    #[must_use]
    pub fn tool(tool_call_id: impl Into<String>, tool_name: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            seq:          0,
            role:         MessageRole::Tool,
            content:      MessageContent::Text(content.into()),
            tool_call_id: Some(tool_call_id.into()),
            tool_name:    Some(tool_name.into()),
            created_at:   Utc::now(),
        }
    }

    /// Create a tool-result message.
    #[must_use]
    pub fn tool_result(tool_call_id: impl Into<String>, tool_name: impl Into<String>, content: impl Into<String>) -> Self {
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

/// Maps an external channel (e.g. Telegram chat id) to a session key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelBinding {
    /// Channel type, e.g. `"telegram"`, `"slack"`.
    pub channel_type: String,
    /// External account / bot identifier.
    pub account:      String,
    /// External chat / conversation identifier.
    pub chat_id:      String,
    /// The session key this binding maps to.
    pub session_key:  SessionKey,
    pub created_at:   DateTime<Utc>,
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
            ContentBlock::Text { text: "line1".to_owned() },
            ContentBlock::ImageUrl { url: "http://img".to_owned() },
            ContentBlock::Text { text: "line2".to_owned() },
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
