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

//! Local file-backed tape memory, modeled after Bub's tape subsystem.
//!
//! # What -- The Tape Model
//!
//! A **tape** is an append-only JSONL timeline that records every conversation
//! event for a single session.  One `.jsonl` file per session, named by the
//! session key (URL-encoded).  Each line is a self-contained [`TapEntry`]:
//!
//! | Field       | Type              | Description                                      |
//! |-------------|-------------------|--------------------------------------------------|
//! | `id`        | `u64`             | Monotonic, store-assigned append-order identifier |
//! | `kind`      | [`TapEntryKind`]  | Category tag (see below)                         |
//! | `payload`   | `serde_json::Value` | Arbitrary JSON whose schema depends on `kind`  |
//! | `timestamp` | `jiff::Timestamp` | Wall-clock time captured at persistence          |
//! | `metadata`  | `Option<Value>`   | Optional free-form data (token counts, model, latency, ...) |
//!
//! Eight entry kinds cover the full lifecycle of an agent turn:
//!
//! | Kind         | Payload semantics                                    |
//! |--------------|------------------------------------------------------|
//! | `Message`    | Chat message (user or assistant), deserialized as [`llm::Message`](crate::llm::Message) |
//! | `ToolCall`   | Assistant tool invocation request (`{"calls": [...]}`) |
//! | `ToolResult` | Tool execution output (string or structured JSON)    |
//! | `Event`      | Non-chat lifecycle / telemetry (`{"name": "...", "data": {...}}`) |
//! | `System`     | System prompt or system-level content (`{"content": "..."}`) |
//! | `Anchor`     | Named checkpoint (`{"name": "...", "state": {...}}`) |
//! | `Note`       | Structured note in a user tape (`{"category": "...", "content": "..."}`) |
//! | `Summary`    | Compaction summary replacing pruned entries (`{"discarded_count": N, "preserved_kinds": [...]}`) |
//!
//! # How -- Architecture
//!
//! ## Data flow (one agent turn)
//!
//! ```text
//! User msg ──► append(Message) ──► tape.jsonl
//!                                      │
//!                              from_last_anchor()
//!                                      │
//!                              default_tape_context()
//!                                      │
//!                                Vec<llm::Message> ──► LLM
//!                                      │
//!                          append(ToolCall / ToolResult / Message)
//! ```
//!
//! ## Component responsibilities
//!
//! | File           | Type                | Role |
//! |----------------|---------------------|------|
//! | [`store`]      | [`FileTapeStore`]   | Low-level JSONL I/O.  A dedicated `rara-tape-io` worker thread receives [`Job`] closures via `mpsc`.  [`TapeFile`] keeps an in-memory entry cache plus a byte-offset cursor for incremental reads, so repeated reads only parse newly appended bytes. |
//! | [`service`]    | [`TapeService`]     | High-level async API.  Not bound to a single tape -- every method takes `tape_name`.  Provides: append helpers, anchor queries, fork/merge, ranked Unicode-aware search over message payload + metadata, LLM context building (`build_llm_context`), tape info, and reset/archive. |
//! | [`context`]    | [`default_tape_context()`] | Stateless conversion of `&[TapEntry]` into `Vec<llm::Message>`.  `Message` entries are deserialized directly.  `ToolCall` becomes an assistant message with a `tool_calls` array.  `ToolResult` becomes one or more tool-role messages.  `Event` / `System` / `Anchor` are skipped (they carry metadata, not LLM-visible content). |
//! | [`anchors`]    | [`AnchorSummary`]   | Lightweight data type pairing an anchor `name` with its captured `state`. |
//! | [`error`]      | [`TapError`]        | `snafu`-based error enum scoped to the tape subsystem (I/O, JSON encode/decode, internal state). |
//!
//! ## Key mechanisms
//!
//! ### Anchors
//!
//! Anchors are named checkpoints inserted into the tape.
//! [`TapeService::from_last_anchor()`] returns only entries whose `id` is
//! greater than or equal to the most recent anchor, giving the LLM a bounded
//! context window.  Crucially, earlier data is **not** deleted -- methods like
//! [`TapeService::search()`] can still find entries across all anchors.
//! Creating an anchor effectively says *"context before this point can be
//! trimmed from the LLM window"*.
//!
//! ### Fork / Merge
//!
//! Before each agent turn the kernel forks the tape:
//!
//! 1. `FileTapeStore::fork()` clones the file and in-memory cache into a new
//!    tape named `{parent}__{suffix}`.
//! 2. The agent loop writes all `ToolCall`, `ToolResult`, and assistant
//!    `Message` entries to the **fork**.
//! 3. On success: `FileTapeStore::merge()` copies fork-local entries back to
//!    the parent tape and deletes the fork file.
//! 4. On failure: `FileTapeStore::discard()` deletes the fork, leaving the
//!    parent tape untouched.
//!
//! This prevents failed or partial LLM turns (hallucinations, mid-tool-call
//! errors) from polluting the canonical conversation history.
//!
//! ### Search
//!
//! [`TapeService::search()`] performs ranked text retrieval over `Message`
//! entries. It normalizes Unicode text, searches both payload and metadata,
//! scores exact substring matches, multi-term overlap, and fuzzy similarity,
//! then returns the best matches first. Search can operate on a single tape or
//! across all tapes in the workspace (cross-session).
//!
//! # Why -- Design Decisions
//!
//! - **Append-only**: Simple, corruption-resistant, and trivially safe for
//!   concurrent readers.  No in-place mutations means no torn-write risk.
//!
//! - **JSONL**: Human-readable, streamable, and easy to debug with standard
//!   tools (`cat`, `grep`, `jq`).  Each line is independently parseable, so a
//!   single corrupt line does not invalidate the rest of the file.
//!
//! - **Anchor-based context truncation**: Avoids unbounded context growth
//!   without losing historical data.  The LLM sees a bounded window; `search()`
//!   retrieves anything ever recorded.
//!
//! - **Fork / merge**: LLM responses can fail, hallucinate, or error
//!   mid-tool-call.  Forking ensures these partial writes never become
//!   permanent -- the parent tape only absorbs entries from a successful turn.
//!
//! - **Dedicated I/O thread** (`rara-tape-io`): Keeps the async Tokio runtime
//!   free from blocking file-system calls.  All reads and writes funnel through
//!   one thread, eliminating lock contention on the file cache.
//!
//! - **In-memory cache (`TapeFile`)**: After initial load, reads are pure
//!   memory lookups.  Incremental file reads only parse bytes appended since
//!   the last read, keeping per-turn I/O proportional to new entries rather
//!   than total tape size.

mod anchors;
mod context;
mod error;
pub mod knowledge;
mod service;
mod store;

pub use anchors::{AnchorSummary, HandoffState};
pub use context::{anchor_context, default_tape_context, user_tape_context};
pub use error::{TapError, TapResult};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub use service::{TapeInfo, TapeService, current_tape};
pub use store::FileTapeStore;

pub(crate) const TAPE_FILE_SUFFIX: &str = ".jsonl";

/// Kinds of persisted tape entries.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    strum::AsRefStr,
    strum::Display,
    strum::EnumString,
    derive_more::IsVariant,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum TapEntryKind {
    /// Raw chat message payload.
    Message,
    /// Assistant tool invocation payload.
    ToolCall,
    /// Tool execution output payload.
    ToolResult,
    /// Non-chat lifecycle or telemetry event.
    Event,
    /// System prompt or system-level content.
    System,
    /// Named checkpoint for relative tape queries.
    Anchor,
    /// Structured note persisted in a user tape (preferences, facts, TODOs).
    Note,
    /// Compaction summary replacing older entries that were pruned.
    Summary,
}

/// Canonical tape name prefix for per-user tapes.
const USER_TAPE_PREFIX: &str = "user:";

/// Derive the canonical user tape name from a user identifier.
///
/// User tapes are stored alongside session tapes in the same
/// [`FileTapeStore`] but keyed by `"user:<user_id>"` instead of a session
/// key.  This keeps storage layout flat and reuses the existing JSONL
/// infrastructure.
pub fn user_tape_name(user_id: &str) -> String { format!("{USER_TAPE_PREFIX}{user_id}") }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_tape_name_formats_correctly() {
        assert_eq!(user_tape_name("alice"), "user:alice");
        assert_eq!(user_tape_name("bob123"), "user:bob123");
    }

    #[test]
    fn user_tape_name_empty_user() {
        assert_eq!(user_tape_name(""), "user:");
    }
}

/// One append-only entry in a tape.
///
/// Entries are immutable once persisted. The store assigns strictly increasing
/// numeric IDs so relative queries such as "after anchor X" can operate on
/// integer ordering rather than reparsing content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TapEntry {
    /// Monotonic, append-order identifier assigned by the store.
    pub id:        u64,
    /// Entry category used by higher-level query helpers.
    pub kind:      TapEntryKind,
    /// Arbitrary JSON payload for the stored event.
    pub payload:   Value,
    /// Timestamp captured when the entry was persisted.
    pub timestamp: Timestamp,
    /// Optional free-form metadata (token counts, source channel, model,
    /// latency, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata:  Option<Value>,
}
