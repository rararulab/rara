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

//! Layer 2 service tool: retrieve full content of a memory document.

use std::sync::Arc;

use async_trait::async_trait;
use rara_agents::tool_registry::AgentTool;
use serde_json::json;

/// Retrieve the full content of a specific memory document by ID.
pub struct MemoryGetTool {
    manager: Arc<rara_memory::manager::MemoryManager>,
}

impl MemoryGetTool {
    pub fn new(manager: Arc<rara_memory::manager::MemoryManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl AgentTool for MemoryGetTool {
    fn name(&self) -> &str { "memory_get" }

    fn description(&self) -> &str {
        "Retrieve the full content of a specific memory document by its ID. \
         Use this after searching to read the complete document."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "doc_id": {
                    "type": "string",
                    "description": "The document ID (relative file path) to retrieve"
                }
            },
            "required": ["doc_id"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
    ) -> rara_agents::err::Result<serde_json::Value> {
        let doc_id = params
            .get("doc_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| rara_agents::err::Error::Other {
                message: "missing required parameter: doc_id".into(),
            })?;

        match self.manager.get_document(doc_id).await {
            Ok(Some(doc)) => {
                let chunks: Vec<serde_json::Value> = doc
                    .chunks
                    .iter()
                    .map(|c| {
                        json!({
                            "chunk_id": c.chunk_id,
                            "heading": c.heading,
                            "content": c.content,
                            "chunk_index": c.chunk_index,
                        })
                    })
                    .collect();
                Ok(json!({
                    "doc_id": doc.id,
                    "title": doc.title,
                    "content": doc.content,
                    "chunks": chunks,
                    "updated_at": doc.updated_at,
                }))
            }
            Ok(None) => Ok(json!({
                "error": format!("document not found: {doc_id}")
            })),
            Err(e) => Ok(json!({ "error": format!("{e}") })),
        }
    }
}
