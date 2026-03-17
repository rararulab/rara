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

//! Close all browser tabs.

use async_trait::async_trait;

use crate::{
    browser::BrowserManagerRef,
    tool::{AgentTool, ToolContext, ToolOutput},
};

/// Close all browser tabs and reset the browser state.
pub struct BrowserCloseTool {
    manager: BrowserManagerRef,
}

impl BrowserCloseTool {
    pub const NAME: &str = crate::tool_names::BROWSER_CLOSE;

    pub fn new(manager: BrowserManagerRef) -> Self { Self { manager } }
}

#[async_trait]
impl AgentTool for BrowserCloseTool {
    fn name(&self) -> &str { Self::NAME }

    fn description(&self) -> &str { "Close all browser tabs and reset the browser state." }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        _context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        self.manager
            .close_all()
            .await
            .map_err(|e| anyhow::anyhow!("close_all failed: {e}"))?;

        Ok(serde_json::json!({ "tabs": [] }).into())
    }
}
