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
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Maximum output size in bytes (50 KB).
const MAX_OUTPUT_BYTES: usize = 50 * 1024;

/// Maximum number of matches to return.
const MAX_MATCHES: usize = 100;

/// Input parameters for the grep tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GrepParams {
    /// Regex pattern to search for.
    pattern:     String,
    /// File or directory to search in (default '.').
    path:        Option<String>,
    /// Glob pattern to filter files (e.g. '*.rs', '*.{ts,tsx}').
    glob:        Option<String>,
    /// Number of context lines to show around each match (default 0).
    context:     Option<u64>,
    /// Enable case-insensitive search (default false).
    ignore_case: Option<bool>,
}

/// Typed result returned by the grep tool.
#[derive(Debug, Clone, Serialize)]
pub struct GrepResult {
    /// Matching lines output.
    pub matches:     String,
    /// Number of matching lines.
    pub match_count: usize,
    /// Whether the output was truncated.
    pub truncated:   bool,
}

/// Layer 1 primitive: regex search across files.
#[derive(ToolDef)]
#[tool(
    name = "grep",
    description = "Search file contents using a regex pattern via ripgrep (rg). Supports file \
                   type filtering with glob patterns, context lines, and case-insensitive search. \
                   Output is truncated to 50KB / 100 matches."
)]
pub struct GrepTool;

impl GrepTool {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl ToolExecute for GrepTool {
    type Output = GrepResult;
    type Params = GrepParams;

    async fn run(&self, params: GrepParams, _context: &ToolContext) -> anyhow::Result<GrepResult> {
        let workspace = rara_paths::workspace_dir();
        let raw_path = params.path.as_deref().unwrap_or(".");
        let resolved = if std::path::Path::new(raw_path).is_absolute() {
            std::path::PathBuf::from(raw_path)
        } else {
            workspace.join(raw_path)
        };
        let path = resolved.to_str().unwrap_or(".");
        let ignore_case = params.ignore_case.unwrap_or(false);
        let context_lines = params.context.unwrap_or(0);

        let result = try_ripgrep(
            &params.pattern,
            path,
            params.glob.as_deref(),
            context_lines,
            ignore_case,
        )
        .await;

        match result {
            Ok(output) => Ok(output),
            Err(_) => try_grep_fallback(&params.pattern, path, ignore_case).await,
        }
    }
}

async fn try_ripgrep(
    pattern: &str,
    path: &str,
    glob_filter: Option<&str>,
    context: u64,
    ignore_case: bool,
) -> anyhow::Result<GrepResult> {
    let mut cmd = tokio::process::Command::new("rg");
    cmd.arg("-n")
        .arg("--max-count")
        .arg(MAX_MATCHES.to_string());
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
    if output.status.code() == Some(2) {
        bail!("rg error: {stderr}");
    }
    format_grep_output(&stdout)
}

async fn try_grep_fallback(
    pattern: &str,
    path: &str,
    ignore_case: bool,
) -> anyhow::Result<GrepResult> {
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

fn format_grep_output(stdout: &str) -> anyhow::Result<GrepResult> {
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
    Ok(GrepResult {
        matches: output,
        match_count,
        truncated,
    })
}
