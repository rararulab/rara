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

//! Wait for a condition in the active browser page.

use async_trait::async_trait;
use rara_browser::BrowserManagerRef;
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::tool::{ToolContext, ToolExecute};

/// Wait for text to appear, disappear, or for a time delay, then snapshot.
#[derive(ToolDef)]
#[tool(
    name = "browser-wait-for",
    description = "Wait for a condition before taking a snapshot. You can wait for text to \
                   appear, text to disappear, or a fixed number of seconds.",
    tier = "deferred"
)]
pub struct BrowserWaitForTool {
    manager: BrowserManagerRef,
}

impl BrowserWaitForTool {
    pub fn new(manager: BrowserManagerRef) -> Self { Self { manager } }
}

/// Parameters for the browser-wait-for tool.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BrowserWaitForParams {
    /// Number of seconds to wait before taking the snapshot
    #[serde(default)]
    time:      Option<f64>,
    /// Wait until this text appears on the page
    #[serde(default)]
    text:      Option<String>,
    /// Wait until this text disappears from the page
    #[serde(default)]
    text_gone: Option<String>,
}

/// Result of the browser-wait-for tool.
#[derive(Debug, Clone, Serialize)]
pub struct BrowserWaitForResult {
    /// Accessibility tree snapshot after waiting
    snapshot: String,
}

#[async_trait]
impl ToolExecute for BrowserWaitForTool {
    type Output = BrowserWaitForResult;
    type Params = BrowserWaitForParams;

    async fn run(
        &self,
        p: BrowserWaitForParams,
        _context: &ToolContext,
    ) -> anyhow::Result<BrowserWaitForResult> {
        let snapshot = self
            .manager
            .wait_for(p.text.as_deref(), p.text_gone.as_deref(), p.time)
            .await
            .map_err(|e| anyhow::anyhow!("wait_for failed: {e}"))?;

        Ok(BrowserWaitForResult { snapshot })
    }
}
