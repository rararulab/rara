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

//! Marketplace tool -- browse, search, install, and manage plugins.

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

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MarketplaceParams {
    /// The operation to perform.
    action:      String,
    /// Search query (for 'search' action).
    query:       Option<String>,
    /// Plugin name (for install/enable/disable).
    plugin_name: Option<String>,
    /// GitHub owner/repo (for 'add_source' action).
    source:      Option<String>,
    /// Limit operation to a specific marketplace.
    marketplace: Option<String>,
    /// Sort order for clawhub_browse.
    sort:        Option<String>,
    /// Max results for clawhub_search/clawhub_browse.
    limit:       Option<u64>,
    /// Skill slug for clawhub_install.
    slug:        Option<String>,
}

#[derive(ToolDef)]
#[tool(
    name = "marketplace",
    description = "Browse, search, and install skills and plugins. Use install_repo to install a \
                   skill repo from GitHub (owner/repo or full URL). Use install to install a \
                   single plugin from a marketplace. Actions: browse, search, install, \
                   install_repo, enable, disable, add_source, remove_source, refresh, \
                   clawhub_search, clawhub_browse, clawhub_install."
)]
pub struct MarketplaceTool {
    service: Arc<MarketplaceService>,
    clawhub: Arc<ClawhubClient>,
}
impl MarketplaceTool {
    pub fn new(service: Arc<MarketplaceService>, clawhub: Arc<ClawhubClient>) -> Self {
        Self { service, clawhub }
    }
}

#[async_trait]
impl ToolExecute for MarketplaceTool {
    type Output = Value;
    type Params = MarketplaceParams;

    async fn run(
        &self,
        params: MarketplaceParams,
        _context: &ToolContext,
    ) -> anyhow::Result<Value> {
        let marketplace = params.marketplace.as_deref();
        match params.action.as_str() {
            "browse" => {
                let plugins = self
                    .service
                    .browse(marketplace)
                    .await
                    .map_err(anyhow::Error::from)?;
                let count = plugins.len();
                Ok(json!({"plugins": plugins, "count": count}))
            }
            "search" => {
                let query = params
                    .query
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("'search' requires 'query' parameter"))?;
                let plugins = self
                    .service
                    .search(query)
                    .await
                    .map_err(anyhow::Error::from)?;
                let count = plugins.len();
                Ok(json!({"query": query, "results": plugins, "count": count}))
            }
            "install" => {
                let name = params
                    .plugin_name
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("'install' requires 'plugin_name' parameter"))?;
                let result = self
                    .service
                    .install_plugin(name, marketplace)
                    .await
                    .map_err(anyhow::Error::from)?;
                Ok(json!(result))
            }
            "install_repo" => {
                let source = params.source.as_deref().ok_or_else(|| {
                    anyhow::anyhow!("'install_repo' requires 'source' parameter (owner/repo)")
                })?;
                let result = self
                    .service
                    .install_repo(source)
                    .await
                    .map_err(anyhow::Error::from)?;
                Ok(json!(result))
            }
            "enable" => {
                let name = params
                    .plugin_name
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("'enable' requires 'plugin_name' parameter"))?;
                self.service
                    .enable_plugin(name)
                    .map_err(anyhow::Error::from)?;
                Ok(json!({"enabled": name}))
            }
            "disable" => {
                let name = params
                    .plugin_name
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("'disable' requires 'plugin_name' parameter"))?;
                self.service
                    .disable_plugin(name)
                    .map_err(anyhow::Error::from)?;
                Ok(json!({"disabled": name}))
            }
            "add_source" => {
                let source = params
                    .source
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("'add_source' requires 'source' parameter"))?;
                self.service
                    .add_source(source)
                    .map_err(anyhow::Error::from)?;
                Ok(json!({"added": source}))
            }
            "refresh" => {
                match marketplace {
                    Some(name) => {
                        let sources = self.service.list_sources();
                        if let Some(src) = sources.iter().find(|s| s.name == name || s.repo == name)
                        {
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
            "clawhub_search" => {
                let query = params.query.as_deref().ok_or_else(|| {
                    anyhow::anyhow!("'clawhub_search' requires 'query' parameter")
                })?;
                let limit = params.limit.unwrap_or(20) as u32;
                let resp = self
                    .clawhub
                    .search(query, limit)
                    .await
                    .map_err(anyhow::Error::from)?;
                let count = resp.results.len();
                Ok(
                    json!({"source": "clawhub", "query": query, "results": resp.results, "count": count}),
                )
            }
            "clawhub_browse" => {
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
            }
            "clawhub_install" => {
                let slug = params.slug.as_deref().ok_or_else(|| {
                    anyhow::anyhow!("'clawhub_install' requires 'slug' parameter")
                })?;
                let install_dir =
                    rara_skills::install::default_install_dir().map_err(anyhow::Error::from)?;
                let result = self
                    .clawhub
                    .install(slug, &install_dir)
                    .await
                    .map_err(anyhow::Error::from)?;
                Ok(json!(result))
            }
            other => Err(anyhow::anyhow!("unknown action: {other}")),
        }
    }
}
