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

//! Queue subsystem for kernel event dispatch.
//!
//! The [`EventQueue`] trait is a push-only sink. The drain/wait hot path
//! goes through `ShardQueue` directly.

mod sharded;

use std::sync::Arc;

pub(crate) use sharded::ShardQueue;
pub use sharded::{ShardedEventQueue, ShardedEventQueueConfig, ShardedQueueRef};

pub use crate::event::{EventPriority, KernelEvent, KernelEventEnvelope};
use crate::io::IOError;

/// Shared reference to an [`EventQueue`] implementation.
pub type EventQueueRef = Arc<dyn EventQueue>;

/// Push-only event sink for the kernel event queue.
///
/// The drain/wait hot path is handled by `ShardQueue` directly —
/// this trait only covers the ingress (push) side.
pub trait EventQueue: Send + Sync + 'static {
    /// Push an event into the queue. Returns `IOError::Full` if at capacity.
    fn push(&self, event: KernelEventEnvelope) -> Result<(), IOError>;

    /// Non-blocking push (identical to `push` for in-memory queues).
    fn try_push(&self, event: KernelEventEnvelope) -> Result<(), IOError>;
}
