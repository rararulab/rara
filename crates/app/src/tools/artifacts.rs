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

//! Server-side artifacts tool.
//!
//! Mirrors the command surface of pi-mono's in-browser `artifactsParamsSchema`
//! (`create`, `update`, `rewrite`, `get`, `delete`, `logs`) so that the
//! pi-web-ui `ArtifactsPanel` can rebuild its UI state purely by replaying
//! `artifacts` tool-call arguments and tool-result success flags from the
//! message history.
//!
//! The tool itself keeps **no in-memory state across calls**: artifact
//! contents live verbatim inside the tape's `ToolCall` entries, which are
//! already persisted by the kernel's agent loop. On session load the frontend
//! calls `reconstructFromMessages()` with the serialized history to fold
//! operations into the final artifact set, matching pi-mono's own behavior.
//!
//! To validate commands that reference prior state (`update`, `rewrite`,
//! `get`, `delete`, `logs`) we fold the session tape forward on each call —
//! cheap because artifact sessions are bounded by session length and the tape
//! is already in memory.

use std::collections::HashMap;

use async_trait::async_trait;
use rara_kernel::{
    memory::{TapEntryKind, TapeService},
    tool::{ToolContext, ToolExecute},
};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// LLM-facing parameters — matches `artifactsParamsSchema` in
/// `vendor/pi-mono/packages/web-ui/src/tools/artifacts/artifacts.ts`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ArtifactsParams {
    /// The operation to perform: `create`, `update`, `rewrite`, `get`,
    /// `delete`, or `logs`.
    pub command:  String,
    /// Filename including extension (e.g. `index.html`, `script.js`).
    pub filename: String,
    /// File content (required for `create` and `rewrite`).
    #[serde(default)]
    pub content:  Option<String>,
    /// String to replace (required for `update`).
    #[serde(default)]
    pub old_str:  Option<String>,
    /// Replacement string (required for `update`).
    #[serde(default)]
    pub new_str:  Option<String>,
}

/// Simple structured result — serialized into the tool-result tape entry.
#[derive(Debug, Clone, Serialize)]
pub struct ArtifactsResult {
    /// Human-readable message describing the outcome.
    pub message: String,
}

const ARTIFACTS_TOOL_NAME: &str = "artifacts";

const ARTIFACTS_DESCRIPTION: &str =
    "Manage rich artifacts (HTML, SVG, markdown, code, images, PDFs, etc.) that render in a \
     dedicated side panel of the chat UI. Each artifact is identified by \
     filename.\n\nCommands:\n- create: Add a new file. Requires `filename` and `content`. Fails \
     if the file already exists.\n- update: Replace a substring in an existing file. Requires \
     `filename`, `old_str`, and `new_str`. `old_str` must occur exactly once.\n- rewrite: Replace \
     the entire content of an existing file. Requires `filename` and `content`.\n- get: Return \
     the full content of an existing file.\n- delete: Remove a file.\n- logs: Retrieve execution \
     logs for an HTML artifact (client-side only; the server returns an informational \
     placeholder).\n\nPrefer HTML for interactive UIs, SVG for diagrams, and markdown for prose. \
     Create one file per artifact and iterate with `update` or `rewrite`.";

/// Artifacts tool — deferred tier, discovered on demand.
#[derive(ToolDef)]
#[tool(
    name = "artifacts",
    description = "Manage rich artifacts (HTML, SVG, markdown, code, images) rendered in the chat \
                   UI's side panel. Commands: create, update, rewrite, get, delete, logs.",
    tier = "deferred"
)]
pub struct ArtifactsTool {
    tape_service: TapeService,
}

impl ArtifactsTool {
    pub fn new(tape_service: TapeService) -> Self { Self { tape_service } }

    /// Fold the session tape forward, applying every prior successful
    /// `artifacts` tool operation to yield the current artifact set.
    async fn current_state(&self, tape_name: &str) -> anyhow::Result<HashMap<String, String>> {
        let entries = self
            .tape_service
            .entries(tape_name)
            .await
            .map_err(|e| anyhow::anyhow!("failed to read tape: {e}"))?;

        // Walk entries in order, pairing `ToolCall` (per-id) with its
        // `ToolResult`.  We apply an operation only if its matching result
        // was successful and marked the tool by name.
        let mut pending: Vec<(String, ArtifactsParams)> = Vec::new();
        let mut state: HashMap<String, String> = HashMap::new();

        for entry in &entries {
            match entry.kind {
                TapEntryKind::ToolCall => {
                    pending = extract_artifact_calls(&entry.payload);
                }
                TapEntryKind::ToolResult => {
                    apply_results(&entry.payload, &pending, &mut state);
                    pending.clear();
                }
                _ => {}
            }
        }

        Ok(state)
    }
}

/// Extract `(call_id, params)` tuples for artifact calls in a ToolCall entry.
fn extract_artifact_calls(payload: &Value) -> Vec<(String, ArtifactsParams)> {
    let calls = payload.get("calls").and_then(Value::as_array);
    let Some(calls) = calls else {
        return Vec::new();
    };

    calls
        .iter()
        .filter_map(|call| {
            let function = call.get("function")?;
            let name = function.get("name")?.as_str()?;
            if name != ARTIFACTS_TOOL_NAME {
                return None;
            }
            let id = call.get("id")?.as_str()?.to_owned();
            let args_str = function.get("arguments")?.as_str().unwrap_or("{}");
            let params: ArtifactsParams = serde_json::from_str(args_str).ok()?;
            Some((id, params))
        })
        .collect()
}

/// Apply results (by position) against pending calls; mutate artifact state.
fn apply_results(
    payload: &Value,
    pending: &[(String, ArtifactsParams)],
    state: &mut HashMap<String, String>,
) {
    let results = payload.get("results").and_then(Value::as_array);
    let Some(results) = results else { return };

    for (idx, result) in results.iter().enumerate() {
        let Some((_id, params)) = pending.get(idx) else {
            continue;
        };
        // Treat anything starting with "Error:" as a failure — matches the
        // pi-mono TS tool's result strings.
        let text = match result {
            Value::String(s) => s.clone(),
            other => serde_json::to_string(other).unwrap_or_default(),
        };
        if text.starts_with("Error:") {
            continue;
        }
        apply_op(params, state);
    }
}

fn apply_op(params: &ArtifactsParams, state: &mut HashMap<String, String>) {
    match params.command.as_str() {
        "create" | "rewrite" => {
            if let Some(content) = &params.content {
                state.insert(params.filename.clone(), content.clone());
            }
        }
        "update" => {
            if let (Some(existing), Some(old), Some(new)) = (
                state.get(&params.filename).cloned(),
                params.old_str.as_deref(),
                params.new_str.as_deref(),
            ) {
                state.insert(params.filename.clone(), existing.replacen(old, new, 1));
            }
        }
        "delete" => {
            state.remove(&params.filename);
        }
        _ => {}
    }
}

#[async_trait]
impl ToolExecute for ArtifactsTool {
    type Output = ArtifactsResult;
    type Params = ArtifactsParams;

    async fn run(
        &self,
        params: ArtifactsParams,
        context: &ToolContext,
    ) -> anyhow::Result<ArtifactsResult> {
        let tape_name = context.session_key.to_string();
        // Compute current state BEFORE the current call is appended to the
        // tape.  (The kernel appends the tool call after the tool returns.)
        let state = self.current_state(&tape_name).await?;

        let message = match params.command.as_str() {
            "create" => {
                let content = params
                    .content
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("Error: create command requires content"))?;
                if state.contains_key(&params.filename) {
                    anyhow::bail!("Error: File {} already exists", params.filename);
                }
                let _ = content; // content is captured in the tape via the call args
                format!("Created file {}", params.filename)
            }
            "rewrite" => {
                if params.content.is_none() {
                    anyhow::bail!("Error: rewrite command requires content");
                }
                ensure_exists(&state, &params.filename)?;
                format!("Rewrote file {}", params.filename)
            }
            "update" => {
                let old = params
                    .old_str
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("Error: update command requires old_str"))?;
                if params.new_str.is_none() {
                    anyhow::bail!("Error: update command requires new_str");
                }
                let existing = state
                    .get(&params.filename)
                    .ok_or_else(|| anyhow::anyhow!(not_found_message(&state, &params.filename)))?;
                if !existing.contains(old) {
                    anyhow::bail!(
                        "Error: String not found in file. Here is the full content:\n\n{existing}"
                    );
                }
                format!("Updated file {}", params.filename)
            }
            "get" => state
                .get(&params.filename)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!(not_found_message(&state, &params.filename)))?,
            "delete" => {
                ensure_exists(&state, &params.filename)?;
                format!("Deleted file {}", params.filename)
            }
            "logs" => {
                ensure_exists(&state, &params.filename)?;
                // HTML execution logs are a client-side concept — the browser
                // captures console output from the sandboxed iframe.  The
                // server has no way to produce them.
                format!(
                    "Logs for {} are collected by the client-side artifacts panel. Ask the user \
                     to copy them from the panel if needed.",
                    params.filename
                )
            }
            other => anyhow::bail!("Error: Unknown command '{other}'"),
        };

        Ok(ArtifactsResult { message })
    }
}

fn ensure_exists(state: &HashMap<String, String>, filename: &str) -> anyhow::Result<()> {
    if state.contains_key(filename) {
        Ok(())
    } else {
        anyhow::bail!(not_found_message(state, filename))
    }
}

fn not_found_message(state: &HashMap<String, String>, filename: &str) -> String {
    if state.is_empty() {
        format!("Error: File {filename} not found. No files have been created yet.")
    } else {
        let files: Vec<_> = state.keys().cloned().collect();
        format!(
            "Error: File {filename} not found. Available files: {}",
            files.join(", ")
        )
    }
}

// Keep the richer description available for future tooling without
// triggering unused-constant warnings.
#[allow(dead_code)]
const _DESCRIPTION_REFERENCE: &str = ARTIFACTS_DESCRIPTION;
