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
//! ## Recall Strategies
//!
//! The orchestrator uses [`MemoryManager::search`] in three recall contexts:
//!
//! 1. **First-turn / short-session pre-fetch** — when `history_len < 3`, the
//!    user's message text is used as a search query to inject relevant context
//!    into the system prompt.
//! 2. **Per-turn recall** (opt-in via `recall_every_turn` setting) — same as
//!    above but runs on every turn regardless of session length.
//! 3. **Post-compaction recall** — after context compaction compresses history
//!    into a summary, the summary text is used as a search query to recover
//!    details that may have been lost during compaction.
//!
//! ## Search Pipeline
//!
//! [`MemoryManager::search`] queries mem0 and Hindsight **in parallel**, then
//! merges the two ranked result lists using [Reciprocal Rank Fusion][crate::fusion]
//! (RRF, k=60). This produces a single ranked list where items appearing in
//! both backends are boosted.
//!
//! ## Session Consolidation
//!
//! [`MemoryManager::consolidate_session`] batches all exchanges from a
//! completed session into long-term memory at session boundaries (inactivity
//! threshold exceeded). It touches **only** mem0 and Hindsight — Memos is
//! reserved for explicit `memory_write` tool calls.
//!
//! ## Explicit Fact Storage
//!
//! [`MemoryManager::add_fact`] stores a single fact into mem0 and Hindsight
//! immediately, used by the `memory_add_fact` tool.
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

#[cfg(feature = "k8s")]
pub mod pod_manager;
#[cfg(feature = "k8s")]
pub mod lazy_client;

pub use error::{MemoryError, MemoryResult};
pub use hindsight_client::HindsightClient;
pub use manager::{MemoryManager, MemorySource, SearchResult};
pub use mem0_client::{Mem0Client, Mem0Memory};
pub use memos_client::{MemosClient, MemoEntry};

#[cfg(feature = "k8s")]
pub use pod_manager::Mem0PodManager;
#[cfg(feature = "k8s")]
pub use lazy_client::LazyMem0Client;
