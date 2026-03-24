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

//! Uninstall a previously installed plugin.

use std::sync::Arc;

use async_trait::async_trait;
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_skills::marketplace::MarketplaceService;
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

/// Parameters for the marketplace-uninstall tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MarketplaceUninstallParams {
    /// Name of the installed plugin to remove.
    plugin_name: String,
}

/// Uninstall a previously installed plugin by name.
#[derive(ToolDef)]
#[tool(
    name = "marketplace-uninstall",
    description = "Uninstall a previously installed plugin by name. The plugin files are removed \
                   from the local installation directory.",
    tier = "deferred"
)]
pub struct MarketplaceUninstallTool {
    service: Arc<MarketplaceService>,
}

impl MarketplaceUninstallTool {
    /// Create a new uninstall tool backed by the marketplace service.
    pub fn new(service: Arc<MarketplaceService>) -> Self { Self { service } }
}

#[async_trait]
impl ToolExecute for MarketplaceUninstallTool {
    type Output = Value;
    type Params = MarketplaceUninstallParams;

    #[tracing::instrument(skip_all)]
    async fn run(
        &self,
        params: MarketplaceUninstallParams,
        _context: &ToolContext,
    ) -> anyhow::Result<Value> {
        self.service
            .uninstall_plugin(&params.plugin_name)
            .await
            .map_err(anyhow::Error::from)?;
        Ok(json!({"uninstalled": params.plugin_name}))
    }
}
