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

//! Unique identifier for workers.

use derive_more::{Debug, Display};
use uuid::Uuid;

/// Unique identifier for a worker.
///
/// Each worker spawned by the Manager receives a unique `WorkerId` that can be
/// used to:
/// - Track the worker in the Manager's internal registry
/// - Stop a specific worker via `manager.terminate()` or `manager.remove()`
/// - Look up worker information
///
/// # Example
///
/// ```rust,no_run
/// # use rsketch_common_worker::{Handle, Manager, Worker, WorkerContext};
/// # use std::time::Duration;
/// # struct MyWorker;
/// # #[async_trait::async_trait]
/// # impl Worker for MyWorker {
/// #     async fn work<S: Clone + Send + Sync>(&mut self, ctx: WorkerContext<S>) {}
/// # }
/// # #[tokio::main]
/// # async fn main() {
/// let mut manager = Manager::new();
///
/// // spawn() returns a handle containing the unique WorkerId
/// let handle = manager
///     .worker(MyWorker)
///     .name("my-worker")
///     .interval(Duration::from_secs(5))
///     .spawn();
///
/// // Later, stop this specific worker using handle.id()
/// manager.remove(handle.id()).await;
/// # }
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Display)]
#[debug("WorkerId({_0})")]
#[display("{_0}")]
pub struct WorkerId(Uuid);

impl WorkerId {
    pub(crate) fn new() -> Self { Self(Uuid::new_v4()) }

    #[must_use]
    pub const fn as_uuid(&self) -> &Uuid { &self.0 }
}
