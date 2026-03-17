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

use async_trait::async_trait;
use serde::Deserialize;

use crate::{
    browser::BrowserManagerRef,
    tool::{AgentTool, ToolContext, ToolOutput},
};

/// Press a keyboard key (e.g. Enter, Escape, ArrowDown) in the active page.
pub struct BrowserPressKeyTool {
    manager: BrowserManagerRef,
}

impl BrowserPressKeyTool {
    pub const NAME: &str = crate::tool_names::BROWSER_PRESS_KEY;

    pub fn new(manager: BrowserManagerRef) -> Self { Self { manager } }
}

#[derive(Debug, Deserialize)]
struct Params {
    key: String,
}

#[async_trait]
impl AgentTool for BrowserPressKeyTool {
    fn name(&self) -> &str { Self::NAME }

    fn description(&self) -> &str {
        "Press a keyboard key in the active browser page. Use key names like 'Enter', 'Escape', \
         'Tab', 'ArrowDown', etc."
    }

    fn parameters_schema(&self) -> serde_json::Value {
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

    async fn execute(
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
