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

//! Sandboxed shell command execution primitive.
//!
//! Runs the LLM-supplied command inside the per-session boxlite microVM so
//! shell commands cannot touch the host outside the workspace bind-mount.
//!
//! # Path translation
//!
//! `cwd` accepts host-style paths so the LLM can keep using the host
//! workspace layout. The translation rules are:
//!
//! - relative path → joined to `/workspace` inside the guest
//! - absolute path inside `rara_paths::workspace_dir()` → rewritten to
//!   `/workspace/<rest>`
//! - absolute path outside the workspace → hard error returned to the LLM (no
//!   approval routing — this is a mount-namespace boundary, not a policy
//!   decision)
//!
//! # Behavior changes vs. the host-shell implementation it replaces
//!
//! - The shell binary is whatever the rootfs image provides (`sh` for alpine,
//!   `bash` for debian-family). The tool always invokes `<shell> -c
//!   "<command>"`.
//! - Timeouts are enforced by boxlite per-exec rather than by signaling a host
//!   process group; there is no `SIGTERM`-then-`SIGKILL` two-phase kill any
//!   more.
//! - Network is **disabled** by default for `bash`. Operators can opt in to a
//!   boxlite allow-list via `sandbox.bash.allow_net` in YAML. Because the
//!   per-session VM is shared with `run_code`, the effective policy is the
//!   fused (most-permissive) union — see `crates/app/src/sandbox.rs`.
//! - Relative `..` traversal in `cwd` is bounded by the guest rootfs: escape
//!   attempts fail at [`rara_sandbox::Sandbox::exec`] argv validation rather
//!   than at host-side path translation.
//! - The 50KB / 2000-line truncation contract and the streaming
//!   `StreamEvent::ToolOutput` chunks are preserved so the agent UI is
//!   unchanged.

use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use rara_kernel::{
    io::{StreamEvent, StreamHandle},
    tool::{ToolContext, ToolExecute},
};
use rara_sandbox::ExecRequest;
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use crate::{
    SandboxToolConfig,
    sandbox::{GUEST_WORKSPACE, SandboxMap, sandbox_for_session, sandbox_not_configured_error},
};

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
    /// Timeout in seconds (default 120). Accepts an integer (`30`), a
    /// stringified integer (`"30"`), or a humantime duration (`"30s"`,
    /// `"2m"`).
    #[serde(default, deserialize_with = "deserialize_timeout")]
    timeout: Option<Duration>,
    /// Working directory for the command. Host paths inside the workspace
    /// are translated to the guest mount; paths outside the workspace are
    /// rejected.
    cwd:     Option<String>,
}

/// Accept `30` (integer), `"30"` (stringified integer), or `"30s"` /
/// `"2m"` (humantime duration) for the timeout field.
fn deserialize_timeout<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
where
    D: Deserializer<'de>,
{
    use std::fmt;

    use serde::de::{self, Visitor};

    struct TimeoutVisitor;

    impl<'de> Visitor<'de> for TimeoutVisitor {
        type Value = Option<Duration>;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("an integer, stringified integer, or humantime duration")
        }

        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> { Ok(None) }

        fn visit_some<D2: Deserializer<'de>>(self, d: D2) -> Result<Self::Value, D2::Error> {
            d.deserialize_any(DurationVisitor).map(Some)
        }
    }

    struct DurationVisitor;

    impl<'de> Visitor<'de> for DurationVisitor {
        type Value = Duration;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str(
                "an integer (seconds), stringified integer, humantime duration, or {\"secs\": N, \
                 \"nanos\": N} map",
            )
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Duration, E> {
            Ok(Duration::from_secs(v))
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Duration, E> {
            let secs = u64::try_from(v).map_err(|_| E::custom(format!("negative timeout: {v}")))?;
            Ok(Duration::from_secs(secs))
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Duration, E> {
            let s = v.trim();
            // Try bare integer first ("30" → 30 seconds).
            if let Ok(secs) = s.parse::<u64>() {
                return Ok(Duration::from_secs(secs));
            }
            // Fall back to humantime ("30s", "2m").
            humantime::parse_duration(s).map_err(|_| E::custom(format!("invalid timeout: {v:?}")))
        }

        /// Accept `{"secs": 30, "nanos": 0}` — the Duration struct layout
        /// that some LLMs (e.g. GPT-5.4) emit when they see the JSON schema.
        fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Duration, A::Error> {
            let mut secs: Option<u64> = None;
            let mut nanos: Option<u32> = None;

            while let Some(key) = map.next_key::<String>()? {
                match key.as_str() {
                    "secs" => secs = Some(map.next_value()?),
                    "nanos" => nanos = Some(map.next_value()?),
                    _ => {
                        let _ = map.next_value::<de::IgnoredAny>()?;
                    }
                }
            }

            let secs = secs.ok_or_else(|| de::Error::missing_field("secs"))?;
            Ok(Duration::new(secs, nanos.unwrap_or(0)))
        }
    }

    deserializer.deserialize_option(TimeoutVisitor)
}

/// Typed result returned by the bash tool.
///
/// Schema-stable with the host-shell implementation it replaces — the LLM
/// must not see this refactor.
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

/// Sandboxed shell command tool.
#[derive(ToolDef)]
#[tool(
    name = "bash",
    description = "Execute a shell command inside a hardware-isolated sandbox (boxlite microVM); \
                   returns exit code, stdout/stderr (truncated to 50KB).",
    timeout_secs = 150,
    destructive
)]
pub struct BashTool {
    /// Sandbox creation parameters resolved from YAML at startup. `None`
    /// means the operator did not configure `sandbox:` in `config.yaml` —
    /// in that case every call returns the standard "sandbox not
    /// configured" error.
    config:    Option<SandboxToolConfig>,
    /// Shared per-session sandbox handles. Reused across `bash` and
    /// `run_code` invocations so a single VM serves the whole session.
    sandboxes: SandboxMap,
}

impl BashTool {
    /// Create a new tool wired to the given config and shared sandbox map.
    pub fn new(config: Option<SandboxToolConfig>, sandboxes: SandboxMap) -> Self {
        Self { config, sandboxes }
    }
}

#[async_trait]
impl ToolExecute for BashTool {
    type Output = BashResult;
    type Params = BashParams;

    #[tracing::instrument(skip_all)]
    async fn run(&self, params: BashParams, context: &ToolContext) -> anyhow::Result<BashResult> {
        let cfg = self
            .config
            .as_ref()
            .ok_or_else(|| sandbox_not_configured_error("bash"))?;

        let timeout_dur = params
            .timeout
            .unwrap_or_else(|| Duration::from_secs(DEFAULT_TIMEOUT_SECS));

        let working_dir = match params.cwd.as_deref() {
            Some(raw) => Some(translate_cwd(raw)?),
            None => Some(GUEST_WORKSPACE.to_owned()),
        };

        let effective_command = rtk_rewrite(&params.command).await;

        let sandbox = sandbox_for_session(cfg, &self.sandboxes, context.session_key).await?;

        let request = ExecRequest::builder()
            .command("sh".to_owned())
            .args(vec!["-c".to_owned(), effective_command])
            .timeout(timeout_dur)
            .maybe_working_dir(working_dir)
            .build();

        // Hold the per-session lock for the whole exec — boxlite's `LiteBox`
        // is not assumed `Sync`, so concurrent calls within the same session
        // must serialize.
        let guard = sandbox.lock().await;
        let mut outcome = match guard.exec(request).await {
            Ok(o) => o,
            Err(e) => {
                return Ok(BashResult {
                    exit_code: -1,
                    stdout:    format!("failed to execute command: {e}"),
                    timed_out: false,
                    truncated: false,
                });
            }
        };

        // Build streaming context up front so each stdout chunk can be
        // forwarded to the agent UI as it arrives.
        let stream_ctx: Option<(StreamHandle, String)> = context
            .stream_handle
            .as_ref()
            .zip(context.tool_call_id.as_ref())
            .map(|(h, id)| (h.clone(), id.clone()));

        let mut combined = String::new();
        let mut bytes_used: usize = 0;
        let mut truncation_notified = false;

        while let Some(line) = outcome.stdout.next().await {
            append_with_cap(
                &line,
                &mut combined,
                &mut bytes_used,
                &mut truncation_notified,
                stream_ctx.as_ref(),
                // stream =
                true,
            );
        }
        if let Some(mut s) = outcome.stderr.take() {
            while let Some(line) = s.next().await {
                append_with_cap(
                    &line,
                    &mut combined,
                    &mut bytes_used,
                    &mut truncation_notified,
                    stream_ctx.as_ref(),
                    // stderr is not real-time-streamed: matches the previous
                    // host-shell behavior, where interleaving stdout+stderr
                    // produced confusing UI.
                    // stream =
                    false,
                );
            }
        }

        // Wait for the underlying execution to complete so we have an exit
        // status. boxlite enforces the per-exec timeout itself; if it fired,
        // `wait` returns an error or a non-zero status — we only treat the
        // dedicated boxlite timeout error as `timed_out = true`. Any other
        // wait error degrades to exit_code = -1 with timed_out = false.
        let (exit_code, timed_out) = match outcome.execution.wait().await {
            Ok(status) => (status.code(), false),
            Err(e) => {
                let msg = e.to_string();
                let timed_out = msg.contains("timeout") || msg.contains("timed out");
                tracing::warn!(error = %msg, timed_out, "sandbox exec wait failed");
                (-1, timed_out)
            }
        };

        let (truncated_output, was_truncated) = truncate_output(&combined);

        Ok(BashResult {
            exit_code,
            stdout: truncated_output,
            timed_out,
            truncated: was_truncated || truncation_notified,
        })
    }
}

/// Translate a host-style `cwd` argument into a guest-mount path.
///
/// See the module-level docs for the full rules.
fn translate_cwd(raw: &str) -> anyhow::Result<String> {
    let path = std::path::Path::new(raw);
    if !path.is_absolute() {
        // Relative path → joined to /workspace inside the guest. Render
        // with forward slashes; sandbox is always linux-flavored.
        let trimmed = raw.trim_start_matches("./");
        if trimmed.is_empty() {
            return Ok(GUEST_WORKSPACE.to_owned());
        }
        return Ok(format!("{GUEST_WORKSPACE}/{trimmed}"));
    }

    let workspace = rara_paths::workspace_dir();
    match path.strip_prefix(workspace) {
        Ok(rest) if rest.as_os_str().is_empty() => Ok(GUEST_WORKSPACE.to_owned()),
        Ok(rest) => Ok(format!(
            "{GUEST_WORKSPACE}/{}",
            rest.to_string_lossy().replace('\\', "/")
        )),
        Err(_) => Err(anyhow::anyhow!(
            "cwd '{}' is outside the workspace ('{}'); cannot run sandboxed bash there",
            raw,
            workspace.display()
        )),
    }
}

/// Append `line` to the combined output buffer with the 50 KB byte cap and
/// emit a streaming chunk when `stream` is true.
fn append_with_cap(
    line: &str,
    combined: &mut String,
    bytes_used: &mut usize,
    truncation_notified: &mut bool,
    stream_ctx: Option<&(StreamHandle, String)>,
    stream: bool,
) {
    let line_bytes = line.len();
    let needs_newline = !line.ends_with('\n');
    let extra = usize::from(needs_newline);
    let remaining = MAX_OUTPUT_BYTES.saturating_sub(*bytes_used);

    if remaining == 0 {
        if !*truncation_notified {
            if let Some((handle, tool_call_id)) = stream_ctx {
                handle.emit(StreamEvent::ToolOutput {
                    tool_call_id: tool_call_id.clone(),
                    chunk:        "\n[output truncated — 50 KB cap reached]\n".to_owned(),
                });
            }
            *truncation_notified = true;
        }
        return;
    }

    // Truncate at a UTF-8 boundary if the line itself overflows.
    let allow_bytes = remaining.saturating_sub(extra);
    let chunk_str = if line_bytes <= allow_bytes {
        line.to_owned()
    } else {
        let safe = line.floor_char_boundary(allow_bytes);
        line[..safe].to_owned()
    };

    let to_emit = if needs_newline {
        format!("{chunk_str}\n")
    } else {
        chunk_str
    };
    *bytes_used += to_emit.len();
    combined.push_str(&to_emit);

    if stream {
        if let Some((handle, tool_call_id)) = stream_ctx {
            handle.emit(StreamEvent::ToolOutput {
                tool_call_id: tool_call_id.clone(),
                chunk:        to_emit,
            });
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
/// rewrite. `rtk` runs on the host (it transforms the command string only);
/// the rewritten string is then handed to the sandboxed shell.
async fn rtk_rewrite(command: &str) -> String {
    let result = tokio::process::Command::new("rtk")
        .args(["rewrite", command])
        .output()
        .await;

    match result {
        Ok(output) if output.status.success() => {
            let rewritten = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !rewritten.is_empty() && rewritten != command {
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

    let text = if output.len() > MAX_OUTPUT_BYTES {
        truncated = true;
        let start = output.len() - MAX_OUTPUT_BYTES;
        let safe_start = output.ceil_char_boundary(start);
        &output[safe_start..]
    } else {
        output
    };

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
    fn translate_cwd_relative_path() {
        let out = translate_cwd("src/foo").expect("ok");
        assert_eq!(out, "/workspace/src/foo");
    }

    #[test]
    fn translate_cwd_relative_dot_slash() {
        let out = translate_cwd("./src").expect("ok");
        assert_eq!(out, "/workspace/src");
    }

    #[test]
    fn translate_cwd_outside_workspace_errors() {
        // /etc is never inside the workspace dir.
        let err = translate_cwd("/etc/passwd").expect_err("must reject");
        assert!(err.to_string().contains("outside the workspace"));
    }

    #[test]
    fn translate_cwd_inside_workspace_rewrites() {
        let workspace = rara_paths::workspace_dir().clone();
        let inside = workspace.join("foo/bar");
        let out = translate_cwd(&inside.to_string_lossy()).expect("ok");
        assert_eq!(out, "/workspace/foo/bar");
    }
}
