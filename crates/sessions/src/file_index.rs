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

//! File-based session index — stores session metadata as JSON files.
//!
//! Each session is stored as `{index_dir}/{key}.json`. Channel bindings
//! are stored in `{index_dir}/bindings/{channel_type}_{chat_id}.json`.
//!
//! Writes are atomic: data is written to a `.tmp` file first, then renamed.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use rara_kernel::{
    channel::types::ChannelType,
    session::{ChannelBinding, SessionEntry, SessionError, SessionIndex, SessionKey},
};
use tokio::fs;

/// File-based implementation of [`SessionIndex`].
///
/// Stores session metadata as individual JSON files on disk. This is
/// intentionally simple — no database, no WAL, just files. Suitable for
/// single-node deployments where the tape subsystem handles message storage.
pub struct FileSessionIndex {
    /// Root directory for session index files.
    index_dir: PathBuf,
}

impl FileSessionIndex {
    /// Create a new `FileSessionIndex` rooted at the given directory.
    ///
    /// Creates the directory (and `bindings/` subdirectory) if they don't
    /// exist.
    pub async fn new(index_dir: impl Into<PathBuf>) -> Result<Self, SessionError> {
        let index_dir = index_dir.into();
        fs::create_dir_all(&index_dir)
            .await
            .map_err(|source| SessionError::FileIo { source })?;
        fs::create_dir_all(index_dir.join("bindings"))
            .await
            .map_err(|source| SessionError::FileIo { source })?;
        Ok(Self { index_dir })
    }

    /// Path to the JSON file for a given session key.
    fn session_path(&self, key: &SessionKey) -> PathBuf {
        self.index_dir.join(format!("{key}.json"))
    }

    /// Path to a temporary file for atomic writes.
    fn tmp_path(&self, key: &SessionKey) -> PathBuf {
        self.index_dir.join(format!("{key}.json.tmp"))
    }

    /// Path to a channel binding file.
    ///
    /// When `thread_id` is `Some`, the suffix `_t{id}` is appended so that
    /// different forum topics within the same chat get separate bindings.
    fn binding_path(&self, channel_type: &str, chat_id: &str, thread_id: Option<&str>) -> PathBuf {
        // Sanitize thread_id to prevent path traversal. Telegram sends
        // integer IDs, but defense-in-depth strips any non-alphanumeric chars.
        let name = match thread_id {
            Some(tid) => {
                let safe_tid: String = tid
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || c == '-' {
                            c
                        } else {
                            '_'
                        }
                    })
                    .collect();
                format!("{channel_type}_{chat_id}_t{safe_tid}.json")
            }
            None => format!("{channel_type}_{chat_id}.json"),
        };
        self.index_dir.join("bindings").join(name)
    }

    /// Atomically write JSON to a file (write .tmp then rename).
    async fn atomic_write(&self, path: &Path, tmp: &Path, data: &[u8]) -> Result<(), SessionError> {
        fs::write(tmp, data)
            .await
            .map_err(|source| SessionError::FileIo { source })?;
        fs::rename(tmp, path)
            .await
            .map_err(|source| SessionError::FileIo { source })?;
        Ok(())
    }

    /// Read and deserialize a JSON file, returning `None` if not found.
    async fn read_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &Path,
    ) -> Result<Option<T>, SessionError> {
        match fs::read(path).await {
            Ok(data) => {
                let value = serde_json::from_slice(&data)
                    .map_err(|source| SessionError::Json { source })?;
                Ok(Some(value))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(SessionError::FileIo { source }),
        }
    }
}

#[async_trait]
impl SessionIndex for FileSessionIndex {
    async fn create_session(&self, entry: &SessionEntry) -> Result<SessionEntry, SessionError> {
        let path = self.session_path(&entry.key);
        if path.exists() {
            return Err(SessionError::AlreadyExists {
                key: entry.key.to_string(),
            });
        }

        let data =
            serde_json::to_vec_pretty(entry).map_err(|source| SessionError::Json { source })?;
        let tmp = self.tmp_path(&entry.key);
        self.atomic_write(&path, &tmp, &data).await?;
        Ok(entry.clone())
    }

    async fn get_session(&self, key: &SessionKey) -> Result<Option<SessionEntry>, SessionError> {
        let path = self.session_path(key);
        self.read_json(&path).await
    }

    async fn list_sessions(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<SessionEntry>, SessionError> {
        let mut entries = Vec::new();
        let mut dir = fs::read_dir(&self.index_dir)
            .await
            .map_err(|source| SessionError::FileIo { source })?;

        while let Some(entry) = dir
            .next_entry()
            .await
            .map_err(|source| SessionError::FileIo { source })?
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            // Skip directories (like "bindings/")
            if path.is_dir() {
                continue;
            }

            if let Some(session) = self.read_json::<SessionEntry>(&path).await? {
                entries.push(session);
            }
        }

        // Sort by updated_at descending.
        entries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

        // Apply offset and limit.
        let offset = offset.max(0) as usize;
        let limit = limit.max(0) as usize;
        let result = entries.into_iter().skip(offset).take(limit).collect();

        Ok(result)
    }

    async fn update_session(&self, entry: &SessionEntry) -> Result<SessionEntry, SessionError> {
        let path = self.session_path(&entry.key);
        if !path.exists() {
            return Err(SessionError::NotFound {
                key: entry.key.to_string(),
            });
        }

        let data =
            serde_json::to_vec_pretty(entry).map_err(|source| SessionError::Json { source })?;
        let tmp = self.tmp_path(&entry.key);
        self.atomic_write(&path, &tmp, &data).await?;
        Ok(entry.clone())
    }

    async fn delete_session(&self, key: &SessionKey) -> Result<(), SessionError> {
        let path = self.session_path(key);
        match fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(SessionError::NotFound {
                key: key.to_string(),
            }),
            Err(source) => Err(SessionError::FileIo { source }),
        }
    }

    async fn bind_channel(&self, binding: &ChannelBinding) -> Result<ChannelBinding, SessionError> {
        let ct = binding.channel_type.to_string();
        let path = self.binding_path(&ct, &binding.chat_id, binding.thread_id.as_deref());
        // Derive tmp path from the sanitized binding path so both go through
        // the same sanitize logic in binding_path().
        let tmp = path.with_extension("json.tmp");

        let data =
            serde_json::to_vec_pretty(binding).map_err(|source| SessionError::Json { source })?;
        self.atomic_write(&path, &tmp, &data).await?;
        Ok(binding.clone())
    }

    async fn get_channel_binding(
        &self,
        channel_type: ChannelType,
        chat_id: &str,
        thread_id: Option<&str>,
    ) -> Result<Option<ChannelBinding>, SessionError> {
        let path = self.binding_path(&channel_type.to_string(), chat_id, thread_id);
        self.read_json(&path).await
    }

    async fn get_channel_binding_by_session(
        &self,
        key: &SessionKey,
    ) -> Result<Option<ChannelBinding>, SessionError> {
        let bindings_dir = self.index_dir.join("bindings");
        let mut dir = fs::read_dir(&bindings_dir)
            .await
            .map_err(|source| SessionError::FileIo { source })?;

        while let Some(entry) = dir
            .next_entry()
            .await
            .map_err(|source| SessionError::FileIo { source })?
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Some(binding) = self.read_json::<ChannelBinding>(&path).await? {
                if binding.session_key == *key {
                    return Ok(Some(binding));
                }
            }
        }
        Ok(None)
    }

    async fn unbind_session(&self, key: &SessionKey) -> Result<(), SessionError> {
        let bindings_dir = self.index_dir.join("bindings");
        let mut dir = fs::read_dir(&bindings_dir)
            .await
            .map_err(|source| SessionError::FileIo { source })?;

        while let Some(entry) = dir
            .next_entry()
            .await
            .map_err(|source| SessionError::FileIo { source })?
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Some(binding) = self.read_json::<ChannelBinding>(&path).await? {
                if binding.session_key == *key {
                    fs::remove_file(&path)
                        .await
                        .map_err(|source| SessionError::FileIo { source })?;
                }
            }
        }
        Ok(())
    }
}
