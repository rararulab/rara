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

//! Retrieve browser console messages (stub — not yet implemented).

use async_trait::async_trait;
use rara_tool_macro::ToolDef;
use serde_json::Value;

use crate::tool::{EmptyParams, ToolContext, ToolExecute};

/// Retrieve console.log/warn/error messages from the browser. Stub — pending
/// Lightpanda support.
#[derive(ToolDef)]
#[tool(
    name = "browser-console-messages",
    description = "Retrieve console messages (log, warn, error) from the browser page."
)]
pub struct BrowserConsoleMessagesTool;

#[async_trait]
impl ToolExecute for BrowserConsoleMessagesTool {
    type Output = Value;
    type Params = EmptyParams;

    async fn run(&self, _p: EmptyParams, _context: &ToolContext) -> anyhow::Result<Value> {
        anyhow::bail!(
            "browser-console-messages is not yet implemented — will be added when Lightpanda \
             supports this feature"
        )
    }
}
