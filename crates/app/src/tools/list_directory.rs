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

//! Directory listing primitive.
//!
//! Lists entries in a directory with name, type, and size metadata.

use anyhow::Context;
use async_trait::async_trait;
use rara_kernel::tool::AgentTool;
use serde_json::json;

/// Maximum number of directory entries to return.
const MAX_ENTRIES: usize = 1000;

/// Layer 1 primitive: list directory contents.
pub struct ListDirectoryTool;

impl ListDirectoryTool {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl AgentTool for ListDirectoryTool {
    fn name(&self) -> &str { "list_directory" }

    fn description(&self) -> &str {
        "List the contents of a directory. Returns each entry's name, type (file/dir/symlink), and \
         size in bytes (for files). Maximum 1000 entries."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the directory to list"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, params: serde_json::Value, _context: &rara_kernel::tool::ToolContext) -> anyhow::Result<serde_json::Value> {
        let path = params
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: path"))?;

        let mut read_dir = tokio::fs::read_dir(path)
            .await
            .context(format!("failed to read directory {path}"))?;

        let mut entries = Vec::new();
        let mut total = 0usize;

        while let Some(entry) = read_dir
            .next_entry()
            .await
            .context("failed to read directory entry")?
        {
            total += 1;

            if entries.len() >= MAX_ENTRIES {
                continue; // Keep counting total but stop collecting.
            }

            let name = entry.file_name().to_string_lossy().into_owned();

            let file_type = entry
                .file_type()
                .await
                .context(format!("failed to get file type for {name}"))?;

            let type_str = if file_type.is_dir() {
                "dir"
            } else if file_type.is_symlink() {
                "symlink"
            } else {
                "file"
            };

            let size = if file_type.is_file() {
                entry.metadata().await.map(|m| m.len()).unwrap_or(0)
            } else {
                0
            };

            entries.push(json!({
                "name": name,
                "type": type_str,
                "size": size,
            }));
        }

        // Sort entries alphabetically by name.
        entries.sort_by(|a, b| {
            let name_a = a.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let name_b = b.get("name").and_then(|v| v.as_str()).unwrap_or("");
            name_a.cmp(name_b)
        });

        let truncated = total > MAX_ENTRIES;

        Ok(json!({
            "entries": entries,
            "total": total,
            "truncated": truncated,
        }))
    }
}
