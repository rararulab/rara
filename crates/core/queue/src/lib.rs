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

//! `rara-queue` — pluggable event queue implementations for the kernel.
//!
//! Provides three [`EventQueue`](rara_kernel::event_queue::EventQueue)
//! implementations:
//!
//! - [`MemoryQueue`](memory::MemoryQueue) — re-exports
//!   `InMemoryEventQueue` from the kernel for convenience.
//! - [`WalQueue`](wal::WalQueue) — file-system WAL (Write-Ahead Log)
//!   with JSON-lines format for crash recovery.
//! - [`HybridQueue`](hybrid::HybridQueue) — combines an in-memory
//!   fast path with WAL persistence for durability.

pub mod hybrid;
pub mod memory;
pub mod wal;
