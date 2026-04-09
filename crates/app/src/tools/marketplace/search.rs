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

//! Search for plugins by keyword across GitHub marketplaces or clawhub.ai.

use std::sync::Arc;

use async_trait::async_trait;
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_skills::{clawhub::ClawhubClient, marketplace::MarketplaceService};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

/// Parameters for the marketplace-search tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MarketplaceSearchParams {
    /// Search keyword to find plugins or skills by name/description.
    query:  String,
    /// Which source to search: "github" (default) or "clawhub".
    /// When "clawhub", searches clawhub.ai for matching skills.
    source: Option<String>,
    /// Maximum number of results (clawhub only, default: 20).
    limit:  Option<u64>,
}

/// Search for plugins or skills by keyword.
#[derive(ToolDef)]
#[tool(
    name = "marketplace-search",
    description = "Search for plugins and skills by keyword in GitHub marketplace indexes or \
                   clawhub.ai. Use `source: \"clawhub\"` to search clawhub.ai (supports `limit`), \
                   or omit `source` to search GitHub marketplace indexes.",
    tier = "deferred",
    read_only,
    concurrency_safe
)]
pub struct MarketplaceSearchTool {
    service: Arc<MarketplaceService>,
    clawhub: Arc<ClawhubClient>,
}

impl MarketplaceSearchTool {
    /// Create a new search tool with access to both marketplace service and
    /// clawhub client.
    pub fn new(service: Arc<MarketplaceService>, clawhub: Arc<ClawhubClient>) -> Self {
        Self { service, clawhub }
    }
}

#[async_trait]
impl ToolExecute for MarketplaceSearchTool {
    type Output = Value;
    type Params = MarketplaceSearchParams;

    #[tracing::instrument(skip_all)]
    async fn run(
        &self,
        params: MarketplaceSearchParams,
        _context: &ToolContext,
    ) -> anyhow::Result<Value> {
        if params.source.as_deref() == Some("clawhub") {
            let limit = params.limit.unwrap_or(20) as u32;
            let resp = self
                .clawhub
                .search(&params.query, limit)
                .await
                .map_err(anyhow::Error::from)?;
            let count = resp.results.len();
            Ok(
                json!({"source": "clawhub", "query": params.query, "results": resp.results, "count": count}),
            )
        } else {
            let plugins = self
                .service
                .search(&params.query)
                .await
                .map_err(anyhow::Error::from)?;
            let count = plugins.len();
            Ok(json!({"query": params.query, "results": plugins, "count": count}))
        }
    }
}
