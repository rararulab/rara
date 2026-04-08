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

//! Type text into an element in the active browser page.

use async_trait::async_trait;
use rara_browser::BrowserManagerRef;
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::tool::{ToolContext, ToolExecute};

/// Type text into an input element identified by its ref ID.
#[derive(ToolDef)]
#[tool(
    name = "browser-type",
    description = "Type text into an input element on the page. Optionally submit the form by \
                   pressing Enter after typing.",
    tier = "deferred"
)]
pub struct BrowserTypeTool {
    manager: BrowserManagerRef,
}

impl BrowserTypeTool {
    pub fn new(manager: BrowserManagerRef) -> Self { Self { manager } }
}

/// Parameters for the browser-type tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BrowserTypeParams {
    /// The ref ID of the input element (from the accessibility snapshot)
    r#ref:   String,
    /// The text to type into the element
    text:    String,
    /// Whether to press Enter after typing to submit the form (default: false)
    #[serde(default)]
    submit:  bool,
    /// Human-readable description of the element (for logging)
    #[serde(default)]
    element: Option<String>,
}

/// Result of the browser-type tool.
#[derive(Debug, Clone, Serialize)]
pub struct BrowserTypeResult {
    /// Accessibility tree snapshot after typing
    snapshot: String,
}

#[async_trait]
impl ToolExecute for BrowserTypeTool {
    type Output = BrowserTypeResult;
    type Params = BrowserTypeParams;

    async fn run(
        &self,
        p: BrowserTypeParams,
        _context: &ToolContext,
    ) -> anyhow::Result<BrowserTypeResult> {
        let snapshot = self
            .manager
            .type_text(&p.r#ref, &p.text, p.submit)
            .await
            .map_err(|e| anyhow::anyhow!("type_text failed: {e}"))?;

        Ok(BrowserTypeResult { snapshot })
    }
}
