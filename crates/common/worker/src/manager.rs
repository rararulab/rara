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
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use rsketch_common_runtime::Runtime;
use smart_default::SmartDefault;
use tokio::{sync::Notify, task::JoinHandle};
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::{
    builder::{SpawnResult, TriggerNotSet, WorkerBuilder},
    context::WorkerContext,
    driver::{
        CronDriver, CronOrNotifyDriver, IntervalDriver, IntervalOrNotifyDriver, NotifyDriver,
        OnceDriver, TriggerDriverEnum,
    },
    id::WorkerId,
    metrics::{
        WORKER_ACTIVE, WORKER_ERRORS, WORKER_EXECUTION_DURATION_SECONDS, WORKER_EXECUTION_ERRORS,
        WORKER_EXECUTIONS, WORKER_SHUTDOWN_ERRORS, WORKER_START_ERRORS, WORKER_STARTED,
        WORKER_STOPPED,
    },
    trigger::Trigger,
    worker::Worker,
};

/// Configuration options for the worker manager.
///
/// # Fields
///
/// - `runtime`: Optional custom Tokio runtime for worker execution. If `None`,
///   uses the global background runtime.
/// - `shutdown_timeout`: Maximum time to wait for workers to finish during
///   shutdown. Defaults to 30 seconds.
///
/// # Example
///
/// ```rust
/// use std::time::Duration;
///
/// use rsketch_common_worker::{Manager, ManagerConfig};
///
/// let config = ManagerConfig {
///     runtime:          None,
///     shutdown_timeout: Duration::from_secs(60),
/// };
/// let manager = Manager::with_config(config);
/// ```
#[derive(Debug, Clone, SmartDefault)]
pub struct ManagerConfig {
    pub runtime:          Option<Arc<Runtime>>,
    #[default(Duration::from_secs(30))]
    pub shutdown_timeout: Duration,
}

struct WorkerEntry {
    name:         &'static str,
    cancel_token: CancellationToken,
    #[allow(dead_code)]
    notify:       Arc<Notify>,
    #[allow(dead_code)]
    paused:       Arc<AtomicBool>,
    #[allow(dead_code)]
    pause_notify: Arc<Notify>,
    join_handle:  JoinHandle<()>,
}

/// Orchestrates lifecycle and execution of multiple background workers.
///
/// The Manager is generic over a shared state type `S` that is cloned and
/// passed to each worker execution via [`WorkerContext`]. For stateless
/// workers, use `Manager<()>`.
///
/// # Lifecycle
///
/// 1. Create manager with `new()` or `with_state()`
/// 2. Configure and spawn workers using the builder API
/// 3. Workers run in background according to their triggers
/// 4. Call `shutdown()` for graceful termination
///
/// # State Management
///
/// State must implement `Clone`. For expensive types, wrap in `Arc<T>`:
///
/// ```rust
/// use std::sync::Arc;
///
/// use rsketch_common_worker::Manager;
///
/// #[derive(Clone)]
/// struct AppState {
///     db:     Arc<Database>, // Expensive, wrapped in Arc
///     config: String,        // Cheap to clone
/// }
/// # struct Database;
///
/// let state = AppState {
///     db:     Arc::new(Database),
///     config: "prod".into(),
/// };
/// let manager = Manager::with_state(state);
/// ```
///
/// # Example
///
/// ```rust,no_run
/// use std::time::Duration;
///
/// use rsketch_common_worker::{Manager, Worker, WorkerContext};
///
/// struct MyWorker;
///
/// #[async_trait::async_trait]
/// impl Worker for MyWorker {
///     async fn work<S: Clone + Send + Sync>(&mut self, ctx: WorkerContext<S>) {
///         println!("Working...");
///     }
/// }
///
/// #[tokio::main]
/// async fn main() {
///     let mut manager = Manager::new();
///
///     // Spawn multiple workers with different triggers
///     let h1 = manager.worker(MyWorker).name("w1").once().spawn();
///     let h2 = manager
///         .worker(MyWorker)
///         .name("w2")
///         .interval(Duration::from_secs(10))
///         .spawn();
///
///     // Graceful shutdown with timeout
///     manager.shutdown().await;
/// }
/// ```
pub struct Manager<S = ()> {
    state:            S,
    cancel_token:     CancellationToken,
    runtime:          Option<Arc<Runtime>>,
    shutdown_timeout: Duration,
    workers:          HashMap<WorkerId, WorkerEntry>,
}

impl Manager<()> {
    /// Creates a new worker manager without shared state.
    ///
    /// Workers will receive `WorkerContext<()>` with no accessible state.
    /// Uses default configuration (30s shutdown timeout).
    #[must_use]
    pub fn new() -> Self { Self::with_config(ManagerConfig::default()) }

    /// Creates a new worker manager with custom configuration.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::time::Duration;
    ///
    /// use rsketch_common_worker::{Manager, ManagerConfig};
    ///
    /// let config = ManagerConfig {
    ///     runtime:          None,
    ///     shutdown_timeout: Duration::from_secs(60),
    /// };
    /// let manager = Manager::with_config(config);
    /// ```
    #[must_use]
    pub fn with_config(config: ManagerConfig) -> Self {
        Self {
            state:            (),
            cancel_token:     CancellationToken::new(),
            runtime:          config.runtime,
            shutdown_timeout: config.shutdown_timeout,
            workers:          HashMap::new(),
        }
    }
}

impl Default for Manager<()> {
    fn default() -> Self { Self::new() }
}

impl<S: Clone + Send + Sync + 'static> Manager<S> {
    /// Creates a worker manager with custom shared state.
    ///
    /// The state will be cloned for each worker execution and passed via
    /// `WorkerContext`. For expensive-to-clone types, wrap them in `Arc<T>`
    /// before passing.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::sync::Arc;
    ///
    /// use rsketch_common_worker::Manager;
    ///
    /// #[derive(Clone)]
    /// struct Config {
    ///     db_url: String,
    /// }
    ///
    /// let config = Config {
    ///     db_url: "postgres://...".into(),
    /// };
    /// let manager = Manager::with_state(config);
    /// ```
    pub fn with_state(state: S) -> Self {
        Self::with_state_and_config(state, ManagerConfig::default())
    }

    /// Create a new worker manager with custom state and configuration.
    pub fn with_state_and_config(state: S, config: ManagerConfig) -> Self {
        Self {
            state,
            cancel_token: CancellationToken::new(),
            runtime: config.runtime,
            shutdown_timeout: config.shutdown_timeout,
            workers: HashMap::new(),
        }
    }

    /// Starts building a worker configuration.
    ///
    /// Returns a builder in the initial state. You must chain methods to:
    /// 1. Optionally set a name with `.name("worker-name")`
    /// 2. Optionally mark as blocking with `.blocking()`
    /// 3. **Required**: Set a trigger (`.once()`, `.on_notify()`,
    ///    `.interval()`, etc.)
    /// 4. **Required**: Call `.spawn()` to actually start the worker
    ///
    /// The type system ensures you can't spawn without setting a trigger.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use rsketch_common_worker::{Manager, Worker, WorkerContext};
    /// # use std::time::Duration;
    /// # struct MyWorker;
    /// # #[async_trait::async_trait]
    /// # impl Worker for MyWorker {
    /// #     async fn work<S: Clone + Send + Sync>(&mut self, ctx: WorkerContext<S>) {}
    /// # }
    /// # let mut manager = Manager::new();
    /// let handle = manager
    ///     .worker(MyWorker)
    ///     .name("my-worker")       // Optional
    ///     .blocking()              // Optional - runs on blocking thread pool
    ///     .interval(Duration::from_secs(5))  // Required trigger
    ///     .spawn(); // Required to start
    /// ```
    pub const fn worker<W: Worker>(&mut self, worker: W) -> WorkerBuilder<'_, S, W, TriggerNotSet> {
        WorkerBuilder::new(self, worker)
    }

    /// Starts building a fallible worker configuration.
    ///
    /// Fallible workers can return errors from their lifecycle hooks. The
    /// runtime handles these errors:
    /// - **Transient errors**: Logged and worker continues to next execution
    /// - **Fatal errors**: Logged and worker stops after calling
    ///   `on_shutdown()`
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use rsketch_common_worker::{Manager, FallibleWorker, WorkerContext, WorkResult, WorkError};
    /// # use std::time::Duration;
    /// # #[derive(Clone)] struct AppState;
    /// struct MyWorker;
    ///
    /// #[async_trait::async_trait]
    /// impl FallibleWorker<AppState> for MyWorker {
    ///     async fn work(&mut self, ctx: WorkerContext<AppState>) -> WorkResult {
    ///         // Return transient error to continue, fatal to stop
    ///         Ok(())
    ///     }
    /// }
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let mut manager = Manager::with_state(AppState);
    /// let handle = manager
    ///     .fallible_worker(MyWorker)
    ///     .name("my-worker")
    ///     .interval(Duration::from_secs(5))
    ///     .spawn();
    /// # }
    /// ```
    pub const fn fallible_worker<W: crate::FallibleWorker<S>>(
        &mut self,
        worker: W,
    ) -> crate::builder::FallibleWorkerBuilder<'_, S, W, TriggerNotSet> {
        crate::builder::FallibleWorkerBuilder::new(self, worker)
    }

    /// Internal method to spawn a worker with a specific trigger.
    pub(crate) fn spawn_worker<W, H>(
        &mut self,
        worker: W,
        name: &'static str,
        blocking: bool,
        trigger: Trigger,
        pause_mode: crate::PauseMode,
    ) -> H
    where
        W: Worker,
        H: SpawnResult,
        S: Clone,
    {
        let id = WorkerId::new();
        let notify = Arc::new(Notify::new());
        let paused = Arc::new(AtomicBool::new(false));
        let pause_notify = Arc::new(Notify::new());

        let worker_cancel_token = self.cancel_token.child_token();

        let ctx = WorkerContext::new(
            self.state.clone(),
            worker_cancel_token.child_token(),
            notify.clone(),
            name,
        );

        let driver = match trigger {
            Trigger::Once => TriggerDriverEnum::Once(OnceDriver::new()),
            Trigger::Notify => TriggerDriverEnum::Notify(NotifyDriver::new()),
            Trigger::Interval(duration) => {
                TriggerDriverEnum::Interval(IntervalDriver::new(duration))
            }
            Trigger::Cron(cron) => TriggerDriverEnum::Cron(CronDriver::new(cron)),
            Trigger::IntervalOrNotify(duration) => {
                TriggerDriverEnum::IntervalOrNotify(IntervalOrNotifyDriver::new(duration))
            }
            Trigger::CronOrNotify(cron) => {
                TriggerDriverEnum::CronOrNotify(CronOrNotifyDriver::new(cron))
            }
        };

        let paused_clone = paused.clone();
        let pause_notify_clone = pause_notify.clone();
        let task = run_worker(
            worker,
            ctx,
            paused_clone,
            pause_notify_clone,
            pause_mode,
            driver,
            name,
        );

        let runtime = self
            .runtime
            .clone()
            .unwrap_or_else(rsketch_common_runtime::background_runtime);

        let join_handle = if blocking {
            let rt_handle = runtime.handle().clone();
            runtime
                .handle()
                .spawn_blocking(move || rt_handle.block_on(task))
        } else {
            runtime.handle().spawn(task)
        };

        let entry = WorkerEntry {
            name,
            cancel_token: worker_cancel_token,
            notify: notify.clone(),
            paused: paused.clone(),
            pause_notify,
            join_handle,
        };
        self.workers.insert(id, entry);

        H::from_parts(id, name, notify, paused)
    }

    /// Internal method to spawn a fallible worker with a specific trigger.
    pub(crate) fn spawn_fallible_worker<W, H>(
        &mut self,
        worker: W,
        name: &'static str,
        blocking: bool,
        trigger: Trigger,
        pause_mode: crate::PauseMode,
    ) -> H
    where
        W: crate::FallibleWorker<S>,
        H: SpawnResult,
        S: Clone,
    {
        let id = WorkerId::new();
        let notify = Arc::new(Notify::new());
        let paused = Arc::new(AtomicBool::new(false));
        let pause_notify = Arc::new(Notify::new());

        let worker_cancel_token = self.cancel_token.child_token();

        let ctx = WorkerContext::new(
            self.state.clone(),
            worker_cancel_token.child_token(),
            notify.clone(),
            name,
        );

        let driver = match trigger {
            Trigger::Once => TriggerDriverEnum::Once(OnceDriver::new()),
            Trigger::Notify => TriggerDriverEnum::Notify(NotifyDriver::new()),
            Trigger::Interval(duration) => {
                TriggerDriverEnum::Interval(IntervalDriver::new(duration))
            }
            Trigger::Cron(cron) => TriggerDriverEnum::Cron(CronDriver::new(cron)),
            Trigger::IntervalOrNotify(duration) => {
                TriggerDriverEnum::IntervalOrNotify(IntervalOrNotifyDriver::new(duration))
            }
            Trigger::CronOrNotify(cron) => {
                TriggerDriverEnum::CronOrNotify(CronOrNotifyDriver::new(cron))
            }
        };

        let paused_clone = paused.clone();
        let pause_notify_clone = pause_notify.clone();
        let task = run_fallible_worker(
            worker,
            ctx,
            paused_clone,
            pause_notify_clone,
            pause_mode,
            driver,
            name,
        );

        let runtime = self
            .runtime
            .clone()
            .unwrap_or_else(rsketch_common_runtime::background_runtime);

        let join_handle = if blocking {
            let rt_handle = runtime.handle().clone();
            runtime
                .handle()
                .spawn_blocking(move || rt_handle.block_on(task))
        } else {
            runtime.handle().spawn(task)
        };

        let entry = WorkerEntry {
            name,
            cancel_token: worker_cancel_token,
            notify: notify.clone(),
            paused: paused.clone(),
            pause_notify,
            join_handle,
        };
        self.workers.insert(id, entry);

        H::from_parts(id, name, notify, paused)
    }

    pub fn terminate(&self, id: WorkerId) -> bool {
        self.workers.get(&id).is_some_and(|entry| {
            entry.cancel_token.cancel();
            true
        })
    }

    pub async fn remove(&mut self, id: WorkerId) -> Option<&'static str> {
        if let Some(mut entry) = self.workers.remove(&id) {
            entry.cancel_token.cancel();
            let _ = (&mut entry.join_handle).await;
            Some(entry.name)
        } else {
            None
        }
    }

    pub fn find_by_name(&self, name: &str) -> Vec<WorkerId> {
        self.workers
            .iter()
            .filter(|(_, entry)| entry.name == name)
            .map(|(id, _)| *id)
            .collect()
    }

    pub fn worker_count(&self) -> usize { self.workers.len() }

    /// Initiates graceful shutdown of all workers and waits for them to
    /// complete.
    ///
    /// This method:
    /// 1. Sends cancellation signal to all workers via their contexts
    /// 2. Waits for workers to finish their current execution and cleanup
    /// 3. Returns when all workers have stopped OR timeout is reached
    /// 4. Aborts any remaining workers if timeout expires
    ///
    /// Workers should check `ctx.is_cancelled()` or await `ctx.cancelled()` to
    /// respond to shutdown requests quickly.
    ///
    /// # Timeout Behavior
    ///
    /// - Default timeout: 30 seconds (configurable via [`ManagerConfig`])
    /// - If workers don't finish within timeout, they are forcefully aborted
    /// - Aborted workers may not run their `on_shutdown` hooks
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use rsketch_common_worker::{Manager, Worker, WorkerContext};
    /// # use std::time::Duration;
    /// # struct MyWorker;
    /// # #[async_trait::async_trait]
    /// # impl Worker for MyWorker {
    /// #     async fn work<S: Clone + Send + Sync>(&mut self, ctx: WorkerContext<S>) {
    /// #         loop {
    /// #             if ctx.is_cancelled() {
    /// #                 break;  // Respond to shutdown
    /// #             }
    /// #             tokio::time::sleep(Duration::from_secs(1)).await;
    /// #         }
    /// #     }
    /// # }
    /// # #[tokio::main]
    /// # async fn main() {
    /// let mut manager = Manager::new();
    /// manager
    ///     .worker(MyWorker)
    ///     .interval(Duration::from_secs(10))
    ///     .spawn();
    ///
    /// // ... application runs ...
    ///
    /// // Graceful shutdown
    /// manager.shutdown().await;
    /// # }
    /// ```
    pub async fn shutdown(&mut self) {
        info!("Shutting down worker manager");
        self.cancel_token.cancel();

        for entry in self.workers.values() {
            entry.cancel_token.cancel();
        }

        let deadline = tokio::time::Instant::now() + self.shutdown_timeout;
        let mut aborted_count = 0;
        let mut total_count = 0;

        let worker_ids: Vec<_> = self.workers.keys().copied().collect();
        for id in worker_ids {
            if let Some(mut entry) = self.workers.remove(&id) {
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                total_count += 1;

                match tokio::time::timeout(remaining, &mut entry.join_handle).await {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => {
                        if e.is_cancelled() {
                            aborted_count += 1;
                        } else {
                            error!(name = entry.name, error = ?e, "Join error during shutdown");
                        }
                    }
                    Err(_) => {
                        entry.join_handle.abort();
                        aborted_count += 1;
                        error!(
                            name = entry.name,
                            "Worker timed out during shutdown, aborted"
                        );
                    }
                }
            }
        }

        if aborted_count > 0 {
            error!(
                stopped = total_count - aborted_count,
                aborted = aborted_count,
                "Worker manager shutdown complete"
            );
        } else {
            info!(stopped = total_count, "Worker manager shutdown complete");
        }
    }
}

/// Unified execution loop for all worker types.
async fn run_worker<S: Clone + Send + Sync, W: Worker>(
    mut worker: W,
    ctx: WorkerContext<S>,
    paused: Arc<AtomicBool>,
    pause_notify: Arc<Notify>,
    pause_mode: crate::PauseMode,
    mut driver: TriggerDriverEnum,
    name: &'static str,
) {
    let span = tracing::info_span!("worker", name);
    let _guard = span.enter();

    info!("Worker starting");
    WORKER_STARTED.with_label_values(&[name]).inc();
    WORKER_ACTIVE.with_label_values(&[name]).set(1);

    // Call on_start hook
    worker.on_start(ctx.clone()).await;

    // Main execution loop
    while driver.wait_next(&ctx).await {
        // Handle pause
        if paused.load(Ordering::Acquire) {
            match pause_mode {
                crate::PauseMode::Soft => {
                    // Soft pause: skip this execution, driver continues
                    continue;
                }
                crate::PauseMode::Hard => {
                    // Hard pause: wait for resume signal
                    loop {
                        tokio::select! {
                            () = pause_notify.notified() => {
                                if !paused.load(Ordering::Acquire) {
                                    break; // Resumed
                                }
                            }
                            () = ctx.cancelled() => {
                                // Shutdown requested while paused
                                worker.on_shutdown(ctx.clone()).await;
                                info!("Worker stopped (cancelled while paused)");
                                WORKER_STOPPED.with_label_values(&[name]).inc();
                                WORKER_ACTIVE.with_label_values(&[name]).set(0);
                                return;
                            }
                        }
                    }
                }
            }
        }

        let start = Instant::now();
        worker.work(ctx.clone()).await;

        WORKER_EXECUTIONS.with_label_values(&[name]).inc();
        WORKER_EXECUTION_DURATION_SECONDS
            .with_label_values(&[name])
            .observe(start.elapsed().as_secs_f64());
    }

    // Call on_shutdown hook
    worker.on_shutdown(ctx.clone()).await;

    info!("Worker stopped");
    WORKER_STOPPED.with_label_values(&[name]).inc();
    WORKER_ACTIVE.with_label_values(&[name]).set(0);
}

/// Execution loop for fallible workers with error handling.
///
/// This loop handles errors returned by the worker:
/// - **Transient errors**: Logged, metrics updated, worker continues
/// - **Fatal errors**: Logged, worker stops after calling `on_shutdown()`
async fn run_fallible_worker<S: Clone + Send + Sync + 'static, W: crate::FallibleWorker<S>>(
    mut worker: W,
    ctx: WorkerContext<S>,
    paused: Arc<AtomicBool>,
    pause_notify: Arc<Notify>,
    pause_mode: crate::PauseMode,
    mut driver: TriggerDriverEnum,
    name: &'static str,
) {
    let span = tracing::info_span!("worker", name);
    let _guard = span.enter();

    info!("Worker starting");
    WORKER_STARTED.with_label_values(&[name]).inc();
    WORKER_ACTIVE.with_label_values(&[name]).set(1);

    // Call on_start hook with error handling
    if let Err(e) = worker.on_start(ctx.clone()).await {
        error!(error = %e, "Worker on_start failed");
        WORKER_START_ERRORS.with_label_values(&[name]).inc();
        WORKER_ERRORS.with_label_values(&[name]).inc();

        if e.is_fatal() {
            error!("Fatal error in on_start, worker will not run");
            WORKER_STOPPED.with_label_values(&[name]).inc();
            WORKER_ACTIVE.with_label_values(&[name]).set(0);
            return;
        }
    }

    // Main execution loop
    'main: while driver.wait_next(&ctx).await {
        // Handle pause
        if paused.load(Ordering::Acquire) {
            match pause_mode {
                crate::PauseMode::Soft => {
                    // Soft pause: skip this execution, driver continues
                    continue;
                }
                crate::PauseMode::Hard => {
                    // Hard pause: wait for resume signal
                    loop {
                        tokio::select! {
                            () = pause_notify.notified() => {
                                if !paused.load(Ordering::Acquire) {
                                    break; // Resumed
                                }
                            }
                            () = ctx.cancelled() => {
                                // Shutdown requested while paused
                                if let Err(e) = worker.on_shutdown(ctx.clone()).await {
                                    error!(error = %e, "Worker on_shutdown failed");
                                    WORKER_SHUTDOWN_ERRORS.with_label_values(&[name]).inc();
                                    WORKER_ERRORS.with_label_values(&[name]).inc();
                                }
                                info!("Worker stopped (cancelled while paused)");
                                WORKER_STOPPED.with_label_values(&[name]).inc();
                                WORKER_ACTIVE.with_label_values(&[name]).set(0);
                                return;
                            }
                        }
                    }
                }
            }
        }

        let start = Instant::now();
        let result = worker.work(ctx.clone()).await;

        WORKER_EXECUTIONS.with_label_values(&[name]).inc();
        WORKER_EXECUTION_DURATION_SECONDS
            .with_label_values(&[name])
            .observe(start.elapsed().as_secs_f64());

        // Handle work errors
        if let Err(e) = result {
            error!(error = %e, "Worker execution failed");
            WORKER_EXECUTION_ERRORS.with_label_values(&[name]).inc();
            WORKER_ERRORS.with_label_values(&[name]).inc();

            if e.is_fatal() {
                error!("Fatal error, stopping worker");
                break 'main;
            }
            // Transient errors: continue to next execution
        }
    }

    // Call on_shutdown hook with error handling
    if let Err(e) = worker.on_shutdown(ctx.clone()).await {
        error!(error = %e, "Worker on_shutdown failed");
        WORKER_SHUTDOWN_ERRORS.with_label_values(&[name]).inc();
        WORKER_ERRORS.with_label_values(&[name]).inc();
    }

    info!("Worker stopped");
    WORKER_STOPPED.with_label_values(&[name]).inc();
    WORKER_ACTIVE.with_label_values(&[name]).set(0);
}
