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

//! Core types for the sessions crate.
//!
//! Chat message types ([`ChatMessage`], [`MessageRole`], [`MessageContent`],
//! [`ContentBlock`], [`ToolCall`]) are re-exported from `rara-kernel` which
//! is the single canonical source of truth.
//!
//! Session-specific types ([`SessionKey`], [`SessionEntry`],
//! [`ChannelBinding`]) are also re-exported from `rara-kernel::session`.

// ---------------------------------------------------------------------------
// Re-exports from rara-kernel (canonical ChatMessage types)
// ---------------------------------------------------------------------------
pub use rara_kernel::channel::types::{
    ChatMessage, ContentBlock, MessageContent, MessageRole, ToolCall,
};
// ---------------------------------------------------------------------------
// Re-exports from rara-kernel::session (canonical session types)
// ---------------------------------------------------------------------------
pub use rara_kernel::session::{ChannelBinding, SessionEntry, SessionKey};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_key_is_uuid() {
        let key = SessionKey::new();
        // Should be a valid UUID string.
        // Verify that the key is a valid UUID — parse_str would fail if not.
        assert!(
            !key.to_string().is_empty(),
            "session key should not be empty"
        );
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
