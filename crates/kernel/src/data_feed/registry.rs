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
//! database persistence layer — callers are responsible for syncing
//! changes to the `data_feeds` table after mutations.
//!
//! # Lifecycle
//!
//! 1. On startup, the caller loads configs from the `data_feeds` table and
//!    calls [`restore`](DataFeedRegistry::restore).
//! 2. At runtime, `register` / `remove` mutate the in-memory map. The caller
//!    persists after each mutation.
//! 3. Concrete feed tasks are started externally — the registry only stores
//!    their [`CancellationToken`]s so that `remove` can cancel a running feed.

use std::{collections::HashMap, sync::Arc};

use parking_lot::Mutex;
use snafu::whatever;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::info;

use super::{DataFeedConfig, FeedEvent, StatusReporterRef};

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
    /// Optional hook for persisting status transitions to the `data_feeds`
    /// table. When absent, runtime status lives only in memory — callers
    /// can still drive DB writes manually, but the kernel will not nudge
    /// them.
    reporter: Arc<Mutex<Option<StatusReporterRef>>>,
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
            reporter: Arc::new(Mutex::new(None)),
        }
    }

    /// Install a [`StatusReporter`](super::StatusReporter) for DB
    /// write-back of runtime status transitions.
    ///
    /// Intended to be called once during bootstrap by the application
    /// layer (e.g. backend-admin) that owns the persistence service.
    pub fn set_reporter(&self, reporter: StatusReporterRef) {
        *self.reporter.lock() = Some(reporter);
    }

    /// Return a clone of the installed
    /// [`StatusReporter`](super::StatusReporter), if any.
    pub fn reporter(&self) -> Option<StatusReporterRef> { self.reporter.lock().clone() }

    /// Register a new data feed configuration.
    ///
    /// Returns an error if a feed with the same name is already registered.
    /// The caller is responsible for persisting configs to the `data_feeds`
    /// table after this call.
    pub fn register(&self, config: DataFeedConfig) -> crate::Result<()> {
        let mut configs = self.configs.lock();
        if configs.contains_key(&config.name) {
            whatever!("data feed already registered: {}", config.name);
        }
        info!(name = %config.name, feed_type = %config.feed_type, "registered data feed");
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
    /// the caller serialises the returned vec and writes it to the
    /// `data_feeds` table.
    pub fn configs(&self) -> Vec<DataFeedConfig> { self.configs.lock().values().cloned().collect() }

    /// Bulk-load configs from a previously persisted state.
    ///
    /// Called once at kernel startup after reading the `data_feeds` table.
    /// Any existing configs are replaced.
    pub fn restore(&self, configs: Vec<DataFeedConfig>) {
        let mut map = self.configs.lock();
        map.clear();
        let count = configs.len();
        for config in configs {
            map.insert(config.name.clone(), config);
        }
        info!(count, "restored data feed configs from database");
    }

    /// Return a clone of the shared event sender.
    ///
    /// Concrete feed implementations need a sender to emit events.
    pub fn event_tx(&self) -> mpsc::Sender<FeedEvent> { self.event_tx.clone() }

    /// Register a cancellation token for a running feed task.
    ///
    /// Called by the feed spawner after `tokio::spawn`-ing the feed's
    /// `run` future. If a [`StatusReporter`](super::StatusReporter) is
    /// installed, this spawns a best-effort report of
    /// [`FeedStatus::Running`](super::FeedStatus::Running) so the DB
    /// reflects reality.
    pub fn set_running(&self, name: String, token: CancellationToken) {
        self.running.lock().insert(name.clone(), token);
        self.spawn_report(name, super::FeedStatus::Running, None);
    }

    /// Check whether a feed has a running task.
    pub fn is_running(&self, name: &str) -> bool { self.running.lock().contains_key(name) }

    /// Remove the cancellation token for a feed that has stopped.
    ///
    /// Called when a feed task completes (either normally or due to error).
    /// Reports [`FeedStatus::Idle`](super::FeedStatus::Idle) through the
    /// installed reporter. Use [`report_error`](Self::report_error) instead
    /// if the task exited due to a fatal error.
    pub fn clear_running(&self, name: &str) {
        self.running.lock().remove(name);
        self.spawn_report(name.to_owned(), super::FeedStatus::Idle, None);
    }

    /// Report a terminal error for a feed's runtime. Clears the cancel
    /// token and persists `FeedStatus::Error` with `last_error = message`.
    pub fn report_error(&self, name: &str, message: String) {
        self.running.lock().remove(name);
        self.spawn_report(name.to_owned(), super::FeedStatus::Error, Some(message));
    }

    /// Spawn a fire-and-forget status report. Does nothing when no
    /// reporter is installed; the kernel must never block or panic on a
    /// failed DB write.
    fn spawn_report(&self, name: String, status: super::FeedStatus, last_error: Option<String>) {
        if let Some(reporter) = self.reporter() {
            tokio::spawn(async move {
                reporter.report(&name, status, last_error).await;
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;
    use tokio::sync::mpsc;

    use super::*;
    use crate::data_feed::{FeedType, config::FeedStatus};

    fn make_config(name: &str) -> DataFeedConfig {
        DataFeedConfig::builder()
            .id(format!("{name}-id"))
            .name(name.to_owned())
            .feed_type(FeedType::Webhook)
            .tags(vec!["test".to_owned()])
            .transport(serde_json::json!({}))
            .enabled(true)
            .status(FeedStatus::Idle)
            .created_at(Timestamp::UNIX_EPOCH)
            .updated_at(Timestamp::UNIX_EPOCH)
            .build()
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

    // ---- Reporter wiring ---------------------------------------------------

    use async_trait::async_trait;
    use tokio::sync::Mutex as AsyncMutex;

    use crate::data_feed::StatusReporter;

    #[derive(Default)]
    struct RecordingReporter {
        events: AsyncMutex<Vec<(String, FeedStatus, Option<String>)>>,
    }

    #[async_trait]
    impl StatusReporter for RecordingReporter {
        async fn report(&self, name: &str, status: FeedStatus, last_error: Option<String>) {
            self.events
                .lock()
                .await
                .push((name.to_owned(), status, last_error));
        }
    }

    #[tokio::test]
    async fn set_running_reports_running_through_reporter() {
        let (tx, _rx) = mpsc::channel(16);
        let registry = DataFeedRegistry::new(tx);
        let reporter = Arc::new(RecordingReporter::default());
        registry.set_reporter(reporter.clone());

        let token = CancellationToken::new();
        registry.set_running("alpha".to_owned(), token);

        // Give the spawned report a tick to land.
        tokio::task::yield_now().await;
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let events = reporter.events.lock().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, "alpha");
        assert_eq!(events[0].1, FeedStatus::Running);
        assert!(events[0].2.is_none());
    }

    #[tokio::test]
    async fn clear_running_reports_idle_through_reporter() {
        let (tx, _rx) = mpsc::channel(16);
        let registry = DataFeedRegistry::new(tx);
        let reporter = Arc::new(RecordingReporter::default());
        registry.set_reporter(reporter.clone());

        registry.set_running("beta".to_owned(), CancellationToken::new());
        registry.clear_running("beta");

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let events = reporter.events.lock().await;
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].1, FeedStatus::Idle);
        assert!(events[1].2.is_none());
    }

    #[tokio::test]
    async fn report_error_propagates_message() {
        let (tx, _rx) = mpsc::channel(16);
        let registry = DataFeedRegistry::new(tx);
        let reporter = Arc::new(RecordingReporter::default());
        registry.set_reporter(reporter.clone());

        registry.set_running("gamma".to_owned(), CancellationToken::new());
        registry.report_error("gamma", "boom".to_owned());

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let events = reporter.events.lock().await;
        assert_eq!(events.last().unwrap().1, FeedStatus::Error);
        assert_eq!(events.last().unwrap().2.as_deref(), Some("boom"));
    }
}
