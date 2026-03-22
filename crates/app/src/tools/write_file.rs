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

//! File writing primitive.

use anyhow::Context;
use async_trait::async_trait;
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteFileParams {
    /// Absolute path to the file to write.
    file_path: String,
    /// The content to write to the file.
    content:   String,
}

#[derive(Debug, Clone, Serialize)]
pub struct WriteFileResult {
    pub bytes_written: usize,
    pub file_path:     String,
}

/// Layer 1 primitive: write content to a file.
#[derive(ToolDef)]
#[tool(
    name = "write-file",
    description = "Write content to a file, creating parent directories as needed."
)]
pub struct WriteFileTool;
impl WriteFileTool {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl ToolExecute for WriteFileTool {
    type Output = WriteFileResult;
    type Params = WriteFileParams;

    async fn run(
        &self,
        params: WriteFileParams,
        _context: &ToolContext,
    ) -> anyhow::Result<WriteFileResult> {
        let file_path = if std::path::Path::new(&params.file_path).is_absolute() {
            std::path::PathBuf::from(&params.file_path)
        } else {
            rara_paths::workspace_dir().join(&params.file_path)
        };
        if let Some(parent) = file_path.parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await.context(format!(
                    "failed to create parent directories for {}",
                    file_path.display()
                ))?;
            }
        }
        let bytes = params.content.as_bytes();
        tokio::fs::write(&file_path, bytes)
            .await
            .context(format!("failed to write file {}", file_path.display()))?;
        Ok(WriteFileResult {
            bytes_written: bytes.len(),
            file_path:     file_path.display().to_string(),
        })
    }
}
