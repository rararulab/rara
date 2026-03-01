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

//! File-system WAL (Write-Ahead Log) for kernel events.
//!
//! ## Format
//!
//! Each line in the WAL file is a JSON object:
//!
//! ```json
//! {"id":1,"event":{...},"completed":false}
//! ```
//!
//! On [`mark_completed`](WalQueue::mark_completed), the corresponding entry
//! is marked `completed: true`. Periodic truncation rewrites the file,
//! discarding completed entries.
//!
//! ## Crash Recovery
//!
//! On startup, call [`WalQueue::recover`] to replay all non-completed entries
//! back into the in-memory queue.

use std::{
    collections::HashSet,
    io::{BufRead, Write as IoWrite},
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use serde::{Deserialize, Serialize};
use snafu::Snafu;

use rara_kernel::unified_event::PersistableEvent;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors from WAL operations.
#[derive(Debug, Snafu)]
pub enum WalError {
    /// I/O error during WAL read/write.
    #[snafu(display("WAL I/O error: {source}"))]
    Io { source: std::io::Error },
    /// JSON serialization/deserialization error.
    #[snafu(display("WAL serialization error: {source}"))]
    Serde { source: serde_json::Error },
}

impl From<std::io::Error> for WalError {
    fn from(source: std::io::Error) -> Self { Self::Io { source } }
}

impl From<serde_json::Error> for WalError {
    fn from(source: serde_json::Error) -> Self { Self::Serde { source } }
}

// ---------------------------------------------------------------------------
// WalEntry
// ---------------------------------------------------------------------------

/// A single line in the WAL file. Uses a tagged enum so both event entries
/// and completion markers share the same JSON-lines file.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WalLine {
    /// An event entry.
    Entry {
        /// Monotonically increasing entry ID.
        id:    u64,
        /// The persistable event payload.
        event: PersistableEvent,
    },
    /// A completion marker — indicates that entry `id` has been processed.
    Completed {
        /// The entry ID that was completed.
        id: u64,
    },
}

// ---------------------------------------------------------------------------
// WalQueue
// ---------------------------------------------------------------------------

/// File-system WAL for durable event persistence.
///
/// Events are appended as JSON lines. Completed entries are tracked in
/// memory and periodically purged via [`truncate`](Self::truncate).
pub struct WalQueue {
    /// Path to the WAL file.
    path:           PathBuf,
    /// Monotonically increasing entry ID counter.
    next_id:        AtomicU64,
    /// Set of completed entry IDs (for truncation).
    completed:      Mutex<HashSet<u64>>,
    /// File handle protected by a mutex for serialized writes.
    writer:         Mutex<std::fs::File>,
    /// How many completed entries trigger an automatic truncation.
    truncate_after: usize,
}

impl WalQueue {
    /// Open (or create) a WAL file at the given path.
    ///
    /// If the file already exists, the next ID is set to one past the
    /// highest existing entry ID.
    pub fn open(path: impl AsRef<Path>, truncate_after: usize) -> Result<Self, WalError> {
        let path = path.as_ref().to_path_buf();

        // Scan existing lines to find the maximum entry ID and completed set.
        let mut max_id: u64 = 0;
        let mut completed_set = HashSet::new();
        if path.exists() {
            let file = std::fs::File::open(&path)?;
            let reader = std::io::BufReader::new(file);
            for line in reader.lines() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                if let Ok(wal_line) = serde_json::from_str::<WalLine>(&line) {
                    match wal_line {
                        WalLine::Entry { id, .. } => {
                            if id > max_id {
                                max_id = id;
                            }
                        }
                        WalLine::Completed { id } => {
                            completed_set.insert(id);
                        }
                    }
                }
            }
        }

        let writer = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;

        Ok(Self {
            path,
            next_id: AtomicU64::new(max_id + 1),
            completed: Mutex::new(completed_set),
            writer: Mutex::new(writer),
            truncate_after,
        })
    }

    /// Append a persistable event to the WAL. Returns the assigned entry ID.
    pub fn append(&self, event: &PersistableEvent) -> Result<u64, WalError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let wal_line = WalLine::Entry {
            id,
            event: event.clone(),
        };
        let mut line = serde_json::to_string(&wal_line)?;
        line.push('\n');

        let mut writer = self.writer.lock().expect("WAL writer lock poisoned");
        writer.write_all(line.as_bytes())?;
        writer.flush()?;
        Ok(id)
    }

    /// Mark an entry as completed by appending a completion marker to the
    /// WAL file. If the completed count exceeds the truncation threshold,
    /// an automatic truncation is triggered.
    pub fn mark_completed(&self, wal_id: u64) -> Result<(), WalError> {
        // Write completion marker to WAL file for crash durability.
        let marker = WalLine::Completed { id: wal_id };
        let mut line = serde_json::to_string(&marker)?;
        line.push('\n');
        {
            let mut writer = self.writer.lock().expect("WAL writer lock poisoned");
            writer.write_all(line.as_bytes())?;
            writer.flush()?;
        }

        let should_truncate = {
            let mut completed = self.completed.lock().expect("WAL completed lock poisoned");
            completed.insert(wal_id);
            completed.len() >= self.truncate_after
        };
        if should_truncate {
            self.truncate()?;
        }
        Ok(())
    }

    /// Recover all non-completed entries from the WAL file.
    ///
    /// Returns entries in ID order (ascending) so they can be replayed
    /// into the in-memory queue in the original push order.
    pub fn recover(&self) -> Result<Vec<(u64, PersistableEvent)>, WalError> {
        let completed = self.completed.lock().expect("WAL completed lock poisoned");
        let file = std::fs::File::open(&self.path)?;
        let reader = std::io::BufReader::new(file);
        let mut entries: Vec<(u64, PersistableEvent)> = Vec::new();

        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(wal_line) = serde_json::from_str::<WalLine>(&line) {
                if let WalLine::Entry { id, event } = wal_line {
                    if !completed.contains(&id) {
                        entries.push((id, event));
                    }
                }
            }
        }

        // Sort by ID to maintain original order.
        entries.sort_by_key(|(id, _)| *id);
        Ok(entries)
    }

    /// Rewrite the WAL file, discarding completed entries and their markers.
    ///
    /// This is the truncation strategy: read all lines, filter out
    /// completed entries (and completion markers), rewrite atomically.
    pub fn truncate(&self) -> Result<(), WalError> {
        let completed = self.completed.lock().expect("WAL completed lock poisoned");

        // Read all lines, keep only non-completed entry lines.
        let file = std::fs::File::open(&self.path)?;
        let reader = std::io::BufReader::new(file);
        let mut kept_lines: Vec<WalLine> = Vec::new();

        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(wal_line) = serde_json::from_str::<WalLine>(&line) {
                match &wal_line {
                    WalLine::Entry { id, .. } if !completed.contains(id) => {
                        kept_lines.push(wal_line);
                    }
                    // Discard completed entries and all completion markers.
                    _ => {}
                }
            }
        }

        let kept_count = kept_lines.len();

        // Write to a temporary file, then rename (atomic on most FS).
        let tmp_path = self.path.with_extension("tmp");
        {
            let mut tmp = std::fs::File::create(&tmp_path)?;
            for wal_line in &kept_lines {
                let mut line = serde_json::to_string(wal_line)?;
                line.push('\n');
                tmp.write_all(line.as_bytes())?;
            }
            tmp.flush()?;
        }

        std::fs::rename(&tmp_path, &self.path)?;

        // Re-open the writer (append mode).
        let new_writer = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let mut writer = self.writer.lock().expect("WAL writer lock poisoned");
        *writer = new_writer;

        // Clear completed set since those entries are now gone from the file.
        drop(completed); // Release the lock above.
        {
            let mut completed = self.completed.lock().expect("WAL completed lock poisoned");
            completed.clear();
        }

        tracing::debug!(
            kept = kept_count,
            "WAL truncated"
        );

        Ok(())
    }

    /// Path to the WAL file (for diagnostics).
    pub fn path(&self) -> &Path { &self.path }
}

impl std::fmt::Debug for WalQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let completed_count = self
            .completed
            .lock()
            .map(|c| c.len())
            .unwrap_or(0);
        f.debug_struct("WalQueue")
            .field("path", &self.path)
            .field("next_id", &self.next_id.load(Ordering::Relaxed))
            .field("completed_count", &completed_count)
            .field("truncate_after", &self.truncate_after)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rara_kernel::{
        channel::types::{ChannelType, MessageContent},
        io::types::{ChannelSource, InboundMessage, MessageId},
        process::{SessionId, principal::UserId},
    };
    use std::collections::HashMap;

    fn test_persistable_event(text: &str) -> PersistableEvent {
        PersistableEvent::UserMessage(InboundMessage {
            id:              MessageId::new(),
            source:          ChannelSource {
                channel_type:        ChannelType::Internal,
                platform_message_id: None,
                platform_user_id:    "test".to_string(),
                platform_chat_id:    None,
            },
            user:            UserId("u1".to_string()),
            session_id:      SessionId::new("s1"),
            target_agent_id: None,
            target_agent:    None,
            content:         MessageContent::Text(text.to_string()),
            reply_context:   None,
            timestamp:       jiff::Timestamp::now(),
            metadata:        HashMap::new(),
        })
    }

    #[test]
    fn wal_append_and_recover() {
        let dir = tempfile::tempdir().unwrap();
        let wal_path = dir.path().join("test.wal");

        let wal = WalQueue::open(&wal_path, 100).unwrap();
        let id1 = wal.append(&test_persistable_event("hello")).unwrap();
        let id2 = wal.append(&test_persistable_event("world")).unwrap();

        assert_eq!(id1, 1);
        assert_eq!(id2, 2);

        let entries = wal.recover().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, 1);
        assert_eq!(entries[1].0, 2);
    }

    #[test]
    fn wal_mark_completed_filters_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let wal_path = dir.path().join("test.wal");

        let wal = WalQueue::open(&wal_path, 100).unwrap();
        let id1 = wal.append(&test_persistable_event("a")).unwrap();
        let _id2 = wal.append(&test_persistable_event("b")).unwrap();
        let id3 = wal.append(&test_persistable_event("c")).unwrap();

        wal.mark_completed(id1).unwrap();
        wal.mark_completed(id3).unwrap();

        let entries = wal.recover().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, 2); // Only "b" remains
    }

    #[test]
    fn wal_truncation_removes_completed() {
        let dir = tempfile::tempdir().unwrap();
        let wal_path = dir.path().join("test.wal");

        let wal = WalQueue::open(&wal_path, 100).unwrap();
        let id1 = wal.append(&test_persistable_event("a")).unwrap();
        let _id2 = wal.append(&test_persistable_event("b")).unwrap();
        let id3 = wal.append(&test_persistable_event("c")).unwrap();

        wal.mark_completed(id1).unwrap();
        wal.mark_completed(id3).unwrap();
        wal.truncate().unwrap();

        // Re-open and recover — only "b" should remain
        let wal2 = WalQueue::open(&wal_path, 100).unwrap();
        let entries = wal2.recover().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, 2);
    }

    #[test]
    fn wal_auto_truncate_on_threshold() {
        let dir = tempfile::tempdir().unwrap();
        let wal_path = dir.path().join("test.wal");

        // Threshold of 2 — after 2 completions, auto-truncate
        let wal = WalQueue::open(&wal_path, 2).unwrap();
        let id1 = wal.append(&test_persistable_event("a")).unwrap();
        let id2 = wal.append(&test_persistable_event("b")).unwrap();
        let _id3 = wal.append(&test_persistable_event("c")).unwrap();

        wal.mark_completed(id1).unwrap();
        // This should trigger auto-truncate (threshold=2)
        wal.mark_completed(id2).unwrap();

        // After truncation, only "c" remains in the file
        let wal2 = WalQueue::open(&wal_path, 100).unwrap();
        let entries = wal2.recover().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, 3);
    }

    #[test]
    fn wal_reopen_continues_id_sequence() {
        let dir = tempfile::tempdir().unwrap();
        let wal_path = dir.path().join("test.wal");

        {
            let wal = WalQueue::open(&wal_path, 100).unwrap();
            wal.append(&test_persistable_event("a")).unwrap();
            wal.append(&test_persistable_event("b")).unwrap();
        }

        // Reopen
        let wal = WalQueue::open(&wal_path, 100).unwrap();
        let id3 = wal.append(&test_persistable_event("c")).unwrap();
        assert_eq!(id3, 3);
    }

    #[test]
    fn wal_empty_file_recover() {
        let dir = tempfile::tempdir().unwrap();
        let wal_path = dir.path().join("test.wal");

        let wal = WalQueue::open(&wal_path, 100).unwrap();
        let entries = wal.recover().unwrap();
        assert_eq!(entries.len(), 0);
    }

    #[test]
    fn wal_timer_event_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let wal_path = dir.path().join("test.wal");

        let wal = WalQueue::open(&wal_path, 100).unwrap();
        let event = PersistableEvent::Timer {
            name:    "tick".to_string(),
            payload: serde_json::json!({"interval": 30}),
        };
        let id = wal.append(&event).unwrap();
        assert_eq!(id, 1);

        let entries = wal.recover().unwrap();
        assert_eq!(entries.len(), 1);
        match &entries[0].1 {
            PersistableEvent::Timer { name, payload } => {
                assert_eq!(name, "tick");
                assert_eq!(*payload, serde_json::json!({"interval": 30}));
            }
            _ => panic!("expected Timer event"),
        }
    }

    #[test]
    fn wal_debug_format() {
        let dir = tempfile::tempdir().unwrap();
        let wal_path = dir.path().join("test.wal");

        let wal = WalQueue::open(&wal_path, 50).unwrap();
        let debug = format!("{:?}", wal);
        assert!(debug.contains("WalQueue"));
        assert!(debug.contains("truncate_after: 50"));
    }
}
