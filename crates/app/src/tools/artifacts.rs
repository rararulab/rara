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

use std::{
    collections::{BTreeSet, HashMap},
    sync::Arc,
};

use async_trait::async_trait;
use rara_kernel::{
    memory::{TapEntryKind, TapeService},
    tool::{ToolContext, ToolExecute},
};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex as AsyncMutex;

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

/// Per-session in-flight overlay for artifact operations within a single
/// tool-call batch. The kernel persists one `ToolCall` tape entry covering
/// an entire wave before dispatching its tools, and only appends the
/// matching `ToolResult` after every tool in the wave has returned. Between
/// those two points an artifact call cannot see the effects of earlier
/// artifact calls in the same wave by replaying the tape alone — it would
/// miss every in-flight operation. The overlay bridges that gap by
/// recording the effect of each successful artifact call so the next one in
/// the same batch validates against the merged state.
#[derive(Default)]
struct InflightOverlay {
    /// Set of tool-call IDs that make up the current in-flight batch.
    /// Reset whenever a new batch is detected.
    batch_ids: BTreeSet<String>,
    /// Artifact writes accumulated within the current batch, merged on top
    /// of the committed tape state.
    writes:    HashMap<String, ArtifactWrite>,
}

/// Recorded effect of a successful in-flight artifact call — either a write
/// (with content) or a delete (no content).
#[derive(Clone)]
enum ArtifactWrite {
    /// Set or replace the file content.
    Set(String),
    /// Remove the file.
    Delete,
}

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
    /// Per-session overlay keyed by tape name. Holds successful writes for
    /// the current in-flight tool-call batch so sibling artifact calls can
    /// validate against merged state.
    inflight:     Arc<AsyncMutex<HashMap<String, InflightOverlay>>>,
}

impl ArtifactsTool {
    pub fn new(tape_service: TapeService) -> Self {
        Self {
            tape_service,
            inflight: Arc::new(AsyncMutex::new(HashMap::new())),
        }
    }

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
        let mut pending: Vec<ArtifactCall> = Vec::new();
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

    /// Extract the set of tool-call IDs for the most recent ToolCall entry
    /// that has not yet been paired with a ToolResult. That entry, when
    /// present, is the current in-flight batch dispatched by the kernel.
    async fn current_batch_call_ids(&self, tape_name: &str) -> anyhow::Result<BTreeSet<String>> {
        let entries = self
            .tape_service
            .entries(tape_name)
            .await
            .map_err(|e| anyhow::anyhow!("failed to read tape: {e}"))?;

        let mut last_batch: Option<BTreeSet<String>> = None;
        for entry in &entries {
            match entry.kind {
                TapEntryKind::ToolCall => {
                    last_batch = Some(extract_all_call_ids(&entry.payload));
                }
                TapEntryKind::ToolResult => {
                    last_batch = None;
                }
                _ => {}
            }
        }
        Ok(last_batch.unwrap_or_default())
    }

    /// Return artifact state with the current in-flight batch overlaid on
    /// top of persisted state. Mutates the per-session overlay to clear
    /// stale entries when a new batch is detected.
    async fn effective_state(
        &self,
        tape_name: &str,
        current_call_id: Option<&str>,
    ) -> anyhow::Result<HashMap<String, String>> {
        let mut state = self.current_state(tape_name).await?;

        let batch_ids = self.current_batch_call_ids(tape_name).await?;
        let mut inflight = self.inflight.lock().await;
        let overlay = inflight.entry(tape_name.to_owned()).or_default();

        // Detect batch rollover: if the stored batch's IDs differ from the
        // tape's current unpaired batch (or the current call is not part of
        // the stored batch), the overlay is stale and must be dropped.
        let current_in_batch = current_call_id
            .map(|id| overlay.batch_ids.contains(id))
            .unwrap_or(false);
        if overlay.batch_ids != batch_ids || !current_in_batch {
            overlay.batch_ids = batch_ids;
            overlay.writes.clear();
        }

        for (filename, write) in &overlay.writes {
            match write {
                ArtifactWrite::Set(content) => {
                    state.insert(filename.clone(), content.clone());
                }
                ArtifactWrite::Delete => {
                    state.remove(filename);
                }
            }
        }

        Ok(state)
    }

    /// Record a successful artifact write into the in-flight overlay so
    /// sibling calls within the same batch observe it.
    async fn record_overlay(&self, tape_name: &str, write: (String, ArtifactWrite)) {
        let mut inflight = self.inflight.lock().await;
        if let Some(overlay) = inflight.get_mut(tape_name) {
            overlay.writes.insert(write.0, write.1);
        }
    }
}

/// Position in the parent ToolCall entry's `calls` array, paired with the
/// parsed artifact params. Preserving the original index lets `apply_results`
/// look up the matching entry in the `results` array even when the batch
/// mixes artifact calls with other tools.
struct ArtifactCall {
    idx:    usize,
    #[allow(dead_code)]
    id:     String,
    params: ArtifactsParams,
}

/// Extract artifact calls from a ToolCall entry, preserving each call's
/// index in the full `calls` array so results can be matched positionally
/// against the matching ToolResult entry.
fn extract_artifact_calls(payload: &Value) -> Vec<ArtifactCall> {
    let calls = payload.get("calls").and_then(Value::as_array);
    let Some(calls) = calls else {
        return Vec::new();
    };

    calls
        .iter()
        .enumerate()
        .filter_map(|(idx, call)| {
            let function = call.get("function")?;
            let name = function.get("name")?.as_str()?;
            if name != ARTIFACTS_TOOL_NAME {
                return None;
            }
            let id = call.get("id")?.as_str()?.to_owned();
            let args_str = function.get("arguments")?.as_str().unwrap_or("{}");
            let params: ArtifactsParams = serde_json::from_str(args_str).ok()?;
            Some(ArtifactCall { idx, id, params })
        })
        .collect()
}

/// Extract every call's id from a ToolCall payload, regardless of tool name.
/// Used to identify the membership of the current in-flight batch.
fn extract_all_call_ids(payload: &Value) -> BTreeSet<String> {
    payload
        .get("calls")
        .and_then(Value::as_array)
        .map(|calls| {
            calls
                .iter()
                .filter_map(|c| c.get("id").and_then(Value::as_str).map(ToOwned::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Apply each artifact call's result from a ToolResult entry against the
/// state map, indexing the results array by the call's original position in
/// the parent ToolCall entry (not by position within `pending`). A failed
/// result is skipped; both pi-mono-style `Error:` string prefixes and rara's
/// `{"error": "..."}` JSON shape are recognised as failures.
fn apply_results(payload: &Value, pending: &[ArtifactCall], state: &mut HashMap<String, String>) {
    let results = payload.get("results").and_then(Value::as_array);
    let Some(results) = results else { return };

    for call in pending {
        let Some(result) = results.get(call.idx) else {
            continue;
        };
        if is_failure_result(result) {
            continue;
        }
        apply_op(&call.params, state);
    }
}

/// Recognise the two failure shapes the agent loop emits for tool results:
///
/// * a bare string starting with `Error:` (pi-mono convention carried over from
///   the TS tool), and
/// * a JSON object with an `error` key (produced by the kernel when the tool's
///   `anyhow::Error` is captured into `ToolOutput`).
fn is_failure_result(result: &Value) -> bool {
    match result {
        Value::String(s) => s.starts_with("Error:"),
        Value::Object(map) => map.contains_key("error"),
        _ => false,
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
        // Compute state from the committed tape, overlaid with any in-flight
        // writes from earlier artifact calls in the current tool-call batch.
        // The kernel persists the wave's ToolCall entry before dispatching
        // its tools and appends the matching ToolResult only after the whole
        // wave returns, so replaying the tape alone cannot see sibling
        // writes from the same batch — the overlay bridges that gap.
        let state = self
            .effective_state(&tape_name, context.tool_call_id.as_deref())
            .await?;

        let mut overlay_update: Option<(String, ArtifactWrite)> = None;

        let message = match params.command.as_str() {
            "create" => {
                let content = params
                    .content
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("Error: create command requires content"))?;
                if state.contains_key(&params.filename) {
                    anyhow::bail!("Error: File {} already exists", params.filename);
                }
                overlay_update = Some((
                    params.filename.clone(),
                    ArtifactWrite::Set(content.to_owned()),
                ));
                format!("Created file {}", params.filename)
            }
            "rewrite" => {
                let content = params
                    .content
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("Error: rewrite command requires content"))?;
                ensure_exists(&state, &params.filename)?;
                overlay_update = Some((
                    params.filename.clone(),
                    ArtifactWrite::Set(content.to_owned()),
                ));
                format!("Rewrote file {}", params.filename)
            }
            "update" => {
                let old = params
                    .old_str
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("Error: update command requires old_str"))?;
                let new = params
                    .new_str
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("Error: update command requires new_str"))?;
                let existing = state
                    .get(&params.filename)
                    .ok_or_else(|| anyhow::anyhow!(not_found_message(&state, &params.filename)))?;
                if !existing.contains(old) {
                    anyhow::bail!(
                        "Error: String not found in file. Here is the full content:\n\n{existing}"
                    );
                }
                let updated = existing.replacen(old, new, 1);
                overlay_update = Some((params.filename.clone(), ArtifactWrite::Set(updated)));
                format!("Updated file {}", params.filename)
            }
            "get" => state
                .get(&params.filename)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!(not_found_message(&state, &params.filename)))?,
            "delete" => {
                ensure_exists(&state, &params.filename)?;
                overlay_update = Some((params.filename.clone(), ArtifactWrite::Delete));
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

        if let Some(update) = overlay_update {
            self.record_overlay(&tape_name, update).await;
        }

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
