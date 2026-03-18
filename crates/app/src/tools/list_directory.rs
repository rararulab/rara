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
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const MAX_ENTRIES: usize = 1000;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListDirectoryParams {
    /// Path to the directory to list.
    path: String,
}

/// Single entry in a directory listing.
#[derive(Debug, Clone, Serialize)]
pub struct DirEntry {
    name:       String,
    #[serde(rename = "type")]
    entry_type: String,
    size:       u64,
}

/// Typed result returned by the list-directory tool.
#[derive(Debug, Clone, Serialize)]
pub struct ListDirectoryResult {
    entries:   Vec<DirEntry>,
    total:     usize,
    truncated: bool,
}

/// Layer 1 primitive: list directory contents.
#[derive(ToolDef)]
#[tool(
    name = "list-directory",
    description = "List the contents of a directory. Returns each entry's name, type \
                   (file/dir/symlink), and size in bytes (for files). Maximum 1000 entries."
)]
pub struct ListDirectoryTool;
impl ListDirectoryTool {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl ToolExecute for ListDirectoryTool {
    type Output = ListDirectoryResult;
    type Params = ListDirectoryParams;

    async fn run(
        &self,
        params: ListDirectoryParams,
        _context: &ToolContext,
    ) -> anyhow::Result<ListDirectoryResult> {
        let path = {
            let p = std::path::PathBuf::from(params.path);
            if p.is_absolute() {
                p
            } else {
                rara_paths::workspace_dir().join(p)
            }
        };
        let mut read_dir = tokio::fs::read_dir(&path)
            .await
            .context(format!("failed to read directory {}", path.display()))?;
        let mut entries = Vec::new();
        let mut total = 0usize;
        while let Some(entry) = read_dir
            .next_entry()
            .await
            .context("failed to read directory entry")?
        {
            total += 1;
            if entries.len() >= MAX_ENTRIES {
                continue;
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

            entries.push(DirEntry {
                name,
                entry_type: type_str.to_owned(),
                size,
            });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(ListDirectoryResult {
            entries,
            total,
            truncated: total > MAX_ENTRIES,
        })
    }
}
