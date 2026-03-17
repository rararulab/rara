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

use crate::tool::{AgentTool, ToolContext, ToolOutput};

/// Retrieve console.log/warn/error messages from the browser. Stub — pending
/// Lightpanda support.
pub struct BrowserConsoleMessagesTool;

impl BrowserConsoleMessagesTool {
    pub const NAME: &str = crate::tool_names::BROWSER_CONSOLE_MESSAGES;
}

#[async_trait]
impl AgentTool for BrowserConsoleMessagesTool {
    fn name(&self) -> &str { Self::NAME }

    fn description(&self) -> &str {
        "Retrieve console messages (log, warn, error) from the browser page."
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
            "browser-console-messages is not yet implemented — will be added when Lightpanda \
             supports this feature"
        )
    }
}
