//! Append-only JSONL message store with binary `.idx` index files.
//!
//! [`SessionStore`] manages per-session message files on the local filesystem.
//! Each session key maps to two files:
//!
//! - `{key}.jsonl` — one JSON-serialized [`ChatMessage`](crate::types::ChatMessage) per line
//! - `{key}.idx` — packed array of `u64` little-endian byte offsets into the JSONL file
//!
//! The index enables O(1) random access by sequence number without scanning
//! the entire JSONL file.

use std::path::PathBuf;

use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::{error::SessionError, types::ChatMessage};

/// Manages append-only JSONL message files with binary index files.
pub struct SessionStore {
    base_dir: PathBuf,
}

/// Sanitize a session key for use as a filename.
///
/// Replaces characters that are problematic in file paths (`:`, `/`, `\`)
/// with underscores.
fn sanitize_key(key: &str) -> String {
    key.replace([':', '/', '\\'], "_")
}

impl SessionStore {
    /// Create a new store rooted at `base_dir`.
    ///
    /// The directory is created if it does not exist.
    pub async fn new(base_dir: impl Into<PathBuf>) -> Result<Self, SessionError> {
        let base_dir = base_dir.into();
        tokio::fs::create_dir_all(&base_dir)
            .await
            .map_err(|e| SessionError::FileIo { source: e })?;
        Ok(Self { base_dir })
    }

    /// Return the path to the JSONL message file for `key`.
    fn jsonl_path(&self, key: &str) -> PathBuf {
        self.base_dir
            .join(format!("{}.jsonl", sanitize_key(key)))
    }

    /// Return the path to the binary index file for `key`.
    fn idx_path(&self, key: &str) -> PathBuf {
        self.base_dir.join(format!("{}.idx", sanitize_key(key)))
    }

    /// Ensure the index file is valid and return the message count.
    ///
    /// If the index is missing or corrupted (size not a multiple of 8), it is
    /// rebuilt by scanning the JSONL file.
    fn ensure_index(&self, key: &str) -> Result<u64, SessionError> {
        let idx = self.idx_path(key);
        let jsonl = self.jsonl_path(key);

        // If neither file exists, this is an empty session.
        let jsonl_exists = jsonl.exists();
        let idx_exists = idx.exists();

        if !jsonl_exists && !idx_exists {
            return Ok(0);
        }

        if !jsonl_exists && idx_exists {
            // Stale index without data — remove and report 0.
            let _ = std::fs::remove_file(&idx);
            return Ok(0);
        }

        // JSONL exists. Check if index is valid.
        if idx_exists {
            let meta =
                std::fs::metadata(&idx).map_err(|e| SessionError::FileIo { source: e })?;
            let size = meta.len();
            if size % 8 == 0 {
                // Check JSONL is not empty when idx reports 0 entries.
                if size == 0 {
                    let jsonl_meta = std::fs::metadata(&jsonl)
                        .map_err(|e| SessionError::FileIo { source: e })?;
                    if jsonl_meta.len() == 0 {
                        return Ok(0);
                    }
                    // JSONL has data but idx is empty — rebuild.
                } else {
                    return Ok(size / 8);
                }
            }
            // Size not a multiple of 8 or inconsistent — rebuild.
        }

        self.rebuild_index(key)
    }

    /// Rebuild the index by scanning the JSONL file line by line.
    ///
    /// Records the byte offset of each non-empty line as a `u64` LE value.
    fn rebuild_index(&self, key: &str) -> Result<u64, SessionError> {
        tracing::warn!(key, "rebuilding session index");

        let jsonl = self.jsonl_path(key);
        let idx = self.idx_path(key);

        let data =
            std::fs::read(&jsonl).map_err(|e| SessionError::FileIo { source: e })?;

        let mut offsets: Vec<u8> = Vec::new();
        let mut count: u64 = 0;
        let mut pos: u64 = 0;

        for line in data.split(|&b| b == b'\n') {
            let trimmed_empty = line.iter().all(|&b| b == b' ' || b == b'\t' || b == b'\r');
            if !line.is_empty() && !trimmed_empty {
                offsets.extend_from_slice(&pos.to_le_bytes());
                count += 1;
            }
            // +1 for the newline delimiter. split does not include it.
            pos += line.len() as u64 + 1;
        }

        std::fs::write(&idx, &offsets)
            .map_err(|e| SessionError::FileIo { source: e })?;

        Ok(count)
    }

    /// Append a message to the session identified by `key`.
    ///
    /// Assigns the next sequence number and returns the message with `seq` set.
    pub async fn append(
        &self,
        key: &str,
        msg: &ChatMessage,
    ) -> Result<ChatMessage, SessionError> {
        let count = self.ensure_index(key)?;
        let seq = count as i64 + 1;

        let mut msg = msg.clone();
        msg.seq = seq;

        let mut line =
            serde_json::to_string(&msg).map_err(|e| SessionError::Json { source: e })?;
        line.push('\n');

        let jsonl = self.jsonl_path(key);

        // Get current file size (= byte offset for the new line).
        let offset: u64 = match tokio::fs::metadata(&jsonl).await {
            Ok(m) => m.len(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
            Err(e) => return Err(SessionError::FileIo { source: e }),
        };

        // Append line to JSONL.
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&jsonl)
            .await
            .map_err(|e| SessionError::FileIo { source: e })?;
        file.write_all(line.as_bytes())
            .await
            .map_err(|e| SessionError::FileIo { source: e })?;
        file.flush()
            .await
            .map_err(|e| SessionError::FileIo { source: e })?;

        // Append offset to idx.
        let idx = self.idx_path(key);
        let mut idx_file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&idx)
            .await
            .map_err(|e| SessionError::FileIo { source: e })?;
        idx_file
            .write_all(&offset.to_le_bytes())
            .await
            .map_err(|e| SessionError::FileIo { source: e })?;
        idx_file
            .flush()
            .await
            .map_err(|e| SessionError::FileIo { source: e })?;

        Ok(msg)
    }

    /// Read messages from the session identified by `key`.
    ///
    /// - `after_seq`: if provided, only messages with `seq > after_seq` are
    ///   returned.
    /// - `limit`: caps the number of returned messages.
    pub async fn read(
        &self,
        key: &str,
        after_seq: Option<i64>,
        limit: Option<i64>,
    ) -> Result<Vec<ChatMessage>, SessionError> {
        let total = self.ensure_index(key)?;
        if total == 0 {
            return Ok(Vec::new());
        }

        let start_seq = (after_seq.unwrap_or(0).max(0) + 1) as u64;
        if start_seq > total {
            return Ok(Vec::new());
        }

        let max_msgs = limit.unwrap_or(i64::MAX).max(0) as u64;
        let messages_to_read = (total - start_seq + 1).min(max_msgs);
        if messages_to_read == 0 {
            return Ok(Vec::new());
        }

        let idx_path = self.idx_path(key);
        let jsonl_path = self.jsonl_path(key);

        // Read the relevant slice of offsets from the idx file.
        let idx_offset = (start_seq - 1) * 8;
        let idx_bytes_needed = messages_to_read * 8;

        let mut idx_file = tokio::fs::File::open(&idx_path)
            .await
            .map_err(|e| SessionError::FileIo { source: e })?;
        idx_file
            .seek(std::io::SeekFrom::Start(idx_offset))
            .await
            .map_err(|e| SessionError::FileIo { source: e })?;

        let mut idx_buf = vec![0u8; idx_bytes_needed as usize];
        idx_file
            .read_exact(&mut idx_buf)
            .await
            .map_err(|e| SessionError::FileIo { source: e })?;

        // Parse offsets.
        let offsets: Vec<u64> = idx_buf
            .chunks_exact(8)
            .map(|chunk| u64::from_le_bytes(chunk.try_into().unwrap()))
            .collect();

        // Read from the JSONL file starting at the first offset.
        let first_offset = offsets[0];
        let mut jsonl_file = tokio::fs::File::open(&jsonl_path)
            .await
            .map_err(|e| SessionError::FileIo { source: e })?;

        let file_len = jsonl_file
            .metadata()
            .await
            .map_err(|e| SessionError::FileIo { source: e })?
            .len();

        jsonl_file
            .seek(std::io::SeekFrom::Start(first_offset))
            .await
            .map_err(|e| SessionError::FileIo { source: e })?;

        let bytes_to_read = file_len - first_offset;
        let mut buf = vec![0u8; bytes_to_read as usize];
        jsonl_file
            .read_exact(&mut buf)
            .await
            .map_err(|e| SessionError::FileIo { source: e })?;

        // Parse each message by offset.
        let mut messages = Vec::with_capacity(offsets.len());
        for (i, &offset) in offsets.iter().enumerate() {
            let local_start = (offset - first_offset) as usize;

            // Find the end of this line (next newline).
            let line_end = buf[local_start..]
                .iter()
                .position(|&b| b == b'\n')
                .map(|p| local_start + p)
                .unwrap_or(buf.len());

            let line = &buf[local_start..line_end];
            let mut msg: ChatMessage =
                serde_json::from_slice(line).map_err(|e| SessionError::Json { source: e })?;
            msg.seq = start_seq as i64 + i as i64;
            messages.push(msg);
        }

        Ok(messages)
    }

    /// Return the number of messages in the session identified by `key`.
    pub async fn count(&self, key: &str) -> Result<i64, SessionError> {
        let n = self.ensure_index(key)?;
        Ok(n as i64)
    }

    /// Remove all messages (and index) for the session identified by `key`.
    pub async fn clear(&self, key: &str) -> Result<(), SessionError> {
        let jsonl = self.jsonl_path(key);
        let idx = self.idx_path(key);

        match tokio::fs::remove_file(&jsonl).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(SessionError::FileIo { source: e }),
        }
        match tokio::fs::remove_file(&idx).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(SessionError::FileIo { source: e }),
        }

        Ok(())
    }

    /// Delete all files for the session identified by `key`. Alias for
    /// [`clear`](Self::clear).
    pub async fn delete(&self, key: &str) -> Result<(), SessionError> {
        self.clear(key).await
    }

    /// Fork the messages of `src_key` up to `fork_at_seq` into `dst_key`.
    ///
    /// The destination is cleared first, then all messages with
    /// `seq <= fork_at_seq` are copied.
    pub async fn fork(
        &self,
        src_key: &str,
        dst_key: &str,
        fork_at_seq: i64,
    ) -> Result<(), SessionError> {
        let total = self.ensure_index(src_key)? as i64;

        if fork_at_seq < 1 || fork_at_seq > total {
            return Err(SessionError::InvalidForkPoint {
                key: src_key.to_owned(),
                seq: fork_at_seq,
            });
        }

        // Read source messages up to fork_at_seq.
        let messages = self
            .read(src_key, None, Some(fork_at_seq))
            .await?;

        // Clear destination.
        self.clear(dst_key).await?;

        // Build JSONL content and idx data together.
        let mut jsonl_content = Vec::new();
        let mut idx_data = Vec::new();

        for msg in &messages {
            let offset = jsonl_content.len() as u64;
            idx_data.extend_from_slice(&offset.to_le_bytes());

            let line =
                serde_json::to_string(msg).map_err(|e| SessionError::Json { source: e })?;
            jsonl_content.extend_from_slice(line.as_bytes());
            jsonl_content.push(b'\n');
        }

        // Write both files.
        let jsonl_path = self.jsonl_path(dst_key);
        let idx_path = self.idx_path(dst_key);

        tokio::fs::write(&jsonl_path, &jsonl_content)
            .await
            .map_err(|e| SessionError::FileIo { source: e })?;
        tokio::fs::write(&idx_path, &idx_data)
            .await
            .map_err(|e| SessionError::FileIo { source: e })?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ChatMessage;

    async fn test_store() -> (SessionStore, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let store = SessionStore::new(tmp.path()).await.unwrap();
        (store, tmp) // keep TempDir alive
    }

    #[test]
    fn sanitize_key_for_filename() {
        assert_eq!(sanitize_key("user:alice"), "user_alice");
        assert_eq!(sanitize_key("dm:alice:bob"), "dm_alice_bob");
        assert_eq!(sanitize_key("simple"), "simple");
    }

    #[tokio::test]
    async fn empty_session_count_is_zero() {
        let (store, _tmp) = test_store().await;
        assert_eq!(store.count("x").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn append_increments_seq() {
        let (store, _tmp) = test_store().await;

        let m1 = store.append("k", &ChatMessage::user("a")).await.unwrap();
        let m2 = store.append("k", &ChatMessage::user("b")).await.unwrap();
        let m3 = store.append("k", &ChatMessage::user("c")).await.unwrap();

        assert_eq!(m1.seq, 1);
        assert_eq!(m2.seq, 2);
        assert_eq!(m3.seq, 3);
    }

    #[tokio::test]
    async fn read_all_messages() {
        let (store, _tmp) = test_store().await;

        store.append("k", &ChatMessage::user("a")).await.unwrap();
        store.append("k", &ChatMessage::assistant("b")).await.unwrap();
        store.append("k", &ChatMessage::user("c")).await.unwrap();

        let msgs = store.read("k", None, None).await.unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].content.as_text(), "a");
        assert_eq!(msgs[1].content.as_text(), "b");
        assert_eq!(msgs[2].content.as_text(), "c");
    }

    #[tokio::test]
    async fn read_with_after_seq() {
        let (store, _tmp) = test_store().await;

        for i in 1..=5 {
            store
                .append("k", &ChatMessage::user(format!("m{i}")))
                .await
                .unwrap();
        }

        let msgs = store.read("k", Some(2), None).await.unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].seq, 3);
        assert_eq!(msgs[0].content.as_text(), "m3");
        assert_eq!(msgs[2].seq, 5);
    }

    #[tokio::test]
    async fn read_with_limit() {
        let (store, _tmp) = test_store().await;

        for i in 1..=5 {
            store
                .append("k", &ChatMessage::user(format!("m{i}")))
                .await
                .unwrap();
        }

        let msgs = store.read("k", None, Some(2)).await.unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].seq, 1);
        assert_eq!(msgs[1].seq, 2);
    }

    #[tokio::test]
    async fn read_with_after_seq_and_limit() {
        let (store, _tmp) = test_store().await;

        for i in 1..=5 {
            store
                .append("k", &ChatMessage::user(format!("m{i}")))
                .await
                .unwrap();
        }

        let msgs = store.read("k", Some(1), Some(2)).await.unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].seq, 2);
        assert_eq!(msgs[0].content.as_text(), "m2");
        assert_eq!(msgs[1].seq, 3);
        assert_eq!(msgs[1].content.as_text(), "m3");
    }

    #[tokio::test]
    async fn clear_removes_both_files() {
        let (store, _tmp) = test_store().await;

        store.append("k", &ChatMessage::user("a")).await.unwrap();

        let jsonl = store.jsonl_path("k");
        let idx = store.idx_path("k");
        assert!(jsonl.exists());
        assert!(idx.exists());

        store.clear("k").await.unwrap();

        assert!(!jsonl.exists());
        assert!(!idx.exists());
        assert_eq!(store.count("k").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn fork_copies_correct_messages() {
        let (store, _tmp) = test_store().await;

        for i in 1..=5 {
            store
                .append("src", &ChatMessage::user(format!("m{i}")))
                .await
                .unwrap();
        }

        store.fork("src", "dst", 3).await.unwrap();

        let dst_msgs = store.read("dst", None, None).await.unwrap();
        assert_eq!(dst_msgs.len(), 3);
        assert_eq!(dst_msgs[0].content.as_text(), "m1");
        assert_eq!(dst_msgs[2].content.as_text(), "m3");

        // Source is unchanged.
        let src_msgs = store.read("src", None, None).await.unwrap();
        assert_eq!(src_msgs.len(), 5);
    }

    #[tokio::test]
    async fn fork_invalid_seq_returns_error() {
        let (store, _tmp) = test_store().await;

        store.append("k", &ChatMessage::user("a")).await.unwrap();
        store.append("k", &ChatMessage::user("b")).await.unwrap();

        // seq 0 is invalid.
        let r1 = store.fork("k", "out", 0).await;
        assert!(matches!(
            r1.unwrap_err(),
            SessionError::InvalidForkPoint { .. }
        ));

        // seq 3 is beyond total (2).
        let r2 = store.fork("k", "out", 3).await;
        assert!(matches!(
            r2.unwrap_err(),
            SessionError::InvalidForkPoint { .. }
        ));
    }

    #[tokio::test]
    async fn missing_idx_triggers_rebuild() {
        let (store, _tmp) = test_store().await;

        store.append("k", &ChatMessage::user("a")).await.unwrap();
        store.append("k", &ChatMessage::user("b")).await.unwrap();
        store.append("k", &ChatMessage::user("c")).await.unwrap();

        // Delete idx.
        let idx = store.idx_path("k");
        tokio::fs::remove_file(&idx).await.unwrap();
        assert!(!idx.exists());

        // Count should still work via rebuild.
        assert_eq!(store.count("k").await.unwrap(), 3);
        assert!(idx.exists());

        // Read should work too.
        let msgs = store.read("k", None, None).await.unwrap();
        assert_eq!(msgs.len(), 3);
    }

    #[tokio::test]
    async fn corrupted_idx_triggers_rebuild() {
        let (store, _tmp) = test_store().await;

        store.append("k", &ChatMessage::user("a")).await.unwrap();
        store.append("k", &ChatMessage::user("b")).await.unwrap();

        // Write garbage to idx (not a multiple of 8 bytes).
        let idx = store.idx_path("k");
        tokio::fs::write(&idx, b"garbage!x").await.unwrap();

        // Should rebuild and still return correct count.
        assert_eq!(store.count("k").await.unwrap(), 2);

        let msgs = store.read("k", None, None).await.unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].content.as_text(), "a");
        assert_eq!(msgs[1].content.as_text(), "b");
    }

    #[tokio::test]
    async fn read_empty_returns_empty() {
        let (store, _tmp) = test_store().await;

        let msgs = store.read("nonexistent", None, None).await.unwrap();
        assert!(msgs.is_empty());
    }

    #[tokio::test]
    async fn read_beyond_total_returns_empty() {
        let (store, _tmp) = test_store().await;

        store.append("k", &ChatMessage::user("a")).await.unwrap();
        store.append("k", &ChatMessage::user("b")).await.unwrap();

        let msgs = store.read("k", Some(5), None).await.unwrap();
        assert!(msgs.is_empty());
    }

    #[tokio::test]
    async fn append_after_clear_resets_seq() {
        let (store, _tmp) = test_store().await;

        store.append("k", &ChatMessage::user("a")).await.unwrap();
        store.append("k", &ChatMessage::user("b")).await.unwrap();

        store.clear("k").await.unwrap();

        let m1 = store.append("k", &ChatMessage::user("x")).await.unwrap();
        assert_eq!(m1.seq, 1);

        let msgs = store.read("k", None, None).await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content.as_text(), "x");
    }
}
