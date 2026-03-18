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
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    browser::BrowserManagerRef,
    tool::{ToolContext, ToolExecute},
};

/// Press a keyboard key (e.g. Enter, Escape, ArrowDown) in the active page.
#[derive(ToolDef)]
#[tool(
    name = "browser-press-key",
    description = "Press a keyboard key in the active browser page. Use key names like 'Enter', \
                   'Escape', 'Tab', 'ArrowDown', etc."
)]
pub struct BrowserPressKeyTool {
    manager: BrowserManagerRef,
}

impl BrowserPressKeyTool {
    pub fn new(manager: BrowserManagerRef) -> Self { Self { manager } }
}

/// Parameters for the browser-press-key tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BrowserPressKeyParams {
    /// The key to press (e.g. 'Enter', 'Escape', 'Tab', 'ArrowDown', 'a')
    key: String,
}

/// Result of the browser-press-key tool.
#[derive(Debug, Clone, Serialize)]
pub struct BrowserPressKeyResult {
    /// Status indicator
    status: String,
}

#[async_trait]
impl ToolExecute for BrowserPressKeyTool {
    type Output = BrowserPressKeyResult;
    type Params = BrowserPressKeyParams;

    async fn run(
        &self,
        p: BrowserPressKeyParams,
        _context: &ToolContext,
    ) -> anyhow::Result<BrowserPressKeyResult> {
        self.manager
            .press_key(&p.key)
            .await
            .map_err(|e| anyhow::anyhow!("press_key failed: {e}"))?;

        Ok(BrowserPressKeyResult {
            status: "ok".to_string(),
        })
    }
}
