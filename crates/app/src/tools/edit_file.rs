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

//! Precise string replacement primitive.
//!
//! Reads a file, replaces an exact substring, and writes the result back.
//! Supports single (unique) replacement and replace-all modes.

use anyhow::{Context, bail};
use async_trait::async_trait;
use rara_kernel::tool::AgentTool;
use serde_json::json;

/// Layer 1 primitive: edit a file by exact string replacement.
pub struct EditFileTool;

impl EditFileTool {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl AgentTool for EditFileTool {
    fn name(&self) -> &str { "edit_file" }

    fn description(&self) -> &str {
        "Edit a file by replacing an exact string with a new string. By default, the old_string \
         must appear exactly once in the file. Use replace_all=true to replace all occurrences."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to edit"
                },
                "old_string": {
                    "type": "string",
                    "description": "The exact string to find and replace"
                },
                "new_string": {
                    "type": "string",
                    "description": "The replacement string"
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace all occurrences (default false)"
                }
            },
            "required": ["file_path", "old_string", "new_string"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _context: &rara_kernel::tool::ToolContext,
    ) -> anyhow::Result<serde_json::Value> {
        let raw_path = params
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: file_path"))?;
        let file_path = if std::path::Path::new(raw_path).is_absolute() {
            std::path::PathBuf::from(raw_path)
        } else {
            rara_paths::workspace_dir().join(raw_path)
        };

        let old_string = params
            .get("old_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: old_string"))?;

        let new_string = params
            .get("new_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: new_string"))?;

        let replace_all = params
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let content = tokio::fs::read_to_string(&file_path)
            .await
            .context(format!("failed to read file {}", file_path.display()))?;

        let count = content.matches(old_string).count();

        if count == 0 {
            bail!(
                "old_string not found in {}. Make sure the string matches exactly.",
                file_path.display()
            );
        }

        if !replace_all && count > 1 {
            bail!(
                "old_string found {count} times in {}. Use replace_all=true to replace \
                 all occurrences, or provide a more specific old_string.",
                file_path.display()
            );
        }

        let new_content = if replace_all {
            content.replace(old_string, new_string)
        } else {
            content.replacen(old_string, new_string, 1)
        };

        tokio::fs::write(&file_path, &new_content)
            .await
            .context(format!("failed to write file {}", file_path.display()))?;

        Ok(json!({
            "replacements": if replace_all { count } else { 1 },
            "file_path": file_path.display().to_string(),
        }))
    }
}
