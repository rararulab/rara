// Copyright 2025 Rararulab
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

use anyhow::{Context, bail};
use async_trait::async_trait;
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EditFileParams {
    /// Absolute path to the file to edit.
    file_path:   String,
    /// The exact string to find and replace.
    old_string:  String,
    /// The replacement string.
    new_string:  String,
    /// Replace all occurrences (default false).
    replace_all: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EditFileResult {
    pub replacements:  usize,
    pub file_path:     String,
    /// Net lines added (0 when lines were only removed).
    pub lines_added:   usize,
    /// Net lines removed (0 when lines were only added).
    pub lines_removed: usize,
}

/// Layer 1 primitive: edit a file by exact string replacement.
#[derive(ToolDef)]
#[tool(
    name = "edit-file",
    description = "Replace an exact string in a file; use replace_all=true for multiple \
                   occurrences."
)]
pub struct EditFileTool;
impl EditFileTool {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl ToolExecute for EditFileTool {
    type Output = EditFileResult;
    type Params = EditFileParams;

    async fn run(
        &self,
        params: EditFileParams,
        _context: &ToolContext,
    ) -> anyhow::Result<EditFileResult> {
        let file_path = if std::path::Path::new(&params.file_path).is_absolute() {
            std::path::PathBuf::from(&params.file_path)
        } else {
            rara_paths::workspace_dir().join(&params.file_path)
        };
        let replace_all = params.replace_all.unwrap_or(false);
        let content = tokio::fs::read_to_string(&file_path)
            .await
            .context(format!("failed to read file {}", file_path.display()))?;
        let count = content.matches(&params.old_string).count();
        if count == 0 {
            bail!(
                "old_string not found in {}. Make sure the string matches exactly.",
                file_path.display()
            );
        }
        if !replace_all && count > 1 {
            bail!(
                "old_string found {count} times in {}. Use replace_all=true to replace all \
                 occurrences, or provide a more specific old_string.",
                file_path.display()
            );
        }
        let new_content = if replace_all {
            content.replace(&params.old_string, &params.new_string)
        } else {
            content.replacen(&params.old_string, &params.new_string, 1)
        };
        tokio::fs::write(&file_path, &new_content)
            .await
            .context(format!("failed to write file {}", file_path.display()))?;
        let old_lines = content.lines().count();
        let new_lines = new_content.lines().count();

        Ok(EditFileResult {
            replacements:  if replace_all { count } else { 1 },
            file_path:     file_path.display().to_string(),
            lines_added:   new_lines.saturating_sub(old_lines),
            lines_removed: old_lines.saturating_sub(new_lines),
        })
    }
}
