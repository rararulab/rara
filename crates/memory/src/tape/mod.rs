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
//! The tape subsystem is intentionally isolated from the rest of the memory
//! crate. It provides:
//! - a persistent append-only JSONL store,
//! - a higher-level service API for anchor-aware workflows,
//! - and a small compatibility wrapper (`TapMemory`) for one-tape callers.
//!
//! The public API is asynchronous so callers can compose tape operations inside
//! async workflows without introducing synchronous adapter layers later.

mod anchors;
mod context;
mod error;
mod service;
mod store;

pub use anchors::AnchorSummary;
pub use context::default_tape_context;
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
}

/// Compatibility facade for callers that only need one bound tape.
#[derive(Debug, Clone)]
pub struct TapMemory {
    store:     FileTapeStore,
    tape_name: String,
}

impl TapMemory {
    /// Create a helper bound to a single tape name.
    pub async fn new(
        home: &std::path::Path,
        workspace_path: &std::path::Path,
        tape_name: &str,
    ) -> TapResult<Self> {
        Ok(Self {
            store:     FileTapeStore::new(home, workspace_path).await?,
            tape_name: tape_name.to_owned(),
        })
    }

    /// Append one entry to the bound tape.
    pub async fn append(&self, kind: TapEntryKind, payload: Value) -> TapResult<TapEntry> {
        self.store.append(&self.tape_name, kind, payload).await
    }

    /// Append a named anchor entry to the bound tape.
    pub async fn anchor(&self, name: &str, state: Value) -> TapResult<TapEntry> {
        self.store
            .append(
                &self.tape_name,
                TapEntryKind::Anchor,
                serde_json::json!({"name": name, "state": state}),
            )
            .await
    }

    /// Read all entries currently stored for the bound tape.
    pub async fn entries(&self) -> TapResult<Vec<TapEntry>> {
        Ok(self.store.read(&self.tape_name).await?.unwrap_or_default())
    }

    /// Read the most recent `limit` anchors for the bound tape.
    pub async fn anchors(&self, limit: usize) -> TapResult<Vec<AnchorSummary>> {
        TapeService::new(self.tape_name.clone(), self.store.clone())
            .anchors(limit)
            .await
    }

    /// Read all entries after the most recent anchor matching `name`.
    pub async fn entries_after_anchor(&self, name: &str) -> TapResult<Vec<TapEntry>> {
        TapeService::new(self.tape_name.clone(), self.store.clone())
            .after_anchor(name, None)
            .await
    }

    /// Search message entries in the bound tape.
    pub async fn search(&self, query: &str, limit: usize) -> TapResult<Vec<TapEntry>> {
        TapeService::new(self.tape_name.clone(), self.store.clone())
            .search(query, limit, false)
            .await
    }

    /// Remove all entries from the bound tape.
    pub async fn reset(&self) -> TapResult<()> { self.store.reset(&self.tape_name).await }
}
