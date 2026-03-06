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

//! Regex search primitive.
//!
//! Shells out to `rg` (ripgrep) for fast, recursive regex search with optional
//! file type filtering and context lines.  Falls back to `grep -rn` if `rg` is
//! not installed.

use anyhow::{Context, bail};
use async_trait::async_trait;
use rara_kernel::tool::AgentTool;
use serde_json::json;

/// Maximum output size in bytes (50 KB).
const MAX_OUTPUT_BYTES: usize = 50 * 1024;

/// Maximum number of matches to return.
const MAX_MATCHES: usize = 100;

/// Layer 1 primitive: regex search across files.
pub struct GrepTool;

impl GrepTool {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl AgentTool for GrepTool {
    fn name(&self) -> &str { "grep" }

    fn description(&self) -> &str {
        "Search file contents using a regex pattern via ripgrep (rg). Supports file type filtering \
         with glob patterns, context lines, and case-insensitive search. Output is truncated to \
         50KB / 100 matches."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Regex pattern to search for"
                },
                "path": {
                    "type": "string",
                    "description": "File or directory to search in (default '.')"
                },
                "glob": {
                    "type": "string",
                    "description": "Glob pattern to filter files (e.g. '*.rs', '*.{ts,tsx}')"
                },
                "context": {
                    "type": "number",
                    "description": "Number of context lines to show around each match (default 0)"
                },
                "ignore_case": {
                    "type": "boolean",
                    "description": "Enable case-insensitive search (default false)"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _context: &rara_kernel::tool::ToolContext,
    ) -> anyhow::Result<serde_json::Value> {
        let pattern = params
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: pattern"))?;

        let path = params.get("path").and_then(|v| v.as_str()).unwrap_or(".");

        let glob_filter = params.get("glob").and_then(|v| v.as_str());

        let context = params.get("context").and_then(|v| v.as_u64()).unwrap_or(0);

        let ignore_case = params
            .get("ignore_case")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Try ripgrep first, fall back to grep.
        let result = try_ripgrep(pattern, path, glob_filter, context, ignore_case).await;

        match result {
            Ok(output) => Ok(output),
            Err(_) => {
                // Fallback to grep -rn.
                try_grep_fallback(pattern, path, ignore_case).await
            }
        }
    }
}

async fn try_ripgrep(
    pattern: &str,
    path: &str,
    glob_filter: Option<&str>,
    context: u64,
    ignore_case: bool,
) -> anyhow::Result<serde_json::Value> {
    let mut cmd = tokio::process::Command::new("rg");
    cmd.arg("-n"); // line numbers
    cmd.arg("--max-count").arg(MAX_MATCHES.to_string());

    if ignore_case {
        cmd.arg("-i");
    }

    if context > 0 {
        cmd.arg("-C").arg(context.to_string());
    }

    if let Some(g) = glob_filter {
        cmd.arg("--glob").arg(g);
    }

    cmd.arg("--").arg(pattern).arg(path);
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let output = cmd.output().await.context("failed to run rg")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // rg returns exit code 1 when no matches found, which is fine.
    if output.status.code() == Some(2) {
        bail!("rg error: {stderr}");
    }

    format_grep_output(&stdout)
}

async fn try_grep_fallback(
    pattern: &str,
    path: &str,
    ignore_case: bool,
) -> anyhow::Result<serde_json::Value> {
    let mut cmd = tokio::process::Command::new("grep");
    cmd.arg("-rn");

    if ignore_case {
        cmd.arg("-i");
    }

    cmd.arg("--").arg(pattern).arg(path);
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let output = cmd.output().await.context("failed to run grep")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    format_grep_output(&stdout)
}

fn format_grep_output(stdout: &str) -> anyhow::Result<serde_json::Value> {
    let lines: Vec<&str> = stdout.lines().collect();
    let match_count = lines.len();
    let mut truncated = false;

    let output = if stdout.len() > MAX_OUTPUT_BYTES {
        truncated = true;
        let safe_end = stdout.floor_char_boundary(MAX_OUTPUT_BYTES);
        format!("{}... [truncated]", &stdout[..safe_end])
    } else {
        stdout.to_owned()
    };

    if match_count > MAX_MATCHES {
        truncated = true;
    }

    Ok(json!({
        "matches": output,
        "match_count": match_count,
        "truncated": truncated,
    }))
}
