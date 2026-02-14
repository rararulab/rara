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

//! Memory manager: incrementally syncs markdown files into the SQLite store.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use snafu::ResultExt;
use tracing::{debug, info};

use crate::{
    chunker,
    error::{self},
    store::SqliteMemoryStore,
    types::{MemoryDocument, SearchResult, SyncStats},
};

/// Manages the memory index by syncing markdown files from a data directory
/// into a [`SqliteMemoryStore`].
pub struct MemoryManager {
    store:    SqliteMemoryStore,
    data_dir: PathBuf,
}

impl MemoryManager {
    /// Create a new manager backed by the given store, reading files from
    /// `data_dir`.
    pub const fn new(store: SqliteMemoryStore, data_dir: PathBuf) -> Self {
        Self { store, data_dir }
    }

    /// Incrementally sync markdown files from the data directory into the
    /// store.
    ///
    /// - Scans all `.md` files recursively
    /// - Compares content hashes with stored hashes
    /// - Upserts changed/new documents, deletes removed ones
    pub async fn sync(&self) -> error::Result<SyncStats> {
        let mut stats = SyncStats::default();

        // Collect all .md files on disk.
        let disk_files = self.scan_markdown_files().await?;
        debug!(
            count = disk_files.len(),
            "scanned markdown files from data dir"
        );

        // Get all stored documents.
        let stored = self.store.list_documents().await?;
        let stored_map: HashMap<String, String> =
            stored.into_iter().collect();

        // Process each file on disk.
        for (doc_id, path) in &disk_files {
            let content =
                tokio::fs::read_to_string(path).await.context(error::IoSnafu)?;
            let hash = compute_hash(&content);

            if let Some(stored_hash) = stored_map.get(doc_id) {
                if *stored_hash == hash {
                    stats.unchanged += 1;
                    continue;
                }
                // Content changed — re-index.
                debug!(doc_id, "updating changed document");
                let doc = build_document(doc_id, &content, &hash);
                self.store.upsert_document(&doc).await?;
                stats.updated += 1;
            } else {
                // New document.
                debug!(doc_id, "adding new document");
                let doc = build_document(doc_id, &content, &hash);
                self.store.upsert_document(&doc).await?;
                stats.added += 1;
            }
        }

        // Detect deleted files.
        let disk_ids: std::collections::HashSet<&String> =
            disk_files.iter().map(|(id, _)| id).collect();
        for stored_id in stored_map.keys() {
            if !disk_ids.contains(stored_id) {
                debug!(doc_id = stored_id.as_str(), "removing deleted document");
                self.store.delete_document(stored_id).await?;
                stats.deleted += 1;
            }
        }

        info!(
            added = stats.added,
            updated = stats.updated,
            deleted = stats.deleted,
            unchanged = stats.unchanged,
            "memory sync complete"
        );

        Ok(stats)
    }

    /// Search the memory index.
    pub async fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> error::Result<Vec<SearchResult>> {
        self.store.search(query, limit).await
    }

    /// Retrieve a full document by ID.
    pub async fn get_document(
        &self,
        doc_id: &str,
    ) -> error::Result<Option<MemoryDocument>> {
        self.store.get_document(doc_id).await
    }

    /// Recursively scan the data directory for `.md` files.
    ///
    /// Returns a list of `(doc_id, absolute_path)` where `doc_id` is the path
    /// relative to the data directory.
    async fn scan_markdown_files(
        &self,
    ) -> error::Result<Vec<(String, PathBuf)>> {
        let mut results = Vec::new();
        if !self.data_dir.exists() {
            return Ok(results);
        }

        // Stack-based traversal to avoid async recursion.
        let mut dirs_to_visit = vec![self.data_dir.clone()];

        while let Some(dir) = dirs_to_visit.pop() {
            let mut entries =
                tokio::fs::read_dir(&dir).await.context(error::IoSnafu)?;

            while let Some(entry) =
                entries.next_entry().await.context(error::IoSnafu)?
            {
                let path = entry.path();
                let file_type =
                    entry.file_type().await.context(error::IoSnafu)?;

                if file_type.is_dir() {
                    dirs_to_visit.push(path);
                } else if file_type.is_file()
                    && let Some(ext) = path.extension()
                    && ext == "md"
                    && let Ok(rel) = path.strip_prefix(&self.data_dir)
                {
                    let doc_id = rel.to_string_lossy().to_string();
                    results.push((doc_id, path));
                }
            }
        }

        Ok(results)
    }
}

/// Compute the SHA-256 hash of content, returned as a hex string.
fn compute_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Build a [`MemoryDocument`] from raw file content.
fn build_document(doc_id: &str, content: &str, hash: &str) -> MemoryDocument {
    let title = chunker::extract_title(content)
        .unwrap_or_else(|| {
            // Fallback to filename without extension.
            Path::new(doc_id)
                .file_stem()
                .map_or_else(|| doc_id.to_owned(), |s| s.to_string_lossy().to_string())
        });

    let chunks = chunker::chunk_markdown(doc_id, content);
    #[allow(clippy::cast_possible_wrap)]
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    MemoryDocument {
        id: doc_id.to_owned(),
        title,
        content: content.to_owned(),
        chunks,
        hash: hash.to_owned(),
        updated_at: now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_sync_new_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let data_dir = tmp.path().join("memory");
        tokio::fs::create_dir_all(&data_dir).await.unwrap();

        // Write two markdown files.
        tokio::fs::write(
            data_dir.join("hello.md"),
            "# Hello\n\nWorld content here.",
        )
        .await
        .unwrap();
        tokio::fs::write(
            data_dir.join("guide.md"),
            "# Guide\n\n## Setup\n\nSetup instructions.\n\n## Usage\n\nUsage info.",
        )
        .await
        .unwrap();

        let db_path = tmp.path().join("test.db");
        let store = SqliteMemoryStore::open(&db_path).unwrap();
        let manager = MemoryManager::new(store, data_dir);

        let stats = manager.sync().await.unwrap();
        assert_eq!(stats.added, 2);
        assert_eq!(stats.updated, 0);
        assert_eq!(stats.deleted, 0);
        assert_eq!(stats.unchanged, 0);
    }

    #[tokio::test]
    async fn test_sync_unchanged() {
        let tmp = tempfile::TempDir::new().unwrap();
        let data_dir = tmp.path().join("memory");
        tokio::fs::create_dir_all(&data_dir).await.unwrap();
        tokio::fs::write(data_dir.join("a.md"), "# A\n\nContent.")
            .await
            .unwrap();

        let db_path = tmp.path().join("test.db");
        let store = SqliteMemoryStore::open(&db_path).unwrap();
        let manager = MemoryManager::new(store, data_dir);

        // First sync.
        let stats1 = manager.sync().await.unwrap();
        assert_eq!(stats1.added, 1);

        // Second sync — no changes.
        let stats2 = manager.sync().await.unwrap();
        assert_eq!(stats2.unchanged, 1);
        assert_eq!(stats2.added, 0);
        assert_eq!(stats2.updated, 0);
    }

    #[tokio::test]
    async fn test_sync_update_and_delete() {
        let tmp = tempfile::TempDir::new().unwrap();
        let data_dir = tmp.path().join("memory");
        tokio::fs::create_dir_all(&data_dir).await.unwrap();

        let file_path = data_dir.join("mutable.md");
        tokio::fs::write(&file_path, "# V1\n\nOriginal.")
            .await
            .unwrap();

        let db_path = tmp.path().join("test.db");
        let store = SqliteMemoryStore::open(&db_path).unwrap();
        let manager = MemoryManager::new(store, data_dir);

        manager.sync().await.unwrap();

        // Modify the file.
        tokio::fs::write(&file_path, "# V2\n\nUpdated content.")
            .await
            .unwrap();
        let stats = manager.sync().await.unwrap();
        assert_eq!(stats.updated, 1);

        // Delete the file.
        tokio::fs::remove_file(&file_path).await.unwrap();
        let stats = manager.sync().await.unwrap();
        assert_eq!(stats.deleted, 1);
    }

    #[tokio::test]
    async fn test_search_after_sync() {
        let tmp = tempfile::TempDir::new().unwrap();
        let data_dir = tmp.path().join("memory");
        tokio::fs::create_dir_all(&data_dir).await.unwrap();

        tokio::fs::write(
            data_dir.join("rust.md"),
            "# Rust\n\nRust is a systems programming language focused on safety.",
        )
        .await
        .unwrap();
        tokio::fs::write(
            data_dir.join("go.md"),
            "# Go\n\nGo is a statically typed compiled language by Google.",
        )
        .await
        .unwrap();

        let db_path = tmp.path().join("test.db");
        let store = SqliteMemoryStore::open(&db_path).unwrap();
        let manager = MemoryManager::new(store, data_dir);
        manager.sync().await.unwrap();

        let results = manager.search("rust safety", 10).await.unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].doc_id, "rust.md");
    }

    #[tokio::test]
    async fn test_get_document_after_sync() {
        let tmp = tempfile::TempDir::new().unwrap();
        let data_dir = tmp.path().join("memory");
        tokio::fs::create_dir_all(&data_dir).await.unwrap();

        tokio::fs::write(data_dir.join("note.md"), "# Note\n\nSome notes.")
            .await
            .unwrap();

        let db_path = tmp.path().join("test.db");
        let store = SqliteMemoryStore::open(&db_path).unwrap();
        let manager = MemoryManager::new(store, data_dir);
        manager.sync().await.unwrap();

        let doc = manager.get_document("note.md").await.unwrap().unwrap();
        assert_eq!(doc.title, "Note");
        assert!(doc.content.contains("Some notes."));
    }

    #[tokio::test]
    async fn test_sync_nested_directories() {
        let tmp = tempfile::TempDir::new().unwrap();
        let data_dir = tmp.path().join("memory");
        let sub_dir = data_dir.join("subdir");
        tokio::fs::create_dir_all(&sub_dir).await.unwrap();

        tokio::fs::write(data_dir.join("top.md"), "# Top\n\nTop level.")
            .await
            .unwrap();
        tokio::fs::write(sub_dir.join("nested.md"), "# Nested\n\nNested file.")
            .await
            .unwrap();

        let db_path = tmp.path().join("test.db");
        let store = SqliteMemoryStore::open(&db_path).unwrap();
        let manager = MemoryManager::new(store, data_dir);

        let stats = manager.sync().await.unwrap();
        assert_eq!(stats.added, 2);

        // Verify nested file is indexed with relative path.
        let doc = manager
            .get_document("subdir/nested.md")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(doc.title, "Nested");
    }

    #[tokio::test]
    async fn test_sync_empty_directory() {
        let tmp = tempfile::TempDir::new().unwrap();
        let data_dir = tmp.path().join("empty");
        tokio::fs::create_dir_all(&data_dir).await.unwrap();

        let db_path = tmp.path().join("test.db");
        let store = SqliteMemoryStore::open(&db_path).unwrap();
        let manager = MemoryManager::new(store, data_dir);

        let stats = manager.sync().await.unwrap();
        assert_eq!(stats.added, 0);
        assert_eq!(stats.unchanged, 0);
    }

    #[tokio::test]
    async fn test_sync_nonexistent_directory() {
        let tmp = tempfile::TempDir::new().unwrap();
        let data_dir = tmp.path().join("does_not_exist");

        let db_path = tmp.path().join("test.db");
        let store = SqliteMemoryStore::open(&db_path).unwrap();
        let manager = MemoryManager::new(store, data_dir);

        let stats = manager.sync().await.unwrap();
        assert_eq!(stats.added, 0);
    }
}
