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

use std::sync::Arc;

use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

/// Execution context passed to workers, providing access to state and control
/// signals.
///
/// This context is cloned for each work execution. Cloning is cheap because
/// [`CancellationToken`] and [`Notify`] use `Arc` internally, and only the
/// state `S` is actually cloned.
///
/// # State Management
///
/// The state `S` must implement `Clone`. For expensive-to-clone types, wrap
/// them in `Arc<T>` before passing to `Manager::with_state()`.
///
/// # Example
///
/// ```rust
/// use std::sync::Arc;
///
/// use rsketch_common_worker::WorkerContext;
///
/// #[derive(Clone)]
/// struct AppState {
///     db:     Arc<Database>, // Expensive type wrapped in Arc
///     config: String,        // Cheap to clone
/// }
///
/// async fn work_fn<S: Clone + Send + Sync>(ctx: WorkerContext<S>) {
///     // Check cancellation
///     if ctx.is_cancelled() {
///         return;
///     }
///
///     // Wait for cancellation
///     tokio::select! {
///         _ = ctx.cancelled() => println!("Cancelled"),
///         _ = do_work() => {},
///     }
/// }
/// # async fn do_work() {}
/// # struct Database;
/// ```
pub struct WorkerContext<S = ()> {
    state:             S,
    cancel_token:      CancellationToken,
    pub(crate) notify: Arc<Notify>,
    worker_name:       &'static str,
}

impl<S: Clone> Clone for WorkerContext<S> {
    fn clone(&self) -> Self {
        Self {
            state:        self.state.clone(),
            cancel_token: self.cancel_token.clone(),
            notify:       self.notify.clone(),
            worker_name:  self.worker_name,
        }
    }
}

impl<S> WorkerContext<S> {
    pub(crate) const fn new(
        state: S,
        cancel_token: CancellationToken,
        notify: Arc<Notify>,
        worker_name: &'static str,
    ) -> Self {
        Self {
            state,
            cancel_token,
            notify,
            worker_name,
        }
    }

    /// Returns a reference to the shared state.
    ///
    /// The state is cloned for each worker execution from the Manager's state.
    pub const fn state(&self) -> &S { &self.state }

    /// Checks if cancellation has been requested.
    ///
    /// Returns `true` immediately if shutdown is in progress.
    /// Use this for non-blocking cancellation checks.
    pub fn is_cancelled(&self) -> bool { self.cancel_token.is_cancelled() }

    /// Waits asynchronously until cancellation is requested.
    ///
    /// Use this in `tokio::select!` to make your work cancellable:
    ///
    /// ```rust,no_run
    /// # use rsketch_common_worker::WorkerContext;
    /// # async fn example(ctx: WorkerContext<()>) {
    /// tokio::select! {
    ///     _ = ctx.cancelled() => println!("Shutdown requested"),
    ///     _ = async_work() => println!("Work completed"),
    /// }
    /// # }
    /// # async fn async_work() {}
    /// ```
    pub async fn cancelled(&self) { self.cancel_token.cancelled().await; }

    /// Waits asynchronously for a notify signal.
    ///
    /// Only relevant for workers with `Trigger::Notify` or hybrid triggers.
    /// The handle's `notify()` method will wake this future.
    pub async fn notified(&self) { self.notify.notified().await; }

    /// Creates a child cancellation token for spawning sub-tasks.
    ///
    /// When the parent token is cancelled (during shutdown), all child tokens
    /// are also cancelled. Use this to propagate cancellation to spawned tasks.
    pub fn child_token(&self) -> CancellationToken { self.cancel_token.child_token() }

    /// Returns the worker's name for logging and debugging.
    ///
    /// The name is set via `WorkerBuilder::name()` or defaults to
    /// "unnamed-worker".
    pub const fn name(&self) -> &'static str { self.worker_name }
}
