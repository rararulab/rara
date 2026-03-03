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

//! Unified session types and repository trait.
//!
//! This module is the canonical source of truth for session-related types
//! used across the kernel and downstream crates (rara-sessions, rara-boot,
//! etc.). All session persistence goes through [`SessionRepository`].

pub mod error;
pub mod types;

pub use error::SessionError;
pub use types::{ChannelBinding, SessionEntry, SessionKey};

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;

use crate::channel::types::ChatMessage;

/// Shared reference to a [`SessionRepository`] implementation.
pub type SessionRepoRef = Arc<dyn SessionRepository>;

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

    // -- create (convenience) -----------------------------------------------

    /// Create a new empty session with a generated UUID key.
    ///
    /// This is the preferred way to create sessions — callers never construct
    /// session keys manually.
    async fn create(&self) -> Result<SessionEntry, SessionError> {
        let now = Utc::now();
        let entry = SessionEntry {
            key:           SessionKey::new(),
            title:         None,
            model:         None,
            system_prompt: None,
            message_count: 0,
            preview:       None,
            metadata:      None,
            created_at:    now,
            updated_at:    now,
        };
        self.create_session(&entry).await
    }

    // -- fork ---------------------------------------------------------------

    /// Fork a session at a specific point in its conversation history.
    ///
    /// Creates a new session with a generated UUID key and copies all messages
    /// from the source session with `seq <= fork_at_seq`. Returns
    /// [`SessionError::InvalidForkPoint`] if `fork_at_seq` is out of range.
    async fn fork_session(
        &self,
        source_key: &SessionKey,
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

    /// Resolve a channel binding by `(channel_type, chat_id)` only, ignoring
    /// the account dimension.
    ///
    /// This is used by the session resolver during ingress, where the bot
    /// account is not yet known. Returns the most recently updated binding
    /// if multiple accounts serve the same chat.
    async fn get_binding_by_chat(
        &self,
        channel_type: &str,
        chat_id: &str,
    ) -> Result<Option<ChannelBinding>, SessionError> {
        // Default: no binding support — callers fall back to raw chat_id.
        let _ = (channel_type, chat_id);
        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// NoopSessionRepository
// ---------------------------------------------------------------------------

mod noop {
    use async_trait::async_trait;
    use chrono::Utc;

    use crate::channel::types::ChatMessage;

    use super::{
        ChannelBinding, SessionEntry, SessionError, SessionKey, SessionRepository,
    };

    /// A no-op session repository for testing — all operations succeed without
    /// persisting.
    pub struct NoopSessionRepository;

    #[async_trait]
    impl SessionRepository for NoopSessionRepository {
        async fn create_session(&self, entry: &SessionEntry) -> Result<SessionEntry, SessionError> {
            Ok(entry.clone())
        }

        async fn get_session(&self, _key: &SessionKey) -> Result<Option<SessionEntry>, SessionError> {
            Ok(None)
        }

        async fn list_sessions(
            &self,
            _limit: i64,
            _offset: i64,
        ) -> Result<Vec<SessionEntry>, SessionError> {
            Ok(vec![])
        }

        async fn update_session(&self, entry: &SessionEntry) -> Result<SessionEntry, SessionError> {
            Ok(entry.clone())
        }

        async fn delete_session(&self, _key: &SessionKey) -> Result<(), SessionError> { Ok(()) }

        async fn append_message(
            &self,
            _session_key: &SessionKey,
            message: &ChatMessage,
        ) -> Result<ChatMessage, SessionError> {
            Ok(message.clone())
        }

        async fn read_messages(
            &self,
            _session_key: &SessionKey,
            _after_seq: Option<i64>,
            _limit: Option<i64>,
        ) -> Result<Vec<ChatMessage>, SessionError> {
            Ok(vec![])
        }

        async fn clear_messages(&self, _session_key: &SessionKey) -> Result<(), SessionError> { Ok(()) }

        async fn fork_session(
            &self,
            _source_key: &SessionKey,
            _fork_at_seq: i64,
        ) -> Result<SessionEntry, SessionError> {
            let now = Utc::now();
            Ok(SessionEntry {
                key:           SessionKey::new(),
                title:         None,
                model:         None,
                system_prompt: None,
                message_count: 0,
                preview:       None,
                metadata:      None,
                created_at:    now,
                updated_at:    now,
            })
        }

        async fn bind_channel(&self, binding: &ChannelBinding) -> Result<ChannelBinding, SessionError> {
            Ok(binding.clone())
        }

        async fn get_channel_binding(
            &self,
            _channel_type: &str,
            _account: &str,
            _chat_id: &str,
        ) -> Result<Option<ChannelBinding>, SessionError> {
            Ok(None)
        }
    }
}

pub use noop::NoopSessionRepository;
