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
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::tool::{ToolContext, ToolExecute};

/// Handle a browser dialog (alert, confirm, prompt). Stub — pending Lightpanda
/// support.
#[derive(ToolDef)]
#[tool(
    name = "browser-handle-dialog",
    description = "Handle a JavaScript dialog (alert, confirm, prompt) by accepting or dismissing \
                   it.",
    tier = "deferred"
)]
pub struct BrowserHandleDialogTool;

/// Parameters for the browser-handle-dialog tool.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BrowserHandleDialogParams {
    /// Whether to accept or dismiss the dialog
    action:      String,
    /// Text to enter in a prompt dialog before accepting
    #[serde(default)]
    prompt_text: Option<String>,
}

#[async_trait]
impl ToolExecute for BrowserHandleDialogTool {
    type Output = Value;
    type Params = BrowserHandleDialogParams;

    async fn run(
        &self,
        _p: BrowserHandleDialogParams,
        _context: &ToolContext,
    ) -> anyhow::Result<Value> {
        anyhow::bail!(
            "browser-handle-dialog is not yet implemented — will be added when Lightpanda \
             supports this feature"
        )
    }
}
