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

use async_trait::async_trait;
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::tool::{ToolContext, ToolExecute};

/// Drag an element from one position to another. Stub — pending Lightpanda
/// support.
#[derive(ToolDef)]
#[tool(
    name = "browser-drag",
    description = "Drag an element from one position to another on the page."
)]
pub struct BrowserDragTool;

/// Parameters for the browser-drag tool.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BrowserDragParams {
    /// The ref ID of the element to drag from
    start_ref: String,
    /// The ref ID of the element to drag to
    end_ref:   String,
}

#[async_trait]
impl ToolExecute for BrowserDragTool {
    type Output = Value;
    type Params = BrowserDragParams;

    async fn run(&self, _p: BrowserDragParams, _context: &ToolContext) -> anyhow::Result<Value> {
        anyhow::bail!(
            "browser-drag is not yet implemented — will be added when Lightpanda supports this \
             feature"
        )
    }
}
