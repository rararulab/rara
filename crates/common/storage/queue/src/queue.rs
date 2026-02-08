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

//! Main queue struct and lifecycle management.
//!
//! The [`Queue`] is the central entry point for the persistent queue library.
//! It manages:
//! - Recovery of existing data on startup
//! - Background I/O worker thread for writes
//! - Global sequence number generation
//! - Factory methods for [`Appender`] and [`Tailer`]
//!
//! ## Usage
//!
//! ```ignore
//! // Create or open a queue
//! let queue = QueueBuilder::new("/path/to/queue").build()?;
//!
//! // Write messages
//! let appender = queue.create_appender();
//! appender.append(b"hello")?;
//!
//! // Read messages  
//! let mut tailer = queue.create_tailer()?;
//! while let Some(msg) = tailer.read_next()? {
//!     println!("{:?}", msg.payload);
//! }
//!
//! // Clean shutdown
//! queue.shutdown()?;
//! ```

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
};

use crossbeam::channel::{Sender, unbounded};
use tracing::info;

use crate::{
    QueueConfig, Result,
    appender::Appender,
    error::InternalSnafu,
    io_worker::IOWorker,
    manifest_writer::ManifestWriter,
    message::WriteEvent,
    recovery::{RecoveryInfo, RecoveryResult},
    tailer::Tailer,
};

/// A persistent append-only queue.
///
/// The queue is thread-safe for creating appenders and tailers. Multiple
/// appenders can write concurrently (sequence numbers are atomically assigned).
/// Multiple tailers can read concurrently (each maintains its own position).
pub struct Queue {
    /// Shared configuration (base path, file size, flush mode, etc.).
    config:           Arc<QueueConfig>,
    /// Sender side of the channel to the `IOWorker`. `None` after shutdown.
    io_tx:            Option<Sender<WriteEvent>>,
    /// Global sequence counter shared across all appenders.
    global_sequence:  Arc<AtomicU64>,
    /// Shutdown flag checked by the `IOWorker`.
    shutdown_flag:    Arc<AtomicBool>,
    /// Handle to the background `IOWorker` thread.
    io_worker_handle: Option<JoinHandle<()>>,
}

impl Queue {
    /// Create a new queue instance.
    ///
    /// If the queue directory exists, performs recovery to find the next
    /// sequence number and file position. Otherwise, creates the directory.
    ///
    /// Spawns a background `IOWorker` thread for handling writes.
    pub(crate) fn new(config: QueueConfig) -> Result<Self> {
        let config = Arc::new(config);

        let RecoveryResult {
            info: recovery_info,
            manifest_writer,
        } = if config.base_path.exists() {
            crate::recovery::recover(&config)?
        } else {
            std::fs::create_dir_all(&config.base_path)?;
            RecoveryResult {
                info:            RecoveryInfo::default(),
                manifest_writer: ManifestWriter::new(&config.base_path)?,
            }
        };

        let (io_tx, io_rx) = unbounded();
        let shutdown_flag = Arc::new(AtomicBool::new(false));
        let global_sequence = Arc::new(AtomicU64::new(recovery_info.next_sequence));

        let io_worker = IOWorker::with_recovery(
            io_rx,
            config.clone(),
            shutdown_flag.clone(),
            recovery_info.file_sequence,
            recovery_info.write_position,
            recovery_info.message_count,
            recovery_info.next_sequence,
            recovery_info.completed_files,
            manifest_writer,
        );

        let io_worker_handle = thread::Builder::new()
            .name("queue-io-worker".into())
            .spawn(move || {
                let mut worker = io_worker;
                worker.run();
            })?;

        info!(
            path = ?config.base_path,
            next_sequence = recovery_info.next_sequence,
            file_sequence = recovery_info.file_sequence,
            "Queue initialized"
        );

        Ok(Self {
            config,
            io_tx: Some(io_tx),
            global_sequence,
            shutdown_flag,
            io_worker_handle: Some(io_worker_handle),
        })
    }

    /// Create a new appender for writing messages.
    ///
    /// Appenders are cheap to create and can be used from any thread.
    /// Multiple appenders can write concurrently.
    ///
    /// # Panics
    ///
    /// Panics if the queue has been shut down.
    #[must_use]
    pub fn create_appender(&self) -> Appender {
        Appender::new(
            self.io_tx.clone().expect("Queue is shut down"),
            self.global_sequence.clone(),
        )
    }

    /// Create a new tailer starting from the beginning of the queue.
    ///
    /// # Errors
    ///
    /// Returns an error if the queue directory cannot be read or the manifest
    /// cannot be loaded.
    pub fn create_tailer(&self) -> Result<Tailer> { Tailer::new(self.config.clone()) }

    /// Create a new tailer starting from a specific sequence number.
    ///
    /// # Errors
    ///
    /// Returns an error if the queue directory cannot be read, the manifest
    /// cannot be loaded, or seeking to the target sequence fails.
    pub fn create_tailer_at(&self, sequence: u64) -> Result<Tailer> {
        Tailer::from_sequence(self.config.clone(), sequence)
    }

    /// Get the current global sequence number.
    ///
    /// This is the sequence that will be assigned to the next appended message.
    #[must_use]
    pub fn current_sequence(&self) -> u64 { self.global_sequence.load(Ordering::Relaxed) }

    /// Shut down the queue gracefully.
    ///
    /// Signals the `IOWorker` to stop, waits for it to flush all pending data,
    /// and joins the background thread. Consumes `self` to prevent further use.
    ///
    /// # Errors
    ///
    /// Returns an error if the IO worker thread panicked or failed to flush
    /// data.
    pub fn shutdown(mut self) -> Result<()> {
        info!("Shutting down queue");

        self.shutdown_flag.store(true, Ordering::SeqCst);
        self.io_tx.take();

        if let Some(handle) = self.io_worker_handle.take() {
            handle.join().map_err(|_| {
                InternalSnafu {
                    message: "IO worker thread panicked".to_string(),
                }
                .build()
            })?;
        }

        info!("Queue shutdown complete");
        Ok(())
    }

    /// Get the queue configuration.
    #[must_use]
    pub fn config(&self) -> &QueueConfig { &self.config }
}

impl Drop for Queue {
    fn drop(&mut self) {
        if self.io_worker_handle.is_some() {
            self.shutdown_flag.store(true, Ordering::SeqCst);
        }
    }
}
