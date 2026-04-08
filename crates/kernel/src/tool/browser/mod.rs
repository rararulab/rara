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

//! Browser tools — LLM-callable tools for controlling the headless browser.
//!
//! Each tool wraps a [`BrowserManagerRef`] and delegates to its methods.

mod click;
mod close;
mod evaluate;
mod fetch;
mod navigate;
mod navigate_back;
mod press_key;
mod snapshot;
mod tabs;
mod type_text;
mod wait_for;

use rara_browser::BrowserManagerRef;

use crate::tool::AgentToolRef;

/// Create all browser tools backed by the given browser manager.
pub fn browser_tools(manager: BrowserManagerRef) -> Vec<AgentToolRef> {
    use std::sync::Arc;
    vec![
        Arc::new(fetch::BrowserFetchTool::new(manager.clone())),
        Arc::new(navigate::BrowserNavigateTool::new(manager.clone())),
        Arc::new(navigate_back::BrowserNavigateBackTool::new(manager.clone())),
        Arc::new(snapshot::BrowserSnapshotTool::new(manager.clone())),
        Arc::new(click::BrowserClickTool::new(manager.clone())),
        Arc::new(type_text::BrowserTypeTool::new(manager.clone())),
        Arc::new(press_key::BrowserPressKeyTool::new(manager.clone())),
        Arc::new(evaluate::BrowserEvaluateTool::new(manager.clone())),
        Arc::new(wait_for::BrowserWaitForTool::new(manager.clone())),
        Arc::new(tabs::BrowserTabsTool::new(manager.clone())),
        Arc::new(close::BrowserCloseTool::new(manager.clone())),
    ]
}
