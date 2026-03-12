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

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use rara_kernel::tool::{OutputInterceptor, ToolOutput};
use rara_mcp::manager::mgr::McpManager;
use serde::Serialize;
use tracing::{debug, warn};

/// Name of the context-mode MCP server in the registry.
const SERVER_NAME: &str = "context-mode";

/// Tool name prefix used by context-mode MCP server.
const TOOL_PREFIX: &str = "context-mode__";

/// Tools excluded from interception (their output is binary, always small,
/// or must be returned verbatim to the agent).
///
/// Everything else is intercepted by default — any tool that can produce
/// large textual output will be indexed into context-mode when it exceeds
/// the size threshold.
const NON_INTERCEPTABLE_TOOLS: &[&str] = &[
    // Binary / image tools
    "send_image",
    "screenshot",
    "set_avatar",
    // Scheduler tools (tiny confirmation output)
    "schedule_once",
    "schedule_interval",
    "schedule_cron",
    "schedule_remove",
    "schedule_list",
    // Small metadata tools
    "settings",
    "session_info",
    "tape_info",
    "tape_handoff",
    // MCP server admin (small output)
    "install_mcp_server",
    "remove_mcp_server",
    "list_mcp_servers",
    // Write-only / confirmation-only tools
    "send_email",
    "update_soul_state",
    "evolve_soul",
    "write_user_note",
    "distill_user_notes",
];

/// Default output size threshold in bytes (32 KB).
const DEFAULT_THRESHOLD: usize = 32 * 1024;

/// Monotonic counter to ensure unique index IDs under concurrent execution.
static INDEX_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Runtime statistics for the context-mode interceptor.
pub struct InterceptorStats {
    /// Number of outputs successfully indexed.
    pub intercepted_count: AtomicU64,
    /// Number of index failures (fell back to original output).
    pub fallback_count: AtomicU64,
    /// Total original payload bytes before indexing.
    pub bytes_before: AtomicU64,
    /// Total compact reference bytes after indexing.
    pub bytes_after: AtomicU64,
}

impl InterceptorStats {
    fn new() -> Self {
        Self {
            intercepted_count: AtomicU64::new(0),
            fallback_count: AtomicU64::new(0),
            bytes_before: AtomicU64::new(0),
            bytes_after: AtomicU64::new(0),
        }
    }

    /// Take a point-in-time snapshot of the stats.
    pub fn stats(&self) -> InterceptorStatsSnapshot {
        InterceptorStatsSnapshot {
            intercepted_count: self.intercepted_count.load(Ordering::Relaxed),
            fallback_count: self.fallback_count.load(Ordering::Relaxed),
            bytes_before: self.bytes_before.load(Ordering::Relaxed),
            bytes_after: self.bytes_after.load(Ordering::Relaxed),
        }
    }
}

/// Point-in-time snapshot of [`InterceptorStats`].
#[derive(Debug, Clone, Serialize)]
pub struct InterceptorStatsSnapshot {
    pub intercepted_count: u64,
    pub fallback_count: u64,
    pub bytes_before: u64,
    pub bytes_after: u64,
}

pub struct ContextModeInterceptor {
    manager:   McpManager,
    threshold: usize,
    stats:     Arc<InterceptorStats>,
}

impl ContextModeInterceptor {
    pub fn new(manager: McpManager) -> Self {
        Self {
            manager,
            threshold: DEFAULT_THRESHOLD,
            stats: Arc::new(InterceptorStats::new()),
        }
    }

    pub fn with_threshold(mut self, threshold: usize) -> Self {
        self.threshold = threshold;
        self
    }

    /// Returns a shared handle to the runtime statistics.
    pub fn stats(&self) -> Arc<InterceptorStats> {
        Arc::clone(&self.stats)
    }
}

#[async_trait]
impl OutputInterceptor for ContextModeInterceptor {
    async fn intercept(&self, tool_name: &str, output: ToolOutput) -> ToolOutput {
        // Intercept everything by default — skip only tools whose output is
        // binary, always small, or must be returned verbatim.
        if NON_INTERCEPTABLE_TOOLS.contains(&tool_name) {
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
                self.stats.bytes_before.fetch_add(json_str.len() as u64, Ordering::Relaxed);
                self.stats.bytes_after.fetch_add(replacement_str.len() as u64, Ordering::Relaxed);

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
}

/// Build a human-readable summary instead of truncating raw JSON.
fn build_summary(tool_name: &str, json_str: &str) -> String {
    let bytes = json_str.len();
    let lines = json_str.chars().filter(|&c| c == '\n').count() + 1;
    format!(
        "{tool_name} output: {bytes} bytes, ~{lines} lines. Use context-mode search to retrieve specific content.",
    )
}
