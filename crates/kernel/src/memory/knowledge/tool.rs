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

//! MemoryTool — LLM-callable tool for querying the Knowledge Layer.
//!
//! Supports three actions:
//! - `search`: semantic vector search across memory items
//! - `categories`: list all knowledge categories for the user
//! - `read_category`: read the full content of a specific category file

use std::sync::Arc;

use async_trait::async_trait;
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use yunara_store::diesel_pool::DieselSqlitePools;

use super::{categories, embedding::EmbeddingService, items};
use crate::{
    memory::{TapEntryKind, TapeService},
    tool::{ToolContext, ToolExecute},
};

/// Weight λ in the search-time re-rank `score = -distance + λ · confidence`.
///
/// Rationale (see issue 2112 spec): practical usearch cosine distances
/// cluster in `[0, 0.5]`. A confidence swing of 1.0 (fully trusted vs
/// fully decayed) shifts the score by 0.5 — enough to beat a typical
/// distance tie (~0.1) but not enough to drag in semantically
/// irrelevant matches that already got into the top-K. Items with
/// `confidence < 0.4` are still surfaced if they are the closest
/// matches, just demoted below higher-confidence neighbors.
///
/// Mechanism-tuning constant (no operator-relevant right value) — lives
/// here, not in YAML.
pub const CONFIDENCE_RANK_WEIGHT: f32 = 0.5;

/// LLM-callable tool for querying the Knowledge Layer.
#[derive(ToolDef)]
#[tool(
    name = "memory",
    description = "Search and read the user's long-term memory by keyword, category listing, or \
                   full category read.",
    tier = "deferred"
)]
pub struct MemoryTool {
    pools:         DieselSqlitePools,
    embedding_svc: Arc<EmbeddingService>,
    /// Tape handle used to persist the per-turn retrieval trace
    /// (`TapEntryKind::ContextSources`). Issue #2113.
    tape_service:  Arc<TapeService>,
}

impl MemoryTool {
    /// Create a new `MemoryTool` with the given pool bundle, embedding
    /// service, and tape handle.
    pub fn new(
        pools: DieselSqlitePools,
        embedding_svc: Arc<EmbeddingService>,
        tape_service: Arc<TapeService>,
    ) -> Self {
        Self {
            pools,
            embedding_svc,
            tape_service,
        }
    }
}

/// Parameters for the `memory` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryParams {
    /// The memory operation to perform: "search", "categories", or
    /// "read_category".
    action:   String,
    /// Search query (required for "search" action).
    query:    Option<String>,
    /// Category name (required for "read_category" action).
    category: Option<String>,
}

#[async_trait]
impl ToolExecute for MemoryTool {
    type Output = Value;
    type Params = MemoryParams;

    async fn run(&self, p: MemoryParams, context: &ToolContext) -> anyhow::Result<Value> {
        let username = context.user_id.as_str();

        match p.action.as_str() {
            "search" => {
                let query = p.query.as_deref().unwrap_or("");
                if query.is_empty() {
                    return Ok(json!({"error": "query is required for search action"}));
                }
                // The ContextSources retrieval trace attaches to the
                // tape that names the calling turn — i.e. the session
                // tape (issue #2113). `SessionKey::to_string` is the
                // tape-name convention used everywhere in the kernel.
                let tape_name = context.session_key.to_string();
                self.exec_search(username, &tape_name, query).await
            }
            "categories" => self.exec_categories(username).await,
            "read_category" => {
                let category = p.category.as_deref().unwrap_or("");
                if category.is_empty() {
                    return Ok(json!({"error": "category is required for read_category action"}));
                }
                self.exec_read_category(username, category).await
            }
            _ => Ok(json!({"error": format!("unknown action: {}", p.action)})),
        }
    }
}

impl MemoryTool {
    async fn exec_search(
        &self,
        username: &str,
        tape_name: &str,
        query: &str,
    ) -> anyhow::Result<Value> {
        // Embed the query.
        let embeddings = self.embedding_svc.embed(&[query.to_string()]).await?;
        let query_emb = embeddings
            .first()
            .ok_or_else(|| anyhow::anyhow!("empty embedding response"))?;

        // Search usearch index.
        let results = self.embedding_svc.search(query_emb, 20)?;

        // Fetch matching items from SQLite.
        let ids: Vec<i64> = results.iter().map(|(key, _)| *key as i64).collect();
        let matched_items = items::get_items_by_ids(&self.pools.reader, &ids).await?;

        // Re-rank by `score = -distance + λ · confidence`. Distance comes
        // from the usearch hits, joined back to items by id; items
        // without a matching distance shouldn't occur in practice (they
        // would mean usearch returned an id `get_items_by_ids` didn't),
        // but if they do we skip them rather than score them with a
        // fabricated distance.
        let distance_by_id: std::collections::HashMap<i64, f32> = results
            .iter()
            .map(|(key, dist)| (*key as i64, *dist))
            .collect();
        let mut ranked: Vec<(super::items::MemoryItem, f32)> = matched_items
            .into_iter()
            .filter(|item| item.username == username)
            .filter_map(|item| {
                distance_by_id.get(&item.id).map(|d| {
                    let score = CONFIDENCE_RANK_WEIGHT.mul_add(item.confidence, -d);
                    (item, score)
                })
            })
            .collect();
        // Descending by score. NaN should not occur here (f32 from
        // usearch and a clamped confidence), so a partial_cmp fallback
        // to Equal is safe.
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Persist the retrieval trace before returning so a later
        // `KnowledgeService::explain` call can recover which items
        // shaped this turn (issue #2113). Weights are `1.0 - distance`
        // clamped to `[0.0, 1.0]` — distances from the usearch index
        // are joined back by id; ids missing from the distance table
        // are skipped above, so every `ranked` row has a known
        // distance. Empty result sets still emit (with empty arrays)
        // so `explain` can distinguish "search ran and found nothing"
        // from "search never ran".
        let item_ids: Vec<i64> = ranked.iter().map(|(item, _)| item.id).collect();
        let weights: Vec<f32> = ranked
            .iter()
            .map(|(item, _)| {
                distance_by_id
                    .get(&item.id)
                    .map(|d| (1.0_f32 - d).clamp(0.0, 1.0))
                    .unwrap_or(0.0)
            })
            .collect();
        self.tape_service
            .store()
            .append(
                tape_name,
                TapEntryKind::ContextSources,
                json!({ "item_ids": item_ids, "weights": weights }),
                None,
            )
            .await?;

        let items_json: Vec<Value> = ranked
            .iter()
            .map(|(item, _)| {
                json!({
                    "id": item.id,
                    "content": item.content,
                    "memory_type": item.memory_type,
                    "category": item.category,
                    "source_tape": item.source_tape,
                    "source_entry_id": item.source_entry_id,
                })
            })
            .collect();

        Ok(json!({"items": items_json}))
    }

    async fn exec_categories(&self, username: &str) -> anyhow::Result<Value> {
        let cats = categories::list_categories(username).await?;
        Ok(json!({"categories": cats}))
    }

    async fn exec_read_category(&self, username: &str, category: &str) -> anyhow::Result<Value> {
        match categories::read_category(username, category).await? {
            Some(content) => Ok(json!({"category": category, "content": content})),
            None => Ok(json!({"error": format!("category \'{category}\' not found")})),
        }
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use diesel::ExpressionMethods;
    use diesel_async::RunQueryDsl;
    use rara_model::schema::memory_items;
    use yunara_store::diesel_pool::{DieselSqlitePool, DieselSqlitePools};

    use super::*;
    use crate::{
        llm::{EmbeddingRequest, EmbeddingResponse, LlmEmbedder, LlmEmbedderRef},
        memory::FileTapeStore,
    };

    const ADD_CONFIDENCE_SQL: &str =
        "ALTER TABLE memory_items ADD COLUMN confidence REAL NOT NULL DEFAULT 1.0";

    /// Build a temp-dir-backed `TapeService` for tests.
    async fn temp_tape_service() -> Arc<TapeService> {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = FileTapeStore::new(dir.path(), dir.path())
            .await
            .expect("tape store");
        // Leak the tempdir so it outlives the test (cleaned up by OS on
        // tmp purge). The store keeps its own file handles.
        std::mem::forget(dir);
        Arc::new(TapeService::new(store))
    }

    /// Build a `MemoryTool` with the confidence column applied, a unit
    /// embedder, a usearch index loaded with the supplied items, and a
    /// fresh tape service. Returns the tool plus the tape handle and
    /// pool bundle so individual tests can inspect them.
    async fn build_tool_with_items(
        items: &[(&str, f32)],
    ) -> (MemoryTool, Arc<TapeService>, DieselSqlitePools, Vec<i64>) {
        let pools = crate::testing::build_memory_diesel_pools().await;
        {
            let mut conn = pools.writer.get().await.expect("pool conn");
            diesel::sql_query(ADD_CONFIDENCE_SQL)
                .execute(&mut *conn)
                .await
                .expect("add confidence column");
        }

        let config = crate::memory::knowledge::KnowledgeConfig::builder()
            .embedding_dimensions(1_usize)
            .search_top_k(5_usize)
            .similarity_threshold(0.0_f32)
            .build();
        let embedder: LlmEmbedderRef = Arc::new(UnitEmbedder);
        let index_path = std::env::temp_dir()
            .join(format!("rara-test-{}", uuid::Uuid::new_v4()))
            .join("memory.usearch");
        let embedding_svc = crate::memory::knowledge::EmbeddingService::with_path(
            config,
            embedder,
            "unit".to_string(),
            index_path,
        )
        .expect("embedding svc");

        let mut ids = Vec::with_capacity(items.len());
        for (content, confidence) in items {
            let id = insert_item_with_confidence(&pools.writer, content, *confidence).await;
            embedding_svc
                .add_to_index(id as u64, &[1.0])
                .expect("index item");
            ids.push(id);
        }

        let tape_service = temp_tape_service().await;
        let tool = MemoryTool::new(pools.clone(), Arc::new(embedding_svc), tape_service.clone());
        (tool, tape_service, pools, ids)
    }

    /// Embedder that hands back a deterministic single-dim vector. Tests
    /// share the same `[1.0]` query so every item ends up at distance 0
    /// — the re-rank is then forced to break the tie via confidence.
    struct UnitEmbedder;

    #[async_trait]
    impl LlmEmbedder for UnitEmbedder {
        async fn embed(
            &self,
            request: EmbeddingRequest,
        ) -> crate::error::Result<EmbeddingResponse> {
            let embeddings = request.input.iter().map(|_| vec![1.0_f32]).collect();
            Ok(EmbeddingResponse::builder()
                .embeddings(embeddings)
                .model("unit".to_string())
                .build())
        }
    }

    async fn insert_item_with_confidence(
        pool: &DieselSqlitePool,
        content: &str,
        confidence: f32,
    ) -> i64 {
        let mut conn = pool.get().await.expect("pool conn");
        let id: Option<i32> = diesel::insert_into(memory_items::table)
            .values((
                memory_items::username.eq("alice"),
                memory_items::content.eq(content),
                memory_items::memory_type.eq("preference"),
                memory_items::category.eq("ui"),
                memory_items::confidence.eq(confidence),
            ))
            .returning(memory_items::id)
            .get_result(&mut *conn)
            .await
            .expect("insert row");
        id.map(i64::from).unwrap_or(0)
    }

    #[tokio::test]
    async fn search_prefers_high_confidence_on_ties() {
        // Item A: high confidence. Item B: low confidence. Both share
        // the same embedding (`[1.0]`), so usearch returns identical
        // distances and the re-rank must use confidence to order them.
        let (tool, _tape, _pools, ids) =
            build_tool_with_items(&[("high-confidence fact", 0.95), ("low-confidence fact", 0.3)])
                .await;
        let (id_high, id_low) = (ids[0], ids[1]);

        let value = tool
            .exec_search("alice", "session-x", "any query")
            .await
            .expect("exec_search ok");

        let items = value
            .get("items")
            .and_then(|v| v.as_array())
            .expect("items array");
        assert_eq!(items.len(), 2, "both items must come back");
        let first_id = items[0].get("id").and_then(|v| v.as_i64()).expect("id");
        let second_id = items[1].get("id").and_then(|v| v.as_i64()).expect("id");
        assert_eq!(
            first_id, id_high,
            "high-confidence item must rank first when distances tie"
        );
        assert_eq!(second_id, id_low);
    }

    /// Scenario: search emits a ContextSources tape entry with item ids
    /// and weights (issue #2113).
    #[tokio::test]
    async fn search_emits_context_sources_entry() {
        let (tool, tape, _pools, ids) = build_tool_with_items(&[
            ("first fact", 0.9),
            ("second fact", 0.7),
            ("third fact", 0.5),
        ])
        .await;

        let value = tool
            .exec_search("alice", "session-x", "any query")
            .await
            .expect("exec_search ok");
        let returned_ids: Vec<i64> = value
            .get("items")
            .and_then(|v| v.as_array())
            .expect("items array")
            .iter()
            .map(|v| v.get("id").and_then(|x| x.as_i64()).expect("id"))
            .collect();
        assert_eq!(returned_ids.len(), 3, "all three items returned");

        let entries = tape.entries("session-x").await.expect("read tape");
        let cs: Vec<&_> = entries
            .iter()
            .filter(|e| e.kind == TapEntryKind::ContextSources)
            .collect();
        assert_eq!(cs.len(), 1, "exactly one ContextSources entry");
        let payload = &cs[0].payload;
        let item_ids: Vec<i64> = payload
            .get("item_ids")
            .and_then(|v| v.as_array())
            .expect("item_ids array")
            .iter()
            .map(|v| v.as_i64().expect("i64"))
            .collect();
        let weights = payload
            .get("weights")
            .and_then(|v| v.as_array())
            .expect("weights array");
        assert_eq!(
            item_ids, returned_ids,
            "item_ids preserves retrieval-rank order"
        );
        assert_eq!(weights.len(), item_ids.len(), "weights same length as ids");
        // Sanity: ids are the same set as what we inserted.
        let mut sorted_actual = item_ids.clone();
        sorted_actual.sort();
        let mut sorted_expected = ids.clone();
        sorted_expected.sort();
        assert_eq!(sorted_actual, sorted_expected);
    }

    /// Scenario: search with no matches still emits an empty
    /// ContextSources entry (issue #2113).
    #[tokio::test]
    async fn search_emits_empty_context_sources_when_no_matches() {
        // No items inserted, no embeddings in the index.
        let (tool, tape, _pools, _ids) = build_tool_with_items(&[]).await;

        tool.exec_search("alice", "session-x", "any query")
            .await
            .expect("exec_search ok");

        let entries = tape.entries("session-x").await.expect("read tape");
        let cs: Vec<&_> = entries
            .iter()
            .filter(|e| e.kind == TapEntryKind::ContextSources)
            .collect();
        assert_eq!(cs.len(), 1, "exactly one ContextSources entry");
        let item_ids = cs[0]
            .payload
            .get("item_ids")
            .and_then(|v| v.as_array())
            .expect("item_ids array");
        let weights = cs[0]
            .payload
            .get("weights")
            .and_then(|v| v.as_array())
            .expect("weights array");
        assert!(item_ids.is_empty(), "empty item_ids on no-match search");
        assert!(weights.is_empty(), "empty weights on no-match search");
    }
}
