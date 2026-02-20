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
//!
//! Writes content to a file, automatically creating parent directories if they
//! do not exist.

use std::path::Path;

use anyhow::Context;
use async_trait::async_trait;
use serde_json::json;

use crate::AgentTool;

/// Layer 1 primitive: write content to a file.
pub struct WriteFileTool;

impl WriteFileTool {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl AgentTool for WriteFileTool {
    fn name(&self) -> &str { "write_file" }

    fn description(&self) -> &str {
        "Write content to a file on the filesystem. Automatically creates parent directories if \
         they do not exist. Overwrites the file if it already exists."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "The content to write to the file"
                }
            },
            "required": ["file_path", "content"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let file_path = params
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: file_path"))?;

        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: content"))?;

        // Create parent directories if necessary.
        if let Some(parent) = Path::new(file_path).parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await.context(format!(
                    "failed to create parent directories for {file_path}"
                ))?;
            }
        }

        let bytes = content.as_bytes();
        tokio::fs::write(file_path, bytes)
            .await
            .context(format!("failed to write file {file_path}"))?;

        Ok(json!({
            "bytes_written": bytes.len(),
            "file_path": file_path,
        }))
    }
}
