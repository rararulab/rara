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
use rara_browser::BrowserManagerRef;
use rara_tool_macro::ToolDef;
use serde::Serialize;

use crate::tool::{EmptyParams, ToolContext, ToolExecute};

/// Close all browser tabs and reset the browser state.
#[derive(ToolDef)]
#[tool(
    name = "browser-close",
    description = "Close all browser tabs and reset the browser state.",
    tier = "deferred"
)]
pub struct BrowserCloseTool {
    manager: BrowserManagerRef,
}

impl BrowserCloseTool {
    pub fn new(manager: BrowserManagerRef) -> Self { Self { manager } }
}

/// Result of the browser-close tool.
#[derive(Debug, Clone, Serialize)]
pub struct BrowserCloseResult {
    /// Remaining tabs (always empty after close-all)
    tabs: Vec<()>,
}

#[async_trait]
impl ToolExecute for BrowserCloseTool {
    type Output = BrowserCloseResult;
    type Params = EmptyParams;

    async fn run(
        &self,
        _p: EmptyParams,
        _context: &ToolContext,
    ) -> anyhow::Result<BrowserCloseResult> {
        self.manager
            .close_all()
            .await
            .map_err(|e| anyhow::anyhow!("close_all failed: {e}"))?;

        Ok(BrowserCloseResult { tabs: vec![] })
    }
}
