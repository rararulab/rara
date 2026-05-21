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

//! Knowledge service — bundles all knowledge layer dependencies.

use std::sync::Arc;

use yunara_store::diesel_pool::DieselSqlitePools;

use super::{EmbeddingService, KnowledgeConfig, outcome::OutcomeKind};
use crate::error::Result;

/// Bundles the knowledge layer's runtime dependencies into a single handle
/// that can be shared across the kernel.
///
/// The extractor agent's `{driver, model}` pair is **not** stored here — it
/// is resolved per-call via
/// [`DriverRegistry::resolve_agent`](crate::llm::DriverRegistry::resolve_agent)
/// keyed by the `knowledge_extractor` manifest, so a single atomic snapshot
/// reaches `extract_knowledge`. See #1636 / #1638.
pub struct KnowledgeService {
    pub pools:         DieselSqlitePools,
    pub embedding_svc: Arc<EmbeddingService>,
    pub config:        KnowledgeConfig,
}

impl KnowledgeService {
    /// Record one outcome row per `item_id` and update each item's
    /// `confidence` via the EMA defined in
    /// [`super::outcome`].
    ///
    /// Both halves of the pair land in a single writer transaction. An
    /// empty `item_ids` slice is a no-op; unknown ids are silently
    /// skipped (the call still returns Ok). See the issue 2112 spec
    /// for the rationale on the skip semantics — memory feedback is a
    /// debugging seam, not a consistency contract.
    ///
    /// `tape_entry_id` is the optional pointer into the tape that
    /// triggered this outcome; populating it from real LLM turns is
    /// issue 2113's job.
    pub async fn commit_outcome(
        &self,
        item_ids: &[i64],
        kind: OutcomeKind,
        tape_entry_id: Option<i64>,
    ) -> Result<()> {
        super::outcome::commit_outcome_inner(&self.pools.writer, item_ids, kind, tape_entry_id)
            .await
    }

    /// Resolve source tape entries for memory items that have source
    /// references.
    ///
    /// Groups lookups by `source_tape` to minimise tape reads, then fetches
    /// the referenced entries via `TapeService::entries_by_ids`.
    pub async fn resolve_sources(
        tape_service: &crate::memory::TapeService,
        items: &[super::items::MemoryItem],
    ) -> Vec<crate::memory::TapEntry> {
        let mut by_tape: std::collections::HashMap<String, Vec<u64>> =
            std::collections::HashMap::new();
        for item in items {
            if let (Some(tape), Some(entry_id)) = (&item.source_tape, item.source_entry_id) {
                by_tape
                    .entry(tape.clone())
                    .or_default()
                    .push(entry_id as u64);
            }
        }
        let mut results = Vec::new();
        for (tape_name, ids) in &by_tape {
            if let Ok(entries) = tape_service.entries_by_ids(tape_name, ids).await {
                results.extend(entries);
            }
        }
        results
    }
}

/// Shared reference to the knowledge service.
pub type KnowledgeServiceRef = Arc<KnowledgeService>;
