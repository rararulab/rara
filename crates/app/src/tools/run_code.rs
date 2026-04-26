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

use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use futures::StreamExt;
use rara_kernel::{
    io::{StreamEvent, StreamHandle},
    lifecycle::{LifecycleHook, SessionEndContext},
    session::SessionKey,
    tool::{ToolContext, ToolExecute},
};
use rara_sandbox::{ExecRequest, Sandbox, SandboxConfig};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::SandboxToolConfig;

/// Per-session sandbox lookup table.
///
/// Wrapped in `Arc` so the tool and the cleanup hook share a single map.
pub type SandboxMap = Arc<DashMap<SessionKey, Arc<Mutex<Sandbox>>>>;

/// Input parameters for the `run_code` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RunCodeParams {
    /// Executable to invoke inside the sandbox (e.g. `"sh"`, `"python"`).
    command: String,
    /// Arguments to pass, in order. Empty vec means no args.
    #[serde(default)]
    args:    Vec<String>,
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
        if let Some(existing) = self.sandboxes.get(&session_key) {
            return Ok(Arc::clone(existing.value()));
        }

        let cfg = self.config.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "run_code is unavailable: `sandbox.default_rootfs_image` is not set in \
                 config.yaml. Add a `sandbox:` block (see config.example.yaml) and restart."
            )
        })?;

        // Use entry() to avoid the create-twice race: if another task
        // raced us, only one Sandbox::create call wins.
        let entry = self.sandboxes.entry(session_key);
        let arc = match entry {
            dashmap::mapref::entry::Entry::Occupied(o) => Arc::clone(o.get()),
            dashmap::mapref::entry::Entry::Vacant(v) => {
                let sandbox = Sandbox::create(
                    SandboxConfig::builder()
                        .rootfs_image(cfg.default_rootfs_image.clone())
                        .build(),
                )
                .await
                .map_err(|e| anyhow::anyhow!("failed to create sandbox: {e}"))?;
                let arc = Arc::new(Mutex::new(sandbox));
                v.insert(Arc::clone(&arc));
                arc
            }
        };
        Ok(arc)
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
        let request = ExecRequest::builder()
            .command(params.command)
            .args(params.args)
            .build();

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

        let exit_code = outcome.execution.wait().await.ok().map(|s| s.code());

        Ok(RunCodeResult {
            exit_code,
            stdout,
            stderr,
        })
    }
}

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
        // can take longer (boxlite tears down the VM), so spawn it
        // detached — the map entry is already removed, and a leaked box
        // is preferable to blocking subsequent session teardown.
        tokio::spawn(async move {
            // `destroy` consumes `self`; pull the inner Sandbox out of
            // the Arc<Mutex<…>>. If another task is still mid-exec, we
            // bail out: the kernel's signal pipeline has already
            // cancelled the turn, but the Sandbox would be leaked
            // anyway since `destroy` cannot run on a borrowed handle.
            let inner = match Arc::try_unwrap(sandbox) {
                Ok(mutex) => mutex.into_inner(),
                Err(arc) => {
                    tracing::warn!(
                        session_key = %session_key,
                        strong_count = Arc::strong_count(&arc),
                        "sandbox still in use at session end; leaking VM until process exit"
                    );
                    return;
                }
            };
            if let Err(e) = inner.destroy().await {
                tracing::warn!(error = %e, "failed to destroy sandbox on session end");
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
}
