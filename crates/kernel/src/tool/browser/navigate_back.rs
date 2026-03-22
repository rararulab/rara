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
use rara_tool_macro::ToolDef;
use serde::Serialize;

use crate::{
    browser::BrowserManagerRef,
    tool::{EmptyParams, ToolContext, ToolExecute},
};

/// Navigate back in the active tab's history.
#[derive(ToolDef)]
#[tool(
    name = "browser-navigate-back",
    description = "Navigate back in the active browser tab. Returns a fresh accessibility \
                   snapshot.",
    tier = "deferred"
)]
pub struct BrowserNavigateBackTool {
    manager: BrowserManagerRef,
}

impl BrowserNavigateBackTool {
    pub fn new(manager: BrowserManagerRef) -> Self { Self { manager } }
}

/// Result of the browser-navigate-back tool.
#[derive(Debug, Clone, Serialize)]
pub struct BrowserNavigateBackResult {
    /// Accessibility tree snapshot after navigating back
    snapshot: String,
}

#[async_trait]
impl ToolExecute for BrowserNavigateBackTool {
    type Output = BrowserNavigateBackResult;
    type Params = EmptyParams;

    async fn run(
        &self,
        _p: EmptyParams,
        _context: &ToolContext,
    ) -> anyhow::Result<BrowserNavigateBackResult> {
        let snapshot = self
            .manager
            .navigate_back()
            .await
            .map_err(|e| anyhow::anyhow!("navigate_back failed: {e}"))?;

        Ok(BrowserNavigateBackResult { snapshot })
    }
}
