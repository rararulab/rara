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
use rara_kernel::session::{ChannelBinding, SessionEntry, SessionError, SessionIndex, SessionKey};
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
    fn binding_path(&self, channel_type: &str, chat_id: &str) -> PathBuf {
        self.index_dir
            .join("bindings")
            .join(format!("{channel_type}_{chat_id}.json"))
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
        let path = self.binding_path(&binding.channel_type, &binding.chat_id);
        let tmp = self.index_dir.join("bindings").join(format!(
            "{}_{}.json.tmp",
            binding.channel_type, binding.chat_id
        ));

        let data =
            serde_json::to_vec_pretty(binding).map_err(|source| SessionError::Json { source })?;
        self.atomic_write(&path, &tmp, &data).await?;
        Ok(binding.clone())
    }

    async fn get_channel_binding(
        &self,
        channel_type: &str,
        chat_id: &str,
    ) -> Result<Option<ChannelBinding>, SessionError> {
        let path = self.binding_path(channel_type, chat_id);
        self.read_json(&path).await
    }
}
