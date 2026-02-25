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

//! Unified memory layer: mem0 (facts) + Memos (notes) + Hindsight (4-network recall).
//!
//! # Architecture
//!
//! Three external services provide a layered memory system, each addressing a
//! different aspect of long-term agent memory:
//!
//! | Service       | Layer     | Role                                       | API     |
//! |---------------|-----------|--------------------------------------------|---------|
//! | **mem0**      | State     | Structured fact extraction & auto-dedup     | REST v1 |
//! | **Memos**     | Storage   | Human-readable Markdown notes & daily logs  | REST v1 |
//! | **Hindsight** | Learning  | 4-network retain / recall / reflect         | REST v1 |
//!
//! ## Data Flow
//!
//! ```text
//!                        ┌──────────────────┐
//!                        │  MemoryManager    │   ← unified facade
//!                        └──┬─────┬──────┬──┘
//!              search/facts │     │notes │ retain/recall/reflect
//!                    ┌──────┘     │      └──────┐
//!                    ▼            ▼              ▼
//!               ┌────────┐  ┌────────┐   ┌────────────┐
//!               │  mem0   │  │ Memos  │   │ Hindsight  │
//!               └────────┘  └────────┘   └────────────┘
//! ```
//!
//! ## Search Pipeline
//!
//! [`MemoryManager::search`] queries mem0 and Hindsight **in parallel**, then
//! merges the two ranked result lists using [Reciprocal Rank Fusion][crate::fusion]
//! (RRF, k=60). This produces a single ranked list where items appearing in
//! both backends are boosted.
//!
//! ## Post-Conversation Reflection
//!
//! [`MemoryManager::reflect_on_exchange`] fans out to all three backends after
//! each conversation turn:
//!
//! 1. **mem0** — extracts and deduplicates structured facts from the exchange.
//! 2. **Hindsight** — retains the exchange across the 4-network model.
//! 3. **Memos** — appends an exchange log entry (human-readable Markdown).
//!
//! Partial failures are logged as warnings but do not fail the operation.
//!
//! ## Configuration
//!
//! The three backend URLs and credentials are loaded from `AppConfig::memory`
//! in the `rara-app` crate (Consul KV / env vars):
//!
//! - `mem0_base_url` — e.g. `http://mem0:8080`
//! - `memos_base_url` — e.g. `http://memos:5230`
//! - `memos_token` — Bearer token for Memos authentication
//! - `hindsight_base_url` — e.g. `http://hindsight:8888`
//! - `hindsight_bank_id` — Hindsight memory bank identifier

pub mod error;
pub mod fusion;
pub mod hindsight_client;
pub mod manager;
pub mod mem0_client;
pub mod memos_client;

pub use error::{MemoryError, MemoryResult};
pub use hindsight_client::HindsightClient;
pub use manager::{MemoryManager, MemorySource, SearchResult};
pub use mem0_client::{Mem0Client, Mem0Memory};
pub use memos_client::{MemosClient, MemoEntry};
