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
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    browser::BrowserManagerRef,
    tool::{ToolContext, ToolExecute},
};

/// Evaluate a JavaScript expression and return the result.
#[derive(ToolDef)]
#[tool(
    name = "browser-evaluate",
    description = "Evaluate a JavaScript expression in the active browser page and return the \
                   result."
)]
pub struct BrowserEvaluateTool {
    manager: BrowserManagerRef,
}

impl BrowserEvaluateTool {
    pub fn new(manager: BrowserManagerRef) -> Self { Self { manager } }
}

/// Parameters for the browser-evaluate tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BrowserEvaluateParams {
    /// The JavaScript expression to evaluate
    expression: String,
}

/// Result of the browser-evaluate tool.
#[derive(Debug, Clone, Serialize)]
pub struct BrowserEvaluateResult {
    /// The evaluation result
    result: Value,
}

#[async_trait]
impl ToolExecute for BrowserEvaluateTool {
    type Output = BrowserEvaluateResult;
    type Params = BrowserEvaluateParams;

    async fn run(
        &self,
        p: BrowserEvaluateParams,
        _context: &ToolContext,
    ) -> anyhow::Result<BrowserEvaluateResult> {
        let result = self
            .manager
            .evaluate(&p.expression)
            .await
            .map_err(|e| anyhow::anyhow!("evaluate failed: {e}"))?;

        Ok(BrowserEvaluateResult { result })
    }
}
