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

//! Press a keyboard key in the active browser page.

use rara_tool_macro::ToolDef;
use serde::Deserialize;

use crate::{
    browser::BrowserManagerRef,
    tool::{ToolContext, ToolOutput},
};

/// Press a keyboard key (e.g. Enter, Escape, ArrowDown) in the active page.
#[derive(ToolDef)]
#[tool(
    name = "browser-press-key",
    description = "Press a keyboard key in the active browser page. Use key names like 'Enter', \
                   'Escape', 'Tab', 'ArrowDown', etc.",
    params_schema = "Self::schema()",
    execute_fn = "self.exec"
)]
pub struct BrowserPressKeyTool {
    manager: BrowserManagerRef,
}

impl BrowserPressKeyTool {
    pub fn new(manager: BrowserManagerRef) -> Self { Self { manager } }

    fn schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["key"],
            "properties": {
                "key": {
                    "type": "string",
                    "description": "The key to press (e.g. 'Enter', 'Escape', 'Tab', 'ArrowDown', 'a')"
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

        self.manager
            .press_key(&p.key)
            .await
            .map_err(|e| anyhow::anyhow!("press_key failed: {e}"))?;

        Ok(serde_json::json!({ "status": "ok" }).into())
    }
}

#[derive(Debug, Deserialize)]
struct Params {
    key: String,
}
