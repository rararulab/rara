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
//! Groups the single-queue and sharded queue implementations under one module.

mod in_memory;
mod observable;
pub(crate) mod shard;
mod sharded;

pub use in_memory::{KernelEvent, EventPriority, EventQueue, EventQueueRef, InMemoryEventQueue, KernelEventEnvelope};
pub use observable::{ObservableEventQueue, ObservableEventQueueRef, ObservableKernelEvent};
pub use sharded::{ShardedEventQueue, ShardedEventQueueConfig, ShardedQueueRef};
