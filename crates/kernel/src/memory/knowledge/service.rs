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

use snafu::ResultExt;
use yunara_store::diesel_pool::DieselSqlitePools;

use super::{EmbeddingService, KnowledgeConfig, outcome::OutcomeKind};
use crate::{
    error::Result,
    memory::{TapEntryKind, TapeService},
};

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
    /// Tape handle used by [`KnowledgeService::explain`] to read the
    /// `ContextSources` retrieval trace persisted by
    /// `MemoryTool::exec_search`. Issue #2113.
    pub tape_service:  Arc<TapeService>,
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

    /// Recover the memory items that shaped a particular turn's
    /// context — the read side of issue #2113's retrieval trace.
    ///
    /// Walks `tape_name` from `turn_entry_id` forward, collecting any
    /// [`TapEntryKind::ContextSources`] entries up to (but not
    /// including) the next non-tool entry — in practice zero or one
    /// per turn — and returns the live `memory_items` rows paired with
    /// the weights recorded at retrieval time, in the original
    /// retrieval-rank order. Ids whose rows have since been deleted
    /// are silently skipped, matching `commit_outcome`'s policy:
    /// `explain` is a debugging seam, not a consistency contract.
    ///
    /// Returns `Ok(vec![])` when the turn did not consult memory
    /// (no `ContextSources` entry exists in the window).
    pub async fn explain(
        &self,
        tape_name: &str,
        turn_entry_id: i64,
    ) -> Result<Vec<(super::items::MemoryItem, f32)>> {
        let entries = self
            .tape_service
            .entries(tape_name)
            .await
            .whatever_context::<_, crate::error::KernelError>("failed to read tape for explain")?;

        // Walk forward from the turn's entry id. The turn window ends
        // at the next entry that is neither a tool exchange nor a
        // ContextSources trace — i.e. the next assistant `Message` (or
        // any other non-tool kind). The starting entry itself is the
        // user Message that opened the turn, so a `Message` kind at
        // `turn_entry_id` is included rather than treated as a
        // terminator.
        let mut found: Option<&serde_json::Value> = None;
        let mut started = false;
        for entry in &entries {
            if (entry.id as i64) < turn_entry_id {
                continue;
            }
            if !started {
                started = true;
                continue;
            }
            match entry.kind {
                TapEntryKind::ContextSources => found = Some(&entry.payload),
                TapEntryKind::ToolCall | TapEntryKind::ToolResult => {}
                _ => break,
            }
        }

        let Some(payload) = found else {
            return Ok(Vec::new());
        };

        let item_ids: Vec<i64> = payload
            .get("item_ids")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(serde_json::Value::as_i64).collect())
            .unwrap_or_default();
        let weights: Vec<f32> = payload
            .get("weights")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(serde_json::Value::as_f64)
                    .map(|w| w as f32)
                    .collect()
            })
            .unwrap_or_default();
        if item_ids.is_empty() {
            return Ok(Vec::new());
        }

        // Fetch live rows; drop ids whose rows have been deleted.
        let live = super::items::get_items_by_ids(&self.pools.reader, &item_ids).await?;
        let by_id: std::collections::HashMap<i64, super::items::MemoryItem> =
            live.into_iter().map(|item| (item.id, item)).collect();
        let result: Vec<(super::items::MemoryItem, f32)> = item_ids
            .iter()
            .zip(weights.iter().copied().chain(std::iter::repeat(0.0)))
            .filter_map(|(id, w)| by_id.get(id).cloned().map(|item| (item, w)))
            .collect();
        Ok(result)
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

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use rara_model::schema::memory_items;
    use serde_json::json;
    use yunara_store::diesel_pool::DieselSqlitePool;

    use super::*;
    use crate::{
        llm::{EmbeddingRequest, EmbeddingResponse, LlmEmbedder, LlmEmbedderRef},
        memory::{FileTapeStore, TapEntryKind},
    };

    const ADD_CONFIDENCE_SQL: &str =
        "ALTER TABLE memory_items ADD COLUMN confidence REAL NOT NULL DEFAULT 1.0";

    struct NoopEmbedder;

    #[async_trait]
    impl LlmEmbedder for NoopEmbedder {
        async fn embed(
            &self,
            request: EmbeddingRequest,
        ) -> crate::error::Result<EmbeddingResponse> {
            let embeddings = request.input.iter().map(|_| vec![0.0_f32; 1]).collect();
            Ok(EmbeddingResponse::builder()
                .embeddings(embeddings)
                .model("noop".to_string())
                .build())
        }
    }

    async fn insert_item(pool: &DieselSqlitePool, content: &str) -> i64 {
        let mut conn = pool.get().await.expect("pool conn");
        let id: Option<i32> = diesel::insert_into(memory_items::table)
            .values((
                memory_items::username.eq("alice"),
                memory_items::content.eq(content),
                memory_items::memory_type.eq("preference"),
                memory_items::category.eq("ui"),
                memory_items::confidence.eq(1.0_f32),
            ))
            .returning(memory_items::id)
            .get_result(&mut *conn)
            .await
            .expect("insert row");
        id.map(i64::from).unwrap_or(0)
    }

    async fn delete_item(pool: &DieselSqlitePool, id: i64) {
        let mut conn = pool.get().await.expect("pool conn");
        diesel::delete(memory_items::table.filter(memory_items::id.eq(id as i32)))
            .execute(&mut *conn)
            .await
            .expect("delete row");
    }

    async fn build_service() -> (KnowledgeServiceRef, tempfile::TempDir) {
        let pools = crate::testing::build_memory_diesel_pools().await;
        {
            let mut conn = pools.writer.get().await.expect("pool conn");
            diesel::sql_query(ADD_CONFIDENCE_SQL)
                .execute(&mut *conn)
                .await
                .expect("add confidence column");
        }

        let config = KnowledgeConfig::builder()
            .embedding_dimensions(1_usize)
            .search_top_k(5_usize)
            .similarity_threshold(0.0_f32)
            .build();
        let embedder: LlmEmbedderRef = Arc::new(NoopEmbedder);
        let index_path = std::env::temp_dir()
            .join(format!("rara-test-{}", uuid::Uuid::new_v4()))
            .join("memory.usearch");
        let embedding_svc = Arc::new(
            EmbeddingService::with_path(config.clone(), embedder, "noop".to_string(), index_path)
                .expect("embedding svc"),
        );

        let dir = tempfile::tempdir().expect("tempdir");
        let store = FileTapeStore::new(dir.path(), dir.path())
            .await
            .expect("tape store");
        let tape_service = Arc::new(TapeService::new(store));

        let svc = Arc::new(KnowledgeService {
            pools,
            embedding_svc,
            config,
            tape_service,
        });
        (svc, dir)
    }

    /// Scenario: explain returns the memory items recorded for a turn
    /// in retrieval order (issue #2113).
    #[tokio::test]
    async fn explain_returns_items_in_retrieval_order() {
        let (svc, _dir) = build_service().await;

        let id_a = insert_item(&svc.pools.writer, "alpha").await;
        let id_b = insert_item(&svc.pools.writer, "beta").await;
        let id_c = insert_item(&svc.pools.writer, "gamma").await;

        // Seed the tape: a Message entry as the turn opener, then a
        // ContextSources entry referencing the three items in
        // retrieval-rank order.
        let tape_name = "session-explain-order";
        let msg = svc
            .tape_service
            .append_message(tape_name, json!({"role": "user", "content": "hi"}), None)
            .await
            .expect("append message");
        svc.tape_service
            .store()
            .append(
                tape_name,
                TapEntryKind::ContextSources,
                json!({
                    "item_ids": [id_a, id_b, id_c],
                    "weights": [0.93_f32, 0.81_f32, 0.55_f32],
                }),
                None,
            )
            .await
            .expect("append cs");

        let result = svc
            .explain(tape_name, msg.id as i64)
            .await
            .expect("explain ok");
        let ids: Vec<i64> = result.iter().map(|(item, _)| item.id).collect();
        let weights: Vec<f32> = result.iter().map(|(_, w)| *w).collect();
        assert_eq!(ids, vec![id_a, id_b, id_c]);
        assert_eq!(weights.len(), 3);
        assert!((weights[0] - 0.93).abs() < 1e-4);
        assert!((weights[1] - 0.81).abs() < 1e-4);
        assert!((weights[2] - 0.55).abs() < 1e-4);
    }

    /// Scenario: explain returns an empty vec when the turn did not
    /// consult memory (issue #2113).
    #[tokio::test]
    async fn explain_returns_empty_for_turn_without_context_sources() {
        let (svc, _dir) = build_service().await;
        let tape_name = "session-empty";
        let msg = svc
            .tape_service
            .append_message(tape_name, json!({"role": "user", "content": "hi"}), None)
            .await
            .expect("append message");
        // No ContextSources entry appended.
        let result = svc
            .explain(tape_name, msg.id as i64)
            .await
            .expect("explain ok");
        assert!(result.is_empty(), "no context sources => empty vec");
    }

    /// Scenario: explain skips item ids whose memory_items rows have
    /// been deleted (issue #2113).
    #[tokio::test]
    async fn explain_skips_deleted_item_ids() {
        let (svc, _dir) = build_service().await;

        let id_a = insert_item(&svc.pools.writer, "alpha").await;
        let id_b = insert_item(&svc.pools.writer, "beta").await;
        let id_c = insert_item(&svc.pools.writer, "gamma").await;

        let tape_name = "session-deleted";
        let msg = svc
            .tape_service
            .append_message(tape_name, json!({"role": "user", "content": "hi"}), None)
            .await
            .expect("append message");
        svc.tape_service
            .store()
            .append(
                tape_name,
                TapEntryKind::ContextSources,
                json!({
                    "item_ids": [id_a, id_b, id_c],
                    "weights": [0.93_f32, 0.81_f32, 0.55_f32],
                }),
                None,
            )
            .await
            .expect("append cs");

        // Delete the middle item.
        delete_item(&svc.pools.writer, id_b).await;

        let result = svc
            .explain(tape_name, msg.id as i64)
            .await
            .expect("explain ok");
        let ids: Vec<i64> = result.iter().map(|(item, _)| item.id).collect();
        let weights: Vec<f32> = result.iter().map(|(_, w)| *w).collect();
        assert_eq!(ids, vec![id_a, id_c], "deleted id dropped, order preserved");
        assert!((weights[0] - 0.93).abs() < 1e-4);
        assert!((weights[1] - 0.55).abs() < 1e-4);
    }
}
