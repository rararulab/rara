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

use rara_tool_macro::ToolDef;

use crate::{
    browser::BrowserManagerRef,
    tool::{ToolContext, ToolOutput},
};

/// Navigate back in the active tab's history.
#[derive(ToolDef)]
#[tool(
    name = "browser-navigate-back",
    description = "Navigate back in the active browser tab. Returns a fresh accessibility \
                   snapshot.",
    params_schema = "Self::schema()",
    execute_fn = "self.exec"
)]
pub struct BrowserNavigateBackTool {
    manager: BrowserManagerRef,
}

impl BrowserNavigateBackTool {
    pub fn new(manager: BrowserManagerRef) -> Self { Self { manager } }

    fn schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn exec(
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
