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

//! Shell command execution primitive.
//!
//! Runs a command via `/bin/bash -c` with configurable timeout and working
//! directory.  Output is truncated to 50 KB / 2000 lines.

use async_trait::async_trait;
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Maximum output size in bytes (50 KB).
const MAX_OUTPUT_BYTES: usize = 50 * 1024;

/// Maximum number of output lines to keep.
const MAX_OUTPUT_LINES: usize = 2000;

/// Default command timeout in seconds.
const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// Input parameters for the bash tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BashParams {
    /// The shell command to execute.
    command: String,
    /// Timeout in seconds (default 120).
    timeout: Option<u64>,
    /// Working directory for the command.
    cwd:     Option<String>,
}

/// Typed result returned by the bash tool.
#[derive(Debug, Serialize)]
pub struct BashResult {
    /// Process exit code (-1 if failed to execute or timed out).
    pub exit_code: i32,
    /// Combined stdout and stderr output.
    pub stdout:    String,
    /// Whether the command was killed due to timeout.
    pub timed_out: bool,
    /// Whether the output was truncated.
    pub truncated: bool,
}

/// Layer 1 primitive: execute a shell command.
#[derive(ToolDef)]
#[tool(
    name = "bash",
    description = "Execute a shell command via /bin/bash -c. Returns exit code, combined \
                   stdout/stderr, and whether the command timed out. Output is truncated to 50KB \
                   / 2000 lines."
)]
pub struct BashTool;

impl BashTool {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl ToolExecute for BashTool {
    type Output = BashResult;
    type Params = BashParams;

    async fn run(&self, params: BashParams, _context: &ToolContext) -> anyhow::Result<BashResult> {
        let timeout_secs = params.timeout.unwrap_or(DEFAULT_TIMEOUT_SECS);
        let effective_command = rtk_rewrite(&params.command).await;

        let mut cmd = tokio::process::Command::new("/bin/bash");
        cmd.arg("-c").arg(&effective_command);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        if let Some(ref dir) = params.cwd {
            cmd.current_dir(dir);
        } else {
            cmd.current_dir(rara_paths::workspace_dir());
        }

        let timeout_dur = std::time::Duration::from_secs(timeout_secs);

        match tokio::time::timeout(timeout_dur, cmd.output()).await {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let combined = format!("{stdout}{stderr}");
                let (truncated_output, was_truncated) = truncate_output(&combined);

                Ok(BashResult {
                    exit_code: output.status.code().unwrap_or(-1),
                    stdout:    truncated_output,
                    timed_out: false,
                    truncated: was_truncated,
                })
            }
            Ok(Err(e)) => Ok(BashResult {
                exit_code: -1,
                stdout:    format!("failed to execute command: {e}"),
                timed_out: false,
                truncated: false,
            }),
            Err(_) => Ok(BashResult {
                exit_code: -1,
                stdout:    format!("command timed out after {timeout_secs}s"),
                timed_out: true,
                truncated: false,
            }),
        }
    }
}

/// Try to rewrite a command via `rtk rewrite` for token-optimized output.
/// Falls back to the original command if rtk is unavailable or declines the
/// rewrite.
async fn rtk_rewrite(command: &str) -> String {
    let result = tokio::process::Command::new("rtk")
        .args(["rewrite", command])
        .output()
        .await;

    match result {
        Ok(output) if output.status.success() => {
            let rewritten = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !rewritten.is_empty() && rewritten != command {
                tracing::debug!(original = command, rewritten = %rewritten, "rtk rewrite applied");
                return rewritten;
            }
        }
        _ => {}
    }

    command.to_string()
}

/// Truncate output to [`MAX_OUTPUT_BYTES`] and [`MAX_OUTPUT_LINES`], keeping
/// the *last* lines when truncation is necessary.
fn truncate_output(output: &str) -> (String, bool) {
    let mut truncated = false;

    // First truncate by byte size.
    let text = if output.len() > MAX_OUTPUT_BYTES {
        truncated = true;
        // Find a safe UTF-8 boundary.
        let start = output.len() - MAX_OUTPUT_BYTES;
        let safe_start = output.ceil_char_boundary(start);
        &output[safe_start..]
    } else {
        output
    };

    // Then truncate by line count.
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() > MAX_OUTPUT_LINES {
        truncated = true;
        let kept = &lines[lines.len() - MAX_OUTPUT_LINES..];
        (
            format!("... [output truncated]\n{}", kept.join("\n")),
            truncated,
        )
    } else if truncated {
        (format!("... [output truncated]\n{text}"), truncated)
    } else {
        (text.to_owned(), truncated)
    }
}
