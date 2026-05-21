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
use crate::tool::{ToolContext, ToolExecute};

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
}

impl MemoryTool {
    /// Create a new `MemoryTool` with the given pool bundle and embedding
    /// service.
    pub fn new(pools: DieselSqlitePools, embedding_svc: Arc<EmbeddingService>) -> Self {
        Self {
            pools,
            embedding_svc,
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
                self.exec_search(username, query).await
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
    async fn exec_search(&self, username: &str, query: &str) -> anyhow::Result<Value> {
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
    use yunara_store::diesel_pool::DieselSqlitePool;

    use super::*;
    use crate::llm::{EmbeddingRequest, EmbeddingResponse, LlmEmbedder, LlmEmbedderRef};

    const ADD_CONFIDENCE_SQL: &str =
        "ALTER TABLE memory_items ADD COLUMN confidence REAL NOT NULL DEFAULT 1.0";

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
        // Pool with the confidence column applied.
        let pools = crate::testing::build_memory_diesel_pools().await;
        {
            let mut conn = pools.writer.get().await.expect("pool conn");
            diesel::sql_query(ADD_CONFIDENCE_SQL)
                .execute(&mut *conn)
                .await
                .expect("add confidence column");
        }

        // Item A: high confidence. Item B: low confidence. Both share
        // the same embedding (`[1.0]`), so usearch returns identical
        // distances and the re-rank must use confidence to order them.
        let id_high =
            insert_item_with_confidence(&pools.writer, "high-confidence fact", 0.95).await;
        let id_low = insert_item_with_confidence(&pools.writer, "low-confidence fact", 0.3).await;

        // EmbeddingService over a unit vector and a temp index path.
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
        embedding_svc
            .add_to_index(id_high as u64, &[1.0])
            .expect("index A");
        embedding_svc
            .add_to_index(id_low as u64, &[1.0])
            .expect("index B");

        let tool = MemoryTool::new(pools, Arc::new(embedding_svc));
        let value = tool
            .exec_search("alice", "any query")
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
}
