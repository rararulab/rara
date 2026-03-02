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

//! Object storage read primitive.
//!
//! Reads a key from the configured `opendal::Operator` and returns UTF-8 text
//! content.

use async_trait::async_trait;
use opendal::Operator;
use rara_kernel::tool::AgentTool;
use serde_json::json;

/// Layer 1 primitive: read text content from object storage.
pub struct StorageReadTool {
    operator: Operator,
}

impl StorageReadTool {
    pub fn new(operator: Operator) -> Self { Self { operator } }
}

#[async_trait]
impl AgentTool for StorageReadTool {
    fn name(&self) -> &str { "storage_read" }

    fn description(&self) -> &str {
        "Read a file from object storage by key. Returns the UTF-8 text content. Useful for \
         reading stored markdown, crawl results, or analysis outputs."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "key": {
                    "type": "string",
                    "description": "The storage key (object path) to read"
                }
            },
            "required": ["key"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let key = params
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: key"))?;

        match self.operator.read(key).await {
            Ok(buf) => {
                let content = String::from_utf8_lossy(&buf.to_vec()).into_owned();
                Ok(json!({ "content": content }))
            }
            Err(e) => Ok(json!({ "error": format!("{e}") })),
        }
    }
}
