//! PostgreSQL + JSONL file implementation of
//! [`SessionRepository`](crate::repository::SessionRepository).
//!
//! Session metadata and channel bindings are stored in PostgreSQL.
//! Messages are stored as append-only JSONL files on the local filesystem,
//! managed by [`SessionStore`](crate::store::SessionStore) which provides
//! binary-indexed O(1) random access by sequence number.

use std::path::PathBuf;

use async_trait::async_trait;
use chrono::Utc;
use rara_model::session::{ChannelBindingRow, ChatSessionRow};
use sqlx::PgPool;
use tracing::instrument;

use crate::{
    error::SessionError,
    store::SessionStore,
    types::{ChannelBinding, ChatMessage, SessionEntry, SessionKey},
};

/// Repository backed by PostgreSQL (sessions, bindings) and
/// [`SessionStore`](crate::store::SessionStore) (messages).
///
/// # Message storage
///
/// Each session's messages are managed by a [`SessionStore`] that maintains
/// JSONL files with companion binary index files for efficient random access.
pub struct PgSessionRepository {
    pool:  PgPool,
    store: SessionStore,
}

impl PgSessionRepository {
    /// Create a new repository.
    ///
    /// `sessions_dir` is the directory where JSONL message files are stored.
    /// The directory is created if it does not exist.
    pub async fn new(pool: PgPool, sessions_dir: impl Into<PathBuf>) -> Result<Self, SessionError> {
        let store = SessionStore::new(sessions_dir).await?;
        Ok(Self { pool, store })
    }
}

// ---------------------------------------------------------------------------
// Row types for sqlx
// ---------------------------------------------------------------------------

impl From<ChatSessionRow> for SessionEntry {
    fn from(row: ChatSessionRow) -> Self {
        Self {
            key:           SessionKey::from_raw(row.key),
            title:         row.title,
            model:         row.model,
            system_prompt: row.system_prompt,
            message_count: row.message_count,
            preview:       row.preview,
            metadata:      row.metadata,
            created_at:    row.created_at,
            updated_at:    row.updated_at,
        }
    }
}

impl From<ChannelBindingRow> for ChannelBinding {
    fn from(row: ChannelBindingRow) -> Self {
        Self {
            channel_type: row.channel_type,
            account:      row.account,
            chat_id:      row.chat_id,
            session_key:  SessionKey::from_raw(row.session_key),
            created_at:   row.created_at,
            updated_at:   row.updated_at,
        }
    }
}

// ---------------------------------------------------------------------------
// Repository implementation
// ---------------------------------------------------------------------------

/// Check whether a `sqlx::Error` is a PostgreSQL unique-constraint violation
/// (SQLSTATE 23505).
fn is_unique_violation(err: &sqlx::Error) -> bool {
    if let sqlx::Error::Database(db_err) = err {
        return db_err.code().as_deref() == Some("23505");
    }
    false
}

#[async_trait]
impl crate::repository::SessionRepository for PgSessionRepository {
    // -- sessions -----------------------------------------------------------

    #[instrument(skip(self, entry), fields(key = %entry.key))]
    async fn create_session(&self, entry: &SessionEntry) -> Result<SessionEntry, SessionError> {
        let row = sqlx::query_as::<_, ChatSessionRow>(
            r"INSERT INTO chat_session
                   (key, title, model, system_prompt, message_count, preview, metadata, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
               RETURNING *",
        )
        .bind(entry.key.as_str())
        .bind(&entry.title)
        .bind(&entry.model)
        .bind(&entry.system_prompt)
        .bind(entry.message_count)
        .bind(&entry.preview)
        .bind(&entry.metadata)
        .bind(entry.created_at)
        .bind(entry.updated_at)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            if is_unique_violation(&e) {
                return SessionError::AlreadyExists {
                    key: entry.key.as_str().to_owned(),
                };
            }
            SessionError::Repository { source: e }
        })?;

        Ok(row.into())
    }

    #[instrument(skip(self))]
    async fn get_session(&self, key: &SessionKey) -> Result<Option<SessionEntry>, SessionError> {
        let row = sqlx::query_as::<_, ChatSessionRow>(
            "SELECT * FROM chat_session WHERE key = $1",
        )
        .bind(key.as_str())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(Into::into))
    }

    #[instrument(skip(self))]
    async fn list_sessions(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<SessionEntry>, SessionError> {
        let rows = sqlx::query_as::<_, ChatSessionRow>(
            "SELECT * FROM chat_session ORDER BY updated_at DESC LIMIT $1 OFFSET $2",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    #[instrument(skip(self, entry), fields(key = %entry.key))]
    async fn update_session(&self, entry: &SessionEntry) -> Result<SessionEntry, SessionError> {
        let row = sqlx::query_as::<_, ChatSessionRow>(
            r"UPDATE chat_session
               SET title = $2, model = $3, system_prompt = $4,
                   message_count = $5, preview = $6, metadata = $7
               WHERE key = $1
               RETURNING *",
        )
        .bind(entry.key.as_str())
        .bind(&entry.title)
        .bind(&entry.model)
        .bind(&entry.system_prompt)
        .bind(entry.message_count)
        .bind(&entry.preview)
        .bind(&entry.metadata)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| SessionError::NotFound {
            key: entry.key.as_str().to_owned(),
        })?;

        Ok(row.into())
    }

    #[instrument(skip(self))]
    async fn delete_session(&self, key: &SessionKey) -> Result<(), SessionError> {
        let result = sqlx::query("DELETE FROM chat_session WHERE key = $1")
            .bind(key.as_str())
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(SessionError::NotFound {
                key: key.as_str().to_owned(),
            });
        }

        // Clean up message files (best-effort).
        let _ = self.store.delete(key.as_str()).await;

        Ok(())
    }

    // -- messages (JSONL files) ---------------------------------------------

    #[instrument(skip(self, message))]
    async fn append_message(
        &self,
        session_key: &SessionKey,
        message: &ChatMessage,
    ) -> Result<ChatMessage, SessionError> {
        self.store.append(session_key.as_str(), message).await
    }

    #[instrument(skip(self))]
    async fn read_messages(
        &self,
        session_key: &SessionKey,
        after_seq: Option<i64>,
        limit: Option<i64>,
    ) -> Result<Vec<ChatMessage>, SessionError> {
        self.store
            .read(session_key.as_str(), after_seq, limit)
            .await
    }

    #[instrument(skip(self))]
    async fn clear_messages(&self, session_key: &SessionKey) -> Result<(), SessionError> {
        self.store.clear(session_key.as_str()).await
    }

    // -- fork ---------------------------------------------------------------

    #[instrument(skip(self))]
    async fn fork_session(
        &self,
        source_key: &SessionKey,
        target_key: &SessionKey,
        fork_at_seq: i64,
    ) -> Result<SessionEntry, SessionError> {
        // Verify source exists.
        let source = self
            .get_session(source_key)
            .await?
            .ok_or_else(|| SessionError::NotFound {
                key: source_key.as_str().to_owned(),
            })?;

        // Fork the message files (validates fork_at_seq internally).
        self.store
            .fork(source_key.as_str(), target_key.as_str(), fork_at_seq)
            .await?;

        let now = Utc::now();

        // Create the target session in PG.
        let new_session = SessionEntry {
            key:           target_key.clone(),
            title:         source.title.map(|t| format!("{t} (fork)")),
            model:         source.model,
            system_prompt: source.system_prompt,
            message_count: fork_at_seq,
            preview:       source.preview,
            metadata:      source.metadata,
            created_at:    now,
            updated_at:    now,
        };
        let created = self.create_session(&new_session).await?;

        Ok(created)
    }

    // -- channel bindings ---------------------------------------------------

    #[instrument(skip(self, binding))]
    async fn bind_channel(&self, binding: &ChannelBinding) -> Result<ChannelBinding, SessionError> {
        let row = sqlx::query_as::<_, ChannelBindingRow>(
            r"INSERT INTO channel_binding
                   (channel_type, account, chat_id, session_key, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6)
               ON CONFLICT (channel_type, account, chat_id)
               DO UPDATE SET session_key = EXCLUDED.session_key
               RETURNING *",
        )
        .bind(&binding.channel_type)
        .bind(&binding.account)
        .bind(&binding.chat_id)
        .bind(binding.session_key.as_str())
        .bind(binding.created_at)
        .bind(binding.updated_at)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.into())
    }

    #[instrument(skip(self))]
    async fn get_channel_binding(
        &self,
        channel_type: &str,
        account: &str,
        chat_id: &str,
    ) -> Result<Option<ChannelBinding>, SessionError> {
        let row = sqlx::query_as::<_, ChannelBindingRow>(
            r"SELECT * FROM channel_binding
               WHERE channel_type = $1 AND account = $2 AND chat_id = $3",
        )
        .bind(channel_type)
        .bind(account)
        .bind(chat_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(Into::into))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use sqlx::postgres::PgPoolOptions;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;

    use super::*;
    use crate::repository::SessionRepository;

    /// Set up a real PostgreSQL container and apply migrations.
    async fn setup() -> (PgSessionRepository, tempfile::TempDir, testcontainers::ContainerAsync<Postgres>) {
        let container = Postgres::default().start().await.unwrap();
        let host = container.get_host().await.unwrap();
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&url)
            .await
            .unwrap();

        sqlx::migrate!("../rara-model/migrations")
            .run(&pool)
            .await
            .unwrap();

        let tmp_dir = tempfile::tempdir().unwrap();
        let repo = PgSessionRepository::new(pool, tmp_dir.path())
            .await
            .unwrap();

        (repo, tmp_dir, container)
    }

    fn make_session(key: &str) -> SessionEntry {
        let now = Utc::now();
        SessionEntry {
            key:           SessionKey::from_raw(key),
            title:         Some("Test session".to_owned()),
            model:         Some("gpt-4o".to_owned()),
            system_prompt: Some("You are helpful".to_owned()),
            message_count: 0,
            preview:       None,
            metadata:      None,
            created_at:    now,
            updated_at:    now,
        }
    }

    // -----------------------------------------------------------------------
    // Session CRUD
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn create_and_get_session() {
        let (repo, _tmp, _container) = setup().await;

        let entry = make_session("test:session1");
        let created = repo.create_session(&entry).await.unwrap();
        assert_eq!(created.key.as_str(), "test:session1");
        assert_eq!(created.title.as_deref(), Some("Test session"));

        let fetched = repo
            .get_session(&SessionKey::from_raw("test:session1"))
            .await
            .unwrap();
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().model.as_deref(), Some("gpt-4o"));
    }

    #[tokio::test]
    async fn create_duplicate_session_returns_already_exists() {
        let (repo, _tmp, _container) = setup().await;

        let entry = make_session("dup:key");
        repo.create_session(&entry).await.unwrap();

        let result = repo.create_session(&entry).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SessionError::AlreadyExists { .. }
        ));
    }

    #[tokio::test]
    async fn list_sessions_ordered_by_updated_at() {
        let (repo, _tmp, _container) = setup().await;

        repo.create_session(&make_session("list:a")).await.unwrap();
        repo.create_session(&make_session("list:b")).await.unwrap();
        repo.create_session(&make_session("list:c")).await.unwrap();

        let sessions = repo.list_sessions(10, 0).await.unwrap();
        assert_eq!(sessions.len(), 3);
    }

    #[tokio::test]
    async fn update_session() {
        let (repo, _tmp, _container) = setup().await;

        let entry = make_session("upd:key");
        let mut created = repo.create_session(&entry).await.unwrap();

        created.title = Some("Updated title".to_owned());
        created.message_count = 5;
        let updated = repo.update_session(&created).await.unwrap();
        assert_eq!(updated.title.as_deref(), Some("Updated title"));
        assert_eq!(updated.message_count, 5);
    }

    #[tokio::test]
    async fn delete_session() {
        let (repo, _tmp, _container) = setup().await;

        repo.create_session(&make_session("del:key")).await.unwrap();
        repo.delete_session(&SessionKey::from_raw("del:key"))
            .await
            .unwrap();

        let fetched = repo
            .get_session(&SessionKey::from_raw("del:key"))
            .await
            .unwrap();
        assert!(fetched.is_none());
    }

    #[tokio::test]
    async fn delete_nonexistent_session_returns_not_found() {
        let (repo, _tmp, _container) = setup().await;

        let result = repo
            .delete_session(&SessionKey::from_raw("ghost"))
            .await;
        assert!(matches!(result.unwrap_err(), SessionError::NotFound { .. }));
    }

    // -----------------------------------------------------------------------
    // Messages (JSONL files)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn append_and_read_messages() {
        let (repo, _tmp, _container) = setup().await;
        let key = SessionKey::from_raw("msg:test");
        repo.create_session(&make_session("msg:test")).await.unwrap();

        let m1 = repo
            .append_message(&key, &ChatMessage::user("hello"))
            .await
            .unwrap();
        assert_eq!(m1.seq, 1);

        let m2 = repo
            .append_message(&key, &ChatMessage::assistant("hi there"))
            .await
            .unwrap();
        assert_eq!(m2.seq, 2);

        let m3 = repo
            .append_message(&key, &ChatMessage::user("how are you"))
            .await
            .unwrap();
        assert_eq!(m3.seq, 3);

        // Read all.
        let messages = repo.read_messages(&key, None, None).await.unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].seq, 1);
        assert_eq!(messages[0].content.as_text(), "hello");
        assert_eq!(messages[2].seq, 3);

        // Read after seq 1.
        let after = repo.read_messages(&key, Some(1), None).await.unwrap();
        assert_eq!(after.len(), 2);
        assert_eq!(after[0].seq, 2);
    }

    #[tokio::test]
    async fn clear_messages() {
        let (repo, _tmp, _container) = setup().await;
        let key = SessionKey::from_raw("clear:test");
        repo.create_session(&make_session("clear:test"))
            .await
            .unwrap();

        repo.append_message(&key, &ChatMessage::user("msg1"))
            .await
            .unwrap();
        repo.append_message(&key, &ChatMessage::assistant("msg2"))
            .await
            .unwrap();

        repo.clear_messages(&key).await.unwrap();

        let messages = repo.read_messages(&key, None, None).await.unwrap();
        assert!(messages.is_empty());

        // Session still exists.
        let session = repo.get_session(&key).await.unwrap();
        assert!(session.is_some());
    }

    #[tokio::test]
    async fn delete_session_removes_messages() {
        let (repo, _tmp, _container) = setup().await;
        let key = SessionKey::from_raw("cascade:test");
        repo.create_session(&make_session("cascade:test"))
            .await
            .unwrap();

        repo.append_message(&key, &ChatMessage::user("msg1"))
            .await
            .unwrap();

        // Verify message exists.
        let msgs = repo.read_messages(&key, None, None).await.unwrap();
        assert_eq!(msgs.len(), 1);

        repo.delete_session(&key).await.unwrap();

        // Messages are cleaned up — reading returns empty.
        let msgs = repo.read_messages(&key, None, None).await.unwrap();
        assert!(msgs.is_empty());
    }

    // -----------------------------------------------------------------------
    // Fork
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn fork_session_copies_messages() {
        let (repo, _tmp, _container) = setup().await;
        let src_key = SessionKey::from_raw("fork:source");
        let tgt_key = SessionKey::from_raw("fork:target");
        repo.create_session(&make_session("fork:source"))
            .await
            .unwrap();

        repo.append_message(&src_key, &ChatMessage::user("m1"))
            .await
            .unwrap();
        repo.append_message(&src_key, &ChatMessage::assistant("m2"))
            .await
            .unwrap();
        repo.append_message(&src_key, &ChatMessage::user("m3"))
            .await
            .unwrap();

        // Fork at seq 2.
        let forked = repo.fork_session(&src_key, &tgt_key, 2).await.unwrap();
        assert_eq!(forked.message_count, 2);
        assert!(forked.title.unwrap().contains("(fork)"));

        let forked_messages = repo.read_messages(&tgt_key, None, None).await.unwrap();
        assert_eq!(forked_messages.len(), 2);
        assert_eq!(forked_messages[0].content.as_text(), "m1");
        assert_eq!(forked_messages[1].content.as_text(), "m2");

        // Source still has all 3.
        let source_messages = repo.read_messages(&src_key, None, None).await.unwrap();
        assert_eq!(source_messages.len(), 3);
    }

    #[tokio::test]
    async fn fork_invalid_seq_returns_error() {
        let (repo, _tmp, _container) = setup().await;
        let key = SessionKey::from_raw("fork:invalid");
        repo.create_session(&make_session("fork:invalid"))
            .await
            .unwrap();
        repo.append_message(&key, &ChatMessage::user("m1"))
            .await
            .unwrap();

        let result = repo
            .fork_session(&key, &SessionKey::from_raw("fork:out"), 99)
            .await;
        assert!(matches!(
            result.unwrap_err(),
            SessionError::InvalidForkPoint { .. }
        ));
    }

    // -----------------------------------------------------------------------
    // Channel bindings
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn bind_and_get_channel() {
        let (repo, _tmp, _container) = setup().await;
        let key = SessionKey::from_raw("chan:test");
        repo.create_session(&make_session("chan:test"))
            .await
            .unwrap();

        let now = Utc::now();
        let binding = ChannelBinding {
            channel_type: "telegram".to_owned(),
            account:      "bot123".to_owned(),
            chat_id:      "chat456".to_owned(),
            session_key:  key.clone(),
            created_at:   now,
            updated_at:   now,
        };

        let created = repo.bind_channel(&binding).await.unwrap();
        assert_eq!(created.session_key.as_str(), "chan:test");

        let fetched = repo
            .get_channel_binding("telegram", "bot123", "chat456")
            .await
            .unwrap();
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().session_key.as_str(), "chan:test");
    }

    #[tokio::test]
    async fn bind_channel_upsert() {
        let (repo, _tmp, _container) = setup().await;

        repo.create_session(&make_session("chan:first"))
            .await
            .unwrap();
        repo.create_session(&make_session("chan:second"))
            .await
            .unwrap();

        let now = Utc::now();
        let binding1 = ChannelBinding {
            channel_type: "slack".to_owned(),
            account:      "team1".to_owned(),
            chat_id:      "ch1".to_owned(),
            session_key:  SessionKey::from_raw("chan:first"),
            created_at:   now,
            updated_at:   now,
        };
        repo.bind_channel(&binding1).await.unwrap();

        let binding2 = ChannelBinding {
            channel_type: "slack".to_owned(),
            account:      "team1".to_owned(),
            chat_id:      "ch1".to_owned(),
            session_key:  SessionKey::from_raw("chan:second"),
            created_at:   now,
            updated_at:   now,
        };
        let updated = repo.bind_channel(&binding2).await.unwrap();
        assert_eq!(updated.session_key.as_str(), "chan:second");

        let fetched = repo
            .get_channel_binding("slack", "team1", "ch1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.session_key.as_str(), "chan:second");
    }

    #[tokio::test]
    async fn cascade_delete_removes_bindings() {
        let (repo, _tmp, _container) = setup().await;
        let key = SessionKey::from_raw("cascade_bind:test");
        repo.create_session(&make_session("cascade_bind:test"))
            .await
            .unwrap();

        let now = Utc::now();
        repo.bind_channel(&ChannelBinding {
            channel_type: "telegram".to_owned(),
            account:      "bot".to_owned(),
            chat_id:      "42".to_owned(),
            session_key:  key.clone(),
            created_at:   now,
            updated_at:   now,
        })
        .await
        .unwrap();

        repo.delete_session(&key).await.unwrap();

        let binding = repo
            .get_channel_binding("telegram", "bot", "42")
            .await
            .unwrap();
        assert!(binding.is_none());
    }

}
