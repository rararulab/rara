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

//! Session info tool for querying current session metadata.

use rara_kernel::{
    session::SessionIndexRef,
    tool::{ToolContext, ToolOutput},
};
use rara_tool_macro::ToolDef;
use serde_json::json;

/// Agent tool that retrieves metadata for the current session.
#[derive(ToolDef)]
#[tool(
    name = "get-session-info",
    description = "Get metadata for the current session, including uploaded image paths and other \
                   session-specific information.",
    params_schema = "Self::schema()",
    execute_fn = "self.exec"
)]
pub struct SessionInfoTool {
    session_index: SessionIndexRef,
}

impl SessionInfoTool {
    pub fn new(session_index: SessionIndexRef) -> Self { Self { session_index } }

    fn schema() -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn exec(
        &self,
        _params: serde_json::Value,
        context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let session_key = &context.session_key;

        let entry = self
            .session_index
            .get_session(session_key)
            .await
            .map_err(|e| anyhow::anyhow!("failed to get session: {e}"))?
            .ok_or_else(|| anyhow::anyhow!("session not found"))?;

        Ok(json!({
            "session_key": entry.key.to_string(),
            "title": entry.title,
            "metadata": entry.metadata,
        })
        .into())
    }
}
