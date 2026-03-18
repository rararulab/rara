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

//! Drag an element (stub — not yet implemented).

use rara_tool_macro::ToolDef;

use crate::tool::{ToolContext, ToolOutput};

/// Drag an element from one position to another. Stub — pending Lightpanda
/// support.
#[derive(ToolDef)]
#[tool(
    name = "browser-drag",
    description = "Drag an element from one position to another on the page.",
    params_schema = "Self::schema()",
    execute_fn = "self.exec"
)]
pub struct BrowserDragTool;

impl BrowserDragTool {
    fn schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["startRef", "endRef"],
            "properties": {
                "startRef": {
                    "type": "string",
                    "description": "The ref ID of the element to drag from"
                },
                "endRef": {
                    "type": "string",
                    "description": "The ref ID of the element to drag to"
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
            "browser-drag is not yet implemented — will be added when Lightpanda supports this \
             feature"
        )
    }
}
