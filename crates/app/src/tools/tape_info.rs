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

//! Tool for querying tape metadata (entry count, anchors, token usage).

use async_trait::async_trait;
use rara_kernel::{
    memory::TapeService,
    tool::{AgentTool, ToolOutput},
};
use serde_json::json;

/// Returns summary information about the current session tape.
pub struct TapeInfoTool {
    tape_service: TapeService,
}

impl TapeInfoTool {
    pub fn new(tape_service: TapeService) -> Self { Self { tape_service } }
}

#[async_trait]
impl AgentTool for TapeInfoTool {
    fn name(&self) -> &str { "tape-info" }

    fn description(&self) -> &str {
        "Return metadata about the current session tape: entry count, anchors, entries since last \
         anchor, and last known token usage."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        context: &rara_kernel::tool::ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let tape_name = context
            .session_key
            .as_ref()
            .map(|k| k.to_string())
            .ok_or_else(|| anyhow::anyhow!("no session context"))?;

        let info = self
            .tape_service
            .info(&tape_name)
            .await
            .map_err(|e| anyhow::anyhow!("failed to read tape info: {e}"))?;

        let last_anchor_display = info.last_anchor.as_deref().unwrap_or("-");
        let usage_display = info
            .last_token_usage
            .map(|n| n.to_string())
            .unwrap_or_else(|| "unknown".to_owned());

        let output = format!(
            "tape={}\nentries={}\nanchors={}\nlast_anchor={}\nentries_since_last_anchor={}\\
             nlast_token_usage={}",
            info.name,
            info.entries,
            info.anchors,
            last_anchor_display,
            info.entries_since_last_anchor,
            usage_display,
        );

        Ok(json!({ "output": output }).into())
    }
}
