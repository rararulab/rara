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

//! Bridge adapter from `rara-sessions` to the kernel's `SessionRepository` trait.
//!
//! [`SessionRepoBridge`] wraps a `rara_sessions::repository::SessionRepository`
//! and implements `rara_kernel::io::session_manager::SessionRepository`, allowing
//! the I/O Bus pipeline to use the real PG-backed session storage.

use std::sync::Arc;

use async_trait::async_trait;

use rara_kernel::io::session_manager::{
    SessionManagerError, SessionRepository as KernelSessionRepository,
};
use rara_kernel::llm::{ChatMessage as KernelChatMessage, ChatRole};
use rara_kernel::process::SessionId;
use rara_kernel::process::principal::UserId;
use rara_sessions::repository::SessionRepository as SessionsRepository;
use rara_sessions::types::{
    ChatMessage as SessionsChatMessage, MessageRole, SessionEntry, SessionKey,
};

/// Bridges the `rara_sessions` repository to the kernel's session repository
/// trait so the I/O Bus pipeline can persist conversations via PostgreSQL.
pub struct SessionRepoBridge {
    inner: Arc<dyn SessionsRepository>,
}

impl SessionRepoBridge {
    /// Create a new bridge wrapping the given sessions repository.
    pub fn new(inner: Arc<dyn SessionsRepository>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl KernelSessionRepository for SessionRepoBridge {
    async fn ensure_session(
        &self,
        id: &SessionId,
        _user: &UserId,
    ) -> Result<(), SessionManagerError> {
        let key = SessionKey::from_raw(&id.0);
        match self.inner.get_session(&key).await {
            Ok(Some(_)) => Ok(()),
            Ok(None) => {
                let entry = SessionEntry {
                    key,
                    title: None,
                    model: None,
                    system_prompt: None,
                    message_count: 0,
                    preview: None,
                    metadata: None,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                };
                self.inner
                    .create_session(&entry)
                    .await
                    .map(|_| ())
                    .map_err(|e| SessionManagerError::Repository {
                        message: e.to_string(),
                    })
            }
            Err(e) => Err(SessionManagerError::Repository {
                message: e.to_string(),
            }),
        }
    }

    async fn get_history(
        &self,
        id: &SessionId,
    ) -> Result<Vec<KernelChatMessage>, SessionManagerError> {
        let key = SessionKey::from_raw(&id.0);
        let messages = self
            .inner
            .read_messages(&key, None, None)
            .await
            .map_err(|e| SessionManagerError::Repository {
                message: e.to_string(),
            })?;
        Ok(messages.into_iter().map(convert_message).collect())
    }

    async fn append_user_message(
        &self,
        id: &SessionId,
        content: &str,
    ) -> Result<(), SessionManagerError> {
        let key = SessionKey::from_raw(&id.0);
        let msg = SessionsChatMessage::user(content);
        self.inner
            .append_message(&key, &msg)
            .await
            .map(|_| ())
            .map_err(|e| SessionManagerError::Repository {
                message: e.to_string(),
            })
    }

    async fn append_assistant_message(
        &self,
        id: &SessionId,
        content: &str,
    ) -> Result<(), SessionManagerError> {
        let key = SessionKey::from_raw(&id.0);
        let msg = SessionsChatMessage::assistant(content);
        self.inner
            .append_message(&key, &msg)
            .await
            .map(|_| ())
            .map_err(|e| SessionManagerError::Repository {
                message: e.to_string(),
            })
    }
}

/// Convert a `rara_sessions` ChatMessage to a kernel ChatMessage.
fn convert_message(msg: SessionsChatMessage) -> KernelChatMessage {
    let role = match msg.role {
        MessageRole::System => ChatRole::System,
        MessageRole::User => ChatRole::User,
        MessageRole::Assistant => ChatRole::Assistant,
        MessageRole::Tool | MessageRole::ToolResult => ChatRole::Tool,
    };
    let text = msg.content.as_text();
    KernelChatMessage {
        role,
        content: if text.is_empty() { None } else { Some(text) },
        tool_calls: vec![],
        tool_call_id: msg.tool_call_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rara_sessions::types::MessageContent;

    #[test]
    fn convert_user_message() {
        let msg = SessionsChatMessage::user("hello world");
        let kernel_msg = convert_message(msg);
        assert_eq!(kernel_msg.role, ChatRole::User);
        assert_eq!(kernel_msg.content, Some("hello world".to_string()));
        assert!(kernel_msg.tool_calls.is_empty());
        assert!(kernel_msg.tool_call_id.is_none());
    }

    #[test]
    fn convert_assistant_message() {
        let msg = SessionsChatMessage::assistant("I can help");
        let kernel_msg = convert_message(msg);
        assert_eq!(kernel_msg.role, ChatRole::Assistant);
        assert_eq!(kernel_msg.content, Some("I can help".to_string()));
    }

    #[test]
    fn convert_system_message() {
        let msg = SessionsChatMessage::system("you are helpful");
        let kernel_msg = convert_message(msg);
        assert_eq!(kernel_msg.role, ChatRole::System);
        assert_eq!(kernel_msg.content, Some("you are helpful".to_string()));
    }

    #[test]
    fn convert_tool_message() {
        let msg = SessionsChatMessage::tool("call-1", "search", "result data");
        let kernel_msg = convert_message(msg);
        assert_eq!(kernel_msg.role, ChatRole::Tool);
        assert_eq!(kernel_msg.content, Some("result data".to_string()));
        assert_eq!(kernel_msg.tool_call_id, Some("call-1".to_string()));
    }

    #[test]
    fn convert_tool_result_message() {
        let msg = SessionsChatMessage::tool_result("call-2", "fetch", "output");
        let kernel_msg = convert_message(msg);
        assert_eq!(kernel_msg.role, ChatRole::Tool);
        assert_eq!(kernel_msg.content, Some("output".to_string()));
        assert_eq!(kernel_msg.tool_call_id, Some("call-2".to_string()));
    }

    #[test]
    fn convert_empty_content_yields_none() {
        let msg = SessionsChatMessage {
            seq:          0,
            role:         MessageRole::Assistant,
            content:      MessageContent::Text(String::new()),
            tool_call_id: None,
            tool_name:    None,
            created_at:   chrono::Utc::now(),
        };
        let kernel_msg = convert_message(msg);
        assert!(kernel_msg.content.is_none());
    }
}
