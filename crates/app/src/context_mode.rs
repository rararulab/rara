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

use std::collections::HashSet;

use async_trait::async_trait;
use rara_kernel::tool::{OutputInterceptor, ToolOutput};
use rara_mcp::manager::mgr::McpManager;
use tracing::{debug, warn};

/// Name of the context-mode MCP server in the registry.
const SERVER_NAME: &str = "context-mode";

/// Default output size threshold in bytes.
const DEFAULT_THRESHOLD: usize = 4096;

pub struct ContextModeInterceptor {
    manager:    McpManager,
    threshold:  usize,
    skip_tools: HashSet<String>,
}

impl ContextModeInterceptor {
    pub fn new(manager: McpManager) -> Self {
        let mut skip_tools = HashSet::new();
        // Skip context-mode's own tools to avoid recursive indexing loops.
        for suffix in [
            "execute",
            "execute_file",
            "search",
            "index",
            "fetch_and_index",
            "batch_execute",
            "stats",
        ] {
            skip_tools.insert(format!("context-mode__{suffix}"));
        }

        Self {
            manager,
            threshold: DEFAULT_THRESHOLD,
            skip_tools,
        }
    }

    pub fn with_threshold(mut self, threshold: usize) -> Self {
        self.threshold = threshold;
        self
    }
}

#[async_trait]
impl OutputInterceptor for ContextModeInterceptor {
    async fn intercept(&self, tool_name: &str, output: ToolOutput) -> ToolOutput {
        // Never intercept context-mode's own tools.
        if self.skip_tools.contains(tool_name) {
            return output;
        }

        // Only large payloads are worth indexing.
        let json_str = output.json.to_string();
        if json_str.len() <= self.threshold {
            return output;
        }

        let index_id = format!("{tool_name}_{}", chrono::Utc::now().timestamp_millis());
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
                ToolOutput::from(serde_json::json!({
                    "indexed": true,
                    "index_id": &index_id,
                    "original_bytes": json_str.len(),
                    "summary": truncate_for_summary(&json_str, 200),
                    "hint": "Use the context-mode search tool with relevant queries to retrieve details from this output."
                }))
            }
            Err(e) => {
                warn!(
                    tool = tool_name,
                    error = %e,
                    "context-mode index failed, returning original output"
                );
                output
            }
        }
    }
}

fn truncate_for_summary(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        format!("{}...", s.chars().take(max_len).collect::<String>())
    }
}
