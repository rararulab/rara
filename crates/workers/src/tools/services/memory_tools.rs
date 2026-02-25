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

//! Layer 2 service tools for memory retrieval and writing.

use std::sync::Arc;

use async_trait::async_trait;
use rara_memory::MemoryManager;
use serde_json::json;
use tool_core::AgentTool;

/// Search unified memory layer (mem0 + Hindsight, fused with RRF).
pub struct MemorySearchTool {
    manager: Arc<MemoryManager>,
}

impl MemorySearchTool {
    /// Create a `memory_search` tool.
    pub fn new(manager: Arc<MemoryManager>) -> Self { Self { manager } }
}

#[async_trait]
impl AgentTool for MemorySearchTool {
    fn name(&self) -> &str { "memory_search" }

    fn description(&self) -> &str {
        "Search long-term memory across mem0 and Hindsight. Returns relevant memories with source \
         and content."
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

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: query"))?;

        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .map_or(8_usize, |v| v as usize)
            .clamp(1, 50);

        let results = self
            .manager
            .search(query, limit)
            .await
            .map_err(|e| anyhow::anyhow!("memory search failed: {e}"))?;

        Ok(json!({
            "query": query,
            "count": results.len(),
            "results": results
                .iter()
                .map(|r| json!({
                    "id": r.id,
                    "source": format!("{:?}", r.source),
                    "content": r.content,
                    "score": r.score,
                }))
                .collect::<Vec<_>>()
        }))
    }
}

/// Deep recall from Hindsight memory network.
pub struct MemoryDeepRecallTool {
    manager: Arc<MemoryManager>,
}

impl MemoryDeepRecallTool {
    /// Create a `memory_deep_recall` tool.
    pub fn new(manager: Arc<MemoryManager>) -> Self { Self { manager } }
}

#[async_trait]
impl AgentTool for MemoryDeepRecallTool {
    fn name(&self) -> &str { "memory_deep_recall" }

    fn description(&self) -> &str {
        "Deep recall from Hindsight memory network. Triggers deep reasoning over the memory bank \
         for a given query."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Query for deep recall reasoning"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: query"))?;

        let result = self
            .manager
            .deep_recall(query)
            .await
            .map_err(|e| anyhow::anyhow!("memory deep recall failed: {e}"))?;

        Ok(json!({
            "query": query,
            "result": result,
        }))
    }
}

/// Add a structured fact about the user to long-term memory (mem0).
pub struct MemoryAddFactTool {
    manager: Arc<MemoryManager>,
}

impl MemoryAddFactTool {
    /// Create a `memory_add_fact` tool.
    pub fn new(manager: Arc<MemoryManager>) -> Self { Self { manager } }
}

#[async_trait]
impl AgentTool for MemoryAddFactTool {
    fn name(&self) -> &str { "memory_add_fact" }

    fn description(&self) -> &str {
        "Add a structured fact about the user to long-term memory (mem0). Facts are \
         auto-deduplicated."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The fact or information to store about the user"
                }
            },
            "required": ["content"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: content"))?;

        self.manager
            .add_fact(content)
            .await
            .map_err(|e| anyhow::anyhow!("memory add fact failed: {e}"))?;

        Ok(json!({
            "status": "ok",
            "message": "Fact stored in long-term memory",
        }))
    }
}

/// Write a note to Memos (persistent Markdown note storage).
pub struct MemoryWriteTool {
    manager: Arc<MemoryManager>,
}

impl MemoryWriteTool {
    /// Create a `memory_write` tool.
    pub fn new(manager: Arc<MemoryManager>) -> Self { Self { manager } }
}

#[async_trait]
impl AgentTool for MemoryWriteTool {
    fn name(&self) -> &str { "memory_write" }

    fn description(&self) -> &str {
        "Write a Markdown note to Memos for long-term storage. Notes are searchable via \
         memory_search."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "Markdown content to write as a note"
                },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional tags for the note (e.g. ['meeting', 'project-x'])"
                }
            },
            "required": ["content"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: content"))?;

        let tags: Vec<&str> = params
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let name = self
            .manager
            .write_note(content, &tags)
            .await
            .map_err(|e| anyhow::anyhow!("memory write failed: {e}"))?;

        Ok(json!({
            "status": "ok",
            "name": name,
        }))
    }
}
