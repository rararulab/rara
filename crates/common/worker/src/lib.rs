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

//! Worker abstraction for task scheduling and execution.
//!
//! This crate provides a flexible worker system with:
//! - **Multiple trigger types**: Once, Notify, Interval, Cron, and hybrid
//!   triggers
//! - **Type-safe builder API**: Compile-time guarantees for trigger
//!   configuration
//! - **Shared state**: Generic state support with Clone constraint
//! - **Lifecycle hooks**: `on_start`, work, `on_shutdown`
//! - **Graceful shutdown**: Coordinated cancellation with timeout
//! - **Pause/Resume/Notify**: Runtime control via handle traits
//!
//! # Quick Start
//!
//! ```rust,no_run
//! use std::time::Duration;
//!
//! use rsketch_common_worker::{Handle, Manager, Pausable, Worker, WorkerContext};
//!
//! struct MyWorker;
//!
//! #[async_trait::async_trait]
//! impl Worker for MyWorker {
//!     async fn work<S: Clone + Send + Sync>(&mut self, ctx: WorkerContext<S>) {
//!         println!("Worker {} executed", ctx.name());
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() {
//!     let mut manager = Manager::new();
//!
//!     // Spawn an interval worker (handle contains worker id)
//!     let handle = manager
//!         .worker(MyWorker)
//!         .name("my-worker")
//!         .interval(Duration::from_secs(5))
//!         .spawn();
//!
//!     // Pause/resume control
//!     handle.pause();
//!     handle.resume();
//!
//!     // Access worker id via handle
//!     let _id = handle.id();
//!
//!     // Graceful shutdown
//!     manager.shutdown().await;
//! }
//! ```
//!
//! # Architecture
//!
//! - [`Worker`]: Trait defining work logic with lifecycle hooks
//! - [`Manager`]: Orchestrates worker lifecycle and shared state
//! - [`WorkerContext`]: Execution context with state, cancellation, and notify
//! - [`Trigger`]: Execution schedule (Once, Notify, Interval, Cron, etc.)
//! - Handle traits: [`Handle`], [`Pausable`], [`Notifiable`] for runtime
//!   control

mod blocking;
mod builder;
mod context;
mod driver;
mod err;
mod handle;
mod id;
mod manager;
mod metrics;
mod trigger;
mod worker;

// Public API
pub use blocking::BlockingWorker;
pub use context::WorkerContext;
pub use err::{CronParseError, ErrorSeverity, WorkError, WorkResult};
pub use handle::{
    CronHandle, CronOrNotifyHandle, Handle, IntervalHandle, IntervalOrNotifyHandle, Notifiable,
    NotifyHandle, OnceHandle, Pausable,
};
pub use id::WorkerId;
pub use manager::{Manager, ManagerConfig};
pub use trigger::{PauseMode, Trigger};
pub use worker::{FallibleWorker, InfallibleWorker, Worker};
