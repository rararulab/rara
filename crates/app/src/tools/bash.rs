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

use std::sync::Arc;

use async_trait::async_trait;
use base::process_group::{kill_process_group, terminate_process_group};
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::{io::AsyncReadExt, sync::Mutex};

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
#[derive(Debug, Clone, Serialize)]
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

    #[tracing::instrument(skip_all)]
    async fn run(&self, params: BashParams, _context: &ToolContext) -> anyhow::Result<BashResult> {
        let timeout_secs = params.timeout.unwrap_or(DEFAULT_TIMEOUT_SECS);
        let effective_command = rtk_rewrite(&params.command).await;

        let mut cmd = tokio::process::Command::new("/bin/bash");
        cmd.arg("-c").arg(&effective_command);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        // Place child in its own process group so we can signal the entire
        // tree on timeout (PGID = child PID).
        #[cfg(unix)]
        cmd.process_group(0);

        if let Some(ref dir) = params.cwd {
            cmd.current_dir(dir);
        } else {
            cmd.current_dir(rara_paths::workspace_dir());
        }

        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                return Ok(BashResult {
                    exit_code: -1,
                    stdout:    format!("failed to execute command: {e}"),
                    timed_out: false,
                    truncated: false,
                });
            }
        };

        let pgid = child.id();

        // Shared buffer for incremental output collection from both pipes.
        let buffer: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));

        // Spawn reader tasks for stdout and stderr that feed into the shared buffer.
        let stdout_handle = child.stdout.take().map(|pipe| {
            let buf = Arc::clone(&buffer);
            tokio::spawn(read_pipe_into(pipe, buf))
        });
        let stderr_handle = child.stderr.take().map(|pipe| {
            let buf = Arc::clone(&buffer);
            tokio::spawn(read_pipe_into(pipe, buf))
        });

        let timeout_dur = std::time::Duration::from_secs(timeout_secs);

        let (status, timed_out) = tokio::select! {
            status = child.wait() => (Some(status), false),
            () = tokio::time::sleep(timeout_dur) => {
                // Graceful two-phase kill: SIGTERM → wait 2s → SIGKILL.
                if let Some(pgid) = pgid {
                    tracing::warn!(pgid, timeout_secs, "bash command timed out, killing process group");
                    let _ = terminate_process_group(pgid);

                    let exited = tokio::time::timeout(
                        std::time::Duration::from_secs(2),
                        child.wait(),
                    ).await;

                    if exited.is_err() {
                        tracing::warn!(pgid, "process group did not exit after SIGTERM, sending SIGKILL");
                        let _ = kill_process_group(pgid);
                        let _ = child.wait().await;
                    }
                } else {
                    // No pgid available — best-effort kill via tokio.
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                }
                (None, true)
            },
        };

        // Wait for reader tasks to drain remaining pipe data.
        if let Some(h) = stdout_handle {
            let _ = h.await;
        }
        if let Some(h) = stderr_handle {
            let _ = h.await;
        }

        let raw = buffer.lock().await;
        let combined = String::from_utf8_lossy(&raw);
        let (truncated_output, was_truncated) = truncate_output(&combined);

        let exit_code = match (timed_out, status) {
            (true, _) => -1,
            (false, Some(Ok(s))) => s.code().unwrap_or(-1),
            _ => -1,
        };

        Ok(BashResult {
            exit_code,
            stdout: truncated_output,
            timed_out,
            truncated: was_truncated,
        })
    }
}

/// Read from an async pipe into a shared buffer, capping at
/// [`MAX_OUTPUT_BYTES`] to prevent unbounded memory growth.
async fn read_pipe_into<R: tokio::io::AsyncRead + Unpin>(mut pipe: R, buffer: Arc<Mutex<Vec<u8>>>) {
    let mut chunk = [0u8; 8192];
    loop {
        match pipe.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let mut buf = buffer.lock().await;
                let remaining = MAX_OUTPUT_BYTES.saturating_sub(buf.len());
                if remaining == 0 {
                    // Buffer full — keep reading to drain the pipe but discard data.
                    continue;
                }
                let to_copy = n.min(remaining);
                buf.extend_from_slice(&chunk[..to_copy]);
            }
        }
    }
}

/// Predicates and actions that `rtk find` does not support.  When the
/// rewriter turns a `find` invocation into `rtk find` but the original
/// command contains any of these tokens, we fall back to the raw `find`
/// command to avoid a runtime error.
const RTK_FIND_UNSUPPORTED: &[&str] = &[
    "-o", "-or", "-not", "!", "\\(", "\\)", "(", ")", "-exec", "-execdir", "-delete", "-ok",
    "-print0",
];

/// Returns `true` when `cmd` is a `find` invocation that uses compound
/// predicates or actions unsupported by `rtk find`.
fn has_unsupported_find_predicates(cmd: &str) -> bool {
    let trimmed = cmd.trim_start();
    if !trimmed.starts_with("find ") {
        return false;
    }
    RTK_FIND_UNSUPPORTED
        .iter()
        .any(|tok| trimmed.split_whitespace().any(|w| w == *tok))
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
                // rtk find cannot handle compound predicates — fall back.
                if rewritten.starts_with("rtk find") && has_unsupported_find_predicates(command) {
                    tracing::debug!(
                        original = command,
                        "rtk rewrite skipped: compound find predicates"
                    );
                    return command.to_string();
                }
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
