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

//! Select an option from a dropdown (stub — not yet implemented).

use async_trait::async_trait;

use crate::tool::{AgentTool, ToolContext, ToolOutput};

/// Select an option from a dropdown element. Stub — pending Lightpanda support.
pub struct BrowserSelectOptionTool;

impl BrowserSelectOptionTool {
    pub const NAME: &str = crate::tool_names::BROWSER_SELECT_OPTION;
}

#[async_trait]
impl AgentTool for BrowserSelectOptionTool {
    fn name(&self) -> &str { Self::NAME }

    fn description(&self) -> &str {
        "Select an option from a dropdown (select) element on the page."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["ref", "values"],
            "properties": {
                "ref": {
                    "type": "string",
                    "description": "The ref ID of the select element"
                },
                "values": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "The option values to select"
                }
            }
        })
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        _context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        anyhow::bail!(
            "browser-select-option is not yet implemented — will be added when Lightpanda \
             supports this feature"
        )
    }
}
