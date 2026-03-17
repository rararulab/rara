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

//! Manage browser tabs — list, select, close, or create new tabs.

use async_trait::async_trait;
use serde::Deserialize;

use crate::{
    browser::BrowserManagerRef,
    tool::{AgentTool, ToolContext, ToolOutput},
};

/// Manage browser tabs: list, select, close, or create new tabs.
pub struct BrowserTabsTool {
    manager: BrowserManagerRef,
}

impl BrowserTabsTool {
    pub const NAME: &str = crate::tool_names::BROWSER_TABS;

    pub fn new(manager: BrowserManagerRef) -> Self { Self { manager } }
}

#[derive(Debug, Deserialize)]
struct Params {
    action: String,
    #[serde(default)]
    index:  Option<usize>,
}

/// Serialize a list of tabs into JSON.
fn tabs_json(tabs: &[crate::browser::TabInfo]) -> Vec<serde_json::Value> {
    tabs.iter()
        .map(|t| {
            serde_json::json!({
                "index": t.index,
                "tab_id": t.tab_id,
                "is_active": t.is_active,
            })
        })
        .collect()
}

#[async_trait]
impl AgentTool for BrowserTabsTool {
    fn name(&self) -> &str { Self::NAME }

    fn description(&self) -> &str {
        "Manage browser tabs. Actions: 'list' — list all tabs; 'select' — switch to a tab by \
         index; 'close' — close a tab by index (or the active tab); 'new' — open a new blank tab."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["action"],
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "new", "close", "select"],
                    "description": "The tab action to perform"
                },
                "index": {
                    "type": "integer",
                    "description": "Tab index for 'select' or 'close' actions"
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

        match p.action.as_str() {
            "list" => {
                let tabs = self.manager.list_tabs().await;
                Ok(serde_json::json!({ "tabs": tabs_json(&tabs) }).into())
            }
            "new" => {
                // Open about:blank as a new tab.
                let result = self
                    .manager
                    .navigate("about:blank")
                    .await
                    .map_err(|e| anyhow::anyhow!("new tab failed: {e}"))?;
                let tabs = self.manager.list_tabs().await;
                Ok(serde_json::json!({
                    "tab_id": result.tab_id,
                    "tabs": tabs_json(&tabs),
                })
                .into())
            }
            "select" => {
                let index = p.index.ok_or_else(|| {
                    anyhow::anyhow!("'index' is required for the 'select' action")
                })?;
                self.manager
                    .select_tab(index)
                    .await
                    .map_err(|e| anyhow::anyhow!("select_tab failed: {e}"))?;
                let tabs = self.manager.list_tabs().await;
                Ok(serde_json::json!({ "tabs": tabs_json(&tabs) }).into())
            }
            "close" => {
                let tabs = self
                    .manager
                    .close_tab(p.index)
                    .await
                    .map_err(|e| anyhow::anyhow!("close_tab failed: {e}"))?;
                Ok(serde_json::json!({ "tabs": tabs_json(&tabs) }).into())
            }
            other => Err(anyhow::anyhow!(
                "unknown tab action '{other}'; expected one of: list, new, select, close"
            )),
        }
    }
}
