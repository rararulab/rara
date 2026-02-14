//! Repository trait for session persistence.
//!
//! Defines the [`SessionRepository`] trait — the sole persistence contract
//! consumed by higher-level services. Implementations are expected to be
//! backed by a relational database (see [`PgSessionRepository`](crate::pg_repository::PgSessionRepository)).

use crate::{
    error::SessionError,
    types::{ChannelBinding, ChatMessage, SessionEntry, SessionKey},
};

/// Async persistence contract for chat sessions, messages, and channel
/// bindings.
///
/// All methods are `&self` (shared reference) so that implementations can
/// be wrapped in `Arc` and shared across async tasks.
#[async_trait::async_trait]
pub trait SessionRepository: Send + Sync {
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
    /// message_count, preview). The database trigger bumps `updated_at`
    /// automatically. Returns [`SessionError::NotFound`] if the session does
    /// not exist.
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
