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

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use tokio::sync::Notify;

use crate::{
    WorkerId,
    metrics::{WORKER_PAUSED, WORKER_RESUMED},
};

/// Base trait for all worker handles, providing access to the worker's name and
/// id.
///
/// All handles implement this trait and are `Clone`, `Send`, and `Sync` to
/// allow sharing across threads and async tasks.
pub trait Handle: Clone + Send + Sync {
    /// Returns the worker's unique identifier.
    fn id(&self) -> WorkerId;

    /// Returns the worker's name for identification and logging.
    fn name(&self) -> &'static str;
}

/// Handle trait for workers that can be paused and resumed.
///
/// Implemented by handles for time-based triggers (Interval, Cron, and their
/// hybrids). Pausing stops the trigger from firing but doesn't cancel the
/// worker - it can be resumed later.
///
/// # Example
///
/// ```rust,no_run
/// # use rsketch_common_worker::{Manager, Worker, WorkerContext, Pausable};
/// # use std::time::Duration;
/// # struct MyWorker;
/// # #[async_trait::async_trait]
/// # impl Worker for MyWorker {
/// #     async fn work<S: Clone + Send + Sync>(&mut self, ctx: WorkerContext<S>) {}
/// # }
/// # #[tokio::main]
/// # async fn main() {
/// let mut manager = Manager::new();
/// let handle = manager
///     .worker(MyWorker)
///     .interval(Duration::from_secs(1))
///     .spawn();
///
/// handle.pause(); // Stop the interval timer
/// // ... do something ...
/// handle.resume(); // Restart the interval timer
/// //
/// # }
/// ```
pub trait Pausable: Handle {
    /// Pauses the worker's trigger, preventing future executions until resumed.
    ///
    /// If the worker is currently executing, this will not interrupt it.
    /// The pause takes effect for the next scheduled execution.
    fn pause(&self);

    /// Resumes a paused worker's trigger, allowing executions to continue.
    ///
    /// If the worker is not paused, this has no effect.
    fn resume(&self);

    /// Returns `true` if the worker is currently paused.
    fn is_paused(&self) -> bool;
}

/// Handle trait for workers that can be manually triggered.
///
/// Implemented by handles for notify-based triggers (Notify and hybrid
/// triggers). Calling `notify()` will trigger an immediate execution.
///
/// # Example
///
/// ```rust,no_run
/// # use rsketch_common_worker::{Manager, Worker, WorkerContext, Notifiable};
/// # struct MyWorker;
/// # #[async_trait::async_trait]
/// # impl Worker for MyWorker {
/// #     async fn work<S: Clone + Send + Sync>(&mut self, ctx: WorkerContext<S>) {}
/// # }
/// # #[tokio::main]
/// # async fn main() {
/// let mut manager = Manager::new();
/// let handle = manager.worker(MyWorker).on_notify().spawn();
///
/// handle.notify(); // Trigger immediate execution
/// //
/// # }
/// ```
pub trait Notifiable: Handle {
    /// Triggers an immediate execution of the worker.
    ///
    /// For hybrid triggers (`IntervalOrNotify`, `CronOrNotify`), this resets
    /// the timer. Multiple `notify()` calls may be coalesced if the worker
    /// is still executing.
    fn notify(&self);
}

/// Handle for workers with `Trigger::Once`.
///
/// This handle has no control methods since the worker runs once and stops.
/// It only provides the worker's name via the `Handle` trait.
#[derive(Clone)]
pub struct OnceHandle {
    id:   WorkerId,
    name: &'static str,
}

impl OnceHandle {
    pub(crate) const fn new(id: WorkerId, name: &'static str) -> Self { Self { id, name } }
}

impl Handle for OnceHandle {
    fn id(&self) -> WorkerId { self.id }

    fn name(&self) -> &'static str { self.name }
}

/// Handle for workers with `Trigger::Notify`.
///
/// Provides `notify()` method to manually trigger execution on demand.
/// The worker will only run when explicitly notified.
#[derive(Clone)]
pub struct NotifyHandle {
    id:     WorkerId,
    name:   &'static str,
    notify: Arc<Notify>,
}

impl NotifyHandle {
    pub(crate) const fn new(id: WorkerId, name: &'static str, notify: Arc<Notify>) -> Self {
        Self { id, name, notify }
    }
}

impl Handle for NotifyHandle {
    fn id(&self) -> WorkerId { self.id }

    fn name(&self) -> &'static str { self.name }
}

impl Notifiable for NotifyHandle {
    fn notify(&self) { self.notify.notify_one(); }
}

/// Handle for workers with `Trigger::Interval`.
///
/// Provides pause/resume methods to control the periodic execution.
/// When paused, the timer stops; when resumed, it restarts.
#[derive(Clone)]
pub struct IntervalHandle {
    id:     WorkerId,
    name:   &'static str,
    notify: Arc<Notify>,
    paused: Arc<AtomicBool>,
}

impl IntervalHandle {
    pub(crate) const fn new(
        id: WorkerId,
        name: &'static str,
        notify: Arc<Notify>,
        paused: Arc<AtomicBool>,
    ) -> Self {
        Self {
            id,
            name,
            notify,
            paused,
        }
    }
}

impl Handle for IntervalHandle {
    fn id(&self) -> WorkerId { self.id }

    fn name(&self) -> &'static str { self.name }
}

impl Pausable for IntervalHandle {
    fn pause(&self) {
        self.paused.store(true, Ordering::Release);
        WORKER_PAUSED.with_label_values(&[self.name]).inc();
    }

    fn resume(&self) {
        self.paused.store(false, Ordering::Release);
        self.notify.notify_one();
        WORKER_RESUMED.with_label_values(&[self.name]).inc();
    }

    fn is_paused(&self) -> bool { self.paused.load(Ordering::Acquire) }
}

/// Handle for workers with `Trigger::Cron`.
///
/// Provides pause/resume methods to control cron-based execution.
/// When paused, no cron triggers fire; when resumed, scheduling continues.
#[derive(Clone)]
pub struct CronHandle {
    id:     WorkerId,
    name:   &'static str,
    notify: Arc<Notify>,
    paused: Arc<AtomicBool>,
}

impl CronHandle {
    pub(crate) const fn new(
        id: WorkerId,
        name: &'static str,
        notify: Arc<Notify>,
        paused: Arc<AtomicBool>,
    ) -> Self {
        Self {
            id,
            name,
            notify,
            paused,
        }
    }
}

impl Handle for CronHandle {
    fn id(&self) -> WorkerId { self.id }

    fn name(&self) -> &'static str { self.name }
}

impl Pausable for CronHandle {
    fn pause(&self) {
        self.paused.store(true, Ordering::Release);
        WORKER_PAUSED.with_label_values(&[self.name]).inc();
    }

    fn resume(&self) {
        self.paused.store(false, Ordering::Release);
        self.notify.notify_one();
        WORKER_RESUMED.with_label_values(&[self.name]).inc();
    }

    fn is_paused(&self) -> bool { self.paused.load(Ordering::Acquire) }
}

/// Handle for workers with `Trigger::IntervalOrNotify`.
///
/// Hybrid handle combining pause/resume and manual notification.
/// The worker runs on an interval schedule OR when explicitly notified.
/// Calling `notify()` triggers immediate execution and resets the interval
/// timer.
#[derive(Clone)]
pub struct IntervalOrNotifyHandle {
    id:     WorkerId,
    name:   &'static str,
    notify: Arc<Notify>,
    paused: Arc<AtomicBool>,
}

impl IntervalOrNotifyHandle {
    pub(crate) const fn new(
        id: WorkerId,
        name: &'static str,
        notify: Arc<Notify>,
        paused: Arc<AtomicBool>,
    ) -> Self {
        Self {
            id,
            name,
            notify,
            paused,
        }
    }
}

impl Handle for IntervalOrNotifyHandle {
    fn id(&self) -> WorkerId { self.id }

    fn name(&self) -> &'static str { self.name }
}

impl Pausable for IntervalOrNotifyHandle {
    fn pause(&self) {
        self.paused.store(true, Ordering::Release);
        WORKER_PAUSED.with_label_values(&[self.name]).inc();
    }

    fn resume(&self) {
        self.paused.store(false, Ordering::Release);
        self.notify.notify_one();
        WORKER_RESUMED.with_label_values(&[self.name]).inc();
    }

    fn is_paused(&self) -> bool { self.paused.load(Ordering::Acquire) }
}

impl Notifiable for IntervalOrNotifyHandle {
    fn notify(&self) { self.notify.notify_one(); }
}

/// Handle for workers with `Trigger::CronOrNotify`.
///
/// Hybrid handle combining pause/resume and manual notification.
/// The worker runs on a cron schedule OR when explicitly notified.
/// Unlike `IntervalOrNotify`, `notify()` doesn't reset the cron schedule - it
/// only triggers an immediate one-time execution.
#[derive(Clone)]
pub struct CronOrNotifyHandle {
    id:     WorkerId,
    name:   &'static str,
    notify: Arc<Notify>,
    paused: Arc<AtomicBool>,
}

impl CronOrNotifyHandle {
    pub(crate) const fn new(
        id: WorkerId,
        name: &'static str,
        notify: Arc<Notify>,
        paused: Arc<AtomicBool>,
    ) -> Self {
        Self {
            id,
            name,
            notify,
            paused,
        }
    }
}

impl Handle for CronOrNotifyHandle {
    fn id(&self) -> WorkerId { self.id }

    fn name(&self) -> &'static str { self.name }
}

impl Pausable for CronOrNotifyHandle {
    fn pause(&self) {
        self.paused.store(true, Ordering::Release);
        WORKER_PAUSED.with_label_values(&[self.name]).inc();
    }

    fn resume(&self) {
        self.paused.store(false, Ordering::Release);
        self.notify.notify_one();
        WORKER_RESUMED.with_label_values(&[self.name]).inc();
    }

    fn is_paused(&self) -> bool { self.paused.load(Ordering::Acquire) }
}

impl Notifiable for CronOrNotifyHandle {
    fn notify(&self) { self.notify.notify_one(); }
}
