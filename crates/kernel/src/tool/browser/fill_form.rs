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

//! Fill a form with multiple values (stub — not yet implemented).

use std::collections::HashMap;

use async_trait::async_trait;
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::tool::{ToolContext, ToolExecute};

/// Fill multiple form fields at once. Stub — pending Lightpanda support.
#[derive(ToolDef)]
#[tool(
    name = "browser-fill-form",
    description = "Fill multiple form fields at once by providing a mapping of ref IDs to values."
)]
pub struct BrowserFillFormTool;

/// Parameters for the browser-fill-form tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BrowserFillFormParams {
    /// A mapping of ref IDs to values to fill in
    fields: HashMap<String, String>,
    /// Whether to submit the form after filling (default: false)
    #[serde(default)]
    submit: bool,
}

#[async_trait]
impl ToolExecute for BrowserFillFormTool {
    type Output = Value;
    type Params = BrowserFillFormParams;

    async fn run(
        &self,
        _p: BrowserFillFormParams,
        _context: &ToolContext,
    ) -> anyhow::Result<Value> {
        anyhow::bail!(
            "browser-fill-form is not yet implemented — will be added when Lightpanda supports \
             this feature"
        )
    }
}
