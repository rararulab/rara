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

#![allow(clippy::cast_possible_truncation)]

//! Sparse index file for efficient random access to queue messages.
//!
//! ## Purpose
//!
//! When seeking to a specific sequence number, scanning from the beginning of
//! a data file would be O(n). The sparse index provides O(log n) lookup by
//! storing periodic sequence→offset mappings. After binary search in the index,
//! only a small linear scan is needed to find the exact message.
//!
//! ## Index File Format
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────┐
//! │                      Header (256 bytes)                       │
//! ├─────────────────┬─────────────────┬──────────────────────────┤
//! │ interval (8B)   │ entry_count (8B)│ reserved (240B)          │
//! └─────────────────┴─────────────────┴──────────────────────────┘
//! ┌──────────────────────────────────────────────────────────────┐
//! │                      Entries (16B each)                       │
//! ├─────────────────┬────────────────────────────────────────────┤
//! │ sequence (8B)   │ offset (8B)                                 │
//! ├─────────────────┼────────────────────────────────────────────┤
//! │ sequence (8B)   │ offset (8B)                                 │
//! └─────────────────┴────────────────────────────────────────────┘
//! ```
//!
//! - **interval**: How many messages between index entries (e.g., 100 = index
//!   every 100th message)
//! - **`entry_count`**: Number of index entries in the file
//! - **sequence**: Message sequence number at this index point
//! - **offset**: Byte offset in the data file where this message starts
//!
//! ## Sparse Indexing
//!
//! Not every message is indexed - only every `interval`th message. For example,
//! with interval=100, sequences 0, 100, 200, ... are indexed. To find sequence
//! 150:
//! 1. Binary search finds entry for sequence 100
//! 2. Seek to offset for sequence 100 in data file
//! 3. Linear scan 50 messages forward to find sequence 150

use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};

use crate::Result;

/// Size of the index file header in bytes.
/// Contains interval, `entry_count`, and reserved space for future use.
const INDEX_HEADER_SIZE: u64 = 256;

/// Size of each index entry in bytes (sequence: 8 + offset: 8).
const INDEX_ENTRY_SIZE: u64 = 16;

/// A single index entry mapping a sequence number to a file offset.
#[derive(Debug, Clone, Copy)]
pub struct IndexEntry {
    /// The sequence number of the indexed message.
    pub sequence: u64,
    /// Byte offset in the data file where this message begins.
    pub offset:   u64,
}

/// Writes sparse index entries to an index file.
///
/// The writer tracks the interval and only writes entries when appropriate,
/// automatically enforcing the sparse indexing policy.
pub struct IndexWriter {
    /// Underlying file handle for writing.
    file:                  File,
    /// How many sequences between indexed entries.
    interval:              u64,
    /// Total number of entries written so far.
    entry_count:           u64,
    /// Last sequence that was indexed, used to determine when to write next
    /// entry.
    last_indexed_sequence: Option<u64>,
}

impl IndexWriter {
    /// Create a new index file at the given path.
    ///
    /// Writes the header with the specified interval and zero entry count.
    /// The file is truncated if it already exists.
    pub fn create<P: AsRef<Path>>(path: P, interval: u64) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path.as_ref())?;

        // Write header: interval (8B) + entry_count (8B) + reserved (240B)
        let mut header = [0u8; INDEX_HEADER_SIZE as usize];
        header[0..8].copy_from_slice(&interval.to_le_bytes());
        header[8..16].copy_from_slice(&0u64.to_le_bytes());
        file.write_all(&header)?;

        Ok(Self {
            file,
            interval,
            entry_count: 0,
            last_indexed_sequence: None,
        })
    }

    /// Open an existing index file for appending.
    ///
    /// Reads the header to recover interval and entry count, then seeks
    /// to the end to continue appending entries.
    pub fn open<P: AsRef<Path>>(path: P, interval: u64) -> Result<Self> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path.as_ref())?;

        let mut header = [0u8; INDEX_HEADER_SIZE as usize];
        file.read_exact(&mut header)?;

        let stored_interval = u64::from_le_bytes(header[0..8].try_into().unwrap());
        let entry_count = u64::from_le_bytes(header[8..16].try_into().unwrap());

        // Read the last entry to know where we left off
        let last_indexed_sequence = if entry_count > 0 {
            let last_entry_pos = INDEX_HEADER_SIZE + (entry_count - 1) * INDEX_ENTRY_SIZE;
            file.seek(SeekFrom::Start(last_entry_pos))?;
            let mut buf = [0u8; INDEX_ENTRY_SIZE as usize];
            file.read_exact(&mut buf)?;
            Some(u64::from_le_bytes(buf[0..8].try_into().unwrap()))
        } else {
            None
        };

        file.seek(SeekFrom::End(0))?;

        Ok(Self {
            file,
            interval: if stored_interval > 0 {
                stored_interval
            } else {
                interval
            },
            entry_count,
            last_indexed_sequence,
        })
    }

    /// Conditionally write an index entry based on the sparse interval.
    ///
    /// An entry is written if:
    /// - This is sequence 0 (first message), or
    /// - At least `interval` sequences have passed since the last indexed entry
    pub fn maybe_write_entry(&mut self, sequence: u64, offset: u64) -> Result<()> {
        let should_write = match self.last_indexed_sequence {
            None => sequence == 0,
            Some(last) => sequence >= last + self.interval,
        };

        if should_write {
            self.write_entry(IndexEntry { sequence, offset })?;
            self.last_indexed_sequence = Some(sequence);
        }

        Ok(())
    }

    /// Write a single index entry to the file.
    fn write_entry(&mut self, entry: IndexEntry) -> Result<()> {
        let mut buf = [0u8; INDEX_ENTRY_SIZE as usize];
        buf[0..8].copy_from_slice(&entry.sequence.to_le_bytes());
        buf[8..16].copy_from_slice(&entry.offset.to_le_bytes());

        self.file.write_all(&buf)?;
        self.entry_count += 1;

        Ok(())
    }

    /// Flush the index to disk, updating the entry count in the header.
    ///
    /// This must be called before closing to ensure the header reflects
    /// the actual number of entries written.
    pub fn flush(&mut self) -> Result<()> {
        self.file.seek(SeekFrom::Start(8))?;
        self.file.write_all(&self.entry_count.to_le_bytes())?;
        self.file.flush()?;
        Ok(())
    }

    #[cfg(test)]
    pub const fn entry_count(&self) -> u64 { self.entry_count }
}

/// Reads index entries for fast sequence→offset lookup.
///
/// Loads all entries into memory at open time for fast binary search.
/// This is acceptable because index files are sparse and small (16 bytes per
/// entry, with entries only every N messages).
pub struct IndexReader {
    /// All index entries, sorted by sequence number.
    entries:  Vec<IndexEntry>,
    #[cfg(test)]
    interval: u64,
}

impl IndexReader {
    /// Open an index file and load all entries into memory.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mut file = File::open(path.as_ref())?;

        let mut header = [0u8; INDEX_HEADER_SIZE as usize];
        file.read_exact(&mut header)?;

        #[cfg(test)]
        let interval = u64::from_le_bytes(header[0..8].try_into().unwrap());
        let entry_count = u64::from_le_bytes(header[8..16].try_into().unwrap());

        let mut entries = Vec::with_capacity(entry_count as usize);

        for _ in 0..entry_count {
            let mut buf = [0u8; INDEX_ENTRY_SIZE as usize];
            file.read_exact(&mut buf)?;

            let sequence = u64::from_le_bytes(buf[0..8].try_into().unwrap());
            let offset = u64::from_le_bytes(buf[8..16].try_into().unwrap());

            entries.push(IndexEntry { sequence, offset });
        }

        Ok(Self {
            entries,
            #[cfg(test)]
            interval,
        })
    }

    /// Find the best starting offset for reading a target sequence.
    ///
    /// Uses binary search to find the largest indexed sequence <= target.
    /// Returns `(sequence, offset)` of the index entry, or the first entry
    /// if the target is before all indexed sequences.
    ///
    /// The caller should seek to `offset` and scan forward to find the exact
    /// target.
    pub fn find_offset_for_sequence(&self, target_sequence: u64) -> Option<(u64, u64)> {
        if self.entries.is_empty() {
            return None;
        }

        // Binary search: find first entry with sequence > target
        let idx = self
            .entries
            .partition_point(|e| e.sequence <= target_sequence);

        if idx == 0 {
            // Target is before all entries, return first entry
            return Some((self.entries[0].sequence, self.entries[0].offset));
        }

        // Return the entry just before the partition point (largest <= target)
        let entry = &self.entries[idx - 1];
        Some((entry.sequence, entry.offset))
    }

    #[cfg(test)]
    pub fn entries(&self) -> &[IndexEntry] { &self.entries }

    #[cfg(test)]
    pub const fn interval(&self) -> u64 { self.interval }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn test_index_writer_create() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.index");

        let mut writer = IndexWriter::create(&path, 100).unwrap();
        writer.flush().unwrap();

        assert!(path.exists());
        assert_eq!(writer.entry_count(), 0);
    }

    #[test]
    fn test_index_write_and_read() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.index");

        {
            let mut writer = IndexWriter::create(&path, 10).unwrap();

            for i in 0..5 {
                let seq = i * 10;
                writer.maybe_write_entry(seq, seq * 100).unwrap();
            }

            writer.flush().unwrap();
            assert_eq!(writer.entry_count(), 5);
        }

        let reader = IndexReader::open(&path).unwrap();
        assert_eq!(reader.entries().len(), 5);
        assert_eq!(reader.interval(), 10);

        assert_eq!(reader.entries()[0].sequence, 0);
        assert_eq!(reader.entries()[0].offset, 0);
        assert_eq!(reader.entries()[4].sequence, 40);
        assert_eq!(reader.entries()[4].offset, 4000);
    }

    #[test]
    fn test_index_find_offset() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.index");

        {
            let mut writer = IndexWriter::create(&path, 100).unwrap();
            writer.maybe_write_entry(0, 0).unwrap();
            writer.maybe_write_entry(100, 1000).unwrap();
            writer.maybe_write_entry(200, 2000).unwrap();
            writer.flush().unwrap();
        }

        let reader = IndexReader::open(&path).unwrap();

        let (seq, offset) = reader.find_offset_for_sequence(0).unwrap();
        assert_eq!(seq, 0);
        assert_eq!(offset, 0);

        let (seq, offset) = reader.find_offset_for_sequence(50).unwrap();
        assert_eq!(seq, 0);
        assert_eq!(offset, 0);

        let (seq, offset) = reader.find_offset_for_sequence(150).unwrap();
        assert_eq!(seq, 100);
        assert_eq!(offset, 1000);

        let (seq, offset) = reader.find_offset_for_sequence(250).unwrap();
        assert_eq!(seq, 200);
        assert_eq!(offset, 2000);
    }

    #[test]
    fn test_index_sparse_interval() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.index");

        {
            let mut writer = IndexWriter::create(&path, 100).unwrap();

            for i in 0..500 {
                writer.maybe_write_entry(i, i * 50).unwrap();
            }

            writer.flush().unwrap();
        }

        let reader = IndexReader::open(&path).unwrap();
        assert_eq!(reader.entries().len(), 5);

        assert_eq!(reader.entries()[0].sequence, 0);
        assert_eq!(reader.entries()[1].sequence, 100);
        assert_eq!(reader.entries()[2].sequence, 200);
        assert_eq!(reader.entries()[3].sequence, 300);
        assert_eq!(reader.entries()[4].sequence, 400);
    }
}
