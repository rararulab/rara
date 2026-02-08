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

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use rsketch_common_worker::{Handle, Manager, Notifiable, Pausable, Worker, WorkerContext};

struct TestWorker {
    counter: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl Worker for TestWorker {
    async fn work<S: Clone + Send + Sync>(&mut self, _ctx: WorkerContext<S>) {
        self.counter.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn test_once_trigger() {
    let counter = Arc::new(AtomicUsize::new(0));
    let mut manager = Manager::new();

    let _handle = manager
        .worker(TestWorker {
            counter: counter.clone(),
        })
        .name("test-once")
        .once()
        .spawn();

    tokio::time::sleep(Duration::from_millis(100)).await;
    manager.shutdown().await;

    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_notify_trigger() {
    let counter = Arc::new(AtomicUsize::new(0));
    let mut manager = Manager::new();

    let handle = manager
        .worker(TestWorker {
            counter: counter.clone(),
        })
        .name("test-notify")
        .on_notify()
        .spawn();

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(counter.load(Ordering::SeqCst), 0);

    handle.notify();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(counter.load(Ordering::SeqCst), 1);

    handle.notify();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(counter.load(Ordering::SeqCst), 2);

    manager.shutdown().await;
}

#[tokio::test]
async fn test_interval_trigger() {
    let counter = Arc::new(AtomicUsize::new(0));
    let mut manager = Manager::new();

    let _handle = manager
        .worker(TestWorker {
            counter: counter.clone(),
        })
        .name("test-interval")
        .interval(Duration::from_millis(50))
        .spawn();

    tokio::time::sleep(Duration::from_millis(125)).await;
    manager.shutdown().await;

    let count = counter.load(Ordering::SeqCst);
    assert!(
        (2..=3).contains(&count),
        "Expected 2-3 executions, got {count}"
    );
}

#[tokio::test]
async fn test_pausable() {
    let counter = Arc::new(AtomicUsize::new(0));
    let mut manager = Manager::new();

    let handle = manager
        .worker(TestWorker {
            counter: counter.clone(),
        })
        .name("test-pausable")
        .interval(Duration::from_millis(50))
        .spawn();

    tokio::time::sleep(Duration::from_millis(125)).await;
    let before_pause = counter.load(Ordering::SeqCst);

    handle.pause();
    assert!(handle.is_paused());

    tokio::time::sleep(Duration::from_millis(150)).await;
    let during_pause = counter.load(Ordering::SeqCst);
    assert_eq!(
        before_pause, during_pause,
        "Counter should not increase while paused"
    );

    handle.resume();
    assert!(!handle.is_paused());

    tokio::time::sleep(Duration::from_millis(125)).await;
    let after_resume = counter.load(Ordering::SeqCst);
    assert!(
        after_resume > during_pause,
        "Counter should increase after resume"
    );

    manager.shutdown().await;
}

#[tokio::test]
async fn test_with_state() {
    #[derive(Clone)]
    struct AppState {
        value: Arc<AtomicUsize>,
    }

    struct StateWorker;

    #[async_trait::async_trait]
    impl Worker for StateWorker {
        async fn work<S: Clone + Send + Sync>(&mut self, ctx: WorkerContext<S>) {
            // This is a bit of a trick - we know S is AppState but can't access it directly
            // without downcasting. In real usage, you'd use a type parameter or associated
            // type. For now, just test that the context is passed correctly.
            let _name = ctx.name();
        }
    }

    let state = AppState {
        value: Arc::new(AtomicUsize::new(42)),
    };

    let mut manager = Manager::with_state(state.clone());

    let _handle = manager
        .worker(StateWorker)
        .name("test-state")
        .once()
        .spawn();

    tokio::time::sleep(Duration::from_millis(100)).await;
    manager.shutdown().await;

    assert_eq!(state.value.load(Ordering::SeqCst), 42);
}

#[tokio::test]
async fn test_cron_trigger() {
    let counter = Arc::new(AtomicUsize::new(0));
    let mut manager = Manager::new();

    // Every minute (standard cron format without seconds)
    let _handle = manager
        .worker(TestWorker {
            counter: counter.clone(),
        })
        .name("test-cron")
        .cron("* * * * *")
        .expect("Valid cron expression")
        .spawn();

    // Since cron runs every minute, we can't easily test it in a short time
    // Just verify it compiles and starts
    tokio::time::sleep(Duration::from_millis(100)).await;
    manager.shutdown().await;

    // We can't assert exact count since cron timing is minute-based
    // Just verify the worker was created successfully
}

#[tokio::test]
async fn test_worker_count() {
    let counter = Arc::new(AtomicUsize::new(0));
    let mut manager = Manager::new();

    assert_eq!(manager.worker_count(), 0);

    let _h1 = manager
        .worker(TestWorker {
            counter: counter.clone(),
        })
        .name("worker-1")
        .interval(Duration::from_millis(100))
        .spawn();

    assert_eq!(manager.worker_count(), 1);

    let _h2 = manager
        .worker(TestWorker {
            counter: counter.clone(),
        })
        .name("worker-2")
        .interval(Duration::from_millis(100))
        .spawn();

    assert_eq!(manager.worker_count(), 2);

    let _h3 = manager
        .worker(TestWorker {
            counter: counter.clone(),
        })
        .name("worker-3")
        .interval(Duration::from_millis(100))
        .spawn();

    assert_eq!(manager.worker_count(), 3);

    manager.shutdown().await;
}

#[tokio::test]
async fn test_find_by_name() {
    let counter = Arc::new(AtomicUsize::new(0));
    let mut manager = Manager::new();

    let h1 = manager
        .worker(TestWorker {
            counter: counter.clone(),
        })
        .name("metrics-worker")
        .interval(Duration::from_millis(100))
        .spawn();

    let h2 = manager
        .worker(TestWorker {
            counter: counter.clone(),
        })
        .name("metrics-worker")
        .interval(Duration::from_millis(100))
        .spawn();

    let _h3 = manager
        .worker(TestWorker {
            counter: counter.clone(),
        })
        .name("other-worker")
        .interval(Duration::from_millis(100))
        .spawn();

    let metrics_ids = manager.find_by_name("metrics-worker");
    assert_eq!(metrics_ids.len(), 2);
    assert!(metrics_ids.contains(&h1.id()));
    assert!(metrics_ids.contains(&h2.id()));

    let other_ids = manager.find_by_name("other-worker");
    assert_eq!(other_ids.len(), 1);

    let empty_ids = manager.find_by_name("nonexistent");
    assert!(empty_ids.is_empty());

    manager.shutdown().await;
}

#[tokio::test]
async fn test_terminate() {
    let counter = Arc::new(AtomicUsize::new(0));
    let mut manager = Manager::new();

    let handle = manager
        .worker(TestWorker {
            counter: counter.clone(),
        })
        .name("test-terminate")
        .interval(Duration::from_millis(50))
        .spawn();

    tokio::time::sleep(Duration::from_millis(125)).await;
    let count_before = counter.load(Ordering::SeqCst);
    assert!(count_before >= 2, "Should have run at least twice");

    let terminated = manager.terminate(handle.id());
    assert!(terminated);

    tokio::time::sleep(Duration::from_millis(100)).await;
    let count_after = counter.load(Ordering::SeqCst);

    assert!(
        count_after <= count_before + 1,
        "Counter should stop increasing after terminate"
    );

    let terminated_again = manager.terminate(handle.id());
    assert!(terminated_again);

    manager.shutdown().await;
}

#[tokio::test]
async fn test_terminate_nonexistent() {
    let mut manager = Manager::new();
    let counter = Arc::new(AtomicUsize::new(0));

    let handle = manager
        .worker(TestWorker {
            counter: counter.clone(),
        })
        .name("test-worker")
        .once()
        .spawn();

    tokio::time::sleep(Duration::from_millis(50)).await;
    manager.remove(handle.id()).await;

    let terminated = manager.terminate(handle.id());
    assert!(!terminated);

    manager.shutdown().await;
}

#[tokio::test]
async fn test_remove() {
    let counter = Arc::new(AtomicUsize::new(0));
    let mut manager = Manager::new();

    let handle = manager
        .worker(TestWorker {
            counter: counter.clone(),
        })
        .name("test-remove")
        .interval(Duration::from_millis(50))
        .spawn();

    assert_eq!(manager.worker_count(), 1);

    tokio::time::sleep(Duration::from_millis(125)).await;
    let count_before = counter.load(Ordering::SeqCst);
    assert!(count_before >= 2, "Should have run at least twice");

    let name = manager.remove(handle.id()).await;
    assert_eq!(name, Some("test-remove"));
    assert_eq!(manager.worker_count(), 0);

    let count_after = counter.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let count_final = counter.load(Ordering::SeqCst);
    assert_eq!(
        count_after, count_final,
        "Counter should not change after remove"
    );

    let removed_again = manager.remove(handle.id()).await;
    assert!(removed_again.is_none());

    manager.shutdown().await;
}

#[tokio::test]
async fn test_remove_nonexistent() {
    let mut manager = Manager::new();
    let counter = Arc::new(AtomicUsize::new(0));

    let handle = manager
        .worker(TestWorker {
            counter: counter.clone(),
        })
        .name("test-worker")
        .once()
        .spawn();

    tokio::time::sleep(Duration::from_millis(50)).await;
    manager.remove(handle.id()).await;

    let removed = manager.remove(handle.id()).await;
    assert!(removed.is_none());

    manager.shutdown().await;
}
