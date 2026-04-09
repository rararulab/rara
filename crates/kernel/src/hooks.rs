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

//! Tool execution hooks inspired by Claude Code's plugin hook protocol.
//!
//! Hooks are external shell commands executed at well-defined points in
//! the tool lifecycle:
//!
//! - **PreToolUse** — before tool execution; can deny the call.
//! - **PostToolUse** — after successful tool execution.
//! - **PostToolUseFailure** — after a failed tool execution.
//!
//! ## Protocol
//!
//! Each hook command receives context via environment variables:
//!
//! | Variable            | Description                        |
//! |---------------------|------------------------------------|
//! | `HOOK_EVENT`        | One of `PreToolUse`, `PostToolUse`, `PostToolUseFailure` |
//! | `HOOK_TOOL_NAME`    | The tool being invoked             |
//! | `HOOK_TOOL_INPUT`   | Raw JSON string of tool arguments  |
//! | `HOOK_TOOL_OUTPUT`  | Tool output (post-hooks only)      |
//! | `HOOK_TOOL_IS_ERROR`| `"1"` if the output is an error, `"0"` otherwise |
//!
//! A JSON payload with the same information is piped to stdin.
//!
//! ## Exit codes
//!
//! | Code | Meaning                                    |
//! |------|--------------------------------------------|
//! | 0    | Allow — tool execution proceeds normally   |
//! | 2    | Deny — tool execution is blocked           |
//! | other| Failure — hook itself broke (logged, not blocking) |

use std::{path::Path, sync::Arc};

use serde::{Deserialize, Serialize};
use serde_json::json;

// ---------------------------------------------------------------------------
// HookEvent
// ---------------------------------------------------------------------------

/// Which point in the tool lifecycle a hook fires at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    PreToolUse,
    PostToolUse,
    PostToolUseFailure,
}

impl HookEvent {
    fn as_str(self) -> &'static str {
        match self {
            Self::PreToolUse => "PreToolUse",
            Self::PostToolUse => "PostToolUse",
            Self::PostToolUseFailure => "PostToolUseFailure",
        }
    }
}

// ---------------------------------------------------------------------------
// HookRunResult
// ---------------------------------------------------------------------------

/// Outcome of running one or more hook commands for a single event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookRunResult {
    denied:   bool,
    failed:   bool,
    messages: Vec<String>,
}

impl HookRunResult {
    /// Construct an allow result (no denial, no failure).
    #[must_use]
    pub fn allow(messages: Vec<String>) -> Self {
        Self {
            denied: false,
            failed: false,
            messages,
        }
    }

    /// Whether any hook denied execution (exit code 2).
    #[must_use]
    pub fn is_denied(&self) -> bool { self.denied }

    /// Whether any hook itself failed (exit code != 0 and != 2).
    #[must_use]
    pub fn is_failed(&self) -> bool { self.failed }

    /// Messages collected from hook stdout.
    #[must_use]
    pub fn messages(&self) -> &[String] { &self.messages }
}

// ---------------------------------------------------------------------------
// HooksConfig
// ---------------------------------------------------------------------------

/// Shell commands to run at each hook point.
///
/// Configured in `config.yaml` under `kernel.hooks`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HooksConfig {
    /// Commands run before tool execution.  Exit 2 = deny.
    #[serde(default)]
    pub pre_tool_use:          Vec<String>,
    /// Commands run after successful tool execution.
    #[serde(default)]
    pub post_tool_use:         Vec<String>,
    /// Commands run after a failed tool execution.
    #[serde(default)]
    pub post_tool_use_failure: Vec<String>,
}

// ---------------------------------------------------------------------------
// HookRunner
// ---------------------------------------------------------------------------

/// Executes configured hook commands at tool lifecycle points.
///
/// Constructed once from [`HooksConfig`] and shared (via `Arc`) across
/// agent sessions.  All command execution is async via
/// [`tokio::process::Command`].
#[derive(Debug, Clone)]
pub struct HookRunner {
    config: HooksConfig,
}

/// Shared reference to a [`HookRunner`].
pub type HookRunnerRef = Arc<HookRunner>;

impl HookRunner {
    /// Create a runner from config.
    pub fn new(config: HooksConfig) -> Self { Self { config } }

    /// Run all `PreToolUse` hooks.
    pub async fn run_pre_tool_use(&self, tool_name: &str, tool_input: &str) -> HookRunResult {
        Self::run_commands(
            HookEvent::PreToolUse,
            &self.config.pre_tool_use,
            tool_name,
            tool_input,
            None,
            false,
        )
        .await
    }

    /// Run all `PostToolUse` hooks.
    pub async fn run_post_tool_use(
        &self,
        tool_name: &str,
        tool_input: &str,
        tool_output: &str,
        is_error: bool,
    ) -> HookRunResult {
        Self::run_commands(
            HookEvent::PostToolUse,
            &self.config.post_tool_use,
            tool_name,
            tool_input,
            Some(tool_output),
            is_error,
        )
        .await
    }

    /// Run all `PostToolUseFailure` hooks.
    pub async fn run_post_tool_use_failure(
        &self,
        tool_name: &str,
        tool_input: &str,
        tool_error: &str,
    ) -> HookRunResult {
        Self::run_commands(
            HookEvent::PostToolUseFailure,
            &self.config.post_tool_use_failure,
            tool_name,
            tool_input,
            Some(tool_error),
            true,
        )
        .await
    }

    /// Execute a list of commands for the given event, stopping on first
    /// denial or failure.
    async fn run_commands(
        event: HookEvent,
        commands: &[String],
        tool_name: &str,
        tool_input: &str,
        tool_output: Option<&str>,
        is_error: bool,
    ) -> HookRunResult {
        if commands.is_empty() {
            return HookRunResult::allow(Vec::new());
        }

        let payload = hook_payload(event, tool_name, tool_input, tool_output, is_error).to_string();

        let mut messages = Vec::new();

        for command in commands {
            match Self::run_command(
                command,
                event,
                tool_name,
                tool_input,
                tool_output,
                is_error,
                &payload,
            )
            .await
            {
                HookCommandOutcome::Allow { message } => {
                    if let Some(message) = message {
                        messages.push(message);
                    }
                }
                HookCommandOutcome::Deny { message } => {
                    messages.push(message.unwrap_or_else(|| {
                        format!("{} hook denied tool `{tool_name}`", event.as_str())
                    }));
                    return HookRunResult {
                        denied: true,
                        failed: false,
                        messages,
                    };
                }
                HookCommandOutcome::Failed { message } => {
                    messages.push(message);
                    return HookRunResult {
                        denied: false,
                        failed: true,
                        messages,
                    };
                }
            }
        }

        HookRunResult::allow(messages)
    }

    /// Run a single shell command with the hook protocol env vars + stdin.
    async fn run_command(
        command: &str,
        event: HookEvent,
        tool_name: &str,
        tool_input: &str,
        tool_output: Option<&str>,
        is_error: bool,
        payload: &str,
    ) -> HookCommandOutcome {
        let mut child_cmd = shell_command(command);
        child_cmd.stdin(std::process::Stdio::piped());
        child_cmd.stdout(std::process::Stdio::piped());
        child_cmd.stderr(std::process::Stdio::piped());
        child_cmd.env("HOOK_EVENT", event.as_str());
        child_cmd.env("HOOK_TOOL_NAME", tool_name);
        child_cmd.env("HOOK_TOOL_INPUT", tool_input);
        child_cmd.env("HOOK_TOOL_IS_ERROR", if is_error { "1" } else { "0" });
        if let Some(tool_output) = tool_output {
            child_cmd.env("HOOK_TOOL_OUTPUT", tool_output);
        }

        let spawn_result = child_cmd.spawn();
        let mut child = match spawn_result {
            Ok(child) => child,
            Err(error) => {
                return HookCommandOutcome::Failed {
                    message: format!(
                        "{} hook `{command}` failed to start for `{tool_name}`: {error}",
                        event.as_str()
                    ),
                };
            }
        };

        // Write payload to stdin, then drop to close the pipe.
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            // Best-effort write — if it fails the command still runs.
            let _ = stdin.write_all(payload.as_bytes()).await;
        }

        match child.wait_with_output().await {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let message = (!stdout.is_empty()).then_some(stdout);
                match output.status.code() {
                    Some(0) => HookCommandOutcome::Allow { message },
                    Some(2) => HookCommandOutcome::Deny { message },
                    Some(code) => HookCommandOutcome::Failed {
                        message: format_hook_warning(
                            command,
                            code,
                            message.as_deref(),
                            stderr.as_str(),
                        ),
                    },
                    None => HookCommandOutcome::Failed {
                        message: format!(
                            "{} hook `{command}` terminated by signal while handling `{tool_name}`",
                            event.as_str()
                        ),
                    },
                }
            }
            Err(error) => HookCommandOutcome::Failed {
                message: format!(
                    "{} hook `{command}` failed for `{tool_name}`: {error}",
                    event.as_str()
                ),
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

/// Outcome of a single hook command execution.
enum HookCommandOutcome {
    Allow { message: Option<String> },
    Deny { message: Option<String> },
    Failed { message: String },
}

/// Build the JSON payload piped to hook stdin.
fn hook_payload(
    event: HookEvent,
    tool_name: &str,
    tool_input: &str,
    tool_output: Option<&str>,
    is_error: bool,
) -> serde_json::Value {
    match event {
        HookEvent::PostToolUseFailure => json!({
            "hook_event_name": event.as_str(),
            "tool_name": tool_name,
            "tool_input": parse_tool_input(tool_input),
            "tool_input_json": tool_input,
            "tool_error": tool_output,
            "tool_result_is_error": true,
        }),
        _ => json!({
            "hook_event_name": event.as_str(),
            "tool_name": tool_name,
            "tool_input": parse_tool_input(tool_input),
            "tool_input_json": tool_input,
            "tool_output": tool_output,
            "tool_result_is_error": is_error,
        }),
    }
}

/// Try to parse tool input as JSON; fall back to a `{ "raw": ... }` wrapper.
fn parse_tool_input(tool_input: &str) -> serde_json::Value {
    serde_json::from_str(tool_input).unwrap_or_else(|_| json!({ "raw": tool_input }))
}

/// Format a human-readable warning for a hook that exited with an unexpected
/// status code.
fn format_hook_warning(command: &str, code: i32, stdout: Option<&str>, stderr: &str) -> String {
    let mut message = format!("Hook `{command}` exited with status {code}");
    if let Some(stdout) = stdout.filter(|s| !s.is_empty()) {
        message.push_str(": ");
        message.push_str(stdout);
    } else if !stderr.is_empty() {
        message.push_str(": ");
        message.push_str(stderr);
    }
    message
}

/// Create an async shell command, mirroring the Claude Code convention:
/// if `command` is a file path, run `sh <path>`; otherwise `sh -lc <command>`.
fn shell_command(command: &str) -> tokio::process::Command {
    if Path::new(command).exists() {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg(command);
        cmd
    } else {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-lc").arg(command);
        cmd
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_hooks_allow_everything() {
        let runner = HookRunner::new(HooksConfig::default());
        let result = runner
            .run_pre_tool_use("bash", r#"{"command":"pwd"}"#)
            .await;
        assert!(!result.is_denied());
        assert!(!result.is_failed());
        assert!(result.messages().is_empty());
    }

    #[tokio::test]
    async fn pre_hook_can_deny() {
        let runner = HookRunner::new(HooksConfig {
            pre_tool_use: vec!["printf 'blocked'; exit 2".to_string()],
            ..Default::default()
        });
        let result = runner
            .run_pre_tool_use("bash", r#"{"command":"rm -rf /"}"#)
            .await;
        assert!(result.is_denied());
        assert_eq!(result.messages(), &["blocked"]);
    }

    #[tokio::test]
    async fn pre_hook_allows_on_exit_zero() {
        let runner = HookRunner::new(HooksConfig {
            pre_tool_use: vec!["printf 'ok'".to_string()],
            ..Default::default()
        });
        let result = runner
            .run_pre_tool_use("bash", r#"{"command":"pwd"}"#)
            .await;
        assert!(!result.is_denied());
        assert!(!result.is_failed());
        assert_eq!(result.messages(), &["ok"]);
    }

    #[tokio::test]
    async fn hook_failure_is_not_denial() {
        let runner = HookRunner::new(HooksConfig {
            pre_tool_use: vec!["exit 1".to_string()],
            ..Default::default()
        });
        let result = runner.run_pre_tool_use("bash", r#"{}"#).await;
        assert!(!result.is_denied());
        assert!(result.is_failed());
    }

    #[tokio::test]
    async fn post_tool_use_receives_output() {
        let runner = HookRunner::new(HooksConfig {
            post_tool_use: vec!["printf 'logged'".to_string()],
            ..Default::default()
        });
        let result = runner
            .run_post_tool_use("bash", r#"{"command":"ls"}"#, "file1\nfile2", false)
            .await;
        assert!(!result.is_denied());
        assert_eq!(result.messages(), &["logged"]);
    }

    #[tokio::test]
    async fn post_tool_use_failure_hook() {
        let runner = HookRunner::new(HooksConfig {
            post_tool_use_failure: vec!["printf 'noticed'".to_string()],
            ..Default::default()
        });
        let result = runner
            .run_post_tool_use_failure("bash", r#"{}"#, "command not found")
            .await;
        assert!(!result.is_denied());
        assert_eq!(result.messages(), &["noticed"]);
    }

    #[tokio::test]
    async fn multiple_hooks_collect_messages() {
        let runner = HookRunner::new(HooksConfig {
            pre_tool_use: vec!["printf 'hook-a'".to_string(), "printf 'hook-b'".to_string()],
            ..Default::default()
        });
        let result = runner
            .run_pre_tool_use("bash", r#"{"command":"pwd"}"#)
            .await;
        assert!(!result.is_denied());
        assert_eq!(result.messages(), &["hook-a", "hook-b"]);
    }

    #[tokio::test]
    async fn failure_stops_subsequent_hooks() {
        let runner = HookRunner::new(HooksConfig {
            pre_tool_use: vec![
                "printf 'broken'; exit 1".to_string(),
                "printf 'never-reached'".to_string(),
            ],
            ..Default::default()
        });
        let result = runner.run_pre_tool_use("bash", r#"{}"#).await;
        assert!(result.is_failed());
        assert!(
            !result
                .messages()
                .iter()
                .any(|m| m.contains("never-reached"))
        );
    }

    #[tokio::test]
    async fn denial_stops_subsequent_hooks() {
        let runner = HookRunner::new(HooksConfig {
            pre_tool_use: vec![
                "printf 'denied'; exit 2".to_string(),
                "printf 'never-reached'".to_string(),
            ],
            ..Default::default()
        });
        let result = runner.run_pre_tool_use("bash", r#"{}"#).await;
        assert!(result.is_denied());
        assert_eq!(result.messages(), &["denied"]);
    }
}
