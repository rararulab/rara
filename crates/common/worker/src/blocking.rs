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

use crate::context::WorkerContext;

/// Trait for synchronous blocking workers with state type at trait level.
///
/// Unlike the async `Worker` trait, this is for CPU-intensive or synchronous
/// blocking operations. Workers implementing this trait run on Tokio's blocking
/// thread pool via `spawn_blocking`.
///
/// The state type `S` is a trait-level generic, providing better type safety
/// and allowing the worker to be stateful with a specific state type.
///
/// # Example
///
/// ```rust
/// use rsketch_common_worker::{BlockingWorker, WorkerContext};
///
/// struct HeavyComputeWorker {
///     batch_size: usize,
/// }
///
/// impl BlockingWorker<()> for HeavyComputeWorker {
///     fn work(&mut self, ctx: WorkerContext<()>) {
///         // CPU-intensive work that would block async runtime
///         for i in 0..self.batch_size {
///             // Heavy computation...
///         }
///     }
/// }
/// ```
pub trait BlockingWorker<S: Clone + Send + Sync + 'static>: Send + 'static {
    fn on_start(&mut self, _ctx: WorkerContext<S>) {}

    fn work(&mut self, ctx: WorkerContext<S>);

    fn on_shutdown(&mut self, _ctx: WorkerContext<S>) {}
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::*;

    struct TestBlockingWorker {
        counter: Arc<AtomicUsize>,
    }

    impl BlockingWorker<()> for TestBlockingWorker {
        fn on_start(&mut self, _ctx: WorkerContext<()>) { self.counter.store(1, Ordering::SeqCst); }

        fn work(&mut self, _ctx: WorkerContext<()>) { self.counter.fetch_add(1, Ordering::SeqCst); }

        fn on_shutdown(&mut self, _ctx: WorkerContext<()>) {
            self.counter.store(999, Ordering::SeqCst);
        }
    }

    #[test]
    fn test_blocking_worker_trait_compiles() {
        let counter = Arc::new(AtomicUsize::new(0));
        let worker = TestBlockingWorker {
            counter: Arc::clone(&counter),
        };

        fn assert_send<T: Send>(_: &T) {}
        assert_send(&worker);
    }
}
