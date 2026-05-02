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

//! SQLite-backed [`SessionIndex`] implementation (issue #2025).
//!
//! Replaces the JSON-file-backed [`super::file_index::FileSessionIndex`]
//! on the kernel/HTTP write path. Session metadata lives in the
//! `sessions` and `session_channel_bindings` tables defined by the
//! `2026-05-01-000000_session_index` diesel migration.
//!
//! Tape-derived fields (`total_entries`, `updated_at`,
//! `last_token_usage`, `estimated_context_tokens`,
//! `entries_since_last_anchor`, `anchors`) are updated synchronously by
//! `TapeService::append` on every successful tape write. The append path
//! calls [`SqliteSessionIndex::update_session_derived`], which performs a
//! single-row `UPDATE` keyed by `key`.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl};
use diesel_async::RunQueryDsl;
use rara_kernel::{
    channel::types::ChannelType,
    session::{
        AnchorRef, ChannelBinding, FileIoSnafu, JsonSnafu, SessionDerivedState, SessionEntry,
        SessionError, SessionIndex, SessionKey, SessionListFilter, SessionStatus, ThinkingLevel,
    },
};
use rara_model::schema::{session_channel_bindings, sessions};
use snafu::ResultExt;
use tracing::{info, instrument, warn};
use yunara_store::diesel_pool::DieselSqlitePools;

/// SQLite-backed [`SessionIndex`].
///
/// Construct with [`SqliteSessionIndex::new`]. The constructor runs a
/// best-effort one-shot migration from the legacy JSON `index_dir`
/// (Decision 9) and a derived-state reconciliation pass (Decision 10).
pub struct SqliteSessionIndex {
    pools: DieselSqlitePools,
}

impl std::fmt::Debug for SqliteSessionIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteSessionIndex").finish_non_exhaustive()
    }
}

impl SqliteSessionIndex {
    /// Create a new SQLite-backed index using the shared diesel pools.
    pub fn new(pools: DieselSqlitePools) -> Self { Self { pools } }

    /// Migrate any legacy JSON session files in `json_index_dir` into
    /// the SQLite tables. Idempotent: a second invocation observes the
    /// non-empty `sessions` table and short-circuits without touching
    /// the filesystem.
    ///
    /// On success, the source `*.json` files are moved into
    /// `<json_index_dir>/legacy/` so a later boot does not pay the
    /// inspection cost again.
    #[instrument(skip(self))]
    pub async fn ensure_migrated_from(&self, json_index_dir: &Path) -> Result<usize, SessionError> {
        let count_existing: i64 = {
            let mut conn = self.pools.reader.get().await.map_err(map_pool_err)?;
            sessions::table
                .count()
                .get_result(&mut *conn)
                .await
                .map_err(map_diesel_err)?
        };
        if count_existing > 0 {
            return Ok(0);
        }

        if !json_index_dir.exists() {
            return Ok(0);
        }

        let (session_files, binding_files) = scan_json_dir(json_index_dir).await?;
        let mut migrated = 0;

        for path in &session_files {
            let bytes = tokio::fs::read(path).await.context(FileIoSnafu)?;
            let entry: SessionEntry = serde_json::from_slice(&bytes).context(JsonSnafu)?;
            self.create_session(&entry).await?;
            migrated += 1;
        }
        for path in &binding_files {
            let bytes = tokio::fs::read(path).await.context(FileIoSnafu)?;
            let binding: ChannelBinding = serde_json::from_slice(&bytes).context(JsonSnafu)?;
            self.bind_channel(&binding).await?;
        }

        // Move the source files into legacy/. Failures here are
        // non-fatal — the next boot's "is the SQLite table empty?"
        // gate is the safety net (Decision 9).
        let legacy_dir = json_index_dir.join("legacy");
        let _ = tokio::fs::create_dir_all(&legacy_dir).await;
        let bindings_legacy = legacy_dir.join("bindings");
        let _ = tokio::fs::create_dir_all(&bindings_legacy).await;
        for path in session_files.iter().chain(binding_files.iter()) {
            let target_root = if path
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|n| n == "bindings")
            {
                &bindings_legacy
            } else {
                &legacy_dir
            };
            if let Some(name) = path.file_name() {
                let target = target_root.join(name);
                if let Err(e) = tokio::fs::rename(path, &target).await {
                    warn!(?path, ?target, %e, "failed to move legacy session file");
                }
            }
        }

        info!(migrated, "migrated legacy JSON session index → SQLite");
        Ok(migrated)
    }

    /// Reconcile each row in `sessions` against the corresponding tape
    /// via `info_provider`. Out-of-sync rows are rebuilt from the tape
    /// (Decision 10).
    pub async fn reconcile_all<F>(&self, info_provider: F) -> Result<usize, SessionError>
    where
        F: ReconcileTape,
    {
        let rows = self
            .list_sessions(i64::MAX, 0, SessionListFilter::All)
            .await?;
        let mut repaired = 0;
        for row in rows {
            let Some(report) = info_provider.read_tape(&row.key).await else {
                continue;
            };
            if row.total_entries != report.total_entries
                || row.anchors.len() != report.anchors.len()
                || row.entries_since_last_anchor != report.entries_since_last_anchor
            {
                self.rebuild_session_with_report(&row.key, &report).await?;
                repaired += 1;
            }
        }
        Ok(repaired)
    }

    /// Replace the derived state of one session with the values supplied
    /// by `report`. Used by the rescue command (Decision 11) and the
    /// boot reconciler.
    pub async fn rebuild_session_with_report(
        &self,
        key: &SessionKey,
        report: &TapeReport,
    ) -> Result<(), SessionError> {
        let derived = SessionDerivedState::builder()
            .total_entries(report.total_entries)
            .updated_at(report.updated_at)
            .maybe_last_token_usage(report.last_token_usage)
            .estimated_context_tokens(report.estimated_context_tokens)
            .entries_since_last_anchor(report.entries_since_last_anchor)
            .anchors(report.anchors.clone())
            .maybe_preview(report.preview.clone())
            .build();
        self.update_session_derived(key, &derived).await
    }
}

/// Adapter trait used by [`SqliteSessionIndex::reconcile_all`] and the
/// rescue command. Inverted so the sessions crate does not depend on
/// `TapeService` directly (which would create a cycle: kernel → sessions
/// → kernel via TapeService).
#[async_trait]
pub trait ReconcileTape {
    /// Read the on-disk tape for `key` and produce a fresh
    /// [`TapeReport`]. Return `None` when no tape file exists for the
    /// session (treat as "nothing to reconcile").
    async fn read_tape(&self, key: &SessionKey) -> Option<TapeReport>;
}

/// A snapshot of tape-derived state computed from the on-disk JSONL.
/// Returned by [`ReconcileTape::read_tape`] and consumed by
/// [`SqliteSessionIndex::rebuild_session_with_report`].
#[derive(Debug, Clone)]
pub struct TapeReport {
    pub total_entries:             i64,
    pub updated_at:                DateTime<Utc>,
    pub last_token_usage:          Option<i64>,
    pub estimated_context_tokens:  i64,
    pub entries_since_last_anchor: i64,
    pub anchors:                   Vec<AnchorRef>,
    pub preview:                   Option<String>,
}

#[async_trait]
impl SessionIndex for SqliteSessionIndex {
    async fn create_session(&self, entry: &SessionEntry) -> Result<SessionEntry, SessionError> {
        let mut conn = self.pools.writer.get().await.map_err(map_pool_err)?;
        let row = SessionRow::from_entry(entry)?;
        diesel::insert_into(sessions::table)
            .values(&row)
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                if matches!(
                    &e,
                    diesel::result::Error::DatabaseError(
                        diesel::result::DatabaseErrorKind::UniqueViolation,
                        _
                    )
                ) {
                    SessionError::AlreadyExists {
                        key: entry.key.to_string(),
                    }
                } else {
                    map_diesel_err(e)
                }
            })?;
        Ok(entry.clone())
    }

    async fn get_session(&self, key: &SessionKey) -> Result<Option<SessionEntry>, SessionError> {
        let mut conn = self.pools.reader.get().await.map_err(map_pool_err)?;
        let row: Option<SessionRow> = sessions::table
            .filter(sessions::key.eq(key.to_string()))
            .first(&mut *conn)
            .await
            .optional()
            .map_err(map_diesel_err)?;
        row.map(SessionRow::into_entry).transpose()
    }

    async fn list_sessions(
        &self,
        limit: i64,
        offset: i64,
        filter: SessionListFilter,
    ) -> Result<Vec<SessionEntry>, SessionError> {
        let mut conn = self.pools.reader.get().await.map_err(map_pool_err)?;
        // Build the query with the status filter applied at the SQL
        // layer so the partial index `idx_sessions_status_updated_at`
        // can short-circuit the default-filtered scan (issue #2043,
        // Decision 3).
        let rows: Vec<SessionRow> = match filter {
            SessionListFilter::All => sessions::table
                .order(sessions::updated_at.desc())
                .limit(limit.max(0))
                .offset(offset.max(0))
                .load(&mut *conn)
                .await
                .map_err(map_diesel_err)?,
            SessionListFilter::Active => sessions::table
                .filter(sessions::status.eq(SessionStatus::Active.to_string()))
                .order(sessions::updated_at.desc())
                .limit(limit.max(0))
                .offset(offset.max(0))
                .load(&mut *conn)
                .await
                .map_err(map_diesel_err)?,
            SessionListFilter::Archived => sessions::table
                .filter(sessions::status.eq(SessionStatus::Archived.to_string()))
                .order(sessions::updated_at.desc())
                .limit(limit.max(0))
                .offset(offset.max(0))
                .load(&mut *conn)
                .await
                .map_err(map_diesel_err)?,
        };
        rows.into_iter().map(SessionRow::into_entry).collect()
    }

    async fn update_session(&self, entry: &SessionEntry) -> Result<SessionEntry, SessionError> {
        let mut conn = self.pools.writer.get().await.map_err(map_pool_err)?;
        let row = SessionRow::from_entry(entry)?;
        let affected = diesel::update(sessions::table.filter(sessions::key.eq(row.key.clone())))
            .set((
                sessions::title.eq(&row.title),
                sessions::model.eq(&row.model),
                sessions::model_provider.eq(&row.model_provider),
                sessions::thinking_level.eq(&row.thinking_level),
                sessions::system_prompt.eq(&row.system_prompt),
                sessions::preview.eq(&row.preview),
                sessions::metadata.eq(&row.metadata),
                sessions::status.eq(&row.status),
                sessions::updated_at.eq(&row.updated_at),
            ))
            .execute(&mut *conn)
            .await
            .map_err(map_diesel_err)?;
        if affected == 0 {
            return Err(SessionError::NotFound {
                key: entry.key.to_string(),
            });
        }
        Ok(entry.clone())
    }

    async fn update_session_derived(
        &self,
        key: &SessionKey,
        derived: &SessionDerivedState,
    ) -> Result<(), SessionError> {
        use diesel_async::{AsyncConnection, scoped_futures::ScopedFutureExt};

        let anchors_json = serde_json::to_string(&derived.anchors).context(JsonSnafu)?;
        let updated_at = derived.updated_at.to_rfc3339();
        let key_str = key.to_string();
        let preview = derived.preview.clone();
        let total_entries = derived.total_entries;
        let last_token_usage = derived.last_token_usage;
        let estimated_context_tokens = derived.estimated_context_tokens;
        let entries_since_last_anchor = derived.entries_since_last_anchor;

        let mut conn = self.pools.writer.get().await.map_err(map_pool_err)?;
        // Wrap both UPDATEs in a single transaction so a concurrent
        // `update_session` (PATCH /sessions) cannot slip a write between
        // the derived-state UPDATE and the conditional preview UPDATE.
        conn.transaction::<_, diesel::result::Error, _>(|tx| {
            async move {
                diesel::update(sessions::table.filter(sessions::key.eq(&key_str)))
                    .set((
                        sessions::total_entries.eq(total_entries),
                        sessions::last_token_usage.eq(last_token_usage),
                        sessions::estimated_context_tokens.eq(estimated_context_tokens),
                        sessions::entries_since_last_anchor.eq(entries_since_last_anchor),
                        sessions::anchors_json.eq(&anchors_json),
                        sessions::updated_at.eq(&updated_at),
                    ))
                    .execute(tx)
                    .await?;

                // Preview is "what this conversation started as" — only
                // set it when the row currently has none. A second
                // UPDATE keeps the contract simple at the cost of one
                // extra statement on the (very rare) preview-write path.
                if let Some(preview) = &preview {
                    diesel::update(
                        sessions::table
                            .filter(sessions::key.eq(&key_str))
                            .filter(sessions::preview.is_null()),
                    )
                    .set(sessions::preview.eq(preview))
                    .execute(tx)
                    .await?;
                }
                Ok(())
            }
            .scope_boxed()
        })
        .await
        .map_err(map_diesel_err)?;
        Ok(())
    }

    async fn delete_session(&self, key: &SessionKey) -> Result<(), SessionError> {
        let mut conn = self.pools.writer.get().await.map_err(map_pool_err)?;
        let affected = diesel::delete(sessions::table.filter(sessions::key.eq(key.to_string())))
            .execute(&mut *conn)
            .await
            .map_err(map_diesel_err)?;
        if affected == 0 {
            return Err(SessionError::NotFound {
                key: key.to_string(),
            });
        }
        Ok(())
    }

    async fn bind_channel(&self, binding: &ChannelBinding) -> Result<ChannelBinding, SessionError> {
        let mut conn = self.pools.writer.get().await.map_err(map_pool_err)?;
        let channel_type = binding.channel_type.to_string();
        let session_key = binding.session_key.to_string();
        let created_at = binding.created_at.to_rfc3339();
        let updated_at = binding.updated_at.to_rfc3339();

        // Delete-then-insert upsert. Splitting the delete on `IS NULL`
        // vs `= ?` avoids SQLite's tri-valued NULL comparison
        // (`thread_id = NULL` never matches), which is the same gotcha
        // `get_channel_binding` solves the same way.
        let delete_query = session_channel_bindings::table
            .filter(session_channel_bindings::channel_type.eq(&channel_type))
            .filter(session_channel_bindings::chat_id.eq(&binding.chat_id));
        match &binding.thread_id {
            Some(tid) => {
                let _ = diesel::delete(
                    delete_query.filter(session_channel_bindings::thread_id.eq(tid)),
                )
                .execute(&mut *conn)
                .await
                .map_err(map_diesel_err)?;
            }
            None => {
                let _ = diesel::delete(
                    delete_query.filter(session_channel_bindings::thread_id.is_null()),
                )
                .execute(&mut *conn)
                .await
                .map_err(map_diesel_err)?;
            }
        }

        diesel::insert_into(session_channel_bindings::table)
            .values((
                session_channel_bindings::channel_type.eq(&channel_type),
                session_channel_bindings::chat_id.eq(&binding.chat_id),
                session_channel_bindings::thread_id.eq(&binding.thread_id),
                session_channel_bindings::session_key.eq(&session_key),
                session_channel_bindings::created_at.eq(&created_at),
                session_channel_bindings::updated_at.eq(&updated_at),
            ))
            .execute(&mut *conn)
            .await
            .map_err(map_diesel_err)?;
        Ok(binding.clone())
    }

    async fn get_channel_binding(
        &self,
        channel_type: ChannelType,
        chat_id: &str,
        thread_id: Option<&str>,
    ) -> Result<Option<ChannelBinding>, SessionError> {
        let mut conn = self.pools.reader.get().await.map_err(map_pool_err)?;
        let ct = channel_type.to_string();
        let row: Option<BindingRow> = match thread_id {
            Some(tid) => session_channel_bindings::table
                .filter(session_channel_bindings::channel_type.eq(&ct))
                .filter(session_channel_bindings::chat_id.eq(chat_id))
                .filter(session_channel_bindings::thread_id.eq(tid))
                .first(&mut *conn)
                .await
                .optional()
                .map_err(map_diesel_err)?,
            None => session_channel_bindings::table
                .filter(session_channel_bindings::channel_type.eq(&ct))
                .filter(session_channel_bindings::chat_id.eq(chat_id))
                .filter(session_channel_bindings::thread_id.is_null())
                .first(&mut *conn)
                .await
                .optional()
                .map_err(map_diesel_err)?,
        };
        row.map(BindingRow::into_binding).transpose()
    }

    async fn get_channel_binding_by_session(
        &self,
        key: &SessionKey,
    ) -> Result<Option<ChannelBinding>, SessionError> {
        let mut conn = self.pools.reader.get().await.map_err(map_pool_err)?;
        let row: Option<BindingRow> = session_channel_bindings::table
            .filter(session_channel_bindings::session_key.eq(key.to_string()))
            .first(&mut *conn)
            .await
            .optional()
            .map_err(map_diesel_err)?;
        row.map(BindingRow::into_binding).transpose()
    }

    async fn list_channel_bindings_by_session(
        &self,
        key: &SessionKey,
    ) -> Result<Vec<ChannelBinding>, SessionError> {
        let mut conn = self.pools.reader.get().await.map_err(map_pool_err)?;
        let rows: Vec<BindingRow> = session_channel_bindings::table
            .filter(session_channel_bindings::session_key.eq(key.to_string()))
            .load(&mut *conn)
            .await
            .map_err(map_diesel_err)?;
        rows.into_iter().map(BindingRow::into_binding).collect()
    }

    async fn unbind_session(&self, key: &SessionKey) -> Result<(), SessionError> {
        let mut conn = self.pools.writer.get().await.map_err(map_pool_err)?;
        let _ = diesel::delete(
            session_channel_bindings::table
                .filter(session_channel_bindings::session_key.eq(key.to_string())),
        )
        .execute(&mut *conn)
        .await
        .map_err(map_diesel_err)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Row marshalling
// ---------------------------------------------------------------------------

#[derive(diesel::Insertable, diesel::Queryable)]
#[diesel(table_name = sessions)]
struct SessionRow {
    key: String,
    title: Option<String>,
    model: Option<String>,
    model_provider: Option<String>,
    thinking_level: Option<String>,
    system_prompt: Option<String>,
    total_entries: i64,
    preview: Option<String>,
    last_token_usage: Option<i64>,
    estimated_context_tokens: i64,
    entries_since_last_anchor: i64,
    anchors_json: String,
    metadata: Option<String>,
    created_at: String,
    updated_at: String,
    /// Lower-cased `SessionStatus` (`"active"` / `"archived"`). Stored
    /// as `TEXT` because SQLite enforces the value space via the
    /// column-level `CHECK` constraint added in the
    /// `2026-05-01-132410-0000_session_status` migration.
    status: String,
}

impl SessionRow {
    fn from_entry(entry: &SessionEntry) -> Result<Self, SessionError> {
        let anchors_json = serde_json::to_string(&entry.anchors).context(JsonSnafu)?;
        let metadata = entry
            .metadata
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .context(JsonSnafu)?;
        Ok(Self {
            key: entry.key.to_string(),
            title: entry.title.clone(),
            model: entry.model.clone(),
            model_provider: entry.model_provider.clone(),
            thinking_level: entry.thinking_level.map(|t| t.to_string()),
            system_prompt: entry.system_prompt.clone(),
            total_entries: entry.total_entries,
            preview: entry.preview.clone(),
            last_token_usage: entry.last_token_usage,
            estimated_context_tokens: entry.estimated_context_tokens,
            entries_since_last_anchor: entry.entries_since_last_anchor,
            anchors_json,
            metadata,
            created_at: entry.created_at.to_rfc3339(),
            updated_at: entry.updated_at.to_rfc3339(),
            status: entry.status.to_string(),
        })
    }

    fn into_entry(self) -> Result<SessionEntry, SessionError> {
        let key = SessionKey::try_from_raw(&self.key).map_err(|_| SessionError::InvalidKey {
            message: format!("invalid session key in DB: {}", self.key),
        })?;
        let thinking_level = self
            .thinking_level
            .as_deref()
            .and_then(|s| s.parse::<ThinkingLevel>().ok());
        let metadata = self
            .metadata
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .context(JsonSnafu)?;
        let anchors: Vec<AnchorRef> =
            serde_json::from_str(&self.anchors_json).context(JsonSnafu)?;
        let created_at = parse_dt(&self.created_at)?;
        let updated_at = parse_dt(&self.updated_at)?;
        // Default to `Active` for any value SQLite produced that the
        // enum cannot parse — the column has a `CHECK` constraint, so
        // the only realistic path here is a row migrated in before the
        // constraint shipped (Decision 5).
        let status = self
            .status
            .parse::<SessionStatus>()
            .unwrap_or(SessionStatus::Active);
        Ok(SessionEntry {
            key,
            title: self.title,
            model: self.model,
            model_provider: self.model_provider,
            thinking_level,
            system_prompt: self.system_prompt,
            total_entries: self.total_entries,
            preview: self.preview,
            last_token_usage: self.last_token_usage,
            estimated_context_tokens: self.estimated_context_tokens,
            entries_since_last_anchor: self.entries_since_last_anchor,
            anchors,
            status,
            metadata,
            created_at,
            updated_at,
        })
    }
}

#[derive(diesel::Queryable)]
#[diesel(table_name = session_channel_bindings)]
struct BindingRow {
    channel_type: String,
    chat_id:      String,
    thread_id:    Option<String>,
    session_key:  String,
    created_at:   String,
    updated_at:   String,
}

impl BindingRow {
    fn into_binding(self) -> Result<ChannelBinding, SessionError> {
        let channel_type: ChannelType =
            self.channel_type
                .parse()
                .map_err(|_| SessionError::InvalidKey {
                    message: format!("invalid channel_type in DB: {}", self.channel_type),
                })?;
        let session_key =
            SessionKey::try_from_raw(&self.session_key).map_err(|_| SessionError::InvalidKey {
                message: format!("invalid session_key in DB: {}", self.session_key),
            })?;
        let created_at = parse_dt(&self.created_at)?;
        let updated_at = parse_dt(&self.updated_at)?;
        Ok(ChannelBinding {
            channel_type,
            chat_id: self.chat_id,
            thread_id: self.thread_id,
            session_key,
            created_at,
            updated_at,
        })
    }
}

fn parse_dt(s: &str) -> Result<DateTime<Utc>, SessionError> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|_| SessionError::InvalidKey {
            message: format!("invalid timestamp in DB: {s}"),
        })
}

fn map_diesel_err(e: diesel::result::Error) -> SessionError {
    SessionError::Database {
        message: format!("diesel: {e}"),
    }
}

fn map_pool_err(e: bb8::RunError<diesel_async::pooled_connection::PoolError>) -> SessionError {
    SessionError::Database {
        message: format!("pool: {e}"),
    }
}

async fn scan_json_dir(
    json_index_dir: &Path,
) -> Result<(Vec<PathBuf>, Vec<PathBuf>), SessionError> {
    let mut sessions_paths = Vec::new();
    let mut binding_paths = Vec::new();

    let mut top = tokio::fs::read_dir(json_index_dir)
        .await
        .context(FileIoSnafu)?;
    while let Some(entry) = top.next_entry().await.context(FileIoSnafu)? {
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("json") {
            sessions_paths.push(path);
        }
    }

    let bindings_dir = json_index_dir.join("bindings");
    if bindings_dir.is_dir() {
        let mut iter = tokio::fs::read_dir(&bindings_dir)
            .await
            .context(FileIoSnafu)?;
        while let Some(entry) = iter.next_entry().await.context(FileIoSnafu)? {
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("json") {
                binding_paths.push(path);
            }
        }
    }

    Ok((sessions_paths, binding_paths))
}
