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
use serde_json::{Value, json};
use sqlx::SqlitePool;

use super::{categories, embedding::EmbeddingService, items};
use crate::tool::{AgentTool, ToolContext, ToolOutput};

/// LLM-callable tool for querying the Knowledge Layer.
pub struct MemoryTool {
    pool:          SqlitePool,
    embedding_svc: Arc<EmbeddingService>,
}

impl MemoryTool {
    pub const NAME: &str = crate::tool_names::MEMORY;

    pub fn new(pool: SqlitePool, embedding_svc: Arc<EmbeddingService>) -> Self {
        Self {
            pool,
            embedding_svc,
        }
    }
}

#[async_trait]
impl AgentTool for MemoryTool {
    fn name(&self) -> &str { Self::NAME }

    fn description(&self) -> &str {
        "Search and read the user's long-term memory. Supports three actions:\n- search: semantic \
         search across memory items\n- categories: list all memory categories for the user\n- \
         read_category: read the full content of a specific category file"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["search", "categories", "read_category"],
                    "description": "The memory operation to perform"
                },
                "query": {
                    "type": "string",
                    "description": "Search query (required for 'search' action)"
                },
                "category": {
                    "type": "string",
                    "description": "Category name (required for 'read_category' action)"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: Value, context: &ToolContext) -> anyhow::Result<ToolOutput> {
        let action = params.get("action").and_then(Value::as_str).unwrap_or("");

        let username = context.user_id.as_deref().unwrap_or("default");

        match action {
            "search" => {
                let query = params.get("query").and_then(Value::as_str).unwrap_or("");
                if query.is_empty() {
                    return Ok(json!({"error": "query is required for search action"}).into());
                }
                self.exec_search(username, query).await.map(Into::into)
            }
            "categories" => self.exec_categories(username).await.map(Into::into),
            "read_category" => {
                let category = params.get("category").and_then(Value::as_str).unwrap_or("");
                if category.is_empty() {
                    return Ok(
                        json!({"error": "category is required for read_category action"}).into(),
                    );
                }
                self.exec_read_category(username, category)
                    .await
                    .map(Into::into)
            }
            _ => Ok(json!({"error": format!("unknown action: {action}")}).into()),
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
        let mut matched_items = items::get_items_by_ids(&self.pool, &ids).await?;

        // Filter by username.
        matched_items.retain(|item| item.username == username);

        let items_json: Vec<Value> = matched_items
            .iter()
            .map(|item| {
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
            None => Ok(json!({"error": format!("category '{category}' not found")})),
        }
    }
}
