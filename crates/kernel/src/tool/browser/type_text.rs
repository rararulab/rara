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

//! Type text into an element in the active browser page.

use async_trait::async_trait;
use serde::Deserialize;

use crate::{
    browser::BrowserManagerRef,
    tool::{AgentTool, ToolContext, ToolOutput},
};

/// Type text into an input element identified by its ref ID.
pub struct BrowserTypeTool {
    manager: BrowserManagerRef,
}

impl BrowserTypeTool {
    pub const NAME: &str = crate::tool_names::BROWSER_TYPE;

    pub fn new(manager: BrowserManagerRef) -> Self { Self { manager } }
}

#[derive(Debug, Deserialize)]
struct Params {
    r#ref:   String,
    text:    String,
    #[serde(default)]
    submit:  bool,
    #[serde(default)]
    element: Option<String>,
}

#[async_trait]
impl AgentTool for BrowserTypeTool {
    fn name(&self) -> &str { Self::NAME }

    fn description(&self) -> &str {
        "Type text into an input element on the page. Optionally submit the form by pressing Enter \
         after typing."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["ref", "text"],
            "properties": {
                "ref": {
                    "type": "string",
                    "description": "The ref ID of the input element (from the accessibility snapshot)"
                },
                "text": {
                    "type": "string",
                    "description": "The text to type into the element"
                },
                "submit": {
                    "type": "boolean",
                    "description": "Whether to press Enter after typing to submit the form (default: false)"
                },
                "element": {
                    "type": "string",
                    "description": "Human-readable description of the element (for logging)"
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

        let snapshot = self
            .manager
            .type_text(&p.r#ref, &p.text, p.submit)
            .await
            .map_err(|e| anyhow::anyhow!("type_text failed: {e}"))?;

        Ok(serde_json::json!({ "snapshot": snapshot }).into())
    }
}
