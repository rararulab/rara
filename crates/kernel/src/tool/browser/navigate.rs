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

use rara_tool_macro::ToolDef;
use serde::Deserialize;

use crate::{
    browser::BrowserManagerRef,
    tool::{ToolContext, ToolOutput},
};

/// Navigate to a URL, returning the page title and accessibility snapshot.
#[derive(ToolDef)]
#[tool(
    name = "browser-navigate",
    description = "Navigate to a URL in the browser. Returns the page title and an accessibility \
                   snapshot of the page content.",
    params_schema = "Self::schema()",
    execute_fn = "self.exec"
)]
pub struct BrowserNavigateTool {
    manager: BrowserManagerRef,
}

impl BrowserNavigateTool {
    pub fn new(manager: BrowserManagerRef) -> Self { Self { manager } }

    fn schema() -> serde_json::Value {
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

    async fn exec(
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

#[derive(Debug, Deserialize)]
struct Params {
    url: String,
}
