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

//! Navigate back in the active browser tab.

use async_trait::async_trait;

use crate::{
    browser::BrowserManagerRef,
    tool::{AgentTool, ToolContext, ToolOutput},
};

/// Navigate back in the active tab's history.
pub struct BrowserNavigateBackTool {
    manager: BrowserManagerRef,
}

impl BrowserNavigateBackTool {
    pub const NAME: &str = crate::tool_names::BROWSER_NAVIGATE_BACK;

    pub fn new(manager: BrowserManagerRef) -> Self { Self { manager } }
}

#[async_trait]
impl AgentTool for BrowserNavigateBackTool {
    fn name(&self) -> &str { Self::NAME }

    fn description(&self) -> &str {
        "Navigate back in the active browser tab. Returns a fresh accessibility snapshot."
    }

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
        let snapshot = self
            .manager
            .navigate_back()
            .await
            .map_err(|e| anyhow::anyhow!("navigate_back failed: {e}"))?;

        Ok(serde_json::json!({ "snapshot": snapshot }).into())
    }
}
