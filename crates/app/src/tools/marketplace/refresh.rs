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

//! Refresh marketplace indexes by clearing caches and re-fetching.

use std::sync::Arc;

use async_trait::async_trait;
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_skills::marketplace::MarketplaceService;
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

/// Parameters for the marketplace-refresh tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MarketplaceRefreshParams {
    /// Optionally limit refresh to a specific marketplace by name or repo.
    /// When omitted, all registered marketplace sources are refreshed.
    marketplace: Option<String>,
}

/// Clear cached marketplace indexes and re-fetch from upstream.
#[derive(ToolDef)]
#[tool(
    name = "marketplace-refresh",
    description = "Clear cached marketplace indexes and re-fetch from upstream. Optionally pass \
                   `marketplace` to refresh only a specific source; otherwise all sources are \
                   refreshed.",
    tier = "deferred"
)]
pub struct MarketplaceRefreshTool {
    service: Arc<MarketplaceService>,
}

impl MarketplaceRefreshTool {
    /// Create a new refresh tool backed by the marketplace service.
    pub fn new(service: Arc<MarketplaceService>) -> Self { Self { service } }
}

#[async_trait]
impl ToolExecute for MarketplaceRefreshTool {
    type Output = Value;
    type Params = MarketplaceRefreshParams;

    #[tracing::instrument(skip_all)]
    async fn run(
        &self,
        params: MarketplaceRefreshParams,
        _context: &ToolContext,
    ) -> anyhow::Result<Value> {
        match params.marketplace.as_deref() {
            Some(name) => {
                let sources = self.service.list_sources();
                if let Some(src) = sources.iter().find(|s| s.name == name || s.repo == name) {
                    self.service.clear_cache_for(&src.repo);
                    let _ = self
                        .service
                        .fetch_index(&src.repo)
                        .await
                        .map_err(anyhow::Error::from)?;
                }
            }
            None => {
                self.service.clear_cache();
                let sources = self.service.list_sources();
                for src in &sources {
                    let _ = self.service.fetch_index(&src.repo).await;
                }
            }
        }
        Ok(json!({"refreshed": true}))
    }
}
