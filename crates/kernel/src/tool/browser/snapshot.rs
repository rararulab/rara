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

//! Take an accessibility tree snapshot of the active page.

use async_trait::async_trait;

use crate::{
    browser::BrowserManagerRef,
    tool::{AgentTool, ToolContext, ToolOutput},
};

/// Capture a fresh accessibility tree snapshot of the active browser page.
pub struct BrowserSnapshotTool {
    manager: BrowserManagerRef,
}

impl BrowserSnapshotTool {
    pub const NAME: &str = crate::tool_names::BROWSER_SNAPSHOT;

    pub fn new(manager: BrowserManagerRef) -> Self { Self { manager } }
}

#[async_trait]
impl AgentTool for BrowserSnapshotTool {
    fn name(&self) -> &str { Self::NAME }

    fn description(&self) -> &str {
        "Take an accessibility snapshot of the current page without performing any action. Use \
         this to inspect the page content after waiting or to refresh your view."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        _context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let snapshot = self
            .manager
            .take_snapshot_active()
            .await
            .map_err(|e| anyhow::anyhow!("snapshot failed: {e}"))?;

        Ok(serde_json::json!({ "snapshot": snapshot }).into())
    }
}
