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

//! Background I/O worker for persisting messages to disk.
//!
//! ## Architecture
//!
//! The `IOWorker` runs on a dedicated thread and receives [`WriteEvent`]s from
//! [`Appender`](crate::Appender)s via a crossbeam channel. This decouples the
//! write latency from disk I/O, allowing appenders to return quickly.
//!
//! ```text
//! ┌──────────────┐     crossbeam      ┌──────────────┐     mmap      ┌──────────────┐
//! │   Appender   │ ──── channel ────► │   IOWorker   │ ──────────►  │  Data File   │
//! │  (caller)    │                    │  (bg thread) │              │   (.data)    │
//! └──────────────┘                    └──────────────┘              └──────────────┘
//! ```
//!
//! ## Responsibilities
//!
//! - **Message persistence**: Write messages to mmap'd data files
//! - **File rolling**: Create new files based on size/time/count strategy
//! - **Index maintenance**: Build sparse index entries for random access
//! - **Flush management**: Sync to disk per configured flush mode
//! - **Recovery**: Resume writing to existing files after restart

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use chrono::Utc;
use crossbeam::channel::{Receiver, RecvTimeoutError};
use tracing::{debug, error, info};

use crate::{
    FlushMode, QueueConfig, Result,
    crc::calculate_message_crc,
    error::InternalSnafu,
    file::DataFile,
    index::IndexWriter,
    manifest::{ActiveFileState, FileEntry, MANIFEST_VERSION, Manifest},
    manifest_writer::ManifestWriter,
    message::{MESSAGE_LENGTH_SIZE, WriteEvent, message_disk_size},
    path::{data_file_path, index_file_path, scan_data_files},
};

/// Background I/O worker that receives `WriteEvents` and persists them to disk.
///
/// This worker runs on a dedicated thread and handles:
/// - Writing messages to memory-mapped data files
/// - Managing file rolling based on configured strategy
/// - Flushing data to disk based on flush mode
/// - Building sparse index entries
/// - Writing manifest on file roll and shutdown
pub struct IOWorker {
    /// Channel receiver for incoming write events from Appenders.
    rx:                  Receiver<WriteEvent>,
    /// Shared queue configuration (base path, file size, flush mode, etc.).
    config:              Arc<QueueConfig>,
    /// Currently active data file being written to.
    current_file:        Option<DataFile>,
    /// Index writer for the current data file.
    current_index:       Option<IndexWriter>,
    /// Byte offset of the next write position in the current file.
    write_position:      u64,
    /// Current file sequence number (increments on each roll).
    file_sequence:       u32,
    /// Number of messages written to the current file.
    message_count:       u64,
    /// When the current file was opened (for time-based rolling).
    file_start_time:     Instant,
    /// Bytes written since last flush (for batch flush mode).
    pending_bytes:       usize,
    /// When the last flush occurred (for batch flush mode).
    last_flush:          Instant,
    /// Shared shutdown flag checked in the run loop.
    shutdown:            Arc<AtomicBool>,
    /// If true, we need to open an existing file instead of creating a new one.
    /// Set during recovery when resuming from a previous session.
    recovered:           bool,
    /// Path to the current data file.
    current_file_path:   Option<PathBuf>,
    /// Sequence number of the first message in the current file.
    file_start_sequence: u64,
    /// Global sequence counter (next sequence to assign).
    global_sequence:     u64,
    /// Metadata for completed (rolled) data files.
    completed_files:     Vec<FileEntry>,
    /// Manifest writer for atomic updates.
    manifest_writer:     ManifestWriter,
}

impl IOWorker {
    /// Create a new IOWorker for testing (starts fresh with no recovery state).
    #[cfg(test)]
    pub fn new(
        rx: Receiver<WriteEvent>,
        config: Arc<QueueConfig>,
        shutdown: Arc<AtomicBool>,
    ) -> Self {
        let manifest_writer =
            ManifestWriter::new(&config.base_path).expect("Failed to create manifest writer");
        Self {
            rx,
            config,
            current_file: None,
            current_index: None,
            write_position: 0,
            file_sequence: 1,
            message_count: 0,
            file_start_time: Instant::now(),
            pending_bytes: 0,
            last_flush: Instant::now(),
            shutdown,
            recovered: false,
            current_file_path: None,
            file_start_sequence: 0,
            global_sequence: 0,
            completed_files: Vec::new(),
            manifest_writer,
        }
    }

    /// Create an `IOWorker` initialized with recovery state from a previous
    /// session.
    ///
    /// Called by [`Queue`](crate::Queue) after scanning existing data files
    /// to determine where to resume writing.
    pub fn with_recovery(
        rx: Receiver<WriteEvent>,
        config: Arc<QueueConfig>,
        shutdown: Arc<AtomicBool>,
        file_sequence: u32,
        write_position: u64,
        message_count: u64,
        next_sequence: u64,
        completed_files: Vec<FileEntry>,
        manifest_writer: ManifestWriter,
    ) -> Self {
        let recovered = write_position > 0 || message_count > 0;
        Self {
            rx,
            config,
            current_file: None,
            current_index: None,
            write_position,
            file_sequence,
            message_count,
            file_start_time: Instant::now(),
            pending_bytes: 0,
            last_flush: Instant::now(),
            shutdown,
            recovered,
            current_file_path: None,
            file_start_sequence: next_sequence.saturating_sub(message_count),
            global_sequence: next_sequence,
            completed_files,
            manifest_writer,
        }
    }

    /// Main run loop for the `IOWorker`.
    ///
    /// Blocks and processes events until shutdown is signaled or the channel
    /// disconnects. On each iteration:
    /// 1. Check shutdown flag
    /// 2. Wait for event with 100μs timeout
    /// 3. Write event to disk (or check flush on timeout)
    /// 4. Repeat
    pub fn run(&mut self) {
        info!("IOWorker starting");

        if let Err(e) = self.ensure_file() {
            error!(error = ?e, "Failed to create initial data file");
            return;
        }

        loop {
            if self.shutdown.load(Ordering::Relaxed) {
                info!("IOWorker received shutdown signal");
                break;
            }

            match self.rx.recv_timeout(Duration::from_micros(100)) {
                Ok(event) => {
                    if let Err(e) = self.write_event(&event) {
                        error!(error = ?e, "Failed to write event");
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    // No event received - check if we need a time-based flush
                    if let Err(e) = self.check_flush() {
                        error!(error = ?e, "Failed to flush");
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    info!("IOWorker channel disconnected");
                    break;
                }
            }
        }

        if let Err(e) = self.final_flush() {
            error!(error = ?e, "Failed to perform final flush");
        }

        info!("IOWorker stopped");
    }

    /// Ensure a data file is open for writing.
    ///
    /// If `recovered` is true, opens the last existing file (found via
    /// `scan_data_files`). Otherwise, creates a new file with the current
    /// timestamp and `file_sequence`.
    fn ensure_file(&mut self) -> Result<()> {
        if self.current_file.is_some() {
            return Ok(());
        }

        if self.recovered {
            let data_files = scan_data_files(&self.config.base_path)?;
            if let Some(last_file_path) = data_files.last() {
                debug!(path = ?last_file_path, "Opening existing data file for recovery");
                let file = DataFile::open(last_file_path)?;
                let index_path = last_file_path.with_extension("index");
                let index = if index_path.exists() {
                    IndexWriter::open(&index_path, self.config.index_interval)?
                } else {
                    IndexWriter::create(&index_path, self.config.index_interval)?
                };

                self.current_file = Some(file);
                self.current_index = Some(index);
                self.current_file_path = Some(last_file_path.clone());
                self.file_start_time = Instant::now();
                self.recovered = false;
                return Ok(());
            }
        }

        let now = Utc::now();
        let data_path = data_file_path(&self.config.base_path, now, self.file_sequence);
        let index_path = index_file_path(&self.config.base_path, now, self.file_sequence);

        debug!(path = ?data_path, "Creating new data file");

        let file = DataFile::create(&data_path, self.config.file_size)?;
        let index = IndexWriter::create(&index_path, self.config.index_interval)?;

        self.current_file = Some(file);
        self.current_index = Some(index);
        self.current_file_path = Some(data_path);
        self.write_position = 0;
        self.file_start_time = Instant::now();

        Ok(())
    }

    /// Roll to a new data file.
    ///
    /// Flushes the current file, closes it, increments `file_sequence`,
    /// and resets counters. The next `ensure_file` call will create the new
    /// file.
    fn roll_file(&mut self) -> Result<()> {
        if let Some(ref file) = self.current_file {
            file.flush(&self.config.flush_mode)?;
        }
        if let Some(ref mut index) = self.current_index {
            index.flush()?;
        }

        if let Some(path) = self.current_file_path.take()
            && self.message_count > 0
        {
            let end_sequence = self.file_start_sequence + self.message_count - 1;
            self.completed_files.push(FileEntry {
                path,
                start_sequence: self.file_start_sequence,
                end_sequence,
                size: self.write_position,
            });
        }

        self.current_file = None;
        self.current_index = None;
        self.file_sequence += 1;
        self.file_start_sequence = self.global_sequence;
        self.message_count = 0;
        self.write_position = 0;

        info!(sequence = self.file_sequence, "Rolled to new data file");

        self.ensure_file()?;
        self.write_manifest()?;

        Ok(())
    }

    /// Write a single event to the current data file.
    ///
    /// Handles file rolling if needed, writes the message in wire format
    /// `[length: 4B][payload: variable][crc: 8B]`, and updates the sparse
    /// index.
    fn write_event(&mut self, event: &WriteEvent) -> Result<()> {
        let total_size = message_disk_size(event.data.len()) as u64;

        let elapsed = self.file_start_time.elapsed();
        if self.config.roll_strategy.should_roll(
            self.write_position + total_size,
            elapsed,
            self.message_count + 1,
        ) {
            self.roll_file()?;
        }

        self.ensure_file()?;

        let file = self.current_file.as_ref().ok_or_else(|| {
            InternalSnafu {
                message: "No data file available".to_string(),
            }
            .build()
        })?;

        let pos = self.write_position;
        let length = event.data.len() as u32;
        let crc = calculate_message_crc(length, &event.data);

        let mut offset = pos;
        file.write_at(offset, &length.to_le_bytes())?;
        offset += MESSAGE_LENGTH_SIZE as u64;
        file.write_at(offset, &event.data)?;
        offset += event.data.len() as u64;
        file.write_at(offset, &crc.to_le_bytes())?;

        self.write_position += total_size;
        self.pending_bytes += total_size as usize;
        self.message_count += 1;
        self.global_sequence = event.sequence + 1;

        if let Some(ref mut index) = self.current_index {
            index.maybe_write_entry(event.sequence, pos)?;
        }

        self.handle_flush()?;

        debug!(
            sequence = event.sequence,
            offset = pos,
            size = total_size,
            "Wrote message"
        );

        Ok(())
    }

    /// Handle flush based on the configured flush mode.
    ///
    /// - `Sync`: Flush after every write
    /// - `Batch`: Flush when bytes threshold or time interval is exceeded
    /// - `Async`: Never explicitly flush (OS handles it)
    fn handle_flush(&mut self) -> Result<()> {
        match &self.config.flush_mode {
            FlushMode::Sync => {
                if let Some(ref file) = self.current_file {
                    file.flush(&FlushMode::Sync)?;
                }
                self.pending_bytes = 0;
                self.last_flush = Instant::now();
            }
            FlushMode::Batch { bytes, interval } => {
                let should_flush =
                    self.pending_bytes >= *bytes || self.last_flush.elapsed() >= *interval;

                if should_flush {
                    if let Some(ref file) = self.current_file {
                        file.flush(&FlushMode::Async)?;
                    }
                    if let Some(ref mut index) = self.current_index {
                        index.flush()?;
                    }
                    self.pending_bytes = 0;
                    self.last_flush = Instant::now();
                }
            }
            FlushMode::Async => {}
        }

        Ok(())
    }

    /// Check for time-based flush when no events are received.
    ///
    /// Called during timeout in the run loop to ensure data is flushed
    /// even during idle periods.
    fn check_flush(&mut self) -> Result<()> {
        if let FlushMode::Batch { interval, .. } = &self.config.flush_mode
            && self.pending_bytes > 0
            && self.last_flush.elapsed() >= *interval
        {
            if let Some(ref file) = self.current_file {
                file.flush(&FlushMode::Async)?;
            }
            if let Some(ref mut index) = self.current_index {
                index.flush()?;
            }
            self.pending_bytes = 0;
            self.last_flush = Instant::now();
        }

        Ok(())
    }

    /// Perform a final sync flush before shutdown.
    ///
    /// Ensures all pending data is durably persisted to disk.
    fn final_flush(&mut self) -> Result<()> {
        if let Some(ref file) = self.current_file {
            file.flush(&FlushMode::Sync)?;
        }
        if let Some(ref mut index) = self.current_index {
            index.flush()?;
        }

        self.write_manifest()?;

        info!(
            position = self.write_position,
            messages = self.message_count,
            "Final flush complete"
        );
        Ok(())
    }

    fn write_manifest(&mut self) -> Result<()> {
        let manifest = Manifest {
            version:       MANIFEST_VERSION,
            next_sequence: self.global_sequence,
            active_file:   ActiveFileState {
                file_sequence:  self.file_sequence,
                write_position: self.write_position,
                message_count:  self.message_count,
                path:           self.current_file_path.clone().unwrap_or_default(),
            },
            files:         self.completed_files.clone(),
        };
        self.manifest_writer.write(&manifest)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use bytes::Bytes;
    use crossbeam::channel::unbounded;
    use tempfile::TempDir;
    use test_case::test_case;

    use super::*;

    fn test_config(base_path: PathBuf) -> Arc<QueueConfig> {
        Arc::new(QueueConfig {
            base_path,
            file_size: 1024 * 1024,
            roll_strategy: crate::RollStrategy::BySize(1024 * 1024),
            flush_mode: FlushMode::Sync,
            index_interval: 10,
            verify_on_startup: false,
        })
    }

    struct WorkerFixture {
        _temp_dir: TempDir,
        worker:    IOWorker,
    }

    impl WorkerFixture {
        fn new() -> Self {
            let temp_dir = TempDir::new().unwrap();
            let config = test_config(temp_dir.path().to_path_buf());
            let (_tx, rx) = unbounded();
            let shutdown = Arc::new(AtomicBool::new(false));
            let worker = IOWorker::new(rx, config, shutdown);
            Self {
                _temp_dir: temp_dir,
                worker,
            }
        }

        fn with_roll_by_count(roll_count: u64) -> Self {
            let temp_dir = TempDir::new().unwrap();
            let config = Arc::new(QueueConfig {
                base_path:         temp_dir.path().to_path_buf(),
                file_size:         1024,
                roll_strategy:     crate::RollStrategy::ByCount(roll_count),
                flush_mode:        FlushMode::Sync,
                index_interval:    10,
                verify_on_startup: false,
            });
            let (_tx, rx) = unbounded();
            let shutdown = Arc::new(AtomicBool::new(false));
            let worker = IOWorker::new(rx, config, shutdown);
            Self {
                _temp_dir: temp_dir,
                worker,
            }
        }
    }

    #[test]
    fn test_write_single_message() {
        let mut fixture = WorkerFixture::new();
        fixture.worker.ensure_file().unwrap();

        let event = WriteEvent {
            sequence: 0,
            data:     Bytes::from("test message"),
        };
        fixture.worker.write_event(&event).unwrap();

        assert!(fixture.worker.write_position > 0);
        assert_eq!(fixture.worker.message_count, 1);
    }

    #[test_case(3, 4, 2 ; "roll after 3 messages triggers at message 4")]
    #[test_case(5, 6, 2 ; "roll after 5 messages triggers at message 6")]
    fn test_file_rolling(roll_count: u64, write_count: u64, expected_file_seq: u32) {
        let mut fixture = WorkerFixture::with_roll_by_count(roll_count);
        fixture.worker.ensure_file().unwrap();

        for i in 0..write_count {
            let event = WriteEvent {
                sequence: i,
                data:     Bytes::from(format!("message {i}")),
            };
            fixture.worker.write_event(&event).unwrap();
        }

        assert_eq!(fixture.worker.file_sequence, expected_file_seq);
    }
}
