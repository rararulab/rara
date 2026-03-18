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
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::tool::{ToolContext, ToolExecute};

/// Select an option from a dropdown element. Stub — pending Lightpanda support.
#[derive(ToolDef)]
#[tool(
    name = "browser-select-option",
    description = "Select an option from a dropdown (select) element on the page."
)]
pub struct BrowserSelectOptionTool;

/// Parameters for the browser-select-option tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BrowserSelectOptionParams {
    /// The ref ID of the select element
    r#ref:  String,
    /// The option values to select
    values: Vec<String>,
}

#[async_trait]
impl ToolExecute for BrowserSelectOptionTool {
    type Output = Value;
    type Params = BrowserSelectOptionParams;

    async fn run(
        &self,
        _p: BrowserSelectOptionParams,
        _context: &ToolContext,
    ) -> anyhow::Result<Value> {
        anyhow::bail!(
            "browser-select-option is not yet implemented — will be added when Lightpanda \
             supports this feature"
        )
    }
}
