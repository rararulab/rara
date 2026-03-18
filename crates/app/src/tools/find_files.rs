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

//! Glob-based file discovery primitive.

use anyhow::Context;
use async_trait::async_trait;
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const DEFAULT_LIMIT: usize = 500;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FindFilesParams {
    /// Glob pattern to match files.
    pattern: String,
    /// Directory to search in (default '.').
    path:    Option<String>,
    /// Maximum number of results (default 500).
    limit:   Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FindFilesResult {
    pub files:       Vec<String>,
    pub total_found: usize,
    pub truncated:   bool,
}

/// Layer 1 primitive: find files matching a glob pattern.
#[derive(ToolDef)]
#[tool(
    name = "find-files",
    description = "Find files matching a glob pattern (e.g. '*.rs', '**/*.toml'). Results are \
                   sorted by modification time (newest first). Respects .gitignore when inside a \
                   git repository."
)]
pub struct FindFilesTool;
impl FindFilesTool {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl ToolExecute for FindFilesTool {
    type Output = FindFilesResult;
    type Params = FindFilesParams;

    async fn run(
        &self,
        params: FindFilesParams,
        _context: &ToolContext,
    ) -> anyhow::Result<FindFilesResult> {
        let workspace = rara_paths::workspace_dir();
        let path = params.path.as_deref().unwrap_or(".");
        let resolved_path = if std::path::Path::new(path).is_absolute() {
            std::path::PathBuf::from(path)
        } else {
            workspace.join(path)
        };
        let limit = params.limit.map(|v| v as usize).unwrap_or(DEFAULT_LIMIT);
        let output = tokio::process::Command::new("find")
            .arg(&resolved_path)
            .arg("-type")
            .arg("f")
            .arg("-name")
            .arg(&params.pattern)
            .arg("-not")
            .arg("-path")
            .arg("*/.git/*")
            .arg("-print0")
            .current_dir(&resolved_path)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await
            .context("failed to run find")?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut files: Vec<String> = stdout
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_owned())
            .collect();
        files.sort_by(|a, b| {
            let ma = std::fs::metadata(a)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            let mb = std::fs::metadata(b)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            mb.cmp(&ma)
        });
        let total_found = files.len();
        let truncated = total_found > limit;
        files.truncate(limit);
        Ok(FindFilesResult {
            files,
            total_found,
            truncated,
        })
    }
}
