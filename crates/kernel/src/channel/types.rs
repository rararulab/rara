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
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    strum::Display,
    strum::IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
pub enum ChannelType {
    /// Web-based chat UI.
    #[strum(serialize = "web")]
    Web,
    /// Telegram bot.
    #[strum(serialize = "telegram")]
    Telegram,
    /// Command-line interface.
    #[strum(serialize = "cli")]
    Cli,
    /// REST/gRPC API call.
    #[strum(serialize = "api")]
    Api,
    /// Internally-triggered proactive task.
    #[strum(serialize = "proactive")]
    Proactive,
    /// Internal synthetic message (workers, SyscallTool, etc.).
    #[strum(serialize = "internal")]
    Internal,
    /// WeChat iLink Bot.
    #[strum(serialize = "wechat")]
    Wechat,
}

impl ChannelType {
    /// Return a stable label for metrics/logging.
    pub fn label(self) -> &'static str { self.into() }
}

/// Policy for handling messages in group chats.
///
/// Controls when the bot responds in group conversations.
/// Ref: OpenFang `openfang-types/src/config.rs` — `GroupPolicy`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default, strum::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum GroupPolicy {
    /// Ignore all group messages.
    Ignore,
    /// Only respond when @mentioned or rara keyword is detected.
    MentionOnly,
    /// Respond in small groups (<= threshold) automatically; require mention
    /// in larger groups. This is the legacy default behavior.
    #[default]
    MentionOrSmallGroup,
    /// Not mentioned -> route as GroupMessage for proactive LLM judgment.
    /// Mentioned -> respond directly.
    ProactiveJudgment,
    /// Respond to all group messages.
    All,
}

// ---------------------------------------------------------------------------
// MessageRole
// ---------------------------------------------------------------------------

/// Role of the entity that produced a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, strum::Display)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    #[strum(serialize = "system")]
    System,
    #[strum(serialize = "user")]
    User,
    #[strum(serialize = "assistant")]
    Assistant,
    #[strum(serialize = "tool")]
    Tool,
    #[strum(serialize = "tool_result")]
    ToolResult,
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
    /// Inline base64-encoded image data.
    ImageBase64 {
        media_type: String,
        data:       String,
    },
    /// Inline base64-encoded audio data (transcribed server-side by STT).
    AudioBase64 {
        media_type: String,
        data:       String,
    },
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
                    ContentBlock::ImageUrl { .. }
                    | ContentBlock::ImageBase64 { .. }
                    | ContentBlock::AudioBase64 { .. } => None,
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
// AgentPhase
// ---------------------------------------------------------------------------

/// Lifecycle phase of an agent's response generation.
///
/// Adapters can use this for UX feedback (typing indicators, emoji
/// reactions, progress spinners).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, strum::Display)]
#[serde(rename_all = "snake_case")]
pub enum AgentPhase {
    /// Task is queued, waiting for execution.
    #[strum(serialize = "queued")]
    Queued,
    /// Agent is processing / thinking.
    #[strum(serialize = "thinking")]
    Thinking,
    /// Agent is executing a tool call.
    #[strum(serialize = "tool_use")]
    ToolUse,
    /// Agent is streaming a response.
    #[strum(serialize = "streaming")]
    Streaming,
    /// Agent finished successfully.
    #[strum(serialize = "done")]
    Done,
    /// Agent encountered an error.
    #[strum(serialize = "error")]
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
    /// Agent loop paused at tool call limit — adapter should prompt user.
    ToolCallLimit {
        session_key:     String,
        limit_id:        u64,
        tool_calls_made: usize,
        elapsed_secs:    u64,
    },
    /// Tool call limit resolved by user.
    ToolCallLimitResolved {
        session_key: String,
        limit_id:    u64,
        continued:   bool,
    },
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
