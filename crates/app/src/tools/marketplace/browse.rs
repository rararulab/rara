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

//! Browse available plugins from GitHub marketplaces or clawhub.ai.

use std::sync::Arc;

use async_trait::async_trait;
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_skills::{
    clawhub::{ClawhubClient, ClawhubSort},
    marketplace::MarketplaceService,
};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

/// Parameters for the marketplace-browse tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MarketplaceBrowseParams {
    /// Which source to browse: "github" (default) or "clawhub".
    /// When "clawhub", queries clawhub.ai for available skills.
    source:      Option<String>,
    /// Limit to a specific marketplace name (GitHub sources only).
    marketplace: Option<String>,
    /// Sort order when browsing clawhub: "trending" (default), "updated",
    /// "downloads", or "stars". Ignored for GitHub sources.
    sort:        Option<String>,
    /// Maximum number of results (clawhub only, default: 20).
    limit:       Option<u64>,
}

/// Browse available plugins and skills from GitHub marketplaces or clawhub.ai.
#[derive(ToolDef)]
#[tool(
    name = "marketplace-browse",
    description = "List available plugins and skills from GitHub marketplace sources or \
                   clawhub.ai. Use `source: \"clawhub\"` to browse clawhub.ai skills (supports \
                   `sort` and `limit`), or omit `source` to browse GitHub marketplace indexes.",
    tier = "deferred"
)]
pub struct MarketplaceBrowseTool {
    service: Arc<MarketplaceService>,
    clawhub: Arc<ClawhubClient>,
}

impl MarketplaceBrowseTool {
    /// Create a new browse tool with access to both marketplace service and
    /// clawhub client.
    pub fn new(service: Arc<MarketplaceService>, clawhub: Arc<ClawhubClient>) -> Self {
        Self { service, clawhub }
    }
}

#[async_trait]
impl ToolExecute for MarketplaceBrowseTool {
    type Output = Value;
    type Params = MarketplaceBrowseParams;

    #[tracing::instrument(skip_all)]
    async fn run(
        &self,
        params: MarketplaceBrowseParams,
        _context: &ToolContext,
    ) -> anyhow::Result<Value> {
        if params.source.as_deref() == Some("clawhub") {
            let sort = match params.sort.as_deref() {
                Some("updated") => ClawhubSort::Updated,
                Some("downloads") => ClawhubSort::Downloads,
                Some("stars") => ClawhubSort::Stars,
                _ => ClawhubSort::Trending,
            };
            let limit = params.limit.unwrap_or(20) as u32;
            let resp = self
                .clawhub
                .browse(sort, limit)
                .await
                .map_err(anyhow::Error::from)?;
            let count = resp.items.len();
            Ok(json!({"source": "clawhub", "skills": resp.items, "count": count}))
        } else {
            let plugins = self
                .service
                .browse(params.marketplace.as_deref())
                .await
                .map_err(anyhow::Error::from)?;
            let count = plugins.len();
            Ok(json!({"plugins": plugins, "count": count}))
        }
    }
}
