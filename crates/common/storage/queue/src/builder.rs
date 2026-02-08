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

use std::path::PathBuf;

use crate::{FlushMode, Queue, QueueConfig, Result, RollStrategy};

/// Builder for constructing a Queue with custom configuration.
pub struct QueueBuilder {
    config: QueueConfig,
}

impl QueueBuilder {
    /// Creates a new builder with the given base path.
    pub fn new<P: Into<PathBuf>>(base_path: P) -> Self {
        Self {
            config: QueueConfig {
                base_path: base_path.into(),
                ..Default::default()
            },
        }
    }

    /// Sets the maximum file size in bytes.
    #[must_use]
    pub const fn file_size(mut self, size: u64) -> Self {
        self.config.file_size = size;
        self
    }

    /// Sets the strategy for rolling to new files.
    #[must_use]
    pub fn roll_strategy(mut self, strategy: RollStrategy) -> Self {
        self.config.roll_strategy = strategy;
        self
    }

    /// Sets the flush mode for writes.
    #[must_use]
    pub const fn flush_mode(mut self, mode: FlushMode) -> Self {
        self.config.flush_mode = mode;
        self
    }

    /// Sets the interval between index entries.
    #[must_use]
    pub const fn index_interval(mut self, interval: u64) -> Self {
        self.config.index_interval = interval;
        self
    }

    /// Enables or disables integrity verification on startup.
    #[must_use]
    pub const fn verify_on_startup(mut self, verify: bool) -> Self {
        self.config.verify_on_startup = verify;
        self
    }

    /// Builds the Queue with the configured options.
    ///
    /// # Errors
    ///
    /// Returns an error if queue initialization fails (e.g., directory
    /// creation, manifest loading, or IO worker spawn failures).
    pub fn build(self) -> Result<Queue> { Queue::new(self.config) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_default_config() {
        let builder = QueueBuilder::new("/tmp/test_queue");
        assert_eq!(builder.config.base_path, PathBuf::from("/tmp/test_queue"));
        assert_eq!(builder.config.file_size, 1024 * 1024 * 1024);
        assert_eq!(builder.config.index_interval, 1024);
        assert!(!builder.config.verify_on_startup);
    }

    #[test]
    fn test_builder_custom_config() {
        let builder = QueueBuilder::new("/tmp/test_queue")
            .file_size(512 * 1024 * 1024)
            .index_interval(2048)
            .verify_on_startup(true)
            .flush_mode(FlushMode::Sync)
            .roll_strategy(RollStrategy::ByCount(1000));

        assert_eq!(builder.config.file_size, 512 * 1024 * 1024);
        assert_eq!(builder.config.index_interval, 2048);
        assert!(builder.config.verify_on_startup);
        assert!(matches!(builder.config.flush_mode, FlushMode::Sync));
        assert!(matches!(
            builder.config.roll_strategy,
            RollStrategy::ByCount(1000)
        ));
    }
}
