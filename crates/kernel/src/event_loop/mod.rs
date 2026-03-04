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

//! Unified event loop — parallel multi-processor loop that processes all
//! [`KernelEvent`](crate::event::KernelEvent) variants.
//!
//! Always backed by a
//! [`ShardedEventQueue`](crate::queue::ShardedEventQueue). When
//! `num_shards == 0` (single-queue mode), only a global
//! [`EventProcessor`](processor::EventProcessor) is spawned. When
//! `num_shards > 0`, N additional shard processors run in parallel for
//! agent-scoped events. The kernel directly manages process state
//! (conversation, turn cancellation, pause buffer) instead of delegating
//! to per-process tokio tasks.

mod lifecycle;
mod message;
pub(crate) mod processor;
pub(crate) mod runtime;
mod turn;

use std::sync::Arc;

pub(crate) use runtime::RuntimeTable;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::{
    event::{KernelEvent, KernelEventEnvelope},
    kernel::Kernel,
};

impl Kernel {}
