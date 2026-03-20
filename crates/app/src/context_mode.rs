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

//! Context-mode output interceptor.
//!
//! Indexes large tool outputs into the context-mode MCP server and replaces
//! them with compact references.

use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use async_trait::async_trait;
use rara_kernel::tool::{OutputInterceptor, ToolOutput};
use rara_mcp::manager::mgr::McpManager;
use serde::Serialize;
use tracing::{debug, warn};

/// Name of the context-mode MCP server in the registry.
const SERVER_NAME: &str = "context-mode";

/// Default output size threshold in bytes (8 KB).
const DEFAULT_THRESHOLD: usize = 8 * 1024;

/// Static system prompt fragment injected when context-mode is active.
const CONTEXT_MODE_PROMPT_FRAGMENT: &str =
    "\
[Context Mode]\nSome tool outputs exceed the context threshold and are automatically \
     indexed.\nWhen you see `[INDEXED]` in a tool result, the output was captured successfully \
     and stored in a searchable index.\n\nIMPORTANT: Do NOT re-invoke the same tool to get the \
     \"real\" content — the indexed result IS the real content, just compressed. Use the \
     `ctx_search` tool to retrieve specific parts. Do NOT use the `tape` tool's search action for \
     indexed content — always use `ctx_search`.";

/// Monotonic counter to ensure unique index IDs under concurrent execution.
static INDEX_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Runtime statistics for the context-mode interceptor.
pub struct InterceptorStats {
    /// Number of outputs successfully indexed.
    pub intercepted_count: AtomicU64,
    /// Number of index failures (fell back to original output).
    pub fallback_count:    AtomicU64,
    /// Total original payload bytes before indexing.
    pub bytes_before:      AtomicU64,
    /// Total compact reference bytes after indexing.
    pub bytes_after:       AtomicU64,
}

impl InterceptorStats {
    fn new() -> Self {
        Self {
            intercepted_count: AtomicU64::new(0),
            fallback_count:    AtomicU64::new(0),
            bytes_before:      AtomicU64::new(0),
            bytes_after:       AtomicU64::new(0),
        }
    }

    /// Take a point-in-time snapshot of the stats.
    pub fn stats(&self) -> InterceptorStatsSnapshot {
        InterceptorStatsSnapshot {
            intercepted_count: self.intercepted_count.load(Ordering::Relaxed),
            fallback_count:    self.fallback_count.load(Ordering::Relaxed),
            bytes_before:      self.bytes_before.load(Ordering::Relaxed),
            bytes_after:       self.bytes_after.load(Ordering::Relaxed),
        }
    }
}

/// Point-in-time snapshot of [`InterceptorStats`].
#[derive(Debug, Clone, Serialize)]
pub struct InterceptorStatsSnapshot {
    pub intercepted_count: u64,
    pub fallback_count:    u64,
    pub bytes_before:      u64,
    pub bytes_after:       u64,
}

pub struct ContextModeInterceptor {
    manager:    McpManager,
    threshold:  usize,
    stats:      Arc<InterceptorStats>,
    bypass_set: HashSet<String>,
}

impl ContextModeInterceptor {
    pub fn new(manager: McpManager) -> Self {
        Self {
            manager,
            threshold: DEFAULT_THRESHOLD,
            stats: Arc::new(InterceptorStats::new()),
            bypass_set: HashSet::new(),
        }
    }

    pub fn with_threshold(mut self, threshold: usize) -> Self {
        self.threshold = threshold;
        self
    }

    /// Set the tool names that should bypass interception.
    ///
    /// Built from `AgentTool::bypass_output_interceptor()` at startup. Note:
    /// dynamically registered MCP tools are not included — they are always
    /// eligible for interception, which is the correct default for most MCP
    /// tools.
    pub fn with_bypass_set(mut self, set: HashSet<String>) -> Self {
        self.bypass_set = set;
        self
    }

    /// Returns a shared handle to the runtime statistics.
    pub fn stats(&self) -> Arc<InterceptorStats> { Arc::clone(&self.stats) }
}

#[async_trait]
impl OutputInterceptor for ContextModeInterceptor {
    async fn intercept(&self, tool_name: &str, output: ToolOutput) -> ToolOutput {
        // Intercept everything by default — skip only tools whose output is
        // binary, always small, or must be returned verbatim.
        if self.bypass_set.contains(tool_name) {
            return output;
        }

        // Only large payloads are worth indexing.
        let json_str = output.json.to_string();
        if json_str.len() <= self.threshold {
            return output;
        }

        let seq = INDEX_COUNTER.fetch_add(1, Ordering::Relaxed);
        let index_id = format!(
            "{tool_name}_{}_{}",
            chrono::Utc::now().timestamp_millis(),
            seq,
        );
        let index_params = serde_json::json!({
            "content": json_str,
            "id": &index_id,
        });

        match self
            .manager
            .call_tool(SERVER_NAME, "index", Some(index_params))
            .await
        {
            Ok(_result) => {
                debug!(
                    tool = tool_name,
                    index_id = &index_id,
                    original_bytes = json_str.len(),
                    "indexed large tool output via context-mode"
                );
                let summary = build_summary(tool_name, &json_str);
                let replacement = serde_json::json!({
                    "indexed": true,
                    "index_id": &index_id,
                    "original_bytes": json_str.len(),
                    "summary": summary,
                });
                let replacement_str = replacement.to_string();

                self.stats.intercepted_count.fetch_add(1, Ordering::Relaxed);
                self.stats
                    .bytes_before
                    .fetch_add(json_str.len() as u64, Ordering::Relaxed);
                self.stats
                    .bytes_after
                    .fetch_add(replacement_str.len() as u64, Ordering::Relaxed);

                ToolOutput::from(replacement)
            }
            Err(e) => {
                warn!(
                    tool = tool_name,
                    error = %e,
                    "context-mode index failed, returning original output"
                );
                self.stats.fallback_count.fetch_add(1, Ordering::Relaxed);
                output
            }
        }
    }

    fn system_prompt_fragment(&self) -> Option<&str> { Some(CONTEXT_MODE_PROMPT_FRAGMENT) }
}

/// Extract top-level JSON keys with type/size hints for a compact preview.
fn extract_structure_preview(json_str: &str) -> String {
    /// Soft limit on structure preview length. The actual output may slightly
    /// exceed this due to the final key being added before the check triggers.
    const MAX_PREVIEW_LEN: usize = 200;

    let Ok(value) = serde_json::from_str::<serde_json::Value>(json_str) else {
        let bytes = json_str.len();
        let lines = json_str.chars().filter(|&c| c == '\n').count() + 1;
        return format!("{bytes} bytes, ~{lines} lines");
    };

    let Some(obj) = value.as_object() else {
        return match &value {
            serde_json::Value::Array(arr) => format!("[...{} items]", arr.len()),
            other => {
                let s = other.to_string();
                if s.len() > 80 {
                    format!("{}...", &s[..s.floor_char_boundary(80)])
                } else {
                    s
                }
            }
        };
    };

    let mut parts = Vec::new();
    let mut total_len = 4; // "{ " + " }"
    for (key, val) in obj {
        let hint = match val {
            serde_json::Value::Array(arr) => format!("[...{} items]", arr.len()),
            serde_json::Value::Object(map) => format!("{{...{} keys}}", map.len()),
            serde_json::Value::String(s) => {
                if s.len() > 50 {
                    format!("\"{}...\"", &s[..s.floor_char_boundary(50)])
                } else {
                    format!("\"{s}\"")
                }
            }
            other => other.to_string(),
        };
        let part = format!("{key}: {hint}");
        total_len += part.len() + 2;
        if total_len > MAX_PREVIEW_LEN {
            parts.push("...".to_owned());
            break;
        }
        parts.push(part);
    }

    format!("{{ {} }}", parts.join(", "))
}

/// Build a human-readable summary for an indexed tool output.
fn build_summary(tool_name: &str, json_str: &str) -> String {
    let bytes = json_str.len();
    let structure = extract_structure_preview(json_str);
    format!(
        "[INDEXED] {tool_name} output ({bytes} bytes).\nStructure: {structure}\nUse the \
         `ctx_search` tool to retrieve details. Do NOT re-call {tool_name}."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_preview_object_with_array() {
        let json = r#"{"servers":[{"name":"a"},{"name":"b"}],"status":"ok"}"#;
        let preview = extract_structure_preview(json);
        assert!(preview.contains("servers: [...2 items]"));
        assert!(preview.contains("status: \"ok\""));
    }

    #[test]
    fn extract_preview_fallback_on_invalid_json() {
        let preview = extract_structure_preview("not json {{{");
        assert!(preview.contains("bytes"));
    }

    #[test]
    fn extract_preview_caps_length() {
        let mut obj = serde_json::Map::new();
        for i in 0..50 {
            obj.insert(format!("key_{i}"), serde_json::json!("value"));
        }
        let json = serde_json::to_string(&obj).expect("serialization should succeed");
        let preview = extract_structure_preview(&json);
        assert!(preview.len() <= 250);
    }

    #[test]
    fn build_summary_includes_indexed_tag() {
        let json = r#"{"data":[1,2,3]}"#;
        let summary = build_summary("test-tool", json);
        assert!(summary.contains("[INDEXED]"));
        assert!(summary.contains("ctx_search"));
        assert!(summary.contains("Do NOT re-call"));
    }
}
