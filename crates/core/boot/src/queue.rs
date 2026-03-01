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

//! Event queue factory — creates the appropriate EventQueue implementation
//! based on configuration.
//!
//! - When a WAL path is provided, creates a [`HybridQueue`] (memory + WAL
//!   persistence) and recovers any pending events from the previous run.
//! - Otherwise, falls back to the in-memory-only [`MemoryQueue`].

use std::{path::PathBuf, sync::Arc};

use rara_kernel::event_queue::EventQueue;
use rara_queue::hybrid::HybridQueue;
use rara_queue::memory::MemoryQueue;

/// Configuration for the event queue factory.
pub struct QueueConfig {
    /// Maximum capacity across all priority tiers.
    pub capacity: usize,
    /// Optional WAL file path. When set, creates a HybridQueue.
    pub wal_path: Option<PathBuf>,
    /// Number of completed WAL entries before auto-truncation.
    /// Only relevant when `wal_path` is set.
    pub truncate_after: usize,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            capacity:       4096,
            wal_path:       None,
            truncate_after: 1024,
        }
    }
}

/// Create an event queue based on the provided configuration.
///
/// If `wal_path` is set, creates a [`HybridQueue`] with crash-recovery WAL
/// and replays any pending events. Otherwise, returns a plain in-memory queue.
pub fn create_event_queue(config: QueueConfig) -> Arc<dyn EventQueue> {
    match config.wal_path {
        Some(wal_path) => {
            match HybridQueue::open(config.capacity, &wal_path, config.truncate_after) {
                Ok(queue) => {
                    // Recover any pending events from the previous run.
                    match queue.recover() {
                        Ok(0) => {
                            tracing::debug!("WAL recovery: no pending events");
                        }
                        Ok(n) => {
                            tracing::info!(recovered = n, "WAL recovery replayed pending events");
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "WAL recovery failed; starting fresh");
                        }
                    }
                    Arc::new(queue)
                }
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        path = %wal_path.display(),
                        "failed to open HybridQueue WAL; falling back to in-memory queue"
                    );
                    Arc::new(MemoryQueue::new(config.capacity))
                }
            }
        }
        None => {
            tracing::debug!(capacity = config.capacity, "using in-memory event queue");
            Arc::new(MemoryQueue::new(config.capacity))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_creates_memory_queue() {
        let config = QueueConfig::default();
        assert!(config.wal_path.is_none());
        assert_eq!(config.capacity, 4096);
        assert_eq!(config.truncate_after, 1024);

        let queue = create_event_queue(config);
        assert_eq!(queue.pending_count(), 0);
    }

    #[test]
    fn wal_config_creates_hybrid_queue() {
        let dir = tempfile::tempdir().unwrap();
        let config = QueueConfig {
            capacity:       100,
            wal_path:       Some(dir.path().join("test.wal")),
            truncate_after: 50,
        };

        let queue = create_event_queue(config);
        assert_eq!(queue.pending_count(), 0);
    }
}
