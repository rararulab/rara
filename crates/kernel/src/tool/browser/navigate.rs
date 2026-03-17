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
use serde::Deserialize;

use crate::{
    browser::BrowserManagerRef,
    tool::{AgentTool, ToolContext, ToolOutput},
};

/// Navigate to a URL, returning the page title and accessibility snapshot.
pub struct BrowserNavigateTool {
    manager: BrowserManagerRef,
}

impl BrowserNavigateTool {
    pub const NAME: &str = crate::tool_names::BROWSER_NAVIGATE;

    pub fn new(manager: BrowserManagerRef) -> Self { Self { manager } }
}

#[derive(Debug, Deserialize)]
struct Params {
    url: String,
}

#[async_trait]
impl AgentTool for BrowserNavigateTool {
    fn name(&self) -> &str { Self::NAME }

    fn description(&self) -> &str {
        "Navigate to a URL in the browser. Returns the page title and an accessibility snapshot of \
         the page content."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["url"],
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to navigate to"
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

        let result = self
            .manager
            .navigate(&p.url)
            .await
            .map_err(|e| anyhow::anyhow!("navigate failed: {e}"))?;

        Ok(serde_json::json!({
            "tab_id": result.tab_id,
            "url": result.url,
            "title": result.title,
            "snapshot": result.snapshot,
        })
        .into())
    }
}
