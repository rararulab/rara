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

//! Retrieve browser network requests (stub — not yet implemented).

use async_trait::async_trait;

use crate::tool::{AgentTool, ToolContext, ToolOutput};

/// Retrieve network requests made by the browser page. Stub — pending
/// Lightpanda support.
pub struct BrowserNetworkRequestsTool;

impl BrowserNetworkRequestsTool {
    pub const NAME: &str = crate::tool_names::BROWSER_NETWORK_REQUESTS;
}

#[async_trait]
impl AgentTool for BrowserNetworkRequestsTool {
    fn name(&self) -> &str { Self::NAME }

    fn description(&self) -> &str {
        "Retrieve network requests made by the browser page, including URLs, methods, and status \
         codes."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        _context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        anyhow::bail!(
            "browser-network-requests is not yet implemented — will be added when Lightpanda \
             supports this feature"
        )
    }
}
