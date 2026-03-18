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

//! Evaluate a JavaScript expression in the active browser page.

use rara_tool_macro::ToolDef;
use serde::Deserialize;

use crate::{
    browser::BrowserManagerRef,
    tool::{ToolContext, ToolOutput},
};

/// Evaluate a JavaScript expression and return the result.
#[derive(ToolDef)]
#[tool(
    name = "browser-evaluate",
    description = "Evaluate a JavaScript expression in the active browser page and return the \
                   result.",
    params_schema = "Self::schema()",
    execute_fn = "self.exec"
)]
pub struct BrowserEvaluateTool {
    manager: BrowserManagerRef,
}

impl BrowserEvaluateTool {
    pub fn new(manager: BrowserManagerRef) -> Self { Self { manager } }

    fn schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["expression"],
            "properties": {
                "expression": {
                    "type": "string",
                    "description": "The JavaScript expression to evaluate"
                }
            }
        })
    }

    async fn exec(
        &self,
        params: serde_json::Value,
        _context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let p: Params =
            serde_json::from_value(params).map_err(|e| anyhow::anyhow!("invalid params: {e}"))?;

        let result = self
            .manager
            .evaluate(&p.expression)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate failed: {e}"))?;

        Ok(serde_json::json!({ "result": result }).into())
    }
}

#[derive(Debug, Deserialize)]
struct Params {
    expression: String,
}
