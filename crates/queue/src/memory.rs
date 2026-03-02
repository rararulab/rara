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

//! Re-export of the kernel's in-memory event queue.
//!
//! This module is a convenience alias so consumers of `rara-queue` can
//! refer to the memory-only queue without importing `rara-kernel` directly.

pub use rara_kernel::event_queue::InMemoryEventQueue as MemoryQueue;
