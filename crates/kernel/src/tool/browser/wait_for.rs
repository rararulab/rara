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
use serde::Deserialize;

use crate::{
    browser::BrowserManagerRef,
    tool::{AgentTool, ToolContext, ToolOutput},
};

/// Wait for text to appear, disappear, or for a time delay, then snapshot.
pub struct BrowserWaitForTool {
    manager: BrowserManagerRef,
}

impl BrowserWaitForTool {
    pub const NAME: &str = crate::tool_names::BROWSER_WAIT_FOR;

    pub fn new(manager: BrowserManagerRef) -> Self { Self { manager } }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Params {
    #[serde(default)]
    time:      Option<f64>,
    #[serde(default)]
    text:      Option<String>,
    #[serde(default)]
    text_gone: Option<String>,
}

#[async_trait]
impl AgentTool for BrowserWaitForTool {
    fn name(&self) -> &str { Self::NAME }

    fn description(&self) -> &str {
        "Wait for a condition before taking a snapshot. You can wait for text to appear, text to \
         disappear, or a fixed number of seconds."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "time": {
                    "type": "number",
                    "description": "Number of seconds to wait before taking the snapshot"
                },
                "text": {
                    "type": "string",
                    "description": "Wait until this text appears on the page"
                },
                "textGone": {
                    "type": "string",
                    "description": "Wait until this text disappears from the page"
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

        let snapshot = self
            .manager
            .wait_for(p.text.as_deref(), p.text_gone.as_deref(), p.time)
            .await
            .map_err(|e| anyhow::anyhow!("wait_for failed: {e}"))?;

        Ok(serde_json::json!({ "snapshot": snapshot }).into())
    }
}
