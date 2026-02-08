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

use std::{path::PathBuf, time::Duration};

/// Configuration for the persistent queue.
#[derive(Debug, Clone)]
pub struct QueueConfig {
    /// Root directory for queue data files.
    pub base_path:         PathBuf,
    /// Maximum size of each data file in bytes.
    pub file_size:         u64,
    /// Strategy for rolling to new data files.
    pub roll_strategy:     RollStrategy,
    /// How writes are flushed to disk.
    pub flush_mode:        FlushMode,
    /// Interval between index entries (every N messages).
    pub index_interval:    u64,
    /// Whether to verify data integrity on startup.
    pub verify_on_startup: bool,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            base_path:         PathBuf::from("./queue_data"),
            file_size:         1024 * 1024 * 1024,
            roll_strategy:     RollStrategy::BySize(1024 * 1024 * 1024),
            flush_mode:        FlushMode::Async,
            index_interval:    1024,
            verify_on_startup: false,
        }
    }
}

/// Strategy for rolling data files.
#[derive(Debug, Clone)]
pub enum RollStrategy {
    /// Roll when file exceeds the given size in bytes.
    BySize(u64),
    /// Roll when the given duration has elapsed.
    ByTime(Duration),
    /// Roll after the given number of messages.
    ByCount(u64),
    /// Roll when any of the contained strategies triggers.
    Combined(Vec<Self>),
}

impl RollStrategy {
    /// Returns true if the file should be rolled based on current metrics.
    #[must_use]
    pub fn should_roll(&self, current_size: u64, elapsed: Duration, count: u64) -> bool {
        match self {
            Self::BySize(size) => current_size >= *size,
            Self::ByTime(duration) => elapsed >= *duration,
            Self::ByCount(max_count) => count >= *max_count,
            Self::Combined(strategies) => strategies
                .iter()
                .any(|s| s.should_roll(current_size, elapsed, count)),
        }
    }
}

/// Controls how writes are flushed to disk.
#[derive(Debug, Clone)]
pub enum FlushMode {
    /// Flush asynchronously (OS decides when to flush).
    Async,
    /// Flush synchronously after each write.
    Sync,
    /// Flush after accumulating bytes or after interval elapses.
    Batch { bytes: usize, interval: Duration },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roll_by_size() {
        let strategy = RollStrategy::BySize(1000);
        assert!(!strategy.should_roll(999, Duration::from_secs(0), 0));
        assert!(strategy.should_roll(1000, Duration::from_secs(0), 0));
        assert!(strategy.should_roll(1001, Duration::from_secs(0), 0));
    }

    #[test]
    fn test_roll_by_time() {
        let strategy = RollStrategy::ByTime(Duration::from_secs(60));
        assert!(!strategy.should_roll(0, Duration::from_secs(59), 0));
        assert!(strategy.should_roll(0, Duration::from_secs(60), 0));
        assert!(strategy.should_roll(0, Duration::from_secs(61), 0));
    }

    #[test]
    fn test_roll_by_count() {
        let strategy = RollStrategy::ByCount(100);
        assert!(!strategy.should_roll(0, Duration::from_secs(0), 99));
        assert!(strategy.should_roll(0, Duration::from_secs(0), 100));
        assert!(strategy.should_roll(0, Duration::from_secs(0), 101));
    }

    #[test]
    fn test_roll_combined() {
        let strategy =
            RollStrategy::Combined(vec![RollStrategy::BySize(1000), RollStrategy::ByCount(100)]);

        assert!(!strategy.should_roll(999, Duration::from_secs(0), 99));
        assert!(strategy.should_roll(1000, Duration::from_secs(0), 99));
        assert!(strategy.should_roll(999, Duration::from_secs(0), 100));
        assert!(strategy.should_roll(1000, Duration::from_secs(0), 100));
    }
}
