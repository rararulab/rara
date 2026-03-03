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

//! AgentFS-backed tool call recording.
//!
//! Provides persistent tool call audit via `agentfs-sdk`.

use std::sync::Arc;

use agentfs_sdk::{AgentFS, AgentFSOptions};
use async_trait::async_trait;

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

/// Initialize an AgentFS instance with a SQLite database at the given data
/// directory.
///
/// Creates the directory structure if it doesn't exist.
pub async fn init_agentfs(data_dir: &std::path::Path) -> anyhow::Result<AgentFS> {
    let db_path = data_dir.join("agentfs").join("kernel.db");
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let path_str = db_path.to_string_lossy().to_string();
    let opts = AgentFSOptions::with_path(path_str);
    AgentFS::open(opts)
        .await
        .map_err(|e| anyhow::anyhow!("AgentFS init failed: {e}"))
}

// ---------------------------------------------------------------------------
// Tool call recorder
// ---------------------------------------------------------------------------

/// Tool call recorder backed by AgentFS tool tracking.
pub struct AgentFsToolCallRecorder {
    agentfs: Arc<AgentFS>,
}

impl AgentFsToolCallRecorder {
    pub fn new(agentfs: Arc<AgentFS>) -> Self { Self { agentfs } }
}

#[async_trait]
impl rara_kernel::audit::ToolCallRecorder for AgentFsToolCallRecorder {
    async fn record_tool_call(
        &self,
        _agent_id: rara_kernel::process::AgentId,
        tool_name: &str,
        args: &serde_json::Value,
        result: &serde_json::Value,
        success: bool,
        _duration_ms: u64,
    ) {
        match self
            .agentfs
            .tools
            .start(tool_name, Some(args.clone()))
            .await
        {
            Ok(call_id) => {
                if success {
                    let _ = self
                        .agentfs
                        .tools
                        .success(call_id, Some(result.clone()))
                        .await;
                } else {
                    let _ = self.agentfs.tools.error(call_id, &result.to_string()).await;
                }
            }
            Err(e) => {
                tracing::warn!("AgentFS tool call recording failed: {e}");
            }
        }
    }
}
