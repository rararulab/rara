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

//! Unified memory layer: mem0 (facts) + Memos (notes) + Hindsight (4-network
//! recall).
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
//! ## Recall Strategy Engine
//!
//! The [`recall_engine`] module provides an agent-configurable rule-based
//! engine that replaces previously-hardcoded recall logic. Agents can
//! register, update, and remove recall rules at runtime via tool calls.
//!
//! Default rules replicate the original hardcoded behavior:
//!
//! 1. **user-profile** — always inject user profile facts (priority 0).
//! 2. **new-session-context** — on new/short sessions, search memory for
//!    context relevant to the user's message.
//! 3. **post-compaction** — after compaction, search using the summary.
//! 4. **session-resume** — on session resume, search for relevant context.
//!
//! ## Search Pipeline
//!
//! [`MemoryManager::search`] queries mem0 and Hindsight **in parallel**, then
//! merges the two ranked result lists using [Reciprocal Rank
//! Fusion][crate::fusion] (RRF, k=60). This produces a single ranked list where
//! items appearing in both backends are boosted.
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
pub mod kernel_impl;
pub mod manager;
pub mod mem0_client;
pub mod memos_client;
pub mod recall_engine;
pub mod tape;

pub use error::{MemoryError, MemoryResult};
pub use hindsight_client::HindsightClient;
pub use manager::{MemoryManager, MemorySource, SearchResult};
pub use mem0_client::{Mem0Client, Mem0Memory};
pub use memos_client::{MemoEntry, MemosClient};
pub use recall_engine::RecallStrategyEngine;
