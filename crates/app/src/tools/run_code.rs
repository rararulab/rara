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

//! Sandboxed code execution tool, backed by `rara-sandbox` (boxlite).
//!
//! The first invocation in a [`SessionKey`] creates a microVM (rootfs image
//! taken from YAML config) and stashes it in a session-keyed map. Subsequent
//! invocations in the same session reuse the same VM — boxlite cold start is
//! ~60 ms but installing dependencies on every call would be wasteful, so
//! the sandbox is held until the session ends. Cleanup is driven by the
//! `LifecycleHook::on_session_end` hook installed at startup
//! (see [`SandboxCleanupHook`]).

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use futures::StreamExt;
use rara_kernel::{
    io::{StreamEvent, StreamHandle},
    lifecycle::{LifecycleHook, SessionEndContext},
    session::SessionKey,
    tool::{ToolContext, ToolExecute},
};
use rara_sandbox::{ExecRequest, Sandbox};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

// Re-export for back-compat with the integration test in
// `tests/run_code_session.rs`, which imports `SandboxMap` from this module.
pub use crate::sandbox::SandboxMap;
use crate::{
    SandboxToolConfig,
    sandbox::{
        RECLAIM_BACKOFF, RECLAIM_MAX_ATTEMPTS, ReclaimOutcome, reclaim_when_idle,
        sandbox_for_session, sandbox_not_configured_error,
    },
    tools::timeout::deserialize_timeout,
};

/// Default hard timeout for a `run_code` exec, in seconds.
///
/// A `run_code`-local mechanism-level safety backstop: without it a
/// non-terminating command (`while true`) holds the per-session sandbox lock
/// until the kernel's coarse per-tool wall drops the future, stalling every
/// other exec in the session and leaving the guest process burning a vCPU.
/// It is deliberately a **separate** const from `bash`'s `DEFAULT_TIMEOUT_SECS`
/// — coupling two independent tools' safety backstops via one symbol is a
/// footgun — and a Rust `const` rather than a YAML knob, per
/// `docs/guides/anti-patterns.md` (mechanism-tuning constants are not config).
/// The value matches bash's proven default rather than inventing a new number.
const DEFAULT_EXEC_TIMEOUT_SECS: u64 = 120;

/// Input parameters for the `run_code` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RunCodeParams {
    /// Executable to invoke inside the sandbox (e.g. `"sh"`, `"python"`).
    command: String,
    /// Arguments to pass, in order. Empty vec means no args.
    #[serde(default)]
    args:    Vec<String>,
    /// Hard timeout for the exec (default 120s). Accepts an integer (`120`),
    /// a stringified integer (`"120"`), a humantime duration (`"2m"`), or a
    /// `{"secs": N, "nanos": N}` map — the same forms `bash` accepts.
    #[serde(default, deserialize_with = "deserialize_timeout")]
    timeout: Option<Duration>,
}

/// Typed result returned by `run_code`.
#[derive(Debug, Clone, Serialize)]
pub struct RunCodeResult {
    /// Process exit code reported by boxlite. `None` if the sandbox never
    /// reported one (e.g. transport error).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Combined stdout captured during execution.
    pub stdout:    String,
    /// Combined stderr captured during execution. Empty when the sandbox
    /// declined to materialise a stderr stream.
    pub stderr:    String,
    /// Whether the exec was hard-killed by boxlite for exceeding its timeout.
    /// Mirrors `BashResult::timed_out` so the model can distinguish a timeout
    /// from an ordinary non-zero exit.
    pub timed_out: bool,
}

/// Sandboxed code execution tool.
///
/// Tier is `Deferred` — most agent turns do not need code execution, and
/// the rootfs image carries non-trivial cost (microVM cold start + image
/// pull on first use), so we keep it out of the always-on tool set.
#[derive(ToolDef)]
#[tool(
    name = "run_code",
    description = "Execute a command inside a hardware-isolated sandbox (boxlite microVM). Reuses \
                   one VM per session; the VM is destroyed when the session ends. Use this for \
                   running LLM-generated code that should not touch the host.",
    tier = "deferred",
    timeout_secs = 150,
    destructive
)]
pub struct RunCodeTool {
    /// Sandbox creation parameters resolved from YAML at startup.
    /// `None` means the operator did not configure `sandbox:` in
    /// `config.yaml` — in that case every call returns an error.
    config:    Option<SandboxToolConfig>,
    /// Shared per-session sandbox handles. Cloned into
    /// [`SandboxCleanupHook`] so session-end cleanup hits the same map.
    sandboxes: SandboxMap,
}

impl RunCodeTool {
    /// Create a new tool wired to the given config and shared sandbox map.
    pub fn new(config: Option<SandboxToolConfig>, sandboxes: SandboxMap) -> Self {
        Self { config, sandboxes }
    }

    /// Look up an existing sandbox for `session_key`, creating one on the
    /// first call. Concurrent invocations within the same session
    /// serialize on the per-session mutex returned here.
    /// Public for the integration test in `tests/run_code_session.rs`.
    /// Not part of the agent-callable surface.
    pub async fn sandbox_for_session(
        &self,
        session_key: SessionKey,
    ) -> anyhow::Result<Arc<Mutex<Sandbox>>> {
        let cfg = self
            .config
            .as_ref()
            .ok_or_else(|| sandbox_not_configured_error("run_code"))?;
        // The shared per-session VM picks its NetworkPolicy from the fused
        // policy across all sandbox-using tools — see
        // `crates/app/src/sandbox.rs::fused_network_policy`. `run_code`
        // contributes its own config-driven, default-deny allow-list
        // (`SandboxToolConfig::run_code`): no egress unless the operator
        // enumerates hosts. Full outbound is not expressible from YAML
        // (#1937, #1946, #2216).
        sandbox_for_session(cfg, &self.sandboxes, session_key).await
    }
}

#[async_trait]
impl ToolExecute for RunCodeTool {
    type Output = RunCodeResult;
    type Params = RunCodeParams;

    #[tracing::instrument(skip_all, fields(command = %params.command))]
    async fn run(
        &self,
        params: RunCodeParams,
        context: &ToolContext,
    ) -> anyhow::Result<RunCodeResult> {
        let sandbox = self.sandbox_for_session(context.session_key).await?;
        let request = build_exec_request(params.command, params.args, params.timeout);

        // Hold the per-session lock for the whole exec — boxlite's `LiteBox`
        // is not assumed `Sync` (see `rara-sandbox/AGENT.md`), so concurrent
        // calls within the same session must serialize.
        let guard = sandbox.lock().await;
        let mut outcome = guard
            .exec(request)
            .await
            .map_err(|e| anyhow::anyhow!("sandbox exec failed: {e}"))?;

        // Build streaming context up front so each stdout chunk can be
        // forwarded to the agent UI as it arrives.
        let stream_ctx: Option<(StreamHandle, String)> = context
            .stream_handle
            .as_ref()
            .zip(context.tool_call_id.as_ref())
            .map(|(h, id)| (h.clone(), id.clone()));

        let mut stdout = String::new();
        while let Some(line) = outcome.stdout.next().await {
            if let Some((ref handle, ref tool_call_id)) = stream_ctx {
                handle.emit(StreamEvent::ToolOutput {
                    tool_call_id: tool_call_id.clone(),
                    chunk:        line.clone(),
                });
            }
            stdout.push_str(&line);
            if !line.ends_with('\n') {
                stdout.push('\n');
            }
        }

        let mut stderr = String::new();
        if let Some(mut s) = outcome.stderr {
            while let Some(line) = s.next().await {
                stderr.push_str(&line);
                if !line.ends_with('\n') {
                    stderr.push('\n');
                }
            }
        }

        // boxlite enforces the per-exec timeout itself; if it fired, `wait`
        // returns an error whose message names the timeout. We surface that as
        // `timed_out = true` (mirroring `bash`) so the model sees an explicit
        // signal rather than a generic error. Any other wait error degrades to
        // `exit_code = None` with `timed_out = false`.
        let (exit_code, timed_out) = match outcome.execution.wait().await {
            Ok(status) => (Some(status.code()), false),
            Err(e) => {
                let msg = e.to_string();
                let timed_out = is_timeout_error(&msg);
                tracing::warn!(error = %msg, timed_out, "sandbox exec wait failed; reporting None");
                (None, timed_out)
            }
        };

        Ok(RunCodeResult {
            exit_code,
            stdout,
            stderr,
            timed_out,
        })
    }
}

/// Build the [`ExecRequest`] for a `run_code` invocation, resolving the
/// timeout to the per-call value or the [`DEFAULT_EXEC_TIMEOUT_SECS`] backstop.
///
/// Extracted as a pure function so the timeout-resolution behavior is unit
/// testable without a live boxlite VM (the real exec path is `#[ignore]`d in
/// `tests/run_code_session.rs`).
fn build_exec_request(
    command: String,
    args: Vec<String>,
    timeout: Option<Duration>,
) -> ExecRequest {
    let timeout_dur = timeout.unwrap_or_else(|| Duration::from_secs(DEFAULT_EXEC_TIMEOUT_SECS));
    ExecRequest::builder()
        .command(command)
        .args(args)
        .timeout(timeout_dur)
        .build()
}

/// Classify a boxlite `wait` error message as a timeout kill.
///
/// boxlite reports a hit timeout as an error string containing "timeout" or
/// "timed out"; any other message is an ordinary failure, not a timeout.
fn is_timeout_error(msg: &str) -> bool { msg.contains("timeout") || msg.contains("timed out") }

/// Lifecycle hook that destroys per-session sandboxes when their owning
/// session ends.
///
/// Holds the same [`SandboxMap`] as the tool itself; the kernel fires
/// `on_session_end` from `cleanup_process` (see `crates/kernel/src/kernel.rs`).
pub struct SandboxCleanupHook {
    sandboxes: SandboxMap,
}

impl SandboxCleanupHook {
    /// Build a hook that watches the given shared map.
    pub fn new(sandboxes: SandboxMap) -> Self { Self { sandboxes } }
}

#[async_trait]
impl LifecycleHook for SandboxCleanupHook {
    fn name(&self) -> &str { "sandbox-cleanup" }

    async fn on_session_end(&self, ctx: &SessionEndContext) {
        let Some((_, sandbox)) = self.sandboxes.remove(&ctx.session_key) else {
            return;
        };
        let session_key = ctx.session_key;
        // The lifecycle pipeline times each hook out at 5s. `Sandbox::destroy`
        // plus the reclamation retry budget can exceed that, so run it
        // detached — the map entry is already removed above.
        //
        // `destroy` consumes `self`, so the reaper must own the `Sandbox`.
        // When a session ends with an `exec` still in flight, that exec holds
        // a clone of the `Arc` for the whole call; the kernel signal pipeline
        // cancels the turn, dropping the clone moments later. Rather than give
        // up on the first `try_unwrap` (the leak in #1866), retry acquiring
        // ownership with a bounded backoff until the clone clears, then
        // destroy. The loop is bounded, so this task always terminates.
        tokio::spawn(async move {
            let outcome = reclaim_when_idle(
                sandbox,
                RECLAIM_MAX_ATTEMPTS,
                RECLAIM_BACKOFF,
                |inner: Sandbox| async move {
                    if let Err(e) = inner.destroy().await {
                        tracing::warn!(error = %e, "failed to destroy sandbox on session end");
                    }
                },
            )
            .await;
            if let ReclaimOutcome::Exhausted { strong_count } = outcome {
                tracing::warn!(
                    session_key = %session_key,
                    strong_count,
                    "sandbox still in use after reclaim budget exhausted; VM not reclaimed \
                     until process exit (see #1866)"
                );
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_code_params_parses_minimal() {
        let v = serde_json::json!({"command": "echo"});
        let p: RunCodeParams = serde_json::from_value(v).expect("parse");
        assert_eq!(p.command, "echo");
        assert!(p.args.is_empty());
    }

    #[test]
    fn run_code_params_parses_with_args() {
        let v = serde_json::json!({"command": "sh", "args": ["-c", "echo hi"]});
        let p: RunCodeParams = serde_json::from_value(v).expect("parse");
        assert_eq!(p.command, "sh");
        assert_eq!(p.args, vec!["-c", "echo hi"]);
    }

    #[test]
    fn run_code_schema_advertises_required_command() {
        let schema = schemars::schema_for!(RunCodeParams);
        let value = serde_json::to_value(&schema).expect("serialize");
        let required = value
            .get("required")
            .and_then(|r| r.as_array())
            .expect("required array");
        assert!(
            required.iter().any(|v| v.as_str() == Some("command")),
            "command must be required, got: {required:?}"
        );
    }

    #[test]
    fn run_code_sets_default_exec_timeout() {
        // No caller-supplied timeout → the ExecRequest carries the default
        // backstop. Before this change the field was `None` (no `.timeout()`
        // was ever set), so this assertion inverts the pre-change behavior.
        let request = build_exec_request("sh".to_owned(), vec!["-c".to_owned()], None);
        assert_eq!(
            request.timeout,
            Some(Duration::from_secs(DEFAULT_EXEC_TIMEOUT_SECS))
        );
    }

    #[test]
    fn run_code_honors_per_call_timeout() {
        // A caller-supplied timeout is used verbatim; the default const is
        // not consulted.
        let five = Duration::from_secs(5);
        let request = build_exec_request("sh".to_owned(), vec![], Some(five));
        assert_eq!(request.timeout, Some(five));
        assert_ne!(five, Duration::from_secs(DEFAULT_EXEC_TIMEOUT_SECS));
    }

    #[test]
    fn run_code_params_accepts_timeout_forms() {
        // The shared `deserialize_timeout` visitor accepts every shape `bash`
        // does: integer, stringified integer, humantime, and secs/nanos map.
        let cases = [
            (serde_json::json!({"command": "sh", "timeout": 120}), 120),
            (serde_json::json!({"command": "sh", "timeout": "120"}), 120),
            (serde_json::json!({"command": "sh", "timeout": "2m"}), 120),
            (
                serde_json::json!({"command": "sh", "timeout": {"secs": 120, "nanos": 0}}),
                120,
            ),
        ];
        for (json, expected_secs) in cases {
            let p: RunCodeParams = serde_json::from_value(json.clone()).expect("parse");
            assert_eq!(
                p.timeout,
                Some(Duration::from_secs(expected_secs)),
                "unexpected timeout for {json}"
            );
        }
    }

    #[test]
    fn run_code_maps_timeout_error_to_flag() {
        // A boxlite wait-error that names the timeout classifies as timed_out;
        // any other wait error does not.
        assert!(is_timeout_error("exec killed: timeout exceeded"));
        assert!(is_timeout_error("command timed out after 120s"));
        assert!(!is_timeout_error("connection reset by peer"));
    }
}
