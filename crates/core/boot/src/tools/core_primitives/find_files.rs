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
//!
//! Uses `find` to locate files matching a glob pattern under a given directory,
//! sorted by modification time (newest first).

use anyhow::Context;
use async_trait::async_trait;
use serde_json::json;

use rara_kernel::tool::AgentTool;

/// Default maximum number of file entries to return.
const DEFAULT_LIMIT: usize = 500;

/// Layer 1 primitive: find files matching a glob pattern.
pub struct FindFilesTool;

impl FindFilesTool {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl AgentTool for FindFilesTool {
    fn name(&self) -> &str { "find_files" }

    fn description(&self) -> &str {
        "Find files matching a glob pattern (e.g. '*.rs', '**/*.toml'). Results are sorted by \
         modification time (newest first). Respects .gitignore when inside a git repository."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern to match files (e.g. '*.rs', '**/*.toml')"
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search in (default '.')"
                },
                "limit": {
                    "type": "number",
                    "description": "Maximum number of results (default 500)"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let pattern = params
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: pattern"))?;

        let path = params.get("path").and_then(|v| v.as_str()).unwrap_or(".");

        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(DEFAULT_LIMIT);

        // Use `find` with `-name` for the glob pattern, sorted by mtime.
        // -printf "%T@\t%p\n" gives epoch time + path for sorting.
        let output = tokio::process::Command::new("find")
            .arg(path)
            .arg("-type")
            .arg("f")
            .arg("-name")
            .arg(pattern)
            .arg("-not")
            .arg("-path")
            .arg("*/.git/*")
            .arg("-print0")
            .current_dir(path)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await
            .context("failed to run find")?;

        let stdout = String::from_utf8_lossy(&output.stdout);

        // Parse null-delimited file paths.
        let mut files: Vec<String> = stdout
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_owned())
            .collect();

        // Sort by modification time (newest first). If stat fails, push to
        // end.
        files.sort_by(|a, b| {
            let mtime_a = std::fs::metadata(a)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            let mtime_b = std::fs::metadata(b)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            mtime_b.cmp(&mtime_a) // newest first
        });

        let total_found = files.len();
        let truncated = total_found > limit;
        files.truncate(limit);

        Ok(json!({
            "files": files,
            "total_found": total_found,
            "truncated": truncated,
        }))
    }
}
