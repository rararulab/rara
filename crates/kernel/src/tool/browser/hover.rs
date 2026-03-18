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

use async_trait::async_trait;
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::tool::{ToolContext, ToolExecute};

/// Hover over an element by ref ID. Stub — pending Lightpanda support.
#[derive(ToolDef)]
#[tool(
    name = "browser-hover",
    description = "Hover over an element on the page using its ref ID."
)]
pub struct BrowserHoverTool;

/// Parameters for the browser-hover tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BrowserHoverParams {
    /// The ref ID of the element to hover over
    r#ref:   String,
    /// Human-readable description of the element
    #[serde(default)]
    element: Option<String>,
}

#[async_trait]
impl ToolExecute for BrowserHoverTool {
    type Output = Value;
    type Params = BrowserHoverParams;

    async fn run(&self, _p: BrowserHoverParams, _context: &ToolContext) -> anyhow::Result<Value> {
        anyhow::bail!(
            "browser-hover is not yet implemented — will be added when Lightpanda supports this \
             feature"
        )
    }
}
