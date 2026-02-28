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

//! Core types for the Channel abstraction.
//!
//! These types define the unified message model that all channel adapters
//! convert to/from. The kernel operates on these types exclusively;
//! platform-specific details are handled by individual adapters.

use std::collections::HashMap;

use base::shared_string::SharedString;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// ChannelType
// ---------------------------------------------------------------------------

/// Identifies the communication platform a message originates from.
///
/// Adapters convert platform-specific events into [`ChannelMessage`]s tagged
/// with the appropriate variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelType {
    /// Web-based chat UI.
    Web,
    /// Telegram bot.
    Telegram,
    /// Command-line interface.
    Cli,
    /// REST/gRPC API call.
    Api,
    /// Internally-triggered scheduled task.
    Scheduled,
    /// Internally-triggered proactive task.
    Proactive,
    /// Pipeline execution.
    Pipeline,
}

impl ChannelType {
    /// Return a stable label for metrics/logging.
    pub fn label(self) -> &'static str {
        match self {
            Self::Web => "web",
            Self::Telegram => "telegram",
            Self::Cli => "cli",
            Self::Api => "api",
            Self::Scheduled => "scheduled",
            Self::Proactive => "proactive",
            Self::Pipeline => "pipeline",
        }
    }
}

impl std::fmt::Display for ChannelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(self.label()) }
}

// ---------------------------------------------------------------------------
// MessageRole
// ---------------------------------------------------------------------------

/// Role of the entity that produced a message.
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
            Self::System => f.write_str("system"),
            Self::User => f.write_str("user"),
            Self::Assistant => f.write_str("assistant"),
            Self::Tool => f.write_str("tool"),
            Self::ToolResult => f.write_str("tool_result"),
        }
    }
}

// ---------------------------------------------------------------------------
// ContentBlock / MessageContent
// ---------------------------------------------------------------------------

/// A single block within multimodal content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// A text fragment.
    Text { text: String },
    /// A reference to an image by URL.
    ImageUrl { url: String },
}

/// Message content — either plain text or multimodal blocks.
///
/// Uses `#[serde(untagged)]` so plain strings serialize naturally.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    /// Plain UTF-8 text.
    Text(String),
    /// Structured multimodal content.
    Multimodal(Vec<ContentBlock>),
}

impl MessageContent {
    /// Extract a plain-text representation.
    ///
    /// For [`Text`](Self::Text), returns the inner string.
    /// For [`Multimodal`](Self::Multimodal), concatenates text blocks,
    /// ignoring images.
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

    /// Returns `true` when the content is empty or whitespace-only.
    pub fn is_empty(&self) -> bool { self.as_text().trim().is_empty() }
}

impl From<String> for MessageContent {
    fn from(s: String) -> Self { Self::Text(s) }
}

impl From<&str> for MessageContent {
    fn from(s: &str) -> Self { Self::Text(s.to_owned()) }
}

// ---------------------------------------------------------------------------
// ToolCall
// ---------------------------------------------------------------------------

/// A tool call requested by an LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id:        SharedString,
    pub name:      SharedString,
    pub arguments: serde_json::Value,
}

// ---------------------------------------------------------------------------
// ChatMessage
// ---------------------------------------------------------------------------

/// A single message in a conversation history.
///
/// This is the canonical chat message type used throughout the kernel and
/// persistence layers. Sequence numbers are assigned by the repository;
/// convenience constructors set `seq` to `0` as a placeholder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Sequence number within the session (1-based, monotonically increasing).
    /// Set to `0` before persistence; the repository assigns the real value.
    #[serde(default)]
    pub seq:          i64,
    /// The role that produced this message.
    pub role:         MessageRole,
    /// Message content — either plain text or a list of multimodal blocks.
    pub content:      MessageContent,
    /// Tool calls requested by the assistant (present on assistant messages
    /// that invoke tools).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls:   Vec<ToolCall>,
    /// Identifier linking a tool invocation to its result. Present on
    /// [`MessageRole::Tool`] and [`MessageRole::ToolResult`] messages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Name of the tool that was invoked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name:    Option<String>,
    /// Timestamp when the message was created.
    pub created_at:   jiff::Timestamp,
}

impl ChatMessage {
    /// Create a user message with plain text content.
    #[must_use]
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            seq:          0,
            role:         MessageRole::User,
            content:      MessageContent::Text(text.into()),
            tool_calls:   Vec::new(),
            tool_call_id: None,
            tool_name:    None,
            created_at:   jiff::Timestamp::now(),
        }
    }

    /// Create an assistant message with plain text content.
    #[must_use]
    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            seq:          0,
            role:         MessageRole::Assistant,
            content:      MessageContent::Text(text.into()),
            tool_calls:   Vec::new(),
            tool_call_id: None,
            tool_name:    None,
            created_at:   jiff::Timestamp::now(),
        }
    }

    /// Create a system message.
    #[must_use]
    pub fn system(text: impl Into<String>) -> Self {
        Self {
            seq:          0,
            role:         MessageRole::System,
            content:      MessageContent::Text(text.into()),
            tool_calls:   Vec::new(),
            tool_call_id: None,
            tool_name:    None,
            created_at:   jiff::Timestamp::now(),
        }
    }

    /// Create a tool-call message representing a tool invocation by the LLM.
    #[must_use]
    pub fn tool(
        tool_call_id: impl Into<String>,
        name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            seq:          0,
            role:         MessageRole::Tool,
            content:      MessageContent::Text(content.into()),
            tool_calls:   Vec::new(),
            tool_call_id: Some(tool_call_id.into()),
            tool_name:    Some(name.into()),
            created_at:   jiff::Timestamp::now(),
        }
    }

    /// Create a tool-result message carrying the output of a tool execution.
    #[must_use]
    pub fn tool_result(
        tool_call_id: impl Into<String>,
        name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            seq:          0,
            role:         MessageRole::ToolResult,
            content:      MessageContent::Text(content.into()),
            tool_calls:   Vec::new(),
            tool_call_id: Some(tool_call_id.into()),
            tool_name:    Some(name.into()),
            created_at:   jiff::Timestamp::now(),
        }
    }
}

// ---------------------------------------------------------------------------
// ChannelUser
// ---------------------------------------------------------------------------

/// Identity of the user within a specific channel.
///
/// The `platform_id` is opaque to the kernel — each adapter defines its own
/// scheme (e.g. Telegram chat-id, web session UUID).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelUser {
    /// Platform-specific user identifier.
    pub platform_id:  String,
    /// Human-readable display name, if available.
    pub display_name: Option<String>,
}

// ---------------------------------------------------------------------------
// ChannelMessage
// ---------------------------------------------------------------------------

/// A unified inbound message from any channel.
///
/// Channel adapters convert platform-specific events into this type.
/// The kernel's router and dispatcher operate exclusively on
/// `ChannelMessage` instances.
#[derive(Debug, Clone)]
pub struct ChannelMessage {
    /// Unique message identifier (e.g. ULID).
    pub id:           String,
    /// Which channel this message arrived from.
    pub channel_type: ChannelType,
    /// The user who sent this message.
    pub user:         ChannelUser,
    /// Session key for conversation continuity.
    pub session_key:  String,
    /// Message role.
    pub role:         MessageRole,
    /// Message content.
    pub content:      MessageContent,
    /// Tool call identifier (for tool/tool_result messages).
    pub tool_call_id: Option<String>,
    /// Tool name (for tool/tool_result messages).
    pub tool_name:    Option<String>,
    /// When the message was created.
    pub timestamp:    jiff::Timestamp,
    /// Arbitrary key-value metadata for adapter-specific extensions.
    pub metadata:     HashMap<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------
// OutboundMessage
// ---------------------------------------------------------------------------

/// A message to send back through a channel.
///
/// The adapter is responsible for formatting the content appropriately
/// for its platform (e.g. Telegram HTML, Slack mrkdwn, plain text).
#[derive(Debug, Clone)]
pub struct OutboundMessage {
    /// Target channel.
    pub channel_type:        ChannelType,
    /// Target session.
    pub session_key:         String,
    /// Response content (markdown-ish, adapter formats for platform).
    pub content:             String,
    /// Optional metadata for platform-specific features.
    pub metadata:            HashMap<String, serde_json::Value>,
    /// Optional photo to attach (bytes + MIME type).
    pub photo:               Option<PhotoAttachment>,
    /// Optional inline keyboard / buttons.
    pub reply_markup:        Option<ReplyMarkup>,
    /// Edit an existing message instead of sending a new one.
    pub edit_message_id:     Option<String>,
    /// Reply to a specific message.
    pub reply_to_message_id: Option<String>,
}

// ---------------------------------------------------------------------------
// AgentPhase
// ---------------------------------------------------------------------------

/// Lifecycle phase of an agent's response generation.
///
/// Adapters can use this for UX feedback (typing indicators, emoji
/// reactions, progress spinners).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPhase {
    /// Task is queued, waiting for execution.
    Queued,
    /// Agent is processing / thinking.
    Thinking,
    /// Agent is executing a tool call.
    ToolUse,
    /// Agent is streaming a response.
    Streaming,
    /// Agent finished successfully.
    Done,
    /// Agent encountered an error.
    Error,
}

impl AgentPhase {
    /// Returns a compact emoji for this phase (useful for reactions).
    pub fn emoji(self) -> &'static str {
        match self {
            Self::Queued => "\u{23f3}",    // ⏳
            Self::Thinking => "\u{1f914}", // 🤔
            Self::ToolUse => "\u{1f527}",  // 🔧
            Self::Streaming => "\u{270d}", // ✍
            Self::Done => "\u{2705}",      // ✅
            Self::Error => "\u{274c}",     // ❌
        }
    }
}

impl std::fmt::Display for AgentPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Queued => f.write_str("queued"),
            Self::Thinking => f.write_str("thinking"),
            Self::ToolUse => f.write_str("tool_use"),
            Self::Streaming => f.write_str("streaming"),
            Self::Done => f.write_str("done"),
            Self::Error => f.write_str("error"),
        }
    }
}

// ---------------------------------------------------------------------------
// StreamEvent
// ---------------------------------------------------------------------------

/// Events emitted during streaming agent response generation.
///
/// Adapters consume these to provide progressive UX feedback (e.g.
/// incremental message edits in Telegram, SSE chunks over WebSocket).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    /// Incremental text output from the LLM.
    TextDelta { text: String },
    /// Incremental reasoning/thinking text (may be hidden from user).
    ReasoningDelta { text: String },
    /// Agent started thinking (no content yet).
    Thinking,
    /// Agent finished thinking phase.
    ThinkingDone,
    /// Agent started a new iteration (multi-turn tool loop).
    Iteration { index: usize },
    /// A tool call has started executing.
    ToolCallStart { id: String, name: String },
    /// A tool call has finished.
    ToolCallEnd {
        id:      String,
        name:    String,
        success: bool,
        error:   Option<String>,
    },
    /// Streaming completed successfully with the final accumulated text.
    Done { text: String },
    /// Streaming terminated with an error.
    Error { message: String },
}

// ---------------------------------------------------------------------------
// CommandInfo
// ---------------------------------------------------------------------------

/// Parsed command extracted from a channel message.
///
/// Adapters parse platform-specific command formats (e.g. `/search keywords`)
/// and populate this struct for routing to command handlers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandInfo {
    /// Command name without the leading slash (e.g. "search", "help").
    pub name: String,
    /// Raw argument string after the command name.
    pub args: String,
    /// The complete raw text including the command prefix.
    pub raw:  String,
}

// ---------------------------------------------------------------------------
// CallbackInfo
// ---------------------------------------------------------------------------

/// Callback data from an interactive element (e.g. inline keyboard button).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallbackInfo {
    /// The callback data string (e.g. "switch:session-123",
    /// "search_more:3:rust@remote").
    pub data:       String,
    /// Platform-specific message ID that originated the callback.
    pub message_id: Option<String>,
}

// ---------------------------------------------------------------------------
// PhotoAttachment
// ---------------------------------------------------------------------------

/// An image attachment for outbound messages.
#[derive(Debug, Clone)]
pub struct PhotoAttachment {
    /// Image data as bytes.
    pub data:      Vec<u8>,
    /// MIME type (e.g. "image/jpeg", "image/png").
    pub mime_type: String,
    /// Optional caption text.
    pub caption:   Option<String>,
}

// ---------------------------------------------------------------------------
// ReplyMarkup / InlineButton
// ---------------------------------------------------------------------------

/// Reply markup for interactive elements.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ReplyMarkup {
    /// Inline keyboard with rows of buttons.
    InlineKeyboard { rows: Vec<Vec<InlineButton>> },
    /// Remove any existing keyboard.
    RemoveKeyboard,
}

/// A single inline button.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InlineButton {
    /// Button label text.
    pub text:          String,
    /// Callback data sent when button is pressed.
    pub callback_data: Option<String>,
    /// URL to open when button is pressed.
    pub url:           Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_type_label() {
        assert_eq!(ChannelType::Web.label(), "web");
        assert_eq!(ChannelType::Telegram.label(), "telegram");
        assert_eq!(ChannelType::Scheduled.label(), "scheduled");
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
    fn message_content_is_empty() {
        assert!(MessageContent::Text(String::new()).is_empty());
        assert!(MessageContent::Text("  ".to_owned()).is_empty());
        assert!(!MessageContent::Text("hi".to_owned()).is_empty());
    }

    #[test]
    fn agent_phase_emoji() {
        assert_eq!(AgentPhase::Queued.emoji(), "\u{23f3}");
        assert_eq!(AgentPhase::Done.emoji(), "\u{2705}");
    }

    #[test]
    fn message_content_from_str() {
        let content: MessageContent = "hello".into();
        assert_eq!(content.as_text(), "hello");
    }

    #[test]
    fn stream_event_serialization() {
        let event = StreamEvent::TextDelta {
            text: "hello".to_owned(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("text_delta"));

        let event = StreamEvent::Done {
            text: "final".to_owned(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("done"));
    }

    #[test]
    fn command_info_basics() {
        let cmd = CommandInfo {
            name: "search".to_owned(),
            args: "rust developer @ remote".to_owned(),
            raw:  "/search rust developer @ remote".to_owned(),
        };
        assert_eq!(cmd.name, "search");
    }

    #[test]
    fn outbound_message_defaults() {
        let msg = OutboundMessage {
            channel_type:        ChannelType::Telegram,
            session_key:         "tg:123".to_owned(),
            content:             "hello".to_owned(),
            metadata:            Default::default(),
            photo:               None,
            reply_markup:        None,
            edit_message_id:     None,
            reply_to_message_id: None,
        };
        assert!(msg.photo.is_none());
        assert!(msg.reply_markup.is_none());
    }

    #[test]
    fn chat_message_constructors() {
        let u = ChatMessage::user("hello");
        assert_eq!(u.role, MessageRole::User);
        assert_eq!(u.content.as_text(), "hello");
        assert_eq!(u.seq, 0);
        assert!(u.tool_calls.is_empty());
        assert!(u.tool_call_id.is_none());

        let a = ChatMessage::assistant("hi there");
        assert_eq!(a.role, MessageRole::Assistant);
        assert_eq!(a.content.as_text(), "hi there");

        let s = ChatMessage::system("you are helpful");
        assert_eq!(s.role, MessageRole::System);
        assert_eq!(s.content.as_text(), "you are helpful");

        let t = ChatMessage::tool("call-1", "search", "result");
        assert_eq!(t.role, MessageRole::Tool);
        assert_eq!(t.tool_call_id, Some("call-1".to_owned()));
        assert_eq!(t.tool_name, Some("search".to_owned()));

        let tr = ChatMessage::tool_result("call-2", "fetch", "data");
        assert_eq!(tr.role, MessageRole::ToolResult);
        assert_eq!(tr.tool_call_id, Some("call-2".to_owned()));
    }
}
