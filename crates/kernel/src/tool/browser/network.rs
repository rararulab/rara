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
use rara_tool_macro::ToolDef;
use serde_json::Value;

use crate::tool::{EmptyParams, ToolContext, ToolExecute};

/// Retrieve network requests made by the browser page. Stub — pending
/// Lightpanda support.
#[derive(ToolDef)]
#[tool(
    name = "browser-network-requests",
    description = "Retrieve network requests made by the browser page, including URLs, methods, \
                   and status codes.",
    tier = "deferred"
)]
pub struct BrowserNetworkRequestsTool;

#[async_trait]
impl ToolExecute for BrowserNetworkRequestsTool {
    type Output = Value;
    type Params = EmptyParams;

    async fn run(&self, _p: EmptyParams, _context: &ToolContext) -> anyhow::Result<Value> {
        anyhow::bail!(
            "browser-network-requests is not yet implemented — will be added when Lightpanda \
             supports this feature"
        )
    }
}
