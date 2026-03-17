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

//! Handle a browser dialog (stub — not yet implemented).

use async_trait::async_trait;

use crate::tool::{AgentTool, ToolContext, ToolOutput};

/// Handle a browser dialog (alert, confirm, prompt). Stub — pending Lightpanda
/// support.
pub struct BrowserHandleDialogTool;

impl BrowserHandleDialogTool {
    pub const NAME: &str = crate::tool_names::BROWSER_HANDLE_DIALOG;
}

#[async_trait]
impl AgentTool for BrowserHandleDialogTool {
    fn name(&self) -> &str { Self::NAME }

    fn description(&self) -> &str {
        "Handle a JavaScript dialog (alert, confirm, prompt) by accepting or dismissing it."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["action"],
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["accept", "dismiss"],
                    "description": "Whether to accept or dismiss the dialog"
                },
                "promptText": {
                    "type": "string",
                    "description": "Text to enter in a prompt dialog before accepting"
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
            "browser-handle-dialog is not yet implemented — will be added when Lightpanda \
             supports this feature"
        )
    }
}
