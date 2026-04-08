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

//! Navigate to a URL in the browser.

use async_trait::async_trait;
use rara_browser::BrowserManagerRef;
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::tool::{ToolContext, ToolExecute};

/// Navigate to a URL, returning the page title and accessibility snapshot.
#[derive(ToolDef)]
#[tool(
    name = "browser-navigate",
    description = "Navigate to a URL in the browser. Returns the page title and an accessibility \
                   snapshot of the page content.",
    tier = "deferred"
)]
pub struct BrowserNavigateTool {
    manager: BrowserManagerRef,
}

impl BrowserNavigateTool {
    pub fn new(manager: BrowserManagerRef) -> Self { Self { manager } }
}

/// Parameters for the browser-navigate tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BrowserNavigateParams {
    /// The URL to navigate to
    url: String,
}

/// Result of the browser-navigate tool.
#[derive(Debug, Clone, Serialize)]
pub struct BrowserNavigateResult {
    /// Unique tab identifier
    tab_id:   String,
    /// The URL after navigation (may differ from requested due to redirects)
    url:      String,
    /// Page title
    title:    String,
    /// Accessibility tree snapshot text
    snapshot: String,
}

#[async_trait]
impl ToolExecute for BrowserNavigateTool {
    type Output = BrowserNavigateResult;
    type Params = BrowserNavigateParams;

    async fn run(
        &self,
        p: BrowserNavigateParams,
        _context: &ToolContext,
    ) -> anyhow::Result<BrowserNavigateResult> {
        let result = self
            .manager
            .navigate(&p.url)
            .await
            .map_err(|e| anyhow::anyhow!("navigate failed: {e}"))?;

        Ok(BrowserNavigateResult {
            tab_id:   result.tab_id,
            url:      result.url,
            title:    result.title,
            snapshot: result.snapshot,
        })
    }
}
