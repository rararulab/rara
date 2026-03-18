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

use async_trait::async_trait;
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    browser::BrowserManagerRef,
    tool::{ToolContext, ToolExecute},
};

/// Click an element identified by its ref ID from the accessibility snapshot.
#[derive(ToolDef)]
#[tool(
    name = "browser-click",
    description = "Click an element on the page using its ref ID from the accessibility snapshot. \
                   Returns a fresh snapshot after clicking."
)]
pub struct BrowserClickTool {
    manager: BrowserManagerRef,
}

impl BrowserClickTool {
    pub fn new(manager: BrowserManagerRef) -> Self { Self { manager } }
}

/// Parameters for the browser-click tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BrowserClickParams {
    /// The ref ID of the element to click (from the accessibility snapshot)
    r#ref:   String,
    /// Human-readable description of the element being clicked (for logging)
    #[serde(default)]
    element: Option<String>,
}

/// Result of the browser-click tool.
#[derive(Debug, Clone, Serialize)]
pub struct BrowserClickResult {
    /// Accessibility tree snapshot after clicking
    snapshot: String,
}

#[async_trait]
impl ToolExecute for BrowserClickTool {
    type Output = BrowserClickResult;
    type Params = BrowserClickParams;

    async fn run(
        &self,
        p: BrowserClickParams,
        _context: &ToolContext,
    ) -> anyhow::Result<BrowserClickResult> {
        let snapshot = self
            .manager
            .click(&p.r#ref)
            .await
            .map_err(|e| anyhow::anyhow!("click failed: {e}"))?;

        Ok(BrowserClickResult { snapshot })
    }
}
