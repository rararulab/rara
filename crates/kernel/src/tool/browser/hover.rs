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

//! Hover over an element (stub — not yet implemented).

use rara_tool_macro::ToolDef;

use crate::tool::{ToolContext, ToolOutput};

/// Hover over an element by ref ID. Stub — pending Lightpanda support.
#[derive(ToolDef)]
#[tool(
    name = "browser-hover",
    description = "Hover over an element on the page using its ref ID.",
    params_schema = "Self::schema()",
    execute_fn = "self.exec"
)]
pub struct BrowserHoverTool;

impl BrowserHoverTool {
    fn schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["ref"],
            "properties": {
                "ref": {
                    "type": "string",
                    "description": "The ref ID of the element to hover over"
                },
                "element": {
                    "type": "string",
                    "description": "Human-readable description of the element"
                }
            }
        })
    }

    async fn exec(
        &self,
        _params: serde_json::Value,
        _context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        anyhow::bail!(
            "browser-hover is not yet implemented — will be added when Lightpanda supports this \
             feature"
        )
    }
}
