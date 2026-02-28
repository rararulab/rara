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

//! SessionManager — conversation history management for the I/O pipeline.
//!
//! Wraps a [`SessionRepository`] to provide session lifecycle operations
//! needed by the [`AgentExecutor`](crate::executor::AgentExecutor):
//! - Ensure a session exists (get or create)
//! - Load conversation history
//! - Persist inbound and assistant messages

use std::sync::Arc;

use async_trait::async_trait;
use snafu::Snafu;

use crate::{
    channel::types::ChatMessage,
    io::types::InboundMessage,
    process::{SessionId, principal::UserId},
};

// ---------------------------------------------------------------------------
// SessionRepository trait
// ---------------------------------------------------------------------------

/// Minimal session persistence trait for the I/O pipeline.
///
/// Implementations back this against PostgreSQL, in-memory stores, etc.
/// The real PG implementation will come in a later issue.
#[async_trait]
pub trait SessionRepository: Send + Sync + 'static {
    /// Ensure a session exists for the given ID and user.
    /// Creates a new one if it doesn't exist yet.
    async fn ensure_session(
        &self,
        id: &SessionId,
        user: &UserId,
    ) -> Result<(), SessionManagerError>;

    /// Load message history for a session (without the current message).
    async fn get_history(&self, id: &SessionId) -> Result<Vec<ChatMessage>, SessionManagerError>;

    /// Persist an inbound (user) message to the session.
    async fn append_user_message(
        &self,
        id: &SessionId,
        content: &str,
    ) -> Result<(), SessionManagerError>;

    /// Persist an assistant response to the session.
    async fn append_assistant_message(
        &self,
        id: &SessionId,
        content: &str,
    ) -> Result<(), SessionManagerError>;
}

// ---------------------------------------------------------------------------
// SessionManagerError
// ---------------------------------------------------------------------------

/// Errors from session management operations.
#[derive(Debug, Snafu)]
pub enum SessionManagerError {
    /// Session not found.
    #[snafu(display("session not found: {id}"))]
    NotFound { id: String },

    /// Repository/storage error.
    #[snafu(display("session repository error: {message}"))]
    Repository { message: String },
}

// ---------------------------------------------------------------------------
// SessionManager
// ---------------------------------------------------------------------------

/// Manages conversation sessions for the I/O pipeline.
///
/// Delegates to a [`SessionRepository`] implementation for actual persistence.
/// Used by [`AgentExecutor`](crate::executor::AgentExecutor) to load history
/// and persist messages during agent execution.
pub struct SessionManager {
    session_repo: Arc<dyn SessionRepository>,
}

impl SessionManager {
    /// Create a new SessionManager with the given repository.
    pub fn new(session_repo: Arc<dyn SessionRepository>) -> Self { Self { session_repo } }

    /// Ensure a session exists for the given ID and user.
    pub async fn ensure_session(
        &self,
        id: &SessionId,
        user: &UserId,
    ) -> Result<(), SessionManagerError> {
        self.session_repo.ensure_session(id, user).await
    }

    /// Get message history for a session (without the current message).
    pub async fn get_history(
        &self,
        id: &SessionId,
    ) -> Result<Vec<ChatMessage>, SessionManagerError> {
        self.session_repo.get_history(id).await
    }

    /// Persist an inbound message to the session.
    pub async fn append_message(
        &self,
        id: &SessionId,
        msg: &InboundMessage,
    ) -> Result<(), SessionManagerError> {
        let text = msg.content.as_text();
        self.session_repo.append_user_message(id, &text).await
    }

    /// Persist an assistant response to the session.
    pub async fn append_assistant_message(
        &self,
        id: &SessionId,
        content: &str,
    ) -> Result<(), SessionManagerError> {
        self.session_repo
            .append_assistant_message(id, content)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defaults::noop::NoopSessionRepository;

    #[tokio::test]
    async fn test_session_manager_noop() {
        let repo = Arc::new(NoopSessionRepository);
        let manager = SessionManager::new(repo);

        let session_id = SessionId::new("test-session");
        let user_id = UserId("test-user".to_string());

        // All operations should succeed without panicking.
        manager.ensure_session(&session_id, &user_id).await.unwrap();

        let history = manager.get_history(&session_id).await.unwrap();
        assert!(history.is_empty());

        // Create a minimal InboundMessage for append_message.
        let msg = InboundMessage {
            id:            crate::io::types::MessageId::new(),
            source:        crate::io::types::ChannelSource {
                channel_type:        crate::channel::types::ChannelType::Telegram,
                platform_message_id: None,
                platform_user_id:    "tg-user".to_string(),
                platform_chat_id:    None,
            },
            user:          user_id.clone(),
            session_id:    session_id.clone(),
            content:       crate::channel::types::MessageContent::Text("hello".to_string()),
            reply_context: None,
            timestamp:     jiff::Timestamp::now(),
            metadata:      std::collections::HashMap::new(),
        };

        manager.append_message(&session_id, &msg).await.unwrap();

        manager
            .append_assistant_message(&session_id, "response")
            .await
            .unwrap();
    }

    #[test]
    fn test_session_manager_error_display() {
        let err = SessionManagerError::NotFound {
            id: "s1".to_string(),
        };
        assert_eq!(err.to_string(), "session not found: s1");

        let err = SessionManagerError::Repository {
            message: "connection failed".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "session repository error: connection failed"
        );
    }
}
