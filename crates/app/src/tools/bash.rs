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
use rara_kernel::tool::{AgentTool, ToolCapabilities, ToolExecutionMode, ToolOutput};
use serde_json::json;

/// Maximum output size in bytes (50 KB).
const MAX_OUTPUT_BYTES: usize = 50 * 1024;

/// Maximum number of output lines to keep.
const MAX_OUTPUT_LINES: usize = 2000;

/// Default command timeout in seconds.
const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// Layer 1 primitive: execute a shell command.
pub struct BashTool;

impl BashTool {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl AgentTool for BashTool {
    fn name(&self) -> &str { "bash" }

    fn description(&self) -> &str {
        "Execute a shell command via /bin/bash -c. Returns exit code, combined stdout/stderr, and \
         whether the command timed out. Output is truncated to 50KB / 2000 lines."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute"
                },
                "timeout": {
                    "type": "number",
                    "description": "Timeout in seconds (default 120)"
                },
                "cwd": {
                    "type": "string",
                    "description": "Working directory for the command"
                }
            },
            "required": ["command"]
        })
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            execution_mode: ToolExecutionMode::Detachable,
            status_label:   Some("shell command running in background".into()),
        }
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _context: &rara_kernel::tool::ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let command = params
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: command"))?;

        let timeout_secs = params
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_TIMEOUT_SECS);

        let cwd = params.get("cwd").and_then(|v| v.as_str());

        // Attempt rtk rewrite for token-optimized output.
        let effective_command = rtk_rewrite(command).await;

        let mut cmd = tokio::process::Command::new("/bin/bash");
        cmd.arg("-c").arg(&effective_command);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        if let Some(dir) = cwd {
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

                Ok(json!({
                    "exit_code": output.status.code().unwrap_or(-1),
                    "stdout": truncated_output,
                    "timed_out": false,
                    "truncated": was_truncated,
                })
                .into())
            }
            Ok(Err(e)) => Ok(json!({
                "exit_code": -1,
                "stdout": format!("failed to execute command: {e}"),
                "timed_out": false,
                "truncated": false,
            })
            .into()),
            Err(_) => {
                // Timeout — the child process was dropped which should kill it.
                Ok(json!({
                    "exit_code": -1,
                    "stdout": format!("command timed out after {timeout_secs}s"),
                    "timed_out": true,
                    "truncated": false,
                })
                .into())
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bash_tool_defaults_to_detached_execution() {
        let caps = BashTool::new().capabilities();
        assert!(matches!(
            caps.execution_mode,
            ToolExecutionMode::Detachable
        ));
        assert_eq!(
            caps.status_label.as_deref(),
            Some("shell command running in background")
        );
    }
}
