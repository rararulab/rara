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

//! Fetch a URL and return its content as Markdown via Lightpanda.

use async_trait::async_trait;
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    browser::BrowserManagerRef,
    tool::{ToolContext, ToolExecute},
};

/// Fetch a URL and return its rendered Markdown content via Lightpanda.
#[derive(ToolDef)]
#[tool(
    name = "browser-fetch",
    description = "Fetch a URL and return its content as clean Markdown. Executes JavaScript so \
                   dynamic and SPA pages render correctly. Preferred over `http-fetch` for \
                   human-readable web pages. Use `browser-navigate` instead when you need to \
                   interact with the page (click, type, etc.).",
    tier = "deferred"
)]
pub struct BrowserFetchTool {
    manager: BrowserManagerRef,
}

impl BrowserFetchTool {
    pub fn new(manager: BrowserManagerRef) -> Self { Self { manager } }
}

/// Parameters for the browser-fetch tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BrowserFetchParams {
    /// The URL to fetch
    url: String,
}

/// Result of the browser-fetch tool.
#[derive(Debug, Clone, Serialize)]
pub struct BrowserFetchResult {
    /// The URL that was fetched
    url:      String,
    /// Page content rendered as Markdown
    markdown: String,
}

#[async_trait]
impl ToolExecute for BrowserFetchTool {
    type Output = BrowserFetchResult;
    type Params = BrowserFetchParams;

    async fn run(
        &self,
        p: BrowserFetchParams,
        _context: &ToolContext,
    ) -> anyhow::Result<BrowserFetchResult> {
        let markdown = self
            .manager
            .fetch_markdown(&p.url)
            .await
            .map_err(|e| anyhow::anyhow!("browser-fetch failed: {e}"))?;

        Ok(BrowserFetchResult {
            url: p.url,
            markdown,
        })
    }
}
