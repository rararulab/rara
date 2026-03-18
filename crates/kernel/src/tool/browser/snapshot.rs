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
use rara_tool_macro::ToolDef;
use serde::Serialize;

use crate::{
    browser::BrowserManagerRef,
    tool::{EmptyParams, ToolContext, ToolExecute},
};

/// Capture a fresh accessibility tree snapshot of the active browser page.
#[derive(ToolDef)]
#[tool(
    name = "browser-snapshot",
    description = "Take an accessibility snapshot of the current page without performing any \
                   action. Use this to inspect the page content after waiting or to refresh your \
                   view."
)]
pub struct BrowserSnapshotTool {
    manager: BrowserManagerRef,
}

impl BrowserSnapshotTool {
    pub fn new(manager: BrowserManagerRef) -> Self { Self { manager } }
}

/// Result of the browser-snapshot tool.
#[derive(Debug, Clone, Serialize)]
pub struct BrowserSnapshotResult {
    /// Accessibility tree snapshot text
    snapshot: String,
}

#[async_trait]
impl ToolExecute for BrowserSnapshotTool {
    type Output = BrowserSnapshotResult;
    type Params = EmptyParams;

    async fn run(
        &self,
        _p: EmptyParams,
        _context: &ToolContext,
    ) -> anyhow::Result<BrowserSnapshotResult> {
        let snapshot = self
            .manager
            .take_snapshot_active()
            .await
            .map_err(|e| anyhow::anyhow!("snapshot failed: {e}"))?;

        Ok(BrowserSnapshotResult { snapshot })
    }
}
