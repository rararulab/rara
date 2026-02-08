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

//! Example demonstrating the new Worker API
//!
//! This example shows:
//! - Creating workers with different trigger types
//! - Using shared state
//! - Pausing and resuming workers
//! - Manually triggering workers

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use rsketch_common_worker::{Manager, Notifiable, Pausable, Worker, WorkerContext};

// Example 1: Simple interval worker
struct CounterWorker {
    counter: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl Worker for CounterWorker {
    async fn work<S: Clone + Send + Sync>(&mut self, ctx: WorkerContext<S>) {
        let count = self.counter.fetch_add(1, Ordering::SeqCst);
        tracing::info!(worker = ctx.name(), count, "Worker executed");
    }
}

// Example 2: Worker with state
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct AppState {
    name:    String,
    version: String,
}

struct StateWorker;

#[async_trait::async_trait]
impl Worker for StateWorker {
    async fn work<S: Clone + Send + Sync>(&mut self, ctx: WorkerContext<S>) {
        tracing::info!(worker = ctx.name(), "Worker with state executed");
    }
}

// Example 3: Worker with lifecycle hooks
struct LifecycleWorker;

#[async_trait::async_trait]
impl Worker for LifecycleWorker {
    async fn on_start<S: Clone + Send + Sync>(&mut self, ctx: WorkerContext<S>) {
        tracing::info!(worker = ctx.name(), "Worker starting up");
    }

    async fn work<S: Clone + Send + Sync>(&mut self, ctx: WorkerContext<S>) {
        tracing::info!(worker = ctx.name(), "Worker executing");

        // Can check cancellation
        if ctx.is_cancelled() {
            tracing::info!("Cancellation requested");
            return;
        }
    }

    async fn on_shutdown<S: Clone + Send + Sync>(&mut self, ctx: WorkerContext<S>) {
        tracing::info!(worker = ctx.name(), "Worker shutting down");
    }
}

#[tokio::main]
async fn main() {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    // Example 1: Worker without state
    {
        let counter = Arc::new(AtomicUsize::new(0));
        let mut manager = Manager::new();

        // Once worker
        let _once_handle = manager
            .worker(CounterWorker {
                counter: counter.clone(),
            })
            .name("once-worker")
            .once()
            .spawn();

        // Interval worker
        let interval_handle = manager
            .worker(CounterWorker {
                counter: counter.clone(),
            })
            .name("interval-worker")
            .interval(Duration::from_secs(1))
            .spawn();

        // Notify worker
        let notify_handle = manager
            .worker(CounterWorker {
                counter: counter.clone(),
            })
            .name("notify-worker")
            .on_notify()
            .spawn();

        // Let workers run
        tokio::time::sleep(Duration::from_secs(3)).await;

        // Pause the interval worker
        interval_handle.pause();
        tracing::info!("Paused interval worker");
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Resume the interval worker
        interval_handle.resume();
        tracing::info!("Resumed interval worker");

        // Manually trigger the notify worker
        notify_handle.notify();
        tracing::info!("Notified worker");

        tokio::time::sleep(Duration::from_secs(1)).await;

        tracing::info!(
            total_executions = counter.load(Ordering::SeqCst),
            "Shutting down"
        );
        manager.shutdown().await;
    }

    // Example 2: Worker with shared state
    {
        let state = AppState {
            name:    "MyApp".to_string(),
            version: "1.0.0".to_string(),
        };

        let mut manager = Manager::with_state(state);

        let _handle = manager
            .worker(StateWorker)
            .name("state-worker")
            .interval(Duration::from_secs(1))
            .spawn();

        tokio::time::sleep(Duration::from_secs(2)).await;
        manager.shutdown().await;
    }

    // Example 3: Worker with lifecycle hooks
    {
        let mut manager = Manager::new();

        let _handle = manager
            .worker(LifecycleWorker)
            .name("lifecycle-worker")
            .interval(Duration::from_millis(500))
            .spawn();

        tokio::time::sleep(Duration::from_secs(2)).await;
        manager.shutdown().await;
    }

    // Example 4: Cron worker
    {
        let counter = Arc::new(AtomicUsize::new(0));
        let mut manager = Manager::new();

        // Run every minute
        let _handle = manager
            .worker(CounterWorker {
                counter: counter.clone(),
            })
            .name("cron-worker")
            .cron("* * * * *")
            .expect("Valid cron expression")
            .spawn();

        tracing::info!("Cron worker created (runs every minute)");
        tokio::time::sleep(Duration::from_secs(2)).await;
        manager.shutdown().await;
    }

    tracing::info!("All examples completed");
}
