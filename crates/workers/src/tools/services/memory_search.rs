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

//! Layer 2 service tool: search the agent's local knowledge base.

use std::sync::Arc;

use async_trait::async_trait;
use rara_agents::tool_registry::AgentTool;
use serde_json::json;

/// Search the agent's local memory knowledge base using keywords.
pub struct MemorySearchTool {
    manager: Arc<rara_memory::manager::MemoryManager>,
}

impl MemorySearchTool {
    pub fn new(manager: Arc<rara_memory::manager::MemoryManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl AgentTool for MemorySearchTool {
    fn name(&self) -> &str { "memory_search" }

    fn description(&self) -> &str {
        "Search the agent's knowledge base for relevant information using \
         keywords. Returns matching document chunks with highlighted snippets, \
         ranked by relevance."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search keywords or phrase to find in the knowledge base"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default 10)",
                    "default": 10
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
            .unwrap_or(10) as usize;

        match self.manager.search(query, limit).await {
            Ok(results) => {
                let items: Vec<serde_json::Value> = results
                    .iter()
                    .map(|r| {
                        json!({
                            "doc_id": r.doc_id,
                            "chunk_id": r.chunk_id,
                            "heading": r.heading,
                            "snippet": r.snippet,
                            "rank": r.rank,
                        })
                    })
                    .collect();
                Ok(json!({
                    "results": items,
                    "count": items.len(),
                }))
            }
            Err(e) => Ok(json!({ "error": format!("{e}") })),
        }
    }
}
