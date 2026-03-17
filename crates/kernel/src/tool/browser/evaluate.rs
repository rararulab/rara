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

use async_trait::async_trait;
use serde::Deserialize;

use crate::{
    browser::BrowserManagerRef,
    tool::{AgentTool, ToolContext, ToolOutput},
};

/// Evaluate a JavaScript expression and return the result.
pub struct BrowserEvaluateTool {
    manager: BrowserManagerRef,
}

impl BrowserEvaluateTool {
    pub const NAME: &str = crate::tool_names::BROWSER_EVALUATE;

    pub fn new(manager: BrowserManagerRef) -> Self { Self { manager } }
}

#[derive(Debug, Deserialize)]
struct Params {
    expression: String,
}

#[async_trait]
impl AgentTool for BrowserEvaluateTool {
    fn name(&self) -> &str { Self::NAME }

    fn description(&self) -> &str {
        "Evaluate a JavaScript expression in the active browser page and return the result."
    }

    fn parameters_schema(&self) -> serde_json::Value {
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

    async fn execute(
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
