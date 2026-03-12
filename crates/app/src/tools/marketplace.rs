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

//! Marketplace tool — browse, search, install, and manage plugins
//! from Claude Code marketplace repos through conversation.

use std::sync::Arc;

use async_trait::async_trait;
use rara_kernel::tool::{AgentTool, ToolContext, ToolOutput};
use rara_skills::clawhub::{ClawhubClient, ClawhubSort};
use rara_skills::marketplace::MarketplaceService;
use serde_json::{json, Value};

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
impl AgentTool for MarketplaceTool {
    fn name(&self) -> &str {
        "marketplace"
    }

    fn description(&self) -> &str {
        "Browse, search, install, enable/disable plugins from Claude Code marketplace repos \
         and ClawHub (clawhub.ai). Actions: browse, search, install, enable, disable, \
         add_source, refresh, clawhub_search, clawhub_browse, clawhub_install."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "browse", "search", "install", "enable", "disable",
                        "add_source", "refresh",
                        "clawhub_search", "clawhub_browse", "clawhub_install"
                    ],
                    "description": "The operation to perform"
                },
                "query": {
                    "type": "string",
                    "description": "Search query (for 'search' action)"
                },
                "plugin_name": {
                    "type": "string",
                    "description": "Plugin name (for install/enable/disable)"
                },
                "source": {
                    "type": "string",
                    "description": "GitHub owner/repo (for 'add_source' action)"
                },
                "marketplace": {
                    "type": "string",
                    "description": "Limit operation to a specific marketplace (optional)"
                },
                "sort": {
                    "type": "string",
                    "enum": ["trending", "updated", "downloads", "stars"],
                    "description": "Sort order for clawhub_browse (default: trending)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Max results for clawhub_search/clawhub_browse (default: 20)"
                },
                "slug": {
                    "type": "string",
                    "description": "Skill slug for clawhub_install"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(
        &self,
        params: Value,
        _context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: action"))?;

        let marketplace = params.get("marketplace").and_then(|v| v.as_str());

        match action {
            "browse" => {
                let plugins = self
                    .service
                    .browse(marketplace)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                let count = plugins.len();
                Ok(json!({
                    "plugins": plugins,
                    "count": count,
                })
                .into())
            }
            "search" => {
                let query = params
                    .get("query")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("'search' requires 'query' parameter"))?;
                let plugins = self
                    .service
                    .search(query)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                let count = plugins.len();
                Ok(json!({
                    "query": query,
                    "results": plugins,
                    "count": count,
                })
                .into())
            }
            "install" => {
                let name = params
                    .get("plugin_name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("'install' requires 'plugin_name' parameter"))?;
                let result = self
                    .service
                    .install_plugin(name, marketplace)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                Ok(json!(result).into())
            }
            "enable" => {
                let name = params
                    .get("plugin_name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("'enable' requires 'plugin_name' parameter"))?;
                self.service
                    .enable_plugin(name)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                Ok(json!({ "enabled": name }).into())
            }
            "disable" => {
                let name = params
                    .get("plugin_name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        anyhow::anyhow!("'disable' requires 'plugin_name' parameter")
                    })?;
                self.service
                    .disable_plugin(name)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                Ok(json!({ "disabled": name }).into())
            }
            "add_source" => {
                let source = params
                    .get("source")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        anyhow::anyhow!("'add_source' requires 'source' parameter")
                    })?;
                self.service
                    .add_source(source)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                Ok(json!({ "added": source }).into())
            }
            "refresh" => {
                match marketplace {
                    Some(name) => {
                        let sources = self.service.list_sources();
                        if let Some(src) =
                            sources.iter().find(|s| s.name == name || s.repo == name)
                        {
                            self.service.clear_cache_for(&src.repo);
                            let _ = self
                                .service
                                .fetch_index(&src.repo)
                                .await
                                .map_err(|e| anyhow::anyhow!("{e}"))?;
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
                Ok(json!({ "refreshed": true }).into())
            }
            "clawhub_search" => {
                let query = params
                    .get("query")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        anyhow::anyhow!("'clawhub_search' requires 'query' parameter")
                    })?;
                let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as u32;
                let resp = self
                    .clawhub
                    .search(query, limit)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                let count = resp.results.len();
                Ok(json!({
                    "source": "clawhub",
                    "query": query,
                    "results": resp.results,
                    "count": count,
                })
                .into())
            }
            "clawhub_browse" => {
                let sort = match params.get("sort").and_then(|v| v.as_str()) {
                    Some("updated") => ClawhubSort::Updated,
                    Some("downloads") => ClawhubSort::Downloads,
                    Some("stars") => ClawhubSort::Stars,
                    _ => ClawhubSort::Trending,
                };
                let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as u32;
                let resp = self
                    .clawhub
                    .browse(sort, limit)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                let count = resp.items.len();
                Ok(json!({
                    "source": "clawhub",
                    "skills": resp.items,
                    "count": count,
                })
                .into())
            }
            "clawhub_install" => {
                let slug = params
                    .get("slug")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("'clawhub_install' requires 'slug' parameter"))?;
                let install_dir =
                    rara_skills::install::default_install_dir().map_err(|e| anyhow::anyhow!("{e}"))?;
                let result = self
                    .clawhub
                    .install(slug, &install_dir)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                Ok(json!(result).into())
            }
            other => Err(anyhow::anyhow!("unknown action: {other}")),
        }
    }
}
