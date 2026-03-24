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

//! Register a new GitHub marketplace source.

use std::sync::Arc;

use async_trait::async_trait;
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_skills::marketplace::MarketplaceService;
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

/// Parameters for the marketplace-add-source tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MarketplaceAddSourceParams {
    /// GitHub "owner/repo" of the marketplace index to register
    /// (e.g. "anthropics/skills").
    repo: String,
}

/// Register a new GitHub repository as a marketplace source.
#[derive(ToolDef)]
#[tool(
    name = "marketplace-add-source",
    description = "Register a new GitHub repository as a marketplace plugin source. Pass the \
                   `repo` as \"owner/repo\" (e.g. \"anthropics/skills\"). After adding, use \
                   marketplace-refresh to fetch the index.",
    tier = "deferred"
)]
pub struct MarketplaceAddSourceTool {
    service: Arc<MarketplaceService>,
}

impl MarketplaceAddSourceTool {
    /// Create a new add-source tool backed by the marketplace service.
    pub fn new(service: Arc<MarketplaceService>) -> Self { Self { service } }
}

#[async_trait]
impl ToolExecute for MarketplaceAddSourceTool {
    type Output = Value;
    type Params = MarketplaceAddSourceParams;

    #[tracing::instrument(skip_all)]
    async fn run(
        &self,
        params: MarketplaceAddSourceParams,
        _context: &ToolContext,
    ) -> anyhow::Result<Value> {
        self.service
            .add_source(&params.repo)
            .map_err(anyhow::Error::from)?;
        Ok(json!({"added": params.repo}))
    }
}
