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
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    browser::BrowserManagerRef,
    tool::{ToolContext, ToolExecute},
};

/// Manage browser tabs: list, select, close, or create new tabs.
#[derive(ToolDef)]
#[tool(
    name = "browser-tabs",
    description = "Manage browser tabs. Actions: 'list' — list all tabs; 'select' — switch to a \
                   tab by index; 'close' — close a tab by index (or the active tab); 'new' — open \
                   a new blank tab."
)]
pub struct BrowserTabsTool {
    manager: BrowserManagerRef,
}

impl BrowserTabsTool {
    pub fn new(manager: BrowserManagerRef) -> Self { Self { manager } }
}

/// Parameters for the browser-tabs tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BrowserTabsParams {
    /// The tab action to perform: list, new, close, or select
    action: String,
    /// Tab index for 'select' or 'close' actions
    #[serde(default)]
    index:  Option<usize>,
}

/// Serializable tab information for tool output.
#[derive(Debug, Clone, Serialize)]
pub struct TabEntry {
    /// Tab position index
    index:     usize,
    /// Unique tab identifier
    tab_id:    String,
    /// Whether this is the currently active tab
    is_active: bool,
}

/// Result of the browser-tabs tool.
#[derive(Debug, Clone, Serialize)]
pub struct BrowserTabsResult {
    /// List of tabs after the action
    tabs:   Vec<TabEntry>,
    /// Tab ID of the newly created tab (only for 'new' action)
    #[serde(skip_serializing_if = "Option::is_none")]
    tab_id: Option<String>,
}

#[async_trait]
impl ToolExecute for BrowserTabsTool {
    type Output = BrowserTabsResult;
    type Params = BrowserTabsParams;

    async fn run(
        &self,
        p: BrowserTabsParams,
        _context: &ToolContext,
    ) -> anyhow::Result<BrowserTabsResult> {
        match p.action.as_str() {
            "list" => {
                let tabs = self.manager.list_tabs().await;
                Ok(BrowserTabsResult {
                    tabs:   to_entries(&tabs),
                    tab_id: None,
                })
            }
            "new" => {
                // Open about:blank as a new tab.
                let result = self
                    .manager
                    .navigate("about:blank")
                    .await
                    .map_err(|e| anyhow::anyhow!("new tab failed: {e}"))?;
                let tabs = self.manager.list_tabs().await;
                Ok(BrowserTabsResult {
                    tabs:   to_entries(&tabs),
                    tab_id: Some(result.tab_id),
                })
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
                Ok(BrowserTabsResult {
                    tabs:   to_entries(&tabs),
                    tab_id: None,
                })
            }
            "close" => {
                let tabs = self
                    .manager
                    .close_tab(p.index)
                    .await
                    .map_err(|e| anyhow::anyhow!("close_tab failed: {e}"))?;
                Ok(BrowserTabsResult {
                    tabs:   to_entries(&tabs),
                    tab_id: None,
                })
            }
            other => Err(anyhow::anyhow!(
                "unknown tab action '{other}'; expected one of: list, new, select, close"
            )),
        }
    }
}

/// Convert internal tab info to serializable entries.
fn to_entries(tabs: &[crate::browser::TabInfo]) -> Vec<TabEntry> {
    tabs.iter()
        .map(|t| TabEntry {
            index:     t.index,
            tab_id:    t.tab_id.clone(),
            is_active: t.is_active,
        })
        .collect()
}
