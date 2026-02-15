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

//! Layer 2 service tools for memory retrieval.

use std::sync::Arc;

use async_trait::async_trait;
use rara_agents::tool_registry::AgentTool;
use serde_json::json;

use rara_memory::MemoryManager;

/// Search local memory index (keyword/hybrid depending on runtime settings).
pub struct MemorySearchTool {
    manager: Arc<MemoryManager>,
}

impl MemorySearchTool {
    /// Create a `memory_search` tool.
    pub fn new(manager: Arc<MemoryManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl AgentTool for MemorySearchTool {
    fn name(&self) -> &str {
        "memory_search"
    }

    fn description(&self) -> &str {
        "Search long-term memory documents (Markdown index). Returns relevant chunk IDs and snippets."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Keyword query for searching memory"
                },
                "limit": {
                    "type": "number",
                    "description": "Maximum number of results (default 8, max 50)"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
    ) -> rara_agents::err::Result<serde_json::Value> {
        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| rara_agents::err::Error::Other {
                message: "missing required parameter: query".into(),
            })?;

        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .map_or(8_usize, |v| v as usize)
            .clamp(1, 50);

        self.manager
            .sync()
            .await
            .map_err(|e| rara_agents::err::Error::Other {
                message: format!("memory sync failed: {e}").into(),
            })?;

        let results = self.manager.search(query, limit).await.map_err(|e| {
            rara_agents::err::Error::Other {
                message: format!("memory search failed: {e}").into(),
            }
        })?;

        Ok(json!({
            "query": query,
            "count": results.len(),
            "results": results
                .iter()
                .map(|r| json!({
                    "chunk_id": r.chunk_id,
                    "path": r.path,
                    "chunk_index": r.chunk_index,
                    "score": r.score,
                    "snippet": r.snippet,
                }))
                .collect::<Vec<_>>()
        }))
    }
}

/// Retrieve full chunk content by chunk ID.
pub struct MemoryGetTool {
    manager: Arc<MemoryManager>,
}

impl MemoryGetTool {
    /// Create a `memory_get` tool.
    pub fn new(manager: Arc<MemoryManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl AgentTool for MemoryGetTool {
    fn name(&self) -> &str {
        "memory_get"
    }

    fn description(&self) -> &str {
        "Get full memory chunk content by chunk_id from local memory index."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "chunk_id": {
                    "type": "number",
                    "description": "Chunk ID returned by memory_search"
                }
            },
            "required": ["chunk_id"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
    ) -> rara_agents::err::Result<serde_json::Value> {
        let chunk_id = params
            .get("chunk_id")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| rara_agents::err::Error::Other {
                message: "missing required parameter: chunk_id".into(),
            })?;

        match self.manager.get_chunk(chunk_id).await.map_err(|e| {
            rara_agents::err::Error::Other {
                message: format!("memory get failed: {e}").into(),
            }
        })? {
            Some(chunk) => Ok(json!({
                "chunk_id": chunk.chunk_id,
                "path": chunk.path,
                "chunk_index": chunk.chunk_index,
                "content": chunk.content,
            })),
            None => Ok(json!({
                "error": format!("chunk not found: {chunk_id}")
            })),
        }
    }
}
