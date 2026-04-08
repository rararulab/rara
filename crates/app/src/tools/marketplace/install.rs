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

//! Install plugins from GitHub marketplaces or clawhub.ai.

use std::sync::Arc;

use async_trait::async_trait;
use rara_kernel::tool::{ToolContext, ToolExecute, ToolHint};
use rara_skills::{clawhub::ClawhubClient, marketplace::MarketplaceService};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

/// Parameters for the marketplace-install tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MarketplaceInstallParams {
    /// Name of a plugin from a GitHub marketplace index. Use with `marketplace`
    /// to specify which source.
    plugin_name: Option<String>,
    /// GitHub "owner/repo" to install all skills from a repository directly.
    repo:        Option<String>,
    /// Clawhub.ai skill slug to install (e.g. "author/skill-name").
    slug:        Option<String>,
    /// Limit to a specific marketplace name when installing by `plugin_name`.
    marketplace: Option<String>,
}

/// Install a plugin or skill from a GitHub marketplace, a GitHub repo, or
/// clawhub.ai.
#[derive(ToolDef)]
#[tool(
    name = "marketplace-install",
    description = "Install a plugin or skill. Provide exactly one of: `slug` (clawhub.ai skill), \
                   `repo` (GitHub owner/repo), or `plugin_name` (from a marketplace index). Use \
                   `marketplace` to narrow by source name when using `plugin_name`.",
    tier = "deferred"
)]
pub struct MarketplaceInstallTool {
    service: Arc<MarketplaceService>,
    clawhub: Arc<ClawhubClient>,
}

impl MarketplaceInstallTool {
    /// Create a new install tool with access to both marketplace service and
    /// clawhub client.
    pub fn new(service: Arc<MarketplaceService>, clawhub: Arc<ClawhubClient>) -> Self {
        Self { service, clawhub }
    }
}

#[async_trait]
impl ToolExecute for MarketplaceInstallTool {
    type Output = Value;
    type Params = MarketplaceInstallParams;

    /// Installation produces large output (download logs, skill listings);
    /// suggest context folding so subsequent turns aren't polluted.
    fn hints(&self) -> Vec<ToolHint> {
        vec![ToolHint::SuggestFold {
            reason: Some("marketplace install produces large output".into()),
        }]
    }

    #[tracing::instrument(skip_all)]
    async fn run(
        &self,
        params: MarketplaceInstallParams,
        _context: &ToolContext,
    ) -> anyhow::Result<Value> {
        if let Some(slug) = &params.slug {
            let install_dir =
                rara_skills::install::default_install_dir().map_err(anyhow::Error::from)?;
            let result = self
                .clawhub
                .install(slug, &install_dir)
                .await
                .map_err(anyhow::Error::from)?;
            Ok(json!(result))
        } else if let Some(repo) = &params.repo {
            let result = self
                .service
                .install_repo(repo)
                .await
                .map_err(anyhow::Error::from)?;
            Ok(json!(result))
        } else if let Some(name) = &params.plugin_name {
            let result = self
                .service
                .install_plugin(name, params.marketplace.as_deref())
                .await
                .map_err(anyhow::Error::from)?;
            Ok(json!(result))
        } else {
            Err(anyhow::anyhow!(
                "provide one of: `slug` (clawhub), `repo` (GitHub owner/repo), or `plugin_name`"
            ))
        }
    }
}
