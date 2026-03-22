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

//! File deletion primitive.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use async_trait::async_trait;
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Input parameters for the delete-file tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteFileParams {
    /// Path to the file to delete.
    file_path: String,
}

/// Typed result returned by the delete-file tool.
#[derive(Debug, Clone, Serialize)]
pub struct DeleteFileResult {
    /// The resolved path of the deleted file.
    pub file_path: String,
}

/// Layer 1 primitive: delete a single file from the filesystem.
#[derive(ToolDef)]
#[tool(
    name = "delete-file",
    description = "Delete a file from the filesystem. Refuses to delete directories — use bash \
                   for that.",
    tier = "deferred"
)]
pub struct DeleteFileTool;

impl DeleteFileTool {
    /// Create a new instance.
    pub fn new() -> Self { Self }
}

#[async_trait]
impl ToolExecute for DeleteFileTool {
    type Output = DeleteFileResult;
    type Params = DeleteFileParams;

    async fn run(
        &self,
        params: DeleteFileParams,
        _context: &ToolContext,
    ) -> anyhow::Result<DeleteFileResult> {
        let file_path = if Path::new(&params.file_path).is_absolute() {
            PathBuf::from(&params.file_path)
        } else {
            rara_paths::workspace_dir().join(&params.file_path)
        };

        // Call remove_file directly and map OS errors to user-friendly messages,
        // avoiding TOCTOU races from separate is_dir()/exists() checks.
        if let Err(e) = tokio::fs::remove_file(&file_path).await {
            match e.kind() {
                std::io::ErrorKind::IsADirectory => bail!(
                    "'{}' is a directory, not a file. Use bash to remove directories.",
                    file_path.display()
                ),
                std::io::ErrorKind::NotFound => {
                    bail!("file '{}' does not exist", file_path.display())
                }
                _ => {
                    return Err(e)
                        .context(format!("failed to delete file {}", file_path.display()));
                }
            }
        }

        Ok(DeleteFileResult {
            file_path: file_path.display().to_string(),
        })
    }
}
