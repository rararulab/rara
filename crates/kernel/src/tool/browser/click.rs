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

//! Click an element in the active browser page.

use rara_tool_macro::ToolDef;
use serde::Deserialize;

use crate::{
    browser::BrowserManagerRef,
    tool::{ToolContext, ToolOutput},
};

/// Click an element identified by its ref ID from the accessibility snapshot.
#[derive(ToolDef)]
#[tool(
    name = "browser-click",
    description = "Click an element on the page using its ref ID from the accessibility snapshot. \
                   Returns a fresh snapshot after clicking.",
    params_schema = "Self::schema()",
    execute_fn = "self.exec"
)]
pub struct BrowserClickTool {
    manager: BrowserManagerRef,
}

impl BrowserClickTool {
    pub fn new(manager: BrowserManagerRef) -> Self { Self { manager } }

    fn schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["ref"],
            "properties": {
                "ref": {
                    "type": "string",
                    "description": "The ref ID of the element to click (from the accessibility snapshot)"
                },
                "element": {
                    "type": "string",
                    "description": "Human-readable description of the element being clicked (for logging)"
                }
            }
        })
    }

    async fn exec(
        &self,
        params: serde_json::Value,
        _context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let p: Params =
            serde_json::from_value(params).map_err(|e| anyhow::anyhow!("invalid params: {e}"))?;

        let snapshot = self
            .manager
            .click(&p.r#ref)
            .await
            .map_err(|e| anyhow::anyhow!("click failed: {e}"))?;

        Ok(serde_json::json!({ "snapshot": snapshot }).into())
    }
}

#[derive(Debug, Deserialize)]
struct Params {
    r#ref:   String,
    #[serde(default)]
    element: Option<String>,
}
