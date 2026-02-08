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

//! Message reader (tailer) for consuming from the queue.
//!
//! The [`Tailer`] provides sequential and random-access reading of messages
//! from the queue. It supports:
//! - Sequential iteration via [`read_next`](Tailer::read_next) or the
//!   `Iterator` trait
//! - Seeking to a specific sequence via [`seek`](Tailer::seek) or
//!   [`from_sequence`](Tailer::from_sequence)
//! - Automatic file advancement when reaching end of a data file
//! - CRC verification on every read
//!
//! ## Zero-Copy Reads
//!
//! The tailer reads from memory-mapped files, enabling efficient zero-copy
//! access to message payloads. The returned [`Message`] payload is a view into
//! the mmap.

use std::{path::PathBuf, sync::Arc};

use bytes::Bytes;
use snafu::ensure;

use crate::{
    QueueConfig, Result,
    crc::verify_message_crc,
    error::CorruptedMessageSnafu,
    file::ReadOnlyDataFile,
    index::IndexReader,
    message::{MESSAGE_CRC_SIZE, MESSAGE_LENGTH_SIZE, Message},
    path::scan_data_files,
};

/// A reader for consuming messages from the queue.
///
/// Multiple tailers can read from the same queue concurrently, each maintaining
/// their own read position. Tailers are not thread-safe; use one per thread.
pub struct Tailer {
    /// Shared queue configuration.
    config:           Arc<QueueConfig>,
    /// Sorted list of all data file paths in the queue.
    data_files:       Vec<PathBuf>,
    /// Index into `data_files` for the currently open file.
    current_file_idx: usize,
    /// Currently open data file (mmap'd read-only).
    current_file:     Option<ReadOnlyDataFile>,
    /// Sparse index for the current file (if exists).
    current_index:    Option<IndexReader>,
    /// Byte offset for the next read in the current file.
    read_position:    u64,
    /// Sequence number of the next message to be read.
    current_sequence: u64,
}

impl Tailer {
    /// Create a new tailer starting from the beginning of the queue.
    ///
    /// Scans for existing data files and opens the first one if present.
    ///
    /// # Errors
    ///
    /// Returns an error if the queue directory cannot be read or if opening
    /// the first data file fails.
    pub fn new(config: Arc<QueueConfig>) -> Result<Self> {
        let data_files = scan_data_files(&config.base_path)?;

        let mut tailer = Self {
            config,
            data_files,
            current_file_idx: 0,
            current_file: None,
            current_index: None,
            read_position: 0,
            current_sequence: 0,
        };

        if !tailer.data_files.is_empty() {
            tailer.open_file_at_index(0)?;
        }

        Ok(tailer)
    }

    /// Create a new tailer starting from a specific sequence number.
    ///
    /// Uses the sparse index for O(log n) seek when available, otherwise
    /// performs a linear scan from the beginning.
    ///
    /// # Errors
    ///
    /// Returns an error if the queue directory cannot be read, opening data
    /// files fails, or seeking to the target sequence fails.
    pub fn from_sequence(config: Arc<QueueConfig>, target_sequence: u64) -> Result<Self> {
        let mut tailer = Self::new(config)?;
        tailer.seek(target_sequence)?;
        Ok(tailer)
    }

    /// Read the next message from the queue.
    ///
    /// Returns `Ok(Some(message))` if a message was read, `Ok(None)` if there
    /// are no more messages, or `Err` if corruption was detected.
    ///
    /// Automatically advances to the next data file when the current one is
    /// exhausted.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Reading from the data file fails
    /// - CRC validation fails (data corruption detected)
    /// - Opening the next data file fails
    pub fn read_next(&mut self) -> Result<Option<Message>> {
        loop {
            let Some(file) = self.current_file.as_ref() else {
                return Ok(None);
            };

            let file_size = file.size();

            if self.read_position + MESSAGE_LENGTH_SIZE as u64 > file_size {
                if !self.advance_to_next_file()? {
                    return Ok(None);
                }
                continue;
            }

            let mut length_buf = [0u8; MESSAGE_LENGTH_SIZE];
            file.read_at(self.read_position, &mut length_buf)?;
            let length = u32::from_le_bytes(length_buf);

            if length == 0 {
                if !self.advance_to_next_file()? {
                    return Ok(None);
                }
                continue;
            }

            let total_size =
                MESSAGE_LENGTH_SIZE as u64 + u64::from(length) + MESSAGE_CRC_SIZE as u64;

            ensure!(
                self.read_position + total_size <= file_size,
                CorruptedMessageSnafu {
                    sequence: self.current_sequence,
                }
            );

            let payload_offset = self.read_position + MESSAGE_LENGTH_SIZE as u64;
            let crc_offset = payload_offset + u64::from(length);

            let mut payload = vec![0u8; length as usize];
            file.read_at(payload_offset, &mut payload)?;

            let mut crc_buf = [0u8; MESSAGE_CRC_SIZE];
            file.read_at(crc_offset, &mut crc_buf)?;
            let stored_crc = u32::from_le_bytes(crc_buf);

            ensure!(
                verify_message_crc(length, &payload, stored_crc),
                CorruptedMessageSnafu {
                    sequence: self.current_sequence,
                }
            );

            let message = Message {
                sequence:  self.current_sequence,
                timestamp: 0,
                payload:   Bytes::from(payload),
            };

            self.read_position += total_size;
            self.current_sequence += 1;

            return Ok(Some(message));
        }
    }

    /// Seek to a specific sequence number.
    ///
    /// After seeking, the next call to `read_next` will return the message
    /// at `target_sequence` (if it exists). Uses the sparse index for
    /// efficient seeking when available.
    ///
    /// # Errors
    ///
    /// Returns an error if opening index files fails, reading messages during
    /// the scan fails, or opening data files fails.
    pub fn seek(&mut self, target_sequence: u64) -> Result<()> {
        for (idx, data_path) in self.data_files.iter().enumerate() {
            let index_path = data_path.with_extension("index");

            if !index_path.exists() {
                continue;
            }

            let index = IndexReader::open(&index_path)?;
            if let Some((start_seq, offset)) = index.find_offset_for_sequence(target_sequence) {
                self.open_file_at_index(idx)?;
                self.read_position = offset;
                self.current_sequence = start_seq;

                while self.current_sequence < target_sequence {
                    if self.read_next()?.is_none() {
                        break;
                    }
                }

                return Ok(());
            }
        }

        self.current_sequence = 0;
        self.read_position = 0;

        while self.current_sequence < target_sequence {
            if self.read_next()?.is_none() {
                break;
            }
        }

        Ok(())
    }

    /// Get the sequence number of the next message to be read.
    #[must_use]
    pub const fn current_sequence(&self) -> u64 { self.current_sequence }

    /// Refresh the list of data files.
    ///
    /// Call this to pick up newly written files when tailing a live queue.
    ///
    /// # Errors
    ///
    /// Returns an error if scanning the queue directory fails.
    pub fn refresh(&mut self) -> Result<()> {
        self.data_files = scan_data_files(&self.config.base_path)?;
        Ok(())
    }

    /// Open the data file at the given index.
    fn open_file_at_index(&mut self, idx: usize) -> Result<()> {
        if idx >= self.data_files.len() {
            self.current_file = None;
            self.current_index = None;
            return Ok(());
        }

        let data_path = &self.data_files[idx];
        let index_path = data_path.with_extension("index");

        self.current_file = Some(ReadOnlyDataFile::open(data_path)?);
        self.current_index = if index_path.exists() {
            Some(IndexReader::open(&index_path)?)
        } else {
            None
        };
        self.current_file_idx = idx;
        self.read_position = 0;

        Ok(())
    }

    /// Advance to the next data file.
    ///
    /// Returns `true` if successfully advanced, `false` if no more files.
    fn advance_to_next_file(&mut self) -> Result<bool> {
        let next_idx = self.current_file_idx + 1;

        if next_idx >= self.data_files.len() {
            self.refresh()?;

            if next_idx >= self.data_files.len() {
                return Ok(false);
            }
        }

        self.open_file_at_index(next_idx)?;
        Ok(true)
    }
}

impl Iterator for Tailer {
    type Item = Result<Message>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.read_next() {
            Ok(Some(msg)) => Some(Ok(msg)),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use tempfile::TempDir;

    use super::*;
    use crate::{
        FlushMode, RollStrategy, crc::calculate_message_crc, file::DataFile, index::IndexWriter,
        path::data_file_path,
    };

    fn write_test_message(file: &DataFile, offset: u64, data: &[u8]) -> u64 {
        let length = u32::try_from(data.len()).expect("failed to convert to u32 from usize");
        let crc = calculate_message_crc(length, data);

        file.write_at(offset, &length.to_le_bytes()).unwrap();
        file.write_at(offset + 4, data).unwrap();
        file.write_at(offset + 4 + data.len() as u64, &crc.to_le_bytes())
            .unwrap();

        4 + data.len() as u64 + 4
    }

    fn test_config(base_path: PathBuf) -> Arc<QueueConfig> {
        Arc::new(QueueConfig {
            base_path,
            file_size: 1024 * 1024,
            roll_strategy: RollStrategy::BySize(1024 * 1024),
            flush_mode: FlushMode::Sync,
            index_interval: 10,
            verify_on_startup: false,
        })
    }

    #[test]
    fn test_tailer_empty_queue() {
        let temp_dir = TempDir::new().unwrap();
        let config = test_config(temp_dir.path().to_path_buf());

        let mut tailer = Tailer::new(config).unwrap();
        assert!(tailer.read_next().unwrap().is_none());
    }

    #[test]
    fn test_tailer_read_single_message() {
        let temp_dir = TempDir::new().unwrap();
        let config = test_config(temp_dir.path().to_path_buf());

        let now = Utc::now();
        let data_path = data_file_path(&config.base_path, now, 1);
        let file = DataFile::create(&data_path, 4096).unwrap();

        write_test_message(&file, 0, b"hello world");
        file.flush(&FlushMode::Sync).unwrap();

        let mut tailer = Tailer::new(config).unwrap();
        let msg = tailer.read_next().unwrap().unwrap();

        assert_eq!(msg.sequence, 0);
        assert_eq!(msg.payload.as_ref(), b"hello world");

        assert!(tailer.read_next().unwrap().is_none());
    }

    #[test]
    fn test_tailer_read_multiple_messages() {
        let temp_dir = TempDir::new().unwrap();
        let config = test_config(temp_dir.path().to_path_buf());

        let now = Utc::now();
        let data_path = data_file_path(&config.base_path, now, 1);
        let file = DataFile::create(&data_path, 4096).unwrap();

        let mut offset = 0u64;
        for i in 0..5 {
            let msg = format!("message {i}");
            offset += write_test_message(&file, offset, msg.as_bytes());
        }
        file.flush(&FlushMode::Sync).unwrap();

        let mut tailer = Tailer::new(config).unwrap();

        for i in 0..5 {
            let msg = tailer.read_next().unwrap().unwrap();
            assert_eq!(msg.sequence, i);
            assert_eq!(
                std::str::from_utf8(&msg.payload).unwrap(),
                format!("message {i}")
            );
        }

        assert!(tailer.read_next().unwrap().is_none());
    }

    #[test]
    fn test_tailer_with_index() {
        let temp_dir = TempDir::new().unwrap();
        let config = test_config(temp_dir.path().to_path_buf());

        let now = Utc::now();
        let data_path = data_file_path(&config.base_path, now, 1);
        let index_path = data_path.with_extension("index");

        let file = DataFile::create(&data_path, 65536).unwrap();
        let mut index = IndexWriter::create(&index_path, 10).unwrap();

        let mut offset = 0u64;
        for i in 0..50 {
            index.maybe_write_entry(i, offset).unwrap();
            let msg = format!("msg-{i:04}");
            offset += write_test_message(&file, offset, msg.as_bytes());
        }

        file.flush(&FlushMode::Sync).unwrap();
        index.flush().unwrap();

        let mut tailer = Tailer::from_sequence(config, 25).unwrap();

        let msg = tailer.read_next().unwrap().unwrap();
        assert_eq!(msg.sequence, 25);
        assert_eq!(std::str::from_utf8(&msg.payload).unwrap(), "msg-0025");
    }
}
