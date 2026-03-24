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
    /// Which operation to perform. Valid actions and their required parameters:
    ///
    /// - `browse` — list all available plugins (optional: `marketplace`)
    /// - `search` — search plugins by keyword (required: `query`)
    /// - `install` — install a single plugin from a marketplace (required:
    ///   `plugin_name`, optional: `marketplace`)
    /// - `install_repo` — install all skills from a GitHub repo (required:
    ///   `source` as "owner/repo")
    /// - `enable` — enable an installed plugin (required: `plugin_name`)
    /// - `disable` — disable an installed plugin (required: `plugin_name`)
    /// - `add_source` — register a new marketplace source (required: `source`
    ///   as "owner/repo")
    /// - `refresh` — re-fetch marketplace indexes (optional: `marketplace`)
    /// - `clawhub_search` — search skills on clawhub.ai (required: `query`,
    ///   optional: `limit`)
    /// - `clawhub_browse` — browse skills on clawhub.ai (optional: `sort`,
    ///   `limit`)
    /// - `clawhub_install` — install a skill from clawhub.ai (required: `slug`)
    action:      String,
    /// Search keyword. Required for `search` and `clawhub_search` actions.
    query:       Option<String>,
    /// Plugin name. Required for `install`, `enable`, and `disable` actions.
    plugin_name: Option<String>,
    /// GitHub owner/repo (e.g. "anthropics/skills"). Required for
    /// `install_repo` and `add_source` actions.
    source:      Option<String>,
    /// Limit operation to a specific marketplace name. Optional for `browse`,
    /// `install`, and `refresh`.
    marketplace: Option<String>,
    /// Sort order for `clawhub_browse`: "trending" (default), "updated",
    /// "downloads", or "stars".
    sort:        Option<String>,
    /// Maximum number of results. Optional for `clawhub_search` and
    /// `clawhub_browse` (default: 20).
    limit:       Option<u64>,
    /// Skill identifier on clawhub.ai. Required for `clawhub_install`.
    slug:        Option<String>,
}

#[derive(ToolDef)]
#[tool(
    name = "marketplace",
    description = "Manage skills and plugins from GitHub marketplaces and clawhub.ai. Key \
                   workflows: `browse` to see what is available, `search` by keyword, `install` a \
                   plugin by name, `install_repo` from GitHub (owner/repo), `clawhub_browse` / \
                   `clawhub_search` / `clawhub_install` for clawhub.ai. Also: `enable`, \
                   `disable`, `add_source`, `refresh`. See the `action` parameter description for \
                   full details.",
    tier = "deferred"
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
