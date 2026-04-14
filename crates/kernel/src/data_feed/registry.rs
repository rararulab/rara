// Copyright 2025 Rararulab
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

//! DataFeedRegistry — manages registered data feed configurations.
//!
//! The registry handles CRUD operations on [`DataFeedConfig`]s and tracks
//! cancellation tokens for running feed tasks. It does **not** own the
//! settings persistence layer — callers are responsible for reading
//! [`configs`](DataFeedRegistry::configs) and writing to the settings
//! store after mutations.
//!
//! # Lifecycle
//!
//! 1. On startup, the caller loads configs from settings and calls
//!    [`restore`](DataFeedRegistry::restore).
//! 2. At runtime, `register` / `remove` mutate the in-memory map. The caller
//!    persists after each mutation.
//! 3. Concrete feed tasks are started externally (Step 3) — the registry only
//!    stores their [`CancellationToken`]s so that `remove` can cancel a running
//!    feed.

use std::{collections::HashMap, sync::Arc};

use parking_lot::Mutex;
use snafu::whatever;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::info;

use super::{DataFeedConfig, FeedEvent};

/// Manages registered data feeds and their runtime state.
///
/// Thread-safe: all interior state is behind `parking_lot::Mutex`.
/// The struct itself is cheaply clonable via the inner `Arc`s.
pub struct DataFeedRegistry {
    /// Registered feed configs keyed by name.
    configs:  Arc<Mutex<HashMap<String, DataFeedConfig>>>,
    /// Event sender shared by all running feeds.
    event_tx: mpsc::Sender<FeedEvent>,
    /// Cancel tokens for running feed tasks, keyed by feed name.
    /// Populated externally when a concrete feed task is spawned.
    running:  Arc<Mutex<HashMap<String, CancellationToken>>>,
}

impl DataFeedRegistry {
    /// Create a new empty registry.
    ///
    /// `event_tx` is the channel sender that all feeds will use to emit
    /// events. The receiving end is owned by the kernel's event dispatcher.
    pub fn new(event_tx: mpsc::Sender<FeedEvent>) -> Self {
        Self {
            configs: Arc::new(Mutex::new(HashMap::new())),
            event_tx,
            running: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Register a new data feed configuration.
    ///
    /// Returns an error if a feed with the same name is already registered.
    /// The caller is responsible for persisting configs to settings after
    /// this call.
    pub fn register(&self, config: DataFeedConfig) -> crate::Result<()> {
        let mut configs = self.configs.lock();
        if configs.contains_key(&config.name) {
            whatever!("data feed already registered: {}", config.name);
        }
        info!(name = %config.name, feed_type = ?config.feed_type, "registered data feed");
        configs.insert(config.name.clone(), config);
        Ok(())
    }

    /// Remove a registered data feed by name.
    ///
    /// If the feed has a running task, its cancellation token is triggered
    /// before the config is removed. Returns an error if no feed with
    /// the given name exists.
    pub fn remove(&self, name: &str) -> crate::Result<()> {
        // Cancel running task first (if any). Extract from lock before
        // acting on it to avoid holding the guard across cancel/log.
        let token = self.running.lock().remove(name);
        if let Some(token) = token {
            token.cancel();
            info!(name, "cancelled running data feed task");
        }

        let mut configs = self.configs.lock();
        if configs.remove(name).is_none() {
            whatever!("data feed not found: {name}");
        }
        info!(name, "removed data feed");
        Ok(())
    }

    /// List all registered feed configurations.
    pub fn list(&self) -> Vec<DataFeedConfig> { self.configs.lock().values().cloned().collect() }

    /// Get a single feed configuration by name.
    pub fn get(&self, name: &str) -> Option<DataFeedConfig> {
        self.configs.lock().get(name).cloned()
    }

    /// Return all configs for external persistence.
    ///
    /// This is the serialisation companion to [`restore`](Self::restore) —
    /// the caller serialises the returned vec to JSON and writes it to the
    /// settings store.
    pub fn configs(&self) -> Vec<DataFeedConfig> { self.configs.lock().values().cloned().collect() }

    /// Bulk-load configs from a previously persisted state.
    ///
    /// Called once at kernel startup after reading the settings store.
    /// Any existing configs are replaced.
    pub fn restore(&self, configs: Vec<DataFeedConfig>) {
        let mut map = self.configs.lock();
        map.clear();
        let count = configs.len();
        for config in configs {
            map.insert(config.name.clone(), config);
        }
        info!(count, "restored data feed configs from settings");
    }

    /// Return a clone of the shared event sender.
    ///
    /// Concrete feed implementations need a sender to emit events.
    pub fn event_tx(&self) -> mpsc::Sender<FeedEvent> { self.event_tx.clone() }

    /// Register a cancellation token for a running feed task.
    ///
    /// Called by the feed spawner (Step 3) after `tokio::spawn`-ing the
    /// feed's `run` future.
    pub fn set_running(&self, name: String, token: CancellationToken) {
        self.running.lock().insert(name, token);
    }

    /// Check whether a feed has a running task.
    pub fn is_running(&self, name: &str) -> bool { self.running.lock().contains_key(name) }

    /// Remove the cancellation token for a feed that has stopped.
    ///
    /// Called when a feed task completes (either normally or due to error).
    pub fn clear_running(&self, name: &str) { self.running.lock().remove(name); }
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;
    use tokio::sync::mpsc;

    use super::*;
    use crate::data_feed::FeedType;

    fn make_config(name: &str) -> DataFeedConfig {
        DataFeedConfig {
            name:       name.to_owned(),
            feed_type:  FeedType::Webhook,
            tags:       vec!["test".to_owned()],
            config:     serde_json::json!({}),
            created_at: Timestamp::UNIX_EPOCH,
        }
    }

    #[test]
    fn register_and_list() {
        let (tx, _rx) = mpsc::channel(16);
        let registry = DataFeedRegistry::new(tx);

        registry.register(make_config("alpha")).unwrap();
        registry.register(make_config("beta")).unwrap();

        let configs = registry.list();
        assert_eq!(configs.len(), 2);
    }

    #[test]
    fn register_duplicate_fails() {
        let (tx, _rx) = mpsc::channel(16);
        let registry = DataFeedRegistry::new(tx);

        registry.register(make_config("alpha")).unwrap();
        let result = registry.register(make_config("alpha"));
        assert!(result.is_err());
    }

    #[test]
    fn remove_nonexistent_fails() {
        let (tx, _rx) = mpsc::channel(16);
        let registry = DataFeedRegistry::new(tx);

        let result = registry.remove("ghost");
        assert!(result.is_err());
    }

    #[test]
    fn remove_cancels_running_task() {
        let (tx, _rx) = mpsc::channel(16);
        let registry = DataFeedRegistry::new(tx);

        registry.register(make_config("alpha")).unwrap();

        let token = CancellationToken::new();
        let token_clone = token.clone();
        registry.set_running("alpha".to_owned(), token);

        assert!(registry.is_running("alpha"));

        registry.remove("alpha").unwrap();

        // Token should have been cancelled.
        assert!(token_clone.is_cancelled());
        assert!(!registry.is_running("alpha"));
    }

    #[test]
    fn get_returns_none_for_missing() {
        let (tx, _rx) = mpsc::channel(16);
        let registry = DataFeedRegistry::new(tx);

        assert!(registry.get("nope").is_none());
    }

    #[test]
    fn restore_replaces_existing_configs() {
        let (tx, _rx) = mpsc::channel(16);
        let registry = DataFeedRegistry::new(tx);

        registry.register(make_config("old")).unwrap();

        let new_configs = vec![make_config("new-a"), make_config("new-b")];
        registry.restore(new_configs);

        assert!(registry.get("old").is_none());
        assert!(registry.get("new-a").is_some());
        assert!(registry.get("new-b").is_some());
        assert_eq!(registry.list().len(), 2);
    }

    #[test]
    fn configs_matches_list() {
        let (tx, _rx) = mpsc::channel(16);
        let registry = DataFeedRegistry::new(tx);

        registry.register(make_config("x")).unwrap();
        assert_eq!(registry.configs().len(), registry.list().len());
    }
}
