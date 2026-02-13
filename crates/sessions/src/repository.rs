//! Repository trait for session persistence.

use crate::{
    error::SessionError,
    types::{ChannelBinding, ChatMessage, SessionEntry, SessionKey},
};

/// Persistence contract for chat sessions, messages, and channel bindings.
#[async_trait::async_trait]
pub trait SessionRepository: Send + Sync {
    // -- sessions -----------------------------------------------------------

    /// Create a new session. Returns `AlreadyExists` if the key is taken.
    async fn create_session(&self, entry: &SessionEntry) -> Result<SessionEntry, SessionError>;

    /// Retrieve a session by key.
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
    /// message_count, preview). Bumps `updated_at`.
    async fn update_session(&self, entry: &SessionEntry) -> Result<SessionEntry, SessionError>;

    /// Delete a session and its messages (cascade).
    async fn delete_session(&self, key: &SessionKey) -> Result<(), SessionError>;

    // -- messages -----------------------------------------------------------

    /// Append a message to the session. The repository assigns the next `seq`.
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

    /// Delete all messages for a session (but keep the session itself).
    async fn clear_messages(&self, session_key: &SessionKey) -> Result<(), SessionError>;

    // -- fork ---------------------------------------------------------------

    /// Fork a session: create a new session and copy messages up to
    /// `fork_at_seq` (inclusive).
    async fn fork_session(
        &self,
        source_key: &SessionKey,
        target_key: &SessionKey,
        fork_at_seq: i64,
    ) -> Result<SessionEntry, SessionError>;

    // -- channel bindings ---------------------------------------------------

    /// Upsert a channel binding (ON CONFLICT update session_key).
    async fn bind_channel(&self, binding: &ChannelBinding) -> Result<ChannelBinding, SessionError>;

    /// Resolve a channel binding.
    async fn get_channel_binding(
        &self,
        channel_type: &str,
        account: &str,
        chat_id: &str,
    ) -> Result<Option<ChannelBinding>, SessionError>;
}
