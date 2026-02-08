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

use crate::{context::WorkerContext, err::WorkResult};

// ============================================================================
// Fallible Worker Trait (Recommended)
// ============================================================================

/// Worker trait with error handling support.
///
/// This is the recommended trait for implementing workers. It allows returning
/// errors from lifecycle hooks which are handled by the runtime:
/// - **Transient errors**: Logged and worker continues to next execution
/// - **Fatal errors**: Logged and worker stops after calling `on_shutdown()`
///
/// # Type Parameter
///
/// The state type `S` is specified at trait level, allowing the worker to know
/// the concrete state type at compile time.
///
/// # Example
///
/// ```rust
/// use rsketch_common_worker::{FallibleWorker, WorkerContext, WorkResult, WorkError};
///
/// struct DatabaseCleanup;
///
/// #[async_trait::async_trait]
/// impl FallibleWorker<AppState> for DatabaseCleanup {
///     async fn work(&mut self, ctx: WorkerContext<AppState>) -> WorkResult {
///         let db = ctx.state().db.clone();
///
///         db.cleanup_old_records().await.map_err(|e| {
///             WorkError::transient_with_source("Database cleanup failed", e)
///         })?;
///
///         Ok(())
///     }
/// }
/// # #[derive(Clone)] struct AppState { db: std::sync::Arc<Db> }
/// # struct Db;
/// # impl Db { async fn cleanup_old_records(&self) -> Result<(), std::io::Error> { Ok(()) } }
/// ```
#[async_trait::async_trait]
pub trait FallibleWorker<S: Clone + Send + Sync + 'static>: Send + 'static {
    /// Called once when the worker starts, before the first work execution.
    ///
    /// If this returns a fatal error, the worker will not start and
    /// `on_shutdown` will not be called.
    ///
    /// Default implementation returns `Ok(())`.
    async fn on_start(&mut self, _ctx: WorkerContext<S>) -> WorkResult { Ok(()) }

    /// Core work unit, called according to the trigger schedule.
    ///
    /// - Return `Ok(())` for successful execution
    /// - Return `Err(WorkError::transient(...))` for recoverable errors
    /// - Return `Err(WorkError::fatal(...))` to stop the worker
    async fn work(&mut self, ctx: WorkerContext<S>) -> WorkResult;

    /// Called once during graceful shutdown, after the last work execution.
    ///
    /// This is always called during normal shutdown, even if `on_start` or
    /// `work` returned errors (unless `on_start` returned a fatal error).
    ///
    /// Default implementation returns `Ok(())`.
    async fn on_shutdown(&mut self, _ctx: WorkerContext<S>) -> WorkResult { Ok(()) }
}

// ============================================================================
// Legacy Worker Trait (Deprecated)
// ============================================================================

/// Core worker trait defining execution logic and lifecycle hooks.
///
/// Workers are stateful tasks that execute according to a trigger schedule.
/// The trait is deliberately simple - configuration (name, trigger, blocking)
/// is handled by the builder API, not by trait methods.
///
/// # Generic State
///
/// All methods are generic over state type `S` to allow the same Worker
/// implementation to work with different state types. The Manager determines
/// the concrete state type.
///
/// # Lifecycle
///
/// 1. `on_start` - Called once before first work execution
/// 2. `work` - Called repeatedly according to trigger schedule
/// 3. `on_shutdown` - Called once during graceful shutdown
///
/// # Example
///
/// ```rust
/// use rsketch_common_worker::{Worker, WorkerContext};
///
/// struct MyWorker {
///     counter: std::sync::Arc<std::sync::atomic::AtomicUsize>,
/// }
///
/// #[async_trait::async_trait]
/// impl Worker for MyWorker {
///     async fn on_start<S: Clone + Send + Sync>(&mut self, ctx: WorkerContext<S>) {
///         println!("Worker {} starting", ctx.name());
///     }
///
///     async fn work<S: Clone + Send + Sync>(&mut self, ctx: WorkerContext<S>) {
///         self.counter
///             .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
///
///         // Check for cancellation
///         if ctx.is_cancelled() {
///             return;
///         }
///     }
///
///     async fn on_shutdown<S: Clone + Send + Sync>(&mut self, ctx: WorkerContext<S>) {
///         println!("Worker {} shutting down", ctx.name());
///     }
/// }
/// ```
#[async_trait::async_trait]
pub trait Worker: Send + 'static {
    /// Called once when the worker starts, before the first work execution.
    ///
    /// Use this hook for initialization logic that should happen once.
    /// This method should not panic or fail - any errors should be logged.
    ///
    /// Default implementation does nothing.
    async fn on_start<S: Clone + Send + Sync>(&mut self, _ctx: WorkerContext<S>) {}

    /// Core work unit, called according to the trigger schedule.
    ///
    /// This is where your main worker logic goes. The method will be called:
    /// - Once for `Trigger::Once`
    /// - On demand for `Trigger::Notify`
    /// - Periodically for `Trigger::Interval` and `Trigger::Cron`
    /// - Combination for hybrid triggers
    ///
    /// Use `ctx.is_cancelled()` to check for shutdown requests.
    /// The work should be atomic or idempotent when possible.
    async fn work<S: Clone + Send + Sync>(&mut self, ctx: WorkerContext<S>);

    /// Called once during graceful shutdown, after the last work execution.
    ///
    /// Use this hook for cleanup logic like flushing buffers or closing
    /// connections. This method should not panic or fail - any errors
    /// should be logged.
    ///
    /// Default implementation does nothing.
    async fn on_shutdown<S: Clone + Send + Sync>(&mut self, _ctx: WorkerContext<S>) {}
}

// ============================================================================
// Bridge: Worker -> FallibleWorker
// ============================================================================

/// Wrapper that adapts a [`Worker`] to implement [`FallibleWorker`].
///
/// This allows legacy workers to be used in the new fallible worker system.
/// All operations return `Ok(())` since legacy workers don't return errors.
pub struct InfallibleWorker<W> {
    inner: W,
}

impl<W> InfallibleWorker<W> {
    /// Wraps a legacy worker to make it fallible.
    pub const fn new(worker: W) -> Self { Self { inner: worker } }

    /// Returns a reference to the inner worker.
    pub const fn inner(&self) -> &W { &self.inner }

    /// Returns a mutable reference to the inner worker.
    pub const fn inner_mut(&mut self) -> &mut W { &mut self.inner }

    /// Consumes the wrapper and returns the inner worker.
    pub fn into_inner(self) -> W { self.inner }
}

#[async_trait::async_trait]
impl<W, S> FallibleWorker<S> for InfallibleWorker<W>
where
    W: Worker,
    S: Clone + Send + Sync + 'static,
{
    async fn on_start(&mut self, ctx: WorkerContext<S>) -> WorkResult {
        self.inner.on_start(ctx).await;
        Ok(())
    }

    async fn work(&mut self, ctx: WorkerContext<S>) -> WorkResult {
        self.inner.work(ctx).await;
        Ok(())
    }

    async fn on_shutdown(&mut self, ctx: WorkerContext<S>) -> WorkResult {
        self.inner.on_shutdown(ctx).await;
        Ok(())
    }
}
